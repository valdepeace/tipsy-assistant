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
use tipsy_ui::{
    ClockView, ConfigEffect, ConfigForm, Hit, KeyInput, LoginLine, LoginPhase, NoticeKind, UiAction, UiState, draw,
};
use tipsy_ui::policy::{
    MicRoute, flush_on_interrupt, hit_at, interrupts_playback, mic_route, speech_rate, starts_turn, takes_the_floor,
    test_interrupts_playback,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::audio::device::{Mic, Player};
use crate::audio::pcm::{envelope, i16_to_f32, rms};
use crate::config::form::{config_from, form_of, provider_config};
use crate::config::store;
use crate::config::Config;
use crate::keys::{env_var_name, KeyResolver};
use crate::providers::{build_one, build_providers, ProviderSlot};
use crate::realtime::{Call, CallSetup, Heard};
use crate::server::Hub;
use crate::session::{SessionEnds, SessionPolicy};
use crate::sysmon;

pub mod audio;
/// The client's own half of the socket bridge: the `--connect` path's three channel ends.
pub mod bridge;
pub mod call;
pub mod mic;

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

    /// Opens the configuration screen on a copy read from the file *now* (Review Focus 3): a
    /// `config.toml` edited while the app runs is what the screen shows. Opening changes nothing
    /// that is running — `Esc` discards the edits, and only a save applies them.
    ///
    /// The screen is put up filled only when the file was read: a read that fails leaves the
    /// placeholder form on screen, unmarked (finding G), and that form refuses `Ctrl+S` and `F4`
    /// rather than writing blank base URLs and models over the user's `config.toml`. The reason goes
    /// where the user reads it — the screen's own notice line, through [`Client::config_notice`] —
    /// and not into the header's status span, which the header's own layout is free to cut to zero
    /// columns while the screen is up.
    ///
    /// A remote client has no file to read for this: the screen that matters is the server's, so it
    /// asks for it instead.
    fn open_screen(&mut self, to_server: &UnboundedSender<ToServer>) {
        if self.attachment.is_remote() {
            return self.open_screen_remote(to_server);
        }
        match store::load_or_create(&self.config_path) {
            Ok(loaded) => {
                let mut form = form_of(&loaded.config);
                // A file that had to be backed up and reset is worth saying in the notice line: the
                // user's own file is gone, and the form no longer holds what they wrote.
                if let Some(warning) = loaded.warning {
                    form.notice = Some((NoticeKind::Warning, warning));
                }
                self.state.show_config_form(form);
            }
            Err(e) => self.config_notice(NoticeKind::Error, format!("{}: {e}", self.config_path.display())),
        }
    }

    /// Performs one effect the screen asked for.
    ///
    /// `events` is the client's own event sender: a Test reports its answer through it, so the
    /// result is rendered like anything else the session says.
    fn on_config_effect(
        &mut self,
        effect: ConfigEffect,
        to_server: &UnboundedSender<ToServer>,
        events: &UnboundedSender<ToClient>,
    ) {
        match effect {
            ConfigEffect::None => {}
            ConfigEffect::Save => self.save(to_server),
            // The Test needs the sender now: over a socket the probe runs on the server.
            ConfigEffect::Test(capability) => self.test_provider(capability, to_server, events),
            // The token row is **native-only**, and now it says so instead of editing a value the
            // wire will not carry: a client's `UpdateConfig` has no `server.token` field at all
            // (spec §13.6 — the row stays inert over the wire, by design), so minting one here would
            // show the user a fresh password that the server never learns and that they cannot use.
            ConfigEffect::RegenerateToken => {
                if self.attachment.is_remote() {
                    self.config_notice(
                        NoticeKind::Warning,
                        "the token belongs to the server; regenerate it on the machine running `serve`",
                    );
                    return;
                }
                self.regenerate_token();
            }
        }
    }

    /// The `--connect` open: the server is asked for its configuration, and until the answer arrives
    /// the screen shows this client's own copy with a notice that says so. Opening on nothing at all
    /// would make `F9` look broken for a round trip.
    fn open_screen_remote(&mut self, to_server: &UnboundedSender<ToServer>) {
        let mut form = form_of(&self.config);
        form.notice = Some((
            NoticeKind::Warning,
            format!("asking {} for its configuration", self.attachment.describe()),
        ));
        self.state.show_config_form(form);
        let _ = to_server.send(ToServer::Command(Command::GetConfig));
    }

    /// Runs the screen's Test against the row on screen and reports the answer into the client's own
    /// event channel, so it is rendered like anything else the session says.
    ///
    /// The connector is built *here*, before the call: a row that names a key nobody holds is then
    /// answered at once, with no latency and without a single request — which is what makes "no key"
    /// a different answer from "the provider refused". The probe itself runs on a task of its own,
    /// so the HUD never waits for a provider.
    fn test_provider(
        &mut self,
        capability: Capability,
        to_server: &UnboundedSender<ToServer>,
        events: &UnboundedSender<ToClient>,
    ) {
        if self.attachment.is_remote() {
            // The providers and the keys are the server's, so the probe is too (spec §6.2). The
            // answer comes back as `Event::TestResult` through the socket and lands in the same
            // notice line as any other result.
            let _ = to_server.send(ToServer::Command(Command::TestProvider { capability }));
            return;
        }
        // The two fields are `Some` together for the whole of a bare run, and the guard above has
        // already sent every remote Test to the server, so neither of these can fail here.
        let Some(keys) = &self.keys else { return };
        // The row the user is looking at, converted the way a save converts it: a Test can never
        // quietly use a connector other than the one on screen.
        let converted = self
            .state
            .config_form
            .as_ref()
            .and_then(|form| form.providers.iter().find(|row| row.capability == capability))
            .map(provider_config);
        let cfg = match converted {
            // No screen, or no row for this capability: nothing to test and nothing to say.
            None => return,
            Some(Err(message)) => return self.config_notice(NoticeKind::Error, message),
            Some(Ok(cfg)) => cfg,
        };
        let connector = match build_one(&cfg, capability, keys) {
            Ok(connector) => connector,
            Err(e) => {
                // No request is made, so there is no latency to report; the message is the builder's
                // own, which names the key and the environment variable that would provide it.
                let _ = events.send(ToClient::Event(Event::TestResult {
                    capability,
                    ok: false,
                    latency_ms: 0,
                    message: e.to_string(),
                }));
                return;
            }
        };
        // The profile's language, read now: the transcription probe sends it, and it is the same tag
        // every other transcription carries.
        let language = self.config.active().language.clone();
        let events = events.clone();
        tokio::spawn(async move {
            let probe = connector.probe(&language).await;
            let _ = events.send(ToClient::Event(Event::TestResult {
                capability,
                ok: probe.ok,
                latency_ms: probe.latency_ms,
                message: probe.message,
            }));
        });
    }

    /// Writes the screen's configuration and applies it to the running session.
    ///
    /// The order is the whole of Review Focus 1: the connectors are built first, then the file is
    /// written through the one writer, and only then does anything running change. Nothing is
    /// written until every row parses and every key resolves, because a `config.toml` the app cannot
    /// start from is damage rather than a saved edit; and a refused write leaves the file, the
    /// running stack and the session exactly as they were.
    ///
    /// Over a socket none of that is this process's to do: the file, the keys and the connectors are
    /// the server's, so the form goes over the wire and the answer comes back as a fresh
    /// `Event::Config` (spec §6.2).
    fn save(&mut self, to_server: &UnboundedSender<ToServer>) {
        if self.attachment.is_remote() {
            return self.save_remote(to_server);
        }
        // 1. The form in the file's shape.
        let parsed = self.state.config_form.as_ref().map(config_from);
        let config = match parsed {
            None => return,
            Some(Err(message)) => return self.config_notice(NoticeKind::Error, message),
            Some(Ok(config)) => config,
        };
        // 2. The connectors that configuration names.
        let (Some(keys), Some(providers)) = (&self.keys, &self.providers) else {
            // Unreachable while `Attachment::InProcess` owns the two `Some`s; the guard is what
            // keeps a remote client from ever reaching a builder.
            return;
        };
        let rebuilt = match build_providers(&config.providers, keys) {
            Ok(providers) => providers,
            Err(e) => return self.config_notice(NoticeKind::Error, e.to_string()),
        };
        // 3. The file itself.
        if let Err(e) = store::save(&self.config_path, &config) {
            return self.config_notice(NoticeKind::Error, format!("{}: {e}", self.config_path.display()));
        }
        // 4. What is running: the stack in place, then the fields the channel carries.
        self.mic_gain = config.listening.mic_gain;
        providers.replace(rebuilt);
        let _ = to_server.send(ToServer::Command(Command::SetListeningMode { mode: config.listening.mode }));
        let _ = to_server.send(ToServer::Command(Command::SetProfile { id: config.active_profile.clone() }));
        self.config = config;
        if let Some(form) = self.state.config_form.as_mut() {
            form.dirty = false;
            form.notice = None;
        }
    }

    /// The `--connect` save: the server owns the file, the keys and the connectors, so this sends the
    /// form and nothing else (spec §6.2). No local `build_providers` — this client holds no key — and
    /// no local `store::save`: a second copy of the server's configuration on another machine is a
    /// second source of truth, and the server answers `ConfigChanged` to everyone when it saves.
    ///
    /// The server may refuse the view, and a refused view is **silent** — the hub writes nothing and
    /// says nothing (spec §4.2) — so this client's own screen keeps the refusal it had and the notice
    /// below is a statement about what was *sent*, never a claim that it was saved.
    fn save_remote(&mut self, to_server: &UnboundedSender<ToServer>) {
        let Some(form) = self.state.config_form.as_ref() else { return };
        // Two shapes, one conversion out: the screen's form is not the wire's view (§4.3), and the
        // view the wire carries is the shape a **client** may send — no provider `base_url`, no
        // provider `key_name`, no `server.token`. Those three rows are still editable on this screen,
        // and a remote save simply does not carry them (`ClientConfigView`), which is the rule rather
        // than a check the server has to make. The server's own answer to `ConfigChanged` puts the
        // server's values back on the screen, so an edit that did not cross is visible rather than
        // mysterious.
        let config = ClientConfigView::from(form);
        let _ = to_server.send(ToServer::Command(Command::UpdateConfig { config: Box::new(config) }));
        if let Some(form) = self.state.config_form.as_mut() {
            form.dirty = false;
            form.notice = Some((
                NoticeKind::Warning,
                format!("sent to {}; this client's own config.toml is unchanged", self.attachment.describe()),
            ));
        }
    }

    /// Fills the Server tab's token row with a fresh token from the OS generator. This is an edit
    /// like any other: the file changes when the user saves, never here.
    fn regenerate_token(&mut self) {
        match store::new_token() {
            Ok(token) => {
                if let Some(form) = self.state.config_form.as_mut() {
                    form.server.token = token;
                    form.dirty = true;
                }
            }
            Err(e) => self.config_notice(NoticeKind::Error, e.to_string()),
        }
    }

    /// Puts a configuration message in the screen's notice line. The text is the low-level truth —
    /// the path and its OS error, the row that would not parse, the provider's own words — because
    /// the form carries no language of its own (the plan's `ConfigForm` has no `lang` field) and
    /// must not invent copy it cannot translate. Falls back to the HUD's notice when no screen is
    /// open.
    fn config_notice(&mut self, kind: NoticeKind, text: impl Into<String>) {
        let text = text.into();
        if let Some(form) = self.state.config_form.as_mut() {
            form.notice = Some((kind, text));
            return;
        }
        self.state.notify(text);
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

    use crate::config::form::form_of;
    use crate::keys::{KeyResolver, MemoryStore, SecretStore};
    use crate::providers::ProviderSlot;
    use tipsy_ui::{ConfigAction, NoticeKind};
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
    fn mock_config(base: &str) -> Config {
        let mut config = Config::default();
        for provider in [&mut config.providers.chat, &mut config.providers.speech, &mut config.providers.transcribe] {
            provider.base_url = format!("{base}/v1");
            provider.key = "openrouter".into();
        }
        config
    }

    /// The screen with the edits a user would have made in it: the form holds them, the file does
    /// not, and `dirty` says so.
    fn open_with_edits(client: &mut Client, edited: &Config) {
        client.state.show_config_form(form_of(edited));
        client.state.config_form.as_mut().unwrap().dirty = true;
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

    /// Plan 3 Task 1.7's audited rule: the `--connect` client must not start a server, must not build a
    /// provider and must not read the credential store (spec §7.2, §6.2).
    ///
    /// The instrument is `providers::builds_on_this_thread`, a counter bumped at the top of
    /// `build_providers` and `build_one`; it is a thread-local because this binary's other tests build
    /// providers on purpose, on other threads, at the same time. The first half of the test drives the
    /// in-process path so a counter that never moves cannot make the audit pass by accident — and it is
    /// an async test because that half's Test runs its probe on a task of its own.
    #[tokio::test]
    async fn the_remote_path_builds_no_provider_and_asks_the_server_instead() {
        let config = mock_config("http://127.0.0.1:9");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // The file this machine owns, written before anything runs: a remote save must leave it byte
        // for byte as it is, because the server's configuration is not this machine's to keep.
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        // The instrument first: the in-process Test does resolve a key and build a connector.
        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut local = Client::new(config.clone(), path.clone(), keys, slot);
        local.state.show_config_form(form_of(&config));
        local.state.config_form.as_mut().unwrap().field = 0;
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        let before_builds = crate::providers::builds_on_this_thread();
        local.on_action(UiAction::Config(ConfigAction::Test), &to_server, &events);
        assert!(
            crate::providers::builds_on_this_thread() > before_builds,
            "the instrument must see the in-process Test build its connector, or this test proves nothing"
        );

        // And now the remote path: opening the screen, testing and saving, with no resolver and no stack.
        let mut remote = Client::remote(config, path.clone(), "ws://127.0.0.1:7070/ws");
        let mut form = form_of(&mock_config("http://127.0.0.1:9"));
        // `form_of` marks the form filled — the file was read — and Save is refused on a form that is
        // not, exactly as the screen's own reducer refuses it.
        assert!(form.filled(), "the screen's save is only reachable from a filled form");
        form.field = 0;
        remote.state.show_config_form(form);
        let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, mut results) = tokio::sync::mpsc::unbounded_channel();

        let before_builds = crate::providers::builds_on_this_thread();
        remote.open_screen(&to_server);
        press_test(&mut remote, &to_server, &events);
        remote.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);
        assert_eq!(
            crate::providers::builds_on_this_thread(),
            before_builds,
            "the remote path built a provider stack, which means it read a key it must not read"
        );
        assert!(results.try_recv().is_err(), "a remote Test must not answer locally");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "a remote save must not write this machine's config.toml: the server owns the file"
        );

        // The three effects went to the server as the three commands, in order (spec §6.2).
        let mut sent = Vec::new();
        while let Ok(ToServer::Command(command)) = commands.try_recv() {
            sent.push(command);
        }
        assert!(
            matches!(
                sent.as_slice(),
                [
                    Command::GetConfig,
                    Command::TestProvider { capability: Capability::Chat },
                    Command::UpdateConfig { .. },
                ]
            ),
            "{sent:?}"
        );
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

    /// Review Focus 3, second half: the screen opens on the file as it is *now*. A `config.toml`
    /// hand-edited while the app runs is what the second open shows, never the copy the first one
    /// read, and opening changes nothing that is running.
    #[test]
    fn opening_the_screen_again_reads_the_file_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config.clone(), path.clone(), keys, slot);
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();

        client.open_screen(&to_server);

        let first = client.state.config_form.as_ref().unwrap();
        assert_eq!(first.listening.silence_ms, "1000");
        assert!(first.server.token.is_empty());

        // The user edits the file in another window while the app keeps running.
        let mut edited = config.clone();
        edited.listening.silence_ms = 4321;
        edited.server.token = "pasted-by-hand".into();
        store::save(&path, &edited).unwrap();

        client.open_screen(&to_server);
        let second = client.state.config_form.as_ref().unwrap();
        assert_eq!(second.listening.silence_ms, "4321", "the screen must read the file again");
        assert_eq!(second.server.token, "pasted-by-hand");
        assert_eq!(client.config, config, "opening the screen changes nothing that is running");
    }

    /// The scoped re-review's finding 2: a file that cannot be read leaves the placeholder screen up
    /// and empty, and the reason is on the screen's own notice line. The header's status span, the
    /// only other place it could show, is the one span the header's layout may cut to zero columns.
    #[test]
    fn a_file_that_cannot_be_read_says_so_on_the_screen_itself() {
        let dir = tempfile::tempdir().unwrap();
        // A parent that is a file rather than a directory: nothing can be created under it, so the
        // load fails the way it does when the user's configuration directory is not writable.
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, "x").unwrap();
        let path = blocker.join("config.toml");

        let config = mock_config("http://127.0.0.1:1");
        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, path.clone(), keys, slot);

        // The real door: `F9` puts the placeholder form up — that is the state's own toggle — and
        // asks the client for the file, exactly as the key loop and a click on the gear do.
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        for action in client.state.on_key(KeyInput::F(9)) {
            client.on_action(action, &to_server, &events);
        }

        let form = client.state.config_form.as_ref().expect("the screen stays up when the file cannot be read");
        let (kind, text) = form.notice.clone().expect("the screen itself must say why it is empty");
        assert_eq!(kind, NoticeKind::Error);
        assert!(text.contains("config.toml"), "the notice names the file it could not read: {text}");
        assert!(text.contains("not-a-dir"), "and the path that failed: {text}");
        assert!(
            client.state.notice.is_none(),
            "the HUD's line is not where a screen notice belongs: {path:?} is already named on screen"
        );
    }

    /// Review Focus 1: a write that fails changes nothing at all. The file is byte for byte what it
    /// was, the running configuration and its provider stack are the ones the app started with, and
    /// the screen says why — while the edits stay in the form, unsaved.
    #[test]
    fn a_refused_save_leaves_the_file_and_the_running_configuration_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let stack_before = slot.current();
        let mut client = Client::new(config.clone(), path.clone(), keys, slot.clone());

        let mut edited = config.clone();
        edited.listening.silence_ms = 4321;
        edited.server.token = "pasted-by-hand".into();
        open_with_edits(&mut client, &edited);

        // A config path the process may not write: read-only is the cheapest way to make the write
        // itself fail — after every row parsed and every connector built, which is where the
        // interesting part of the order is.
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        assert!(std::fs::metadata(&path).unwrap().permissions().readonly(), "the file must be read-only for this test to prove anything");

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);

        assert_eq!(std::fs::read(&path).unwrap(), before, "a refused save must not touch the file");
        assert_eq!(client.config, config, "the running configuration is the one it was");
        assert!(
            std::sync::Arc::ptr_eq(&stack_before.chat, &slot.current().chat),
            "the running provider stack must not have been swapped"
        );
        let form = client.state.config_form.as_ref().unwrap();
        let (kind, text) = form.notice.clone().expect("a refused save must say so");
        assert_eq!(kind, NoticeKind::Error);
        assert!(text.contains("config.toml"), "the notice must name the file: {text}");
        assert!(form.dirty, "the edits are still unsaved: {form:?}");
        // And the file still holds the value a next launch must read.
        assert!(!std::fs::read_to_string(&path).unwrap().contains("4321"));

        // Let the temp directory go away. Clearing the bit back is the one place `false` is the
        // point, so clippy's `permissions_set_readonly_false` is silenced rather than obeyed.
        #[allow(clippy::permissions_set_readonly_false)]
        {
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(false);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
    }

    /// The other half of Review Focus 1, and Review Focus 5's "no key": a configuration whose
    /// connectors cannot be built is never written at all. Writing it would leave a `config.toml`
    /// the app cannot start from — a fresh launch fails with exit code 2 — so the key is checked
    /// before the file is touched.
    #[test]
    fn a_save_whose_providers_cannot_be_built_never_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let stack_before = slot.current();
        let mut client = Client::new(config.clone(), path.clone(), keys, slot.clone());

        // The user typed a key name nobody holds. Everything else about this save is valid, and the
        // write itself would have succeeded.
        let mut edited = config.clone();
        edited.providers.chat.key = "not-in-the-store".into();
        open_with_edits(&mut client, &edited);

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);

        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "a configuration the app cannot start from must never be written"
        );
        assert_eq!(client.config, config);
        assert!(std::sync::Arc::ptr_eq(&stack_before.chat, &slot.current().chat));
        let (kind, text) = client.state.config_form.as_ref().unwrap().notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Error);
        assert!(
            text.contains("not-in-the-store") && text.contains("TIPSY_KEY_NOT_IN_THE_STORE"),
            "the notice must name the key and how to provide it: {text}"
        );
    }

    /// A save that works: the file holds the edits, the running configuration is the one that was
    /// written, the connectors are rebuilt *in place*, and the session is told the fields the
    /// channel carries — in that order.
    #[test]
    fn a_saved_configuration_is_written_and_swapped_into_the_running_stack() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let stack_before = slot.current();
        let mut client = Client::new(config.clone(), path.clone(), keys, slot.clone());

        let mut edited = config.clone();
        edited.listening.silence_ms = 4321;
        edited.listening.mode = ListeningMode::Auto;
        edited.active_profile = "senyor".into();
        open_with_edits(&mut client, &edited);

        let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("silence_ms = 4321"), "{text}");
        assert!(text.contains("mode = \"auto\""), "{text}");
        assert!(text.contains("active_profile = \"senyor\""), "{text}");
        assert_eq!(store::load_or_create(&path).unwrap().config, edited, "the file is the whole form");
        assert_eq!(client.config, edited, "the running configuration is what was written");
        assert!(!std::sync::Arc::ptr_eq(&stack_before.chat, &slot.current().chat), "rebuilt in place");
        assert!(!client.state.config_form.as_ref().unwrap().dirty, "saved means nothing unsaved");
        assert!(client.state.config_form.as_ref().unwrap().notice.is_none());

        // The session learns the mode and the profile over the channel it already uses.
        assert_eq!(
            commands.try_recv().unwrap(),
            ToServer::Command(Command::SetListeningMode { mode: ListeningMode::Auto })
        );
        assert_eq!(
            commands.try_recv().unwrap(),
            ToServer::Command(Command::SetProfile { id: "senyor".into() })
        );
    }

    /// Regenerating fills the row the screen shows and writes nothing: the file changes on save,
    /// only on save, and only through `store::save`.
    #[test]
    fn regenerating_the_token_fills_the_row_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, path.clone(), keys, slot);
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.open_screen(&to_server);
        assert!(client.state.config_form.as_ref().unwrap().server.token.is_empty());

        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::RegenerateToken), &to_server, &events);

        let form = client.state.config_form.as_ref().unwrap();
        assert_eq!(form.server.token.len(), 32, "16 bytes as lowercase hex: {:?}", form.server.token);
        assert!(
            form.server.token.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "the token's alphabet is 0-9a-f: {:?}",
            form.server.token
        );
        assert!(form.dirty, "a fresh token is an unsaved edit like any other");
        assert_eq!(std::fs::read(&path).unwrap(), before, "nothing is written until the user saves");
    }

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The screen's Test button, pressed the way the client receives it: the focused row of the
    /// form decides which capability is probed, and the probe makes a real call against the row.
    fn press_test(client: &mut Client, to_server: &UnboundedSender<ToServer>, events: &UnboundedSender<ToClient>) {
        client.on_action(UiAction::Config(ConfigAction::Test), to_server, events);
    }

    /// Review Focus 5: the result of a Test comes back through the client's own event channel and
    /// lands in the screen's notice line with the latency the call really took.
    #[tokio::test]
    async fn the_test_button_reports_a_real_answer_with_its_latency() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(30))
                    .set_body_raw(
                        "data: {\"choices\":[{\"delta\":{\"content\":\"pong\"}}]}\n\ndata: [DONE]\n\n",
                        "text/event-stream",
                    ),
            )
            .mount(&server)
            .await;

        let config = mock_config(&server.uri());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config.clone(), path, keys, slot);
        client.state.show_config_form(form_of(&config));
        // Task 3's Providers tab lists the three capabilities in order, so its first row is chat's.
        // The assertion on `capability` below is what catches a different layout.
        client.state.config_form.as_mut().unwrap().field = 0;
        // The reply `F2` replays, and the audio the device may still have queued: a probe is a
        // diagnostic, so pressing Test must leave both exactly where they are.
        client.reply = vec![7, 8, 9];

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, mut results) = tokio::sync::mpsc::unbounded_channel();
        press_test(&mut client, &to_server, &events);
        assert_eq!(client.reply, vec![7, 8, 9], "a Test is not a turn: the last reply stays replayable");

        // The probe runs on a task of its own — the HUD never waits for a provider.
        let message = tokio::time::timeout(std::time::Duration::from_secs(5), results.recv())
            .await
            .expect("the probe must answer within five seconds")
            .expect("the client must still hold the channel");
        let ToClient::Event(Event::TestResult { capability, ok, latency_ms, message: text }) = &message else {
            panic!("a Test must answer with TestResult, got {message:?}");
        };
        // Copied out of the borrowed event: the message itself is moved into `on_message` below.
        let (capability, ok, latency_ms, text) = (*capability, *ok, *latency_ms, text.clone());
        assert_eq!(capability, Capability::Chat);
        assert!(ok, "the provider answered: {text}");
        assert!(latency_ms >= 20, "the latency must be the measured one, got {latency_ms}");

        client.on_message(&to_server, message);
        let (kind, notice) = client.state.config_form.as_ref().unwrap().notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Ok);
        assert!(notice.starts_with("chat: "), "{notice}");
        assert!(notice.contains(&format!("{latency_ms} ms")), "{notice}");
    }

    /// "No key" is not "the provider refused": the row names a key nobody holds, so the answer comes
    /// back with no latency at all and no request is ever made.
    #[tokio::test]
    async fn a_test_for_a_row_with_no_key_answers_without_calling_anything() {
        let server = MockServer::start().await; // nothing may reach it
        let config = mock_config(&server.uri());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let keys = resolver(); // holds `openrouter` only
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config.clone(), path, keys, slot);

        let mut edited = config.clone();
        edited.providers.chat.key = "not-in-the-store".into();
        client.state.show_config_form(form_of(&edited));
        client.state.config_form.as_mut().unwrap().field = 0;

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, mut results) = tokio::sync::mpsc::unbounded_channel();
        press_test(&mut client, &to_server, &events);

        let message = results.try_recv().expect("a row with no key is answered at once, with no call");
        let ToClient::Event(Event::TestResult { capability, ok, latency_ms, message: text }) = &message else {
            panic!("a Test must answer with TestResult, got {message:?}");
        };
        let (capability, ok, latency_ms, text) = (*capability, *ok, *latency_ms, text.clone());
        assert_eq!(capability, Capability::Chat);
        assert!(!ok);
        assert_eq!(latency_ms, 0, "no call was made, so there is no latency to report: {text}");
        assert!(
            text.contains("not-in-the-store") && text.contains("TIPSY_KEY_NOT_IN_THE_STORE"),
            "the answer names the key and how to provide it: {text}"
        );
        assert!(server.received_requests().await.unwrap().is_empty(), "no key must not become a request");

        client.on_message(&to_server, message);
        let (kind, notice) = client.state.config_form.as_ref().unwrap().notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Error);
        assert!(notice.contains("not-in-the-store"), "{notice}");
    }
}
