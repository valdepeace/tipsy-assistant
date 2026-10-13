//! The native terminal client: keys and mouse in, HUD out, audio played and captured here.
//!
//! The client owns everything the protocol cannot carry: the terminal, the output device and the
//! microphone. It holds no conversation state beyond what it draws.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    self, EnableMouseCapture, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui::crossterm::execute;
use tipsy_protocol::{
    Capability, ClientConfigView, ClientKind, Command, Event, Frame, PROTOCOL_VERSION, ToClient, ToServer,
};
use tipsy_ui::policy::{
    MicRoute, flush_on_interrupt, hit_at, interrupts_playback, mic_route, speech_rate, starts_turn, takes_the_floor,
    test_interrupts_playback,
};
use tipsy_ui::{
    ClockView, ConfigEffect, ConfigForm, Hit, KeyInput, LoginLine, LoginPhase, NoticeKind, UiAction, UiState, draw,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::audio::device::{Mic, Player};
use crate::audio::pcm::{envelope, i16_to_f32, rms};
use crate::config::Config;
use crate::config::form::{config_from, form_of, provider_config};
use crate::config::store;
use crate::keys::{KeyResolver, env_var_name};
use crate::providers::{ProviderSlot, build_one, build_providers};
use crate::realtime::{Call, CallSetup, Heard};
use crate::server::Hub;
use crate::session::{SessionEnds, SessionPolicy};
use crate::sysmon;

pub mod audio;
/// The client's own half of the socket bridge: the `--connect` path's three channel ends.
pub mod bridge;
pub mod call;
pub mod mic;
/// The pseudo-terminal a web terminal's child runs in. Nothing in this file uses it: `serve`'s
/// `/terminal/ws` route owns it, and it is here because `client` is the parent module.
pub mod pty;
pub mod settings;

use call::call_ended;

/// One frame of the HUD.
const FRAME: Duration = Duration::from_millis(33);
/// Microphone frames that may wait for the client before the oldest are dropped.
const MIC_QUEUE: usize = 16;
/// Keys handled in one frame. Pasting a wall of text should not stall the HUD.
const MAX_INPUT_PER_FRAME: usize = 64;

/// Runs the terminal client until the user quits.
pub fn run(config_path: PathBuf) -> Result<(), String> {
    let loaded = store::load_or_create(&config_path).map_err(|e| format!("{}: {e}", config_path.display()))?;
    let config = loaded.config;
    let keys = KeyResolver::system();
    // One stack, shared with the session from here on: a save swaps the connectors inside it.
    let providers = ProviderSlot::new(build_providers(&config.providers, &keys).map_err(|e| e.to_string())?);
    let mut client = Client::new(config, config_path, keys, providers);
    // The in-process client's own hub: no listener, one session, and a save from the screen still
    // reaches it through the same three commands the server uses. It is built here and held for the
    // whole of `terminal`, because a hub is its registry of live sessions: the session spawned from
    // it answers configuration commands *through* this handle, so a handle that went out of scope
    // before the frame loop would leave that session with no registry to write to.
    let hub = Hub::new(
        client.config.clone(),
        client.config_path.clone(),
        client.keys.clone().unwrap_or_else(KeyResolver::system),
        client.providers.clone().expect("`Client::new` is built with a provider stack"),
    );
    let ends = open_in_process(&hub);
    client.terminal(ends, Some(hub), loaded.warning)
}

/// The handshake every client sends first (spec §4.1), built in one place so a socket attach and an
/// in-process one cannot drift apart. The `--connect` path calls this too (`client/bridge.rs`), which
/// is also what keeps the two from ever both sending one.
fn hello(token: Option<String>) -> Command {
    Command::Hello { token, client: ClientKind::Tui, protocol_version: PROTOCOL_VERSION }
}

/// Starts the in-process session and says `Hello` to it **before the terminal is entered**.
///
/// This is not optional and it is not a formality: a session is silent until it has judged its first
/// message, and `SessionPolicy::admits` answers `ProtocolVersion` to anything that is not a `Hello`
/// (`session.rs`'s prelude). Without this line the bare run gets no `Welcome`, no greeting, and the
/// user's first keypress — an `Ask`, or a `LoginStart` — is read as a failed handshake and closes
/// the session for the rest of the run. Task 1.2's ruling ("every real client must send `Hello`
/// first") is what this line is, and the `--connect` path's bridge has always done it.
fn open_in_process(hub: &Hub) -> SessionEnds {
    let ends = hub.spawn(SessionPolicy::local());
    let _ = ends.to_server.send(ToServer::Command(hello(None)));
    ends
}

/// The `--connect` run: the same TUI, whose two channels come from a socket instead of this process.
///
/// The socket is opened *before* the terminal is entered, so a refusal — no token on an exposed
/// server, the wrong token, a protocol mismatch — is a line on an ordinary terminal rather than a
/// half-drawn alternate screen (success criterion 2). Nothing on this path builds a provider or
/// touches the credential store (spec §7.2): the server owns both.
pub async fn run_remote(url: String, token: Option<String>) -> Result<(), String> {
    let path = store::default_path();
    let loaded = store::load_or_create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ends = bridge::connect(&url, token.as_deref()).await.map_err(|e| e.to_string())?;
    // W11: the client keeps its own system monitor in `--connect` mode. The server's bridge does not
    // spawn one for a remote peer, so this stays the only producer of `Stats` on this channel.
    sysmon::spawn(ends.events.clone());
    let mut client = Client::remote(loaded.config, path, &url);
    client.terminal(ends, None, loaded.warning)
}

/// Which process runs the session this HUD draws: this one, or a server behind a socket.
#[derive(Clone, PartialEq, Eq)]
enum Attachment {
    /// The bare run: a session spawned from this process's own hub, with the connectors built here.
    InProcess,
    /// `--connect`: a socket to a server that owns the session, the connectors and the keys. The
    /// token is deliberately not kept — it went into `Hello`, and nothing else needs it — and the URL
    /// is a [`bridge::DisplayUrl`], so the query a token used to travel in is not here to print.
    Remote { url: bridge::DisplayUrl },
}

impl Attachment {
    fn is_remote(&self) -> bool {
        matches!(self, Attachment::Remote { .. })
    }

    /// How to name the other end in a notice. **Never the token**, and now that is a fact of the type
    /// rather than a promise in a comment: `DisplayUrl` has no query to print.
    fn describe(&self) -> String {
        match self {
            Attachment::InProcess => "this process".to_string(),
            Attachment::Remote { url } => url.to_string(),
        }
    }
}

/// Everything the client keeps from one frame to the next.
struct Client {
    state: UiState,
    /// The configuration the running session was built from: a save that worked replaces it, so the
    /// client always holds the configuration the file holds. In `--connect` mode it is this
    /// machine's own `config.toml` — the microphone gain lives there — and the screen's content is
    /// the server's from the first `Event::Config` on.
    config: Config,
    /// Where the screen writes, and where it reads every time it opens.
    config_path: PathBuf,
    /// The resolver the connectors are built with: `None` for the whole of a `--connect` run, which
    /// is what makes "this client resolves no key" a fact of the type rather than a promise.
    keys: Option<KeyResolver>,
    /// The live provider stack, shared with the session. `None` for the whole of a `--connect` run.
    providers: Option<ProviderSlot>,
    /// Which process runs the session, which is what every configuration action branches on.
    attachment: Attachment,
    /// `None` when there is no output device: the HUD still works, silently.
    player: Option<Player>,
    /// The audio of the reply being spoken, kept for `Replay` until a new turn starts.
    reply: Vec<i16>,
    /// The live call, when one is running: the microphone feeds it instead of the session and the
    /// audio the device plays arrives from it.
    call: Option<Call>,
    /// Whether a login capture was *running* at the client's last look at it: the microphone stream
    /// and its `LoginStart` are opened and closed on the edges of this flag and never on its value,
    /// which is what keeps a repeated `Welcome` from opening a second stream over the first.
    login_mic: bool,
    /// Where a call reports: the same channel the session reports on, so the HUD reads one stream.
    /// `None` only before the frame loop has started.
    events: Option<UnboundedSender<ToClient>>,
    /// Sample rate announced by the last `SpeechChunk`.
    rate: u32,
    mic_gain: f32,
    mic: Option<Mic>,
    mic_frames: std::sync::mpsc::Receiver<Vec<i16>>,
    mic_tx: std::sync::mpsc::SyncSender<Vec<i16>>,
}

impl Client {
    fn new(config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot) -> Self {
        let (mic_tx, mic_frames) = std::sync::mpsc::sync_channel(MIC_QUEUE);
        let mic_gain = config.listening.mic_gain;
        Self {
            state: UiState::new(),
            config,
            config_path,
            keys: Some(keys),
            providers: Some(providers),
            attachment: Attachment::InProcess,
            player: None,
            reply: Vec::new(),
            call: None,
            login_mic: false,
            events: None,
            rate: 24000,
            mic_gain,
            mic: None,
            mic_frames,
            mic_tx,
        }
    }

    /// The `--connect` client: no resolver, no provider stack, and no way to reach either, because
    /// both fields stay `None` for the whole run (Task 1.7's audit test).
    ///
    /// `config` is still read from this machine's `config.toml`: it carries what belongs to *this*
    /// machine — the microphone gain — and the screen's content is the server's from the first
    /// `Event::Config` on. Nothing here acts on its provider rows.
    fn remote(config: Config, config_path: PathBuf, url: &str) -> Self {
        let (mic_tx, mic_frames) = std::sync::mpsc::sync_channel(MIC_QUEUE);
        let mic_gain = config.listening.mic_gain;
        Self {
            state: UiState::new(),
            config,
            config_path,
            keys: None,
            providers: None,
            attachment: Attachment::Remote { url: bridge::DisplayUrl::new(url) },
            player: None,
            reply: Vec::new(),
            call: None,
            login_mic: false,
            events: None,
            rate: 24000,
            mic_gain,
            mic: None,
            mic_frames,
            mic_tx,
        }
    }

    /// The terminal half both entry points share: the alternate screen, mouse capture, the warning,
    /// the output device and the frame loop. Keeping it in one place is what stops the two runs
    /// drifting apart in how they own the terminal.
    ///
    /// `hub` is `Some` for a bare run and `None` for a `--connect` one, and it is only *held* here:
    /// nothing in the loop reads it, but the in-process session answers its configuration commands
    /// through it, so it has to outlive the loop.
    fn terminal(&mut self, ends: SessionEnds, hub: Option<Hub>, warning: Option<String>) -> Result<(), String> {
        // `init` enters the alternate screen, enables raw mode and installs the panic hook that
        // restores the terminal; `restore` undoes all of it.
        let mut terminal = ratatui::init();
        // Mouse capture is not part of a bare terminal: without it the clickable titles do nothing.
        let _ = execute!(std::io::stdout(), EnableMouseCapture);
        if let Some(warning) = warning {
            self.state.notify(warning);
        }
        self.open_player();
        let outcome = self.frame_loop(&mut terminal, ends);
        let _ = execute!(std::io::stdout(), event::DisableMouseCapture);
        ratatui::restore();
        drop(hub);
        outcome
    }

    /// Opens the default output device. A machine without one keeps a silent HUD.
    fn open_player(&mut self) {
        match Player::new() {
            Ok(player) => self.player = Some(player),
            Err(e) => self.state.notify(e),
        }
    }

    /// The frame loop: input, messages from the session, audio and drawing, about 30 times a second.
    ///
    /// The three ends come from wherever the session lives: `Hub::spawn` in this process for a bare
    /// run, a socket for a `--connect` one. Nothing below this line can tell the two apart (W6).
    fn frame_loop(&mut self, terminal: &mut ratatui::DefaultTerminal, ends: SessionEnds) -> Result<(), String> {
        let to_server = ends.to_server;
        let mut from_server = ends.from_server;
        // A second sender for the same client channel: the system monitor and the screen's Test
        // results (Task 7) both arrive with the session's own events.
        let events = ends.events.clone();
        // A live call reports through the session's own channel too; the monitor keeps the other
        // clone, which is the one `sysmon` takes below.
        self.events = Some(events.clone());
        sysmon::spawn(ends.events);

        let mut drawn = Instant::now();
        loop {
            let hits = draw_frame(terminal, &self.state)?;

            let waited = FRAME.saturating_sub(drawn.elapsed());
            // What this frame's input came to: the keys that were typed, and the hits that were
            // clicked, in the order they arrived. The two are dispatched differently on purpose — a
            // key goes to the state's own translator, a click names its target — and both end up as
            // `UiAction`s below, so a click and the keystroke it replaces cannot disagree.
            let mut keys = Vec::new();
            let mut clicked = Vec::new();
            if event::poll(waited).map_err(|e| e.to_string())? {
                // Everything already typed is handled in this frame; a held key is not throttled
                // to one character per frame.
                while event::poll(Duration::ZERO).map_err(|e| e.to_string())? {
                    match event::read().map_err(|e| e.to_string())? {
                        TermEvent::Key(key) if key.kind != KeyEventKind::Release => {
                            if let Some(input) = key_input(&key) {
                                keys.push(input);
                            }
                        }
                        TermEvent::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                            if let Some(hit) = hit_at(&hits, mouse.column, mouse.row) {
                                clicked.push(hit);
                            }
                        }
                        _ => {}
                    }
                    if keys.len() + clicked.len() >= MAX_INPUT_PER_FRAME {
                        break;
                    }
                }
            }

            // A frame can take a little longer than `FRAME`; the clocks follow the real interval.
            let dt = drawn.elapsed().as_millis() as u64;
            drawn = Instant::now();
            self.state.tick(dt);

            while let Ok(message) = from_server.try_recv() {
                self.on_message(&to_server, message);
            }
            // The capture first, then the device, then the frames: the login's state says whether
            // there is a microphone at all, `sync_mic` opens it (or finds out that it cannot), and
            // only then may a frame be routed — `drain_mic` reads `self.mic` to decide, which is what
            // keeps a queue that filled before the device existed from being sent (review finding 1).
            self.sync_login_mic(&to_server);
            let stream = self.sync_mic();
            debug_assert_eq!(stream, self.mic.is_some(), "`sync_mic` reports the device it just set");
            self.drain_mic(&to_server);
            self.sync_call();

            let mut quit = false;
            for key in keys {
                for action in self.state.on_key(key) {
                    quit |= self.on_action(action, &to_server, &events);
                }
            }
            for hit in clicked {
                for action in self.state.on_hit(&hit) {
                    quit |= self.on_action(action, &to_server, &events);
                }
            }
            if quit {
                break;
            }
        }
        // Silence the device and stop capturing before the terminal comes back.
        self.flush_playback();
        self.mic = None;
        // And close the call with the terminal: `Esc` ends a call by ending the client.
        self.call = None;
        Ok(())
    }

    /// `to_server` is what a remote configuration event answers on: `ConfigChanged` carries no
    /// payload, so the client has to ask with `GetConfig` (spec §4.4).
    fn on_message(&mut self, to_server: &UnboundedSender<ToServer>, message: ToClient) {
        match message {
            ToClient::Event(event) => {
                if let Some(rate) = speech_rate(&event) {
                    self.rate = rate;
                }
                // A remote screen follows the server's configuration: `GetConfig` when it opens, and
                // again whenever the server says something changed — `Event::ConfigChanged` carries
                // no payload by design (spec §4.4). The answer arrives as `Event::Config`.
                if let Event::Config { config } = &event
                    && self.attachment.is_remote()
                {
                    // Two shapes, one conversion in: the screen draws with `tipsy_ui::ConfigForm` and
                    // the wire carries `tipsy_protocol::ConfigView`, and `From` is the only bridge
                    // between them (§4.3). Nothing is redacted here: the server decided what this
                    // client may see before it sent it, with `ConfigView::redacted`.
                    self.state.show_config_form(ConfigForm::from(config.as_ref().clone()));
                }
                if matches!(&event, Event::ConfigChanged)
                    && self.attachment.is_remote()
                    && self.state.config_form.is_some()
                {
                    let _ = to_server.send(ToServer::Command(Command::GetConfig));
                }
                // A Test result belongs to the screen's notice line, not to the HUD's log: the
                // answer stays beside the rows it is about. `UiState::apply` deliberately has
                // nothing to say about it.
                if let Event::TestResult { capability, ok, latency_ms, message } = &event
                    && let Some(form) = self.state.config_form.as_mut()
                {
                    form.test_result(*capability, *ok, *latency_ms, message.clone());
                }
                // A call reports that it is over through `Live { active: false }`, whatever ended
                // it. The client lets go of the handle and silences the device here, because the
                // device is only reachable from this thread and the audio still queued belongs to a
                // conversation that is gone (Review Focus 1).
                if call_ended(&event) {
                    self.end_call();
                }
                // No event ever touches the queued audio or the audio kept for `Replay`: the status
                // that ends a reply arrives while the device is still speaking it, so the only
                // interruption is one the user asks for, in `on_action`.
                self.state.apply(&event);
                // `Welcome.login` is the only authority on whether there is a screen (spec §3), and
                // the screen is shown **once per run** — `show_login` owns that rule and says so,
                // because the state cannot express it with `login` alone: a second `Welcome` (the one
                // `SetProfile` and `SetListeningMode` make the session send) must leave the half-typed
                // answer, the phase and the refusal streak exactly where the user left them, and must
                // not reopen a door that has already been walked through.
                if let Event::Welcome { login, .. } = &event
                    && let Some(view) = login
                    && self.state.show_login(view.voice)
                    && let Some(login) = self.state.login.as_mut()
                {
                    // With the voice on, the capture opens *with* the screen: the reconciliation
                    // below sees the state on the very first frame and opens the stream then. The
                    // phase follows it, because the screen's own key table reads the phase to
                    // decide what `Tab` and `Enter` mean.
                    login.open = view.voice;
                    login.phase = if view.voice { LoginPhase::Listening } else { LoginPhase::Waiting };
                }
            }
            ToClient::Audio(Frame::Speech { samples, .. }) => self.on_speech(samples),
            ToClient::Audio(Frame::Mic(_)) => {}
        }
    }

    /// Applies one action from the HUD. True when the user asked to quit.
    fn on_action(
        &mut self,
        action: UiAction,
        to_server: &UnboundedSender<ToServer>,
        events: &UnboundedSender<ToClient>,
    ) -> bool {
        match action {
            UiAction::Send(command) => {
                // A call replaces the whole pipeline for as long as it lasts, so while one is live
                // the session is told nothing: a `Reset` or a profile change could not reach a
                // socket whose `instructions` were fixed when it opened, and a typed question would
                // be spoken over the call.
                if let Some(call) = self.call.as_ref() {
                    // The verbs that mean "the user is taking the floor" become the local barge-in.
                    // The count goes first, while the flag is still down, so no stale reading can
                    // reach a truncate that a later server-reported barge-in may send. Then the
                    // device is silenced, and last the socket task is asked for `response.cancel`:
                    // it is the only side that can send, and it raises the interrupt flag the frame
                    // loop acts on.
                    if takes_the_floor(&command) {
                        let heard = Arc::clone(call.heard());
                        heard.clear();
                        self.flush_playback();
                        call.interrupt();
                    }
                    return false;
                }
                // The only two things that quiet the device: the user starting a new turn over the
                // reply still being spoken (barge-in), and the cancel the user asked for. Reaching
                // the end of a turn is not an interruption — see `flush_on_interrupt`.
                if flush_on_interrupt(interrupts_playback(&command), self.playing()) {
                    self.flush_playback();
                }
                if starts_turn(&command) {
                    // What `Replay` replays is the last reply, so it goes when the next turn begins.
                    self.reply.clear();
                }
                let _ = to_server.send(ToServer::Command(command));
                false
            }
            UiAction::Replay(_) => {
                // The audio kept is the last reply, which is what `F2` replays.
                if let (Some(player), false) = (&self.player, self.reply.is_empty()) {
                    player.play(self.reply.clone(), self.rate);
                }
                false
            }
            // The screen asks for a form it cannot build: `tipsy-ui` owns no native type. The file
            // is read here and now, which is what keeps a hand-edited `config.toml` from being
            // served out of a copy taken at startup — and over a socket the *server's* configuration
            // is what the screen shows, which is why this arm branches first.
            UiAction::OpenConfig => {
                self.open_screen(to_server);
                false
            }
            // Every other screen action goes through the form's own reducer, and the effects it asks
            // for are performed here, on the native side.
            UiAction::Config(action) => {
                let effects = match self.state.config_form.as_mut() {
                    Some(form) => form.apply(action),
                    None => Vec::new(),
                };
                for effect in effects {
                    // A Test is the screen's half of `Command::TestProvider`, and it goes through the
                    // same rule a command does: it is a diagnostic that says nothing out loud, so it
                    // leaves the reply being spoken where it is. Decided here, from `starts_turn`,
                    // rather than inherited from the fact that the Test button never sends a
                    // command.
                    if let ConfigEffect::Test(capability) = effect
                        && flush_on_interrupt(test_interrupts_playback(capability), self.playing())
                    {
                        self.flush_playback();
                    }
                    self.on_config_effect(effect, to_server, events);
                }
                false
            }
            // The door opened and the user pressed `Enter`: the screen goes, and so does the capture
            // it was holding. The microphone returns to what the listening mode wanted, which is the
            // user's own switch and is not this arm's to move — `sync_mic` reconciles the device from
            // it on the next frame (spec §6.2, review finding 3).
            UiAction::LoginDone => {
                self.state.login = None;
                self.close_login_mic(to_server);
                false
            }
            UiAction::Quit => true,
            // `F10`: the key asks, the socket answers. The indicator follows the call task's own
            // `Event::Live`, never this arm.
            UiAction::ToggleLive => {
                self.toggle_call();
                false
            }
        }
    }
}

/// Draws one frame and reports where its clickable titles landed.
fn draw_frame(terminal: &mut ratatui::DefaultTerminal, state: &UiState) -> Result<Vec<Hit>, String> {
    let clock = clock_view();
    let title = state.profile.as_ref().map(|p| p.name.clone()).unwrap_or_default();
    let mut hits = Vec::new();
    terminal
        .draw(|frame| {
            hits = draw(frame, state, &clock, &title);
        })
        .map_err(|e| e.to_string())?;
    Ok(hits)
}

/// Maps a terminal key to the HUD's own key vocabulary.
fn key_input(key: &KeyEvent) -> Option<KeyInput> {
    match key.code {
        KeyCode::Enter => Some(KeyInput::Enter),
        KeyCode::Backspace => Some(KeyInput::Backspace),
        KeyCode::Tab => Some(KeyInput::Tab),
        KeyCode::Esc => Some(KeyInput::Esc),
        KeyCode::Up => Some(KeyInput::Up),
        KeyCode::Down => Some(KeyInput::Down),
        KeyCode::Left => Some(KeyInput::Left),
        KeyCode::Right => Some(KeyInput::Right),
        KeyCode::F(n) => Some(KeyInput::F(n)),
        // A modifier is not part of the HUD's vocabulary (`KeyInput` has no modifier at all), so a
        // control chord arrives as the control character it stands for: `Ctrl+S` and `Ctrl+R` are
        // `'\u{13}'` and `'\u{12}'`, which is the one class of `char` no text row wants
        // (`ConfigForm::SAVE_KEY`, `ConfigForm::REGENERATE_KEY`). `Ctrl+Shift+letter` reaches the
        // same control character, which is why the mapping folds the case first; a chord that names
        // no letter is not a key this HUD has, and gets none.
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let lower = c.to_ascii_lowercase();
            lower.is_ascii_lowercase().then_some(KeyInput::Char((lower as u8 & 0x1f) as char))
        }
        KeyCode::Char(c) => Some(KeyInput::Char(c)),
        _ => None,
    }
}

/// The clock the HUD shows, read once per frame.
fn clock_view() -> ClockView {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    ClockView {
        time: format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second()),
        // The HUD numbers the week from Monday and the months from January.
        weekday: now.weekday().num_days_from_monday() as u8,
        day: now.day() as u8,
        month: now.month0() as u8,
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use tipsy_protocol::{ListeningMode, LoginView, ProfileView, UiLanguage};

    use crate::keys::{KeyResolver, MemoryStore, SecretStore};
    use crate::providers::ProviderSlot;

    // The bare run's `Hello`-before-the-terminal test is the only one left here that reaches a
    // provider; the screen's own tests, which used these too, are in `client/settings.rs` now.
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    /// A resolver that knows one key — `openrouter`, the name the default configuration uses — and
    /// reads no environment. The playback tests in `client/audio.rs` name it too, which is why it is
    /// reachable from a child.
    pub(super) fn resolver() -> KeyResolver {
        let store = MemoryStore::default();
        SecretStore::set(&store, "openrouter", "k-test").unwrap();
        KeyResolver::new(Box::new(store), Box::new(|_| None))
    }

    /// A configuration whose three connectors point at `base`. Nothing in these tests calls them
    /// unless it says so, and it keeps `build_providers` off the real network.
    ///
    /// The settings tests in `client/settings.rs` name it too, which is why it is reachable from a
    /// child.
    pub(super) fn mock_config(base: &str) -> Config {
        let mut config = Config::default();
        for provider in [&mut config.providers.chat, &mut config.providers.speech, &mut config.providers.transcribe] {
            provider.base_url = format!("{base}/v1");
            provider.key = "openrouter".into();
        }
        config
    }

    // ---- the login screen (spec 2026-10-11-login-screen-design.md, §3 and §6) ----

    /// A bare client: no terminal, no device, no call. The configuration is the default one, whose
    /// connectors are never touched by the tests below.
    pub(super) fn mock_client() -> Client {
        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        Client::new(config, PathBuf::from("config.toml"), keys, providers)
    }

    pub(super) fn welcome(login: Option<LoginView>) -> Event {
        Event::Welcome {
            session_id: 1,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: UiLanguage::Es,
            listening_mode: ListeningMode::Push,
            web_bundle: false,
            login,
        }
    }

    /// Runs the reconciliation the frame loop runs, and hands back everything it sent the server.
    pub(super) fn sync_login(client: &mut Client) -> Vec<Command> {
        let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
        client.sync_login_mic(&to_server);
        let mut sent = Vec::new();
        while let Ok(message) = commands.try_recv() {
            if let ToServer::Command(command) = message {
                sent.push(command);
            }
        }
        sent
    }

    /// `Welcome.login` is the only authority on whether there is a screen (spec §3): a client that
    /// reads `[login]` itself could disagree with the server that will judge it, so the client never
    /// reads it and this is what it does instead.
    #[test]
    fn a_login_view_shows_the_screen_and_no_login_view_goes_straight_to_the_hud() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(None)));
        assert!(client.state.login.is_none(), "no login view is no screen");
        assert!(sync_login(&mut client).is_empty(), "and nothing to open");

        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: false }))));
        let login = client.state.login.as_ref().expect("a login view is the screen");
        assert!(!login.voice, "and it carries the screen's own switch");

        let mut client = mock_client();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
        let login = client.state.login.as_ref().expect("a login view is the screen");
        assert!(login.voice);
        assert!(login.open, "the screen opens listening, without a key");
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart], "and the server is told");
    }

    /// Every `Welcome` re-announces the session — a save on the configuration screen sends one — and
    /// the screen is shown **once per run** (spec §3): a second one must leave the answer being
    /// typed, the phase and the streak exactly where the user left them.
    #[test]
    fn a_second_welcome_leaves_the_login_screen_alone() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
        for c in "andres".chars() {
            client.state.on_key(KeyInput::Char(c));
        }
        for _ in 0..2 {
            client.on_message(&to_server, ToClient::Event(Event::LoginVerdict { ok: false, name: None }));
        }
        let typed = client.state.login.clone().unwrap();
        assert_eq!(typed.input, "andres");
        assert_eq!(typed.refusals, 2);

        // The session re-announces itself, as `SetProfile` and `SetListeningMode` make it do.
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));

        let after = client.state.login.clone().expect("the screen is still up");
        assert_eq!(after, typed, "a second Welcome is not a second screen");
    }

    /// The door, once walked through, does not come back. The session re-sends `Welcome` on
    /// `SetProfile` and `SetListeningMode` — a save from the configuration screen sends both — and the
    /// login spec's rule is **once per client run** (login spec §3). The guard this replaces,
    /// `self.state.login.is_none()`, could not tell "let in" from "never asked": `UiAction::LoginDone`
    /// puts `login` back to `None`, so the next `Welcome` showed the screen again and the user met the
    /// door a second time on a save.
    #[test]
    fn a_welcome_after_the_door_opened_does_not_reopen_it() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();

        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
        assert!(client.state.login.is_some(), "the first Welcome shows the screen");

        client.on_action(UiAction::LoginDone, &to_server, &events);
        assert!(client.state.login.is_none(), "the door opens");

        // Exactly what `SetProfile` and `SetListeningMode` make the session send.
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
        assert!(client.state.login.is_none(), "and the screen does not come back");
        assert!(client.state.login_seen, "the run remembers that it was asked");
    }

    /// With no screen up, the login's events belong to nothing: they must not touch the HUD, open a
    /// device or send a command.
    #[test]
    fn a_login_verdict_without_a_screen_touches_nothing() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(Event::LoginVerdict { ok: true, name: Some("Andrés".into()) }));
        client.on_message(&to_server, ToClient::Event(Event::LoginHeard { text: "andres del core".into() }));
        assert!(client.state.login.is_none());
        assert!(!client.state.mic_open);
        assert!(client.mic.is_none());
        assert!(client.state.log.is_empty(), "and the conversation is untouched");
        assert!(sync_login(&mut client).is_empty());
    }

    /// While the door is shut the keys stop at the screen: `F9` does not open the configuration and
    /// `Esc` still leaves, which is the client's half of the screen being modal.
    #[test]
    fn the_login_screen_swallows_the_huds_keys() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: false }))));
        assert!(client.state.on_key(KeyInput::F(9)).is_empty(), "no second screen while the door is shut");
        assert!(client.state.config_form.is_none());
        assert_eq!(client.state.on_key(KeyInput::Esc), vec![UiAction::Quit]);
    }

    /// B1, and the property that was missing: a session is silent until its first message and
    /// refuses anything that is not a `Hello` (`session.rs`'s prelude), so `client::run` says hello
    /// **before** `terminal()` is entered. Without it the bare run got no `Welcome`, no greeting, and
    /// the user's first keypress ended the session — the review reproduced exactly that
    /// (`Error { code: ProtocolVersion, recoverable: false }`, zero chat calls).
    ///
    /// Nothing in this test sends anything by itself, so the `Welcome` it waits for can only arrive
    /// because `open_in_process` put a `Hello` on the channel first. The second half is the same
    /// session **without** that handshake, kept so the assertion above cannot pass because a session
    /// serves whatever it is sent first.
    #[tokio::test]
    async fn the_bare_run_says_hello_before_the_terminal() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"choices\":[{\"delta\":{\"content\":\"hola\"}}]}\n\ndata: [DONE]\n\n",
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let config = mock_config(&server.uri());
        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        store::save(&path, &config).unwrap();
        let hub = Hub::new(config, path, keys, slot);

        let mut ends = open_in_process(&hub);
        let first = tokio::time::timeout(Duration::from_secs(5), ends.from_server.recv())
            .await
            .expect("a session with no `Hello` says nothing at all, so the handshake must have been sent")
            .expect("the session's channel must stay open");
        assert!(
            matches!(first, ToClient::Event(Event::Welcome { .. })),
            "the bare run's first answer is the `Welcome`, got {first:?}"
        );

        // The other half of the same rule, which is the defect: a session nobody greeted refuses the
        // first thing a user does, and ends rather than serving it.
        let mut refused = hub.spawn(SessionPolicy::local());
        refused.to_server.send(ToServer::Command(Command::Ask { text: "¿qué pasa?".into() })).unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(5), refused.from_server.recv())
            .await
            .expect("a session refuses a first message that is not a `Hello`")
            .expect("the refusal is an event on the channel");
        match answer {
            ToClient::Event(Event::Error { code, recoverable, .. }) => {
                assert_eq!(code, tipsy_protocol::ErrorCode::ProtocolVersion);
                assert!(!recoverable, "and the refusal is not recoverable: the session is over");
            }
            other => panic!("a first `Ask` on an ungreeted session is refused, got {other:?}"),
        }
    }
}
