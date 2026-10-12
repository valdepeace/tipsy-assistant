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
use tipsy_protocol::{Capability, ClientKind, Command, ConfigView, Event, Frame, PROTOCOL_VERSION, ToClient, ToServer};
use tipsy_ui::{
    ClockView, ConfigEffect, ConfigForm, Hit, KeyInput, LoginLine, LoginPhase, NoticeKind, UiAction, UiState, draw,
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

/// The client's own half of the socket bridge: the `--connect` path's three channel ends.
pub mod bridge;

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
    let mut client = Client::remote(loaded.config, path, url);
    client.terminal(ends, None, loaded.warning)
}

/// Which process runs the session this HUD draws: this one, or a server behind a socket.
#[derive(Clone, PartialEq, Eq)]
enum Attachment {
    /// The bare run: a session spawned from this process's own hub, with the connectors built here.
    InProcess,
    /// `--connect`: a socket to a server that owns the session, the connectors and the keys. The
    /// token is deliberately not kept — it went into `Hello`, and nothing else needs it.
    Remote { url: String },
}

impl Attachment {
    fn is_remote(&self) -> bool {
        matches!(self, Attachment::Remote { .. })
    }

    /// How to name the other end in a notice. Never the token: it is not in this value.
    fn describe(&self) -> String {
        match self {
            Attachment::InProcess => "this process".to_string(),
            Attachment::Remote { url } => url.clone(),
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
    fn remote(config: Config, config_path: PathBuf, url: String) -> Self {
        let (mic_tx, mic_frames) = std::sync::mpsc::sync_channel(MIC_QUEUE);
        let mic_gain = config.listening.mic_gain;
        Self {
            state: UiState::new(),
            config,
            config_path,
            keys: None,
            providers: None,
            attachment: Attachment::Remote { url },
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
                if let Event::SpeechChunk { sample_rate, .. } = &event {
                    self.rate = *sample_rate;
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

    /// True while the device still has audio queued or being played.
    ///
    /// The device is the only honest source for this: the session reports the end of a turn before
    /// the device has finished speaking, so its status cannot say what is still audible.
    fn playing(&self) -> bool {
        self.player.as_ref().is_some_and(Player::playing)
    }

    /// Silences the device: whatever rodio still has queued is audio the assistant no longer wants
    /// to say. The samples already kept for `Replay` are untouched.
    ///
    /// Takes `&self` on purpose: the only thing it touches is the device, so a caller holding the
    /// live call's bookkeeping can silence the device without giving that borrow up.
    fn flush_playback(&self) {
        if let Some(player) = &self.player {
            player.stop();
        }
    }

    /// Lets go of the call and silences the device: the audio still queued belongs to a call that is
    /// over, which is the same rule the user's own barge-in follows (`flush_on_interrupt`).
    fn end_call(&mut self) {
        self.call = None;
        if flush_on_interrupt(true, self.playing()) {
            self.flush_playback();
        }
    }

    /// Starts a live call, or ends the one that is running.
    ///
    /// `self.call` is the client's own handle on the socket, so it — and not the HUD's indicator,
    /// which only the call task may raise — is what decides which way the key goes.
    fn toggle_call(&mut self) {
        if let Some(call) = self.call.take() {
            call.stop();
            return;
        }
        if let Err(why) = self.start_call() {
            // The mode is never silently on (Review Focus 3): a refusal is a sentence the user
            // reads, never a key press that quietly did nothing.
            self.state.notify(why);
        }
    }

    /// Opens the socket, or explains why it will not.
    ///
    /// The key is resolved the way every provider's is (`KeyResolver`: the environment first, then
    /// the credential store), and resolving it *here* is what makes a name nobody holds an answer to
    /// the key press instead of an asynchronous failure: no socket is touched until it has resolved.
    /// Only the key *name* reaches `CallSetup` — the value is never logged, never put in an event and
    /// never in an error message, and the handshake resolves the name again when it opens.
    fn start_call(&mut self) -> Result<(), String> {
        let realtime = self.config.realtime.clone();
        if !realtime.enabled {
            return Err(self.state.strings().live_off.to_string());
        }
        // A `--connect` client holds no resolver: the keys are the server's, and a call is opened
        // here, in this process. Refused as a sentence rather than a panic, because the mode is never
        // silently on (Review Focus 3).
        let Some(keys) = &self.keys else {
            return Err("a remote client holds no keys; live calls run in the client that owns them".to_string());
        };
        if keys.resolve(&realtime.key).map_err(|e| e.to_string())?.is_none() {
            return Err(format!(
                "no key named `{}`: set {} or store it",
                realtime.key,
                env_var_name(&realtime.key)
            ));
        }
        let events = self.events.clone().ok_or("the client's frame loop is not running")?;
        // Whatever the session was still saying belongs to the pipeline the call replaces.
        self.flush_playback();
        self.call = Some(Call::open(
            CallSetup::from_config(&self.config, Arc::new(keys.clone()), realtime.key),
            Heard::new(),
            events,
        ));
        Ok(())
    }

    /// Plays one spoken sentence, and shows the envelope of the reply it belongs to.
    fn on_speech(&mut self, samples: Vec<i16>) {
        if samples.is_empty() {
            return;
        }
        self.reply.extend_from_slice(&samples);
        let bytes = samples.len() * 2;
        if let Some(player) = &self.player {
            player.play(samples, self.rate);
        }
        // While a call is live the device is asked what it has actually played — the queue it still
        // holds is subtracted, because a response is generated faster than it is spoken and the
        // bytes merely handed over would overstate what the user heard (Review Focus 2). The device
        // is reachable only from this thread, so this is where the number is worked out.
        if let (Some(call), Some(player)) = (&self.call, &self.player) {
            let (remaining, pos) = player.progress();
            call.heard().push(bytes, remaining, pos.as_millis() as u64);
        }
        // The oscilloscope shows the reply as it is heard, so it is rebuilt as it grows.
        self.state.start_voice(envelope(&i16_to_f32(&self.reply), self.rate));
    }

    /// Acts on what the call's socket task decided since the last frame.
    ///
    /// A barge-in is the one thing the socket task cannot do itself: stopping the device means
    /// touching the `Player`, which lives on this thread. The task marks it, the frame loop flushes,
    /// and the count starts again from nothing — the chunks it just dropped were never heard.
    fn sync_call(&mut self) {
        // The bookkeeping is an `Arc` shared with the socket task, so the handle on the client is
        // only read to reach it: nothing here borrows the client while the device is flushed.
        let Some(heard) = self.call.as_ref().map(|call| Arc::clone(call.heard())) else {
            return;
        };
        if heard.take_interrupted() {
            // The count first: whatever the device had queued when the flag was raised was not
            // heard, and a stale reading left behind after the flush would overstate it.
            heard.clear();
            self.flush_playback();
        }
    }

    /// Forwards every captured frame and shows its loudness in the oscilloscope.
    ///
    /// Every frame goes to exactly one place, and [`mic_route`] is where that is decided: the login's
    /// own capture first — it is the only thing on screen, its answer is spoken rather than typed, and
    /// a live call is not what is being answered — then a live call, then the session's listening
    /// path. With no stream open there is nothing to route at all, which is the whole of the fix: a
    /// queue that emptied before `Mic::start` succeeded used to send audio the spec promises never
    /// leaves the machine (spec §9, §10).
    fn drain_mic(&mut self, to_server: &UnboundedSender<ToServer>) {
        let route = mic_route(self.mic.is_some(), self.login_capture(), self.call.is_some());
        while let Ok(samples) = self.mic_frames.try_recv() {
            match (route, &self.call) {
                // A call owns the microphone while it is live: the session's own
                // `ListenStart`/`ListenStop` bookkeeping must not see frames it never asked for.
                (MicRoute::Call, Some(call)) => call.push_mic(samples.clone()),
                (MicRoute::Session, _) => {
                    let _ = to_server.send(ToServer::Audio(Frame::Mic(samples.clone())));
                }
                // No capture is running — or the call this frame was routed to is gone, which is the
                // same thing one frame later: the frame is dropped rather than kept, exactly as it is
                // dropped when the HUD's own microphone is shut.
                (MicRoute::Drop | MicRoute::Call, _) => {}
            }
            self.state.push_mic_level(rms(&i16_to_f32(&samples)) * self.mic_gain);
        }
    }

    /// True while a login capture should be running: the screen is up, it has a voice switch, and its
    /// state says the microphone is open. That state is the whole of the authority — it is what the
    /// third refusal and a failed answer lower — so this is a read and never a decision, and it says
    /// nothing about the user's own switch or about the device.
    fn login_capture(&self) -> bool {
        self.state.login.as_ref().is_some_and(|login| login.voice && login.open)
    }

    /// Reconciles the microphone with [`Client::login_capture`]: the server's own `LoginStart` /
    /// `LoginStop` are sent **on the edges** of that state, never on its value, because the two are
    /// not the same thing any more than a key press is.
    ///
    /// The reducer cannot raise an action from `apply` — a refusal that stops listening is expressed
    /// as `LoginState::open == false` and nothing else — so the client reads that state once a frame
    /// and tells the server what it is doing. It touches nothing of the user's: `state.mic_open` is
    /// the HUD's own switch, and this screen has no business writing it (spec §6.2, review finding 3).
    /// It runs before `sync_mic`, so the device a `LoginStart` asks for is opened before any frame is
    /// routed (`drain_mic`).
    fn sync_login_mic(&mut self, to_server: &UnboundedSender<ToServer>) {
        // `login_mic` is the request in flight, not the capture: a `LoginStart` that has gone out has
        // to be answered by a `LoginStop` even if the capture has already given up on it.
        if !self.login_mic && self.login_capture() {
            if to_server.send(ToServer::Command(Command::LoginStart)).is_ok() {
                self.login_mic = true;
            }
        } else if self.login_mic && !self.login_capture() {
            self.close_login_mic(to_server);
        }
    }

    /// Ends the login capture: the server is told to stop, and the device is let go by the next
    /// `sync_mic` — which is now a step of its own, because the capture is no longer written into the
    /// user's microphone switch. Calling it with the capture already down is harmless, which is what
    /// lets `UiAction::LoginDone` and the reconciliation both end it.
    fn close_login_mic(&mut self, to_server: &UnboundedSender<ToServer>) {
        self.login_mic = false;
        let _ = to_server.send(ToServer::Command(Command::LoginStop));
    }

    /// Opens or closes the microphone so it matches what wants it: the user's own switch, a live call
    /// that streams until it ends, or the login capture, which is the screen's own state.
    ///
    /// Returns whether a stream is open afterwards, which is what `drain_mic` routes on. A device that
    /// refuses to open also ends the login capture and says why on the screen's own line (spec §9):
    /// leaving it claiming a microphone it does not have is the other half of the same defect.
    fn sync_mic(&mut self) -> bool {
        let wanted = mic_wanted(self.state.mic_open, self.call.is_some(), self.login_capture());
        match (wanted, self.mic.is_some()) {
            (true, false) => {
                let tx = self.mic_tx.clone();
                match Mic::start(move |samples| {
                    // Bounded and lossy: a busy client drops frames rather than stalling the device.
                    let _ = tx.try_send(samples);
                }) {
                    Ok(mic) => self.mic = Some(mic),
                    Err(e) => {
                        // Nothing to capture from; leave the HUD saying so.
                        self.state.mic_open = false;
                        let no_mic = self.state.strings().login_no_mic;
                        if self.state.login.is_some() {
                            // The screen's own line, because the HUD's notice is behind the veil and
                            // nobody can read it there (spec §9). `open = false` stops the capture and
                            // the client's own gate stops asking for a stream, so the request is retired
                            // here rather than with a `LoginStop`: the server's stop sets a flag, clears
                            // a buffer and cancels a token — it sends no event and touches no screen
                            // state — so a stop would be a round trip that changes nothing a user can
                            // see (the scoped re-review caught this comment claiming otherwise).
                            if let Some(login) = self.state.login.as_mut() {
                                login.line = LoginLine::Failed(no_mic.to_string());
                                login.open = false;
                            }
                            self.login_mic = false;
                        } else {
                            self.state.notify(e);
                        }
                    }
                }
            }
            (false, true) => self.mic = None,
            _ => {}
        }
        self.mic.is_some()
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
            ConfigEffect::RegenerateToken => self.regenerate_token(),
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
        // Two shapes, one conversion out: the screen's form is not the wire's view (§4.3).
        let config = ConfigView::from(form);
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

/// Whether the audio the device has queued must be dropped right now.
///
/// The two inputs are the whole rule: the user is interrupting (`interrupting`) and there is
/// something left to drop (`playing`). Nothing else flushes. A status change in particular is never
/// a trigger: a reply that finishes normally crosses `Speaking` → `Idle` (`brain.rs:110-115`,
/// `:153-158`) while the device is still speaking it out, so flushing there would cut every reply
/// off mid-sentence — and, because rodio's `clear` pauses the sink, leave the rest of the session
/// silent.
fn flush_on_interrupt(interrupting: bool, playing: bool) -> bool {
    interrupting && playing
}

/// Whether a live-state event ends the call here.
///
/// The socket task decides when a call is over — it says so with `Live { active: false }` — and the
/// client's only job is to let the handle go and quiet the device. A `Live { active: true }` is a
/// call starting, which `start_call` already holds the handle for.
fn call_ended(event: &Event) -> bool {
    matches!(event, Event::Live { active: false })
}

/// Whether the microphone stream should be open: the user asked for it — with the microphone key, or
/// because the login capture said so — or a call is live and streams it for as long as it lasts.
///
/// The login capture is its own reason and never a reason to write the switch: with the door shut
/// there is exactly one microphone that matters, and it is the screen's, not the HUD's (spec §6.2,
/// review finding 3).
fn mic_wanted(mic_open: bool, live: bool, login: bool) -> bool {
    mic_open || live || login
}

/// Where one captured frame goes, or nowhere at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicRoute {
    /// To the session as a `Frame::Mic`: the listening path, and the login's own detector on the
    /// server.
    Session,
    /// To the live call, which owns the microphone while it lasts.
    Call,
    /// Nowhere: no capture is running, so there is nothing to send and nothing to keep.
    Drop,
}

/// The one routing decision `drain_mic` makes, as a function of what is open.
///
/// `device` is `Mic::is_some()` and it comes first on purpose: a frame belongs to a *stream*, and
/// "the screen wants a capture" is not the same fact (spec §9: a denied microphone means no stream,
/// no `LoginStart`). The login beats a live call because it is the only thing on screen and its answer
/// is not the call's.
fn mic_route(device: bool, login: bool, live: bool) -> MicRoute {
    match (device, login, live) {
        (false, _, _) => MicRoute::Drop,
        (true, true, _) => MicRoute::Session,
        (true, false, true) => MicRoute::Call,
        (true, false, false) => MicRoute::Session,
    }
}

/// Whether a command means the user is taking the floor while a call is live.
///
/// In push mode these verbs are the microphone key and the cancel; in a call the microphone is
/// already open for the whole call, so both directions of that key mean the same thing — "stop
/// talking, I am talking" — and they are answered locally with `response.cancel` rather than sent to
/// a session that is not driving the conversation (Review Focus 2's safe side; the server's own VAD
/// is what reports a barge-in and what knows the number).
fn takes_the_floor(command: &Command) -> bool {
    matches!(
        command,
        Command::Ask { .. }
            | Command::Say { .. }
            | Command::ListenStart
            | Command::ListenStop
            | Command::Cancel
    )
}

/// Whether a command starts the next turn, which is what makes the audio kept for `Replay` stale.
///
/// The session cancels the running turn and starts another one for `Ask`, `Say` and `ListenStop`
/// (`session.rs:283-287`); `Reset` starts the conversation over (`session.rs:272-281`).
fn starts_turn(command: &Command) -> bool {
    match command {
        // The session ignores an empty question and a bare `!` (`session.rs:233-238`, `:242-249`),
        // so those begin nothing and must not silence a reply in progress.
        Command::Ask { text } | Command::Say { text } => !text.trim().is_empty(),
        Command::ListenStop | Command::Reset => true,
        // A Test is the one command the native client answers itself, in `test_provider`: it makes a
        // real call to a provider, but it starts no turn, speaks nothing of its own and writes
        // nothing to the log. This arm is therefore load-bearing for the screen as well —
        // `test_interrupts_playback` reads the decision from it — and may not be flipped to `true`
        // without also deciding that a one-word probe should cut a sentence in half.
        Command::Hello { .. }
        | Command::TestProvider { .. }
        | Command::ListenStart
        | Command::Cancel
        | Command::SetProfile { .. }
        | Command::SetListeningMode { .. } => false,
        // The login screen's three commands belong to the login capture, never to the conversation:
        // they start no turn, and Task 6 routes them to the screen rather than through this decision.
        Command::LoginStart | Command::LoginStop | Command::LoginAnswer { .. } => false,
        // Task 1.1's three: reading, writing and keying the configuration start no turn, speak
        // nothing and write nothing to the log. Task 1.3 gives them their behaviour; this task only
        // keeps the decision exhaustive.
        Command::GetConfig | Command::UpdateConfig { .. } | Command::SetKey { .. } => false,
    }
}

/// Whether the screen's Test quiets the audio the device is still speaking.
///
/// It does not, and the answer comes from the same rule [`starts_turn`] applies to commands: the
/// probe is a diagnostic — it says nothing out loud, plays nothing (the speech probe counts the
/// samples and drops them) and adds nothing to the log — so a reply in progress keeps playing. This
/// function exists so the screen's path, which never passes through `UiAction::Send`, takes that
/// decision deliberately instead of inheriting it from an arm that only ever existed to make a match
/// exhaustive.
fn test_interrupts_playback(capability: Capability) -> bool {
    starts_turn(&Command::TestProvider { capability })
}

/// Whether the queued audio has to go because the turn it belongs to is over.
///
/// Everything that starts a turn leaves nothing of the previous one to hear; `Cancel`
/// (`session.rs:266-270`) is the one command that ends a turn without starting another.
fn interrupts_playback(command: &Command) -> bool {
    starts_turn(command) || matches!(command, Command::Cancel)
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

/// The hit whose clickable title contains the click, if any.
fn hit_at(hits: &[Hit], column: u16, row: u16) -> Option<Hit> {
    hits.iter().copied().find(|hit| {
        column >= hit.area.x && column < hit.area.right() && row >= hit.area.y && row < hit.area.bottom()
    })
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
mod tests {
    use super::*;
    use tipsy_protocol::{AssistantState, ClientKind, ListeningMode, LoginView, ProfileView, UiLanguage};

    use crate::config::form::form_of;
    use crate::keys::{KeyResolver, MemoryStore, SecretStore};
    use crate::providers::ProviderSlot;
    use tipsy_ui::{ConfigAction, LOGIN_AUTO_REARM, NoticeKind};
    /// A resolver that knows one key — `openrouter`, the name the default configuration uses — and
    /// reads no environment.
    fn resolver() -> KeyResolver {
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
    fn mock_client() -> Client {
        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        Client::new(config, PathBuf::from("config.toml"), keys, providers)
    }

    fn welcome(login: Option<LoginView>) -> Event {
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
    fn sync_login(client: &mut Client) -> Vec<Command> {
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

    /// Audio frames queued for the client, as the microphone thread queues them.
    fn queue_frames(client: &Client, frames: &[&[i16]]) {
        for frame in frames {
            client.mic_tx.try_send(frame.to_vec()).expect("the queue is bounded but empty here");
        }
    }

    /// Review finding 1: the login's frames are routed by the state, but **the device is what gates
    /// them**. With no stream open there is nothing to drain, so a frame already sitting in the queue
    /// — what a screen that opens mid-capture meets — must not reach the server (spec §9's "no stream,
    /// no `LoginStart`" and §10's "nothing leaves until it is speech").
    ///
    /// This is the test the reviewer could not find: no test on this machine can open a device, and
    /// the whole of the failure path is reachable from that one fact.
    #[test]
    fn a_login_capture_with_no_stream_forwards_no_audio_at_all() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
        assert!(client.login_capture(), "the screen wants a capture");
        assert!(client.mic.is_none(), "and the device cannot be opened by a test");
        let wanted = mic_wanted(client.state.mic_open, false, client.login_capture());
        assert!(wanted, "the stream *is* wanted: this test is about the frame before it exists");
        queue_frames(&client, &[&[1, 2, 3], &[4, 5, 6]]);

        let (to_server, mut sent) = tokio::sync::mpsc::unbounded_channel();
        client.drain_mic(&to_server);

        assert!(sent.try_recv().is_err(), "a capture with no stream must forward no audio");
        assert!(client.mic_frames.try_recv().is_err(), "and the frames are not kept either");
    }

    /// The routing decision itself, as a table: the login capture goes to the session even while a
    /// call is live, a call keeps its own frames otherwise, and **nothing goes anywhere without a
    /// device**. `Mic` wraps a `cpal::Stream`, so a test cannot build one; this is the half of
    /// `drain_mic` a test can pin, and it is the half the defect was in.
    #[test]
    fn the_login_capture_is_routed_to_the_session_and_needs_a_device() {
        use MicRoute::{Call, Drop, Session};
        // (device open, login capture, a call is live) → where the frame goes
        for (open, login, live, want) in [
            (false, true, false, Drop),
            (false, true, true, Drop),
            (false, false, false, Drop),
            (false, false, true, Drop),
            (true, true, true, Session),
            (true, true, false, Session),
            (true, false, true, Call),
            (true, false, false, Session),
        ] {
            assert_eq!(mic_route(open, login, live), want, "device={open} login={login} live={live}");
        }
    }

    /// Review finding 3: the login screen must not touch the switch the user set for themselves. A
    /// user who had `Tab`-toggled the microphone on before the door finds it exactly as they left it
    /// afterwards — the spec's own words are "unless the listening mode wants it open", and the switch
    /// is where that is written down.
    #[test]
    fn the_login_screen_leaves_the_huds_microphone_switch_alone() {
        for before in [false, true] {
            let mut client = mock_client();
            let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
            client.state.mic_open = before;
            client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
            assert_eq!(sync_login(&mut client), vec![Command::LoginStart], "before = {before}");
            assert_eq!(client.state.mic_open, before, "the screen does not touch the user's switch");

            // The door: the screen goes, the capture goes, and what the microphone returns to is the
            // switch's own value — the "unless the listening mode wants it open" the spec asks for.
            let (events, _results) = tokio::sync::mpsc::unbounded_channel();
            client.on_action(UiAction::LoginDone, &to_server, &events);
            client.sync_login_mic(&to_server);

            assert!(client.state.login.is_none(), "the screen is down");
            assert_eq!(client.state.mic_open, before, "and the user's switch is where they left it");
            assert_eq!(
                mic_wanted(client.state.mic_open, client.call.is_some(), client.login_capture()),
                before,
                "the device follows that switch, not the screen (before = {before})"
            );
            assert_eq!(commands.try_recv().unwrap(), ToServer::Command(Command::LoginStop));
            assert!(commands.try_recv().is_err(), "and nothing else was sent");
        }
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

    /// The switch is the whole difference: with `voice = false` no stream is asked for at all, so
    /// nothing is recorded and no audio leaves the machine (spec §5, §10).
    #[test]
    fn the_voice_switch_off_opens_no_stream_and_sends_nothing() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: false }))));
        assert!(sync_login(&mut client).is_empty(), "there is no capture to start");
        assert!(!client.state.mic_open, "and no microphone to light the badge");
        assert!(client.mic.is_none(), "and no device open");
        // `Tab` has nothing to toggle, either: the screen's own state refuses it for the same reason.
        assert!(client.state.on_key(KeyInput::Tab).is_empty());
    }

    /// The capture is a **state the client reconciles** and not an action the reducer sends: a
    /// refusal that stops listening is expressed as `LoginState::open == false`, and this is where
    /// that becomes a `LoginStop` and a closed stream (spec §6.2).
    #[test]
    fn the_capture_opens_and_closes_with_the_login_state() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart]);
        // The three refusals in a row shut the capture from the screen's own state, and the client
        // follows it on the next reconciliation.
        for _ in 0..LOGIN_AUTO_REARM {
            client.on_message(&to_server, ToClient::Event(Event::LoginVerdict { ok: false, name: None }));
        }
        let login = client.state.login.as_ref().unwrap();
        assert!(!login.open, "the third refusal stops the automatic re-arm");
        assert_eq!(sync_login(&mut client), vec![Command::LoginStop], "and the server is told to stop");
        assert!(!client.state.mic_open, "and the badge goes out");
        assert!(client.mic.is_none(), "and the device is let go");
        // `Tab` is consent: it re-arms and the client follows it back the other way.
        assert_eq!(client.state.on_key(KeyInput::Tab), vec![UiAction::Send(Command::LoginStart)]);
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart]);
    }

    /// `UiAction::LoginDone` is the door: the screen goes, the capture stops, and the microphone
    /// returns to what the listening mode wanted **rather than staying open for the login** — and
    /// "what the listening mode wanted" is the user's own switch, which this fixture has on.
    #[test]
    fn the_door_closes_the_screen_and_the_capture() {
        let mut client = mock_client();
        client.state.mic_open = true;
        client.state.show_login(true);
        client.state.login.as_mut().unwrap().open = true;
        // The capture as the frame loop would have it: reconciled once, with no second start.
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart]);
        assert_eq!(sync_login(&mut client), Vec::<Command>::new(), "and never twice");

        let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::LoginDone, &to_server, &events);
        client.sync_login_mic(&to_server);

        assert!(client.state.login.is_none(), "the screen is down");
        assert!(
            client.state.mic_open,
            "the microphone returns to the switch the user set, not to off: review finding 3"
        );
        assert!(client.mic.is_none(), "no device in this fixture");
        assert_eq!(
            commands.try_recv().unwrap(),
            ToServer::Command(Command::LoginStop),
            "the server must know the capture is over"
        );
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

    /// A pass ends the capture and tells the server so (spec §6.2). The state closes `open` on the
    /// verdict, and this reconciliation is what turns that falling edge into the `LoginStop`: without
    /// it the session's own capture stays active (`session.rs:346-349` routes frames to the login
    /// while it is), so a word said after "Pasa, Andrés." was still judged and could shut the door.
    #[test]
    fn a_pass_sends_login_stop_and_ends_the_capture() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
        assert!(client.login_capture(), "the capture opens with the screen");
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart], "and the server is told");

        client.on_message(&to_server, ToClient::Event(Event::LoginVerdict { ok: true, name: Some("Andrés".into()) }));

        assert!(!client.login_capture(), "the pass ends the capture");
        assert_eq!(sync_login(&mut client), vec![Command::LoginStop], "and the server hears about it");
        assert!(client.state.login.is_some(), "the screen stays until Enter, as the spec says");
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

    /// A failed answer closes the capture — the microphone is off rather than repeating a failure —
    /// and the client follows the state, exactly as it does for the third refusal. What it does not
    /// touch is the user's own switch (review finding 3): the fixture has it on, and it stays on.
    #[test]
    fn a_failed_answer_turns_the_capture_off() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.state.mic_open = true;
        client.state.show_login(true);
        client.state.login.as_mut().unwrap().open = true;
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart]);

        client.on_message(&to_server, ToClient::Event(Event::Error {
            code: tipsy_protocol::ErrorCode::NoKey,
            message: "no key named `openrouter`".into(),
            recoverable: true,
        }));
        assert!(!client.state.login.as_ref().unwrap().open);
        assert_eq!(sync_login(&mut client), vec![Command::LoginStop]);
        assert!(client.state.mic_open, "the switch the user set is not the screen's to move");
        assert!(!client.login_capture(), "but the login's own capture is over");
        assert!(
            mic_wanted(client.state.mic_open, false, client.login_capture()),
            "and the microphone stays open for the reason the user opened it"
        );
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

    #[test]
    fn only_an_interrupt_with_audio_left_to_drop_flushes() {
        // Barge-in, or the cancel the user asked for, while the reply is being spoken: the rest of
        // the queued audio goes.
        assert!(flush_on_interrupt(true, true));
        // An interrupt with nothing queued has nothing to drop — a cancel that arrives while the
        // session is still thinking, with no sentence synthesised yet.
        assert!(!flush_on_interrupt(true, false));
        // The status change that ends a normal reply. `Done` and `Idle` arrive while the device is
        // still speaking, so this row is what keeps every completed reply whole.
        assert!(!flush_on_interrupt(false, true));
        assert!(!flush_on_interrupt(false, false));
    }

    #[test]
    fn the_commands_that_end_a_turn_are_the_ones_that_interrupt_it() {
        // Barge-in: the session cancels the running turn and starts another one in its place.
        for command in [
            Command::Ask { text: "¿qué pasa?".into() },
            Command::Say { text: "hola compadre".into() },
            Command::ListenStop,
            Command::Reset,
        ] {
            assert!(interrupts_playback(&command), "{command:?} ends the running turn");
        }
        // `Cancel` ends a turn without starting one, and is the other half of the rule.
        assert!(interrupts_playback(&Command::Cancel));
        // Nothing else quiets the device: opening the microphone leaves the reply audible until the
        // utterance that follows it, and the session ignores the empty question a bare `!` sends.
        for command in [
            Command::ListenStart,
            Command::Ask { text: String::new() },
            Command::Say { text: "   ".into() },
            Command::SetProfile { id: "compae".into() },
        ] {
            assert!(!interrupts_playback(&command), "{command:?} leaves the reply alone");
        }
    }

    /// The three reasons a stream may be open, and the two ways it is closed. The login capture is its
    /// own reason (review finding 3): it needs no help from the user's switch, and it leaves it alone.
    #[test]
    fn the_stream_opens_for_the_switch_the_call_or_the_login_capture() {
        assert!(mic_wanted(true, false, false), "the microphone key still opens it");
        assert!(mic_wanted(false, true, false), "a call streams the microphone for as long as it lasts");
        assert!(mic_wanted(false, false, true), "and the login capture opens it with the screen");
        assert!(mic_wanted(true, true, true));
        assert!(!mic_wanted(false, false, false), "with none of the three there is nothing to open");
        // The switch is not consulted for the login's sake and never written by the screen: the value
        // it held is the value it keeps, whichever of the three opened the stream.
        for switch in [false, true] {
            assert_eq!(mic_wanted(switch, false, false), switch, "the switch alone is the switch");
        }
    }

    #[test]
    fn a_chunk_header_sets_the_rate_the_device_is_told() {
        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(Event::SpeechChunk { seq: 0, sample_rate: 24000 }));
        client.on_message(&to_server, ToClient::Audio(Frame::Speech { seq: 0, samples: vec![5, 6] }));
        // There is no output device in a test — `Player::new` needs one — so the playback itself is
        // skipped; what is pinned is the rate it would be handed, which is the announced one and
        // never a guess about which mode is running.
        assert_eq!(client.rate, 24000);
        assert_eq!(client.reply, vec![5, 6], "the reply is kept for `F2` as it always was");
    }

    #[test]
    fn the_client_lets_go_of_a_call_when_the_socket_says_it_is_over() {
        // A `Call` needs a socket, so what this pins is the rule: a call's ending is a live-flag
        // event and nothing else. A status change must never be mistaken for it — that is what
        // stops a finished reply from being cut off mid-sentence (`flush_on_interrupt`).
        assert!(call_ended(&Event::Live { active: false }));
        assert!(!call_ended(&Event::Live { active: true }));
        assert!(!call_ended(&Event::Status { state: AssistantState::Idle, detail: None }));
        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(Event::Live { active: false }));
        assert!(client.call.is_none(), "there was no call, and none is invented");
    }

    #[test]
    fn a_call_never_starts_while_the_mode_is_disabled() {
        let mut config = Config::default();
        config.realtime.enabled = false;
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);
        let why = client.start_call().expect_err("a disabled mode starts nothing");
        assert!(why.contains("enabled"), "the refusal names the setting: {why}");
        assert!(client.call.is_none(), "Review Focus 3: the mode is never silently on");
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
        let mut remote = Client::remote(config, path.clone(), "ws://127.0.0.1:7070/ws".into());
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

    #[test]
    fn a_finished_reply_keeps_its_audio_until_a_new_turn_starts() {
        let (to_server, _from_client) = tokio::sync::mpsc::unbounded_channel();
        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);

        client.on_message(&to_server, ToClient::Event(Event::Status { state: AssistantState::Speaking, detail: None }));
        client.on_message(&to_server, ToClient::Event(Event::SpeechChunk { seq: 0, sample_rate: 24000 }));
        client.on_message(&to_server, ToClient::Audio(Frame::Speech { seq: 0, samples: vec![7, 8, 9] }));
        client.on_message(&to_server, ToClient::Event(Event::Reply { text: "Aquí andamos.".into() }));
        client.on_message(&to_server, ToClient::Event(Event::Done { first_audio_ms: Some(1), total_ms: 2 }));
        assert_eq!(client.reply, vec![7, 8, 9], "`F2` has the reply as soon as it is spoken");

        // The idle that ends every normal reply: the device is still speaking, so it must keep both
        // the queue and what `F2` replays.
        client.on_message(&to_server, ToClient::Event(Event::Status { state: AssistantState::Idle, detail: None }));
        assert_eq!(client.reply, vec![7, 8, 9], "a normal `Speaking -> Idle` keeps the last reply");

        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Send(Command::Ask { text: "¿y eso?".into() }), &to_server, &events);
        assert!(client.reply.is_empty(), "the next turn speaks a reply of its own");
    }

    #[test]
    fn the_verbs_that_take_the_floor_are_the_ones_a_call_answers_locally() {
        for command in [
            Command::Ask { text: "¿qué pasa?".into() },
            Command::Say { text: "hola compadre".into() },
            Command::ListenStart,
            Command::ListenStop,
            Command::Cancel,
        ] {
            assert!(takes_the_floor(&command), "{command:?} means the user is taking the floor");
        }
        for command in [
            Command::Reset,
            Command::SetProfile { id: "meigo".into() },
            Command::SetListeningMode { mode: ListeningMode::Push },
            Command::Hello { token: None, client: ClientKind::Tui, protocol_version: 1 },
        ] {
            assert!(!takes_the_floor(&command), "{command:?} is not an interruption");
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

    /// The interruption decision Task 7 had to take, named and proved rather than inherited: a Test
    /// makes a real call, but it is a diagnostic — it speaks nothing of its own, writes nothing to
    /// the log and starts no turn — so it never quiets the reply being spoken. The answer comes from
    /// `starts_turn`, the one helper that decides whether a command interrupts, so flipping that arm
    /// flips this too.
    #[test]
    fn a_test_probe_is_not_a_turn_and_leaves_the_playing_audio_alone() {
        for capability in [Capability::Chat, Capability::Speech, Capability::Transcribe] {
            assert!(
                !test_interrupts_playback(capability),
                "{capability} is a diagnostic, not a turn"
            );
            assert!(!interrupts_playback(&Command::TestProvider { capability }));
        }
        // The whole rule at once: audio is queued and the user asked for a Test, and the audio stays
        // where it is. Had this been true, a one-word probe would cut a sentence in half.
        assert!(!flush_on_interrupt(test_interrupts_playback(Capability::Speech), true));
    }

    /// The one test that presses the key: `Tab` with a live call is the user taking the floor, and it
    /// has to reach the socket as `response.cancel` without ending the call. Every link is unit-tested
    /// on its own — `UiState::on_key(Tab)` → `ListenStart`, `takes_the_floor`, `Call::interrupt`, the
    /// socket task's control arm — and this is the one that drives them all the way to a real socket
    /// and back (the final review's item 7).
    #[tokio::test]
    async fn tab_with_a_live_call_cancels_the_response_and_leaves_the_call_running() {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        // A loopback socket that completes a realtime handshake, records what the call sends, and
        // answers a cancel with one audio delta — the proof that the same call is still reading.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let url = format!("ws://{}/v1/realtime?model=gpt-realtime-2.1", listener.local_addr().unwrap());
        let (frames, mut sent) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("the call connects");
            let mut socket = tokio_tungstenite::accept_async(stream).await.expect("the handshake");
            socket
                .send(Message::text(r#"{"type":"session.created","session":{"id":"sess_fixture"}}"#))
                .await
                .expect("the client is listening");
            while let Some(Ok(message)) = socket.next().await {
                let Message::Text(text) = message else { continue };
                let value: serde_json::Value = serde_json::from_str(text.as_str()).expect("JSON");
                let cancel = value["type"] == "response.cancel";
                let _ = frames.send(text.as_str().to_string());
                if cancel {
                    socket
                        .send(Message::text(
                            r#"{"type":"response.output_audio.delta","response_id":"resp_1","item_id":"item_a","content_index":0,"delta":"AAECAw=="}"#,
                        ))
                        .await
                        .expect("the call is listening");
                }
            }
        });

        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);
        // The call the key will interrupt: the loopback socket above, with an empty key *name* so the
        // handshake carries no `Authorization` header and no fixture here is a secret.
        let (events, mut from_call) = tokio::sync::mpsc::unbounded_channel();
        let options = crate::realtime::socket::CallOptions::at(&url);
        let setup = CallSetup {
            url,
            options,
            config: crate::config::RealtimeConfig { key: String::new(), ..Default::default() },
            resolver: Arc::new(KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None))),
            model: "gpt-realtime-2.1".into(),
            instructions: "eres un compae".into(),
            voice: "marin".into(),
            silence_ms: 1000,
            language: "es-ES".into(),
            limit: Duration::from_secs(600),
        };
        client.call = Some(Call::open(setup, Heard::new(), events.clone()));
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();

        // The session is configured first, so the key press meets a live call and not a handshake.
        let update = tokio::time::timeout(Duration::from_secs(5), sent.recv())
            .await
            .expect("the call never configured its session")
            .expect("the fake server is running");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&update).unwrap()["type"], "session.update");

        // The key press, through the same two functions the frame loop uses.
        for action in client.state.on_key(KeyInput::Tab) {
            client.on_action(action, &to_server, &events);
        }

        let cancel = tokio::time::timeout(Duration::from_secs(5), sent.recv())
            .await
            .expect("pressing Tab with a live call never sent `response.cancel`")
            .expect("the fake server is running");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&cancel).unwrap()["type"], "response.cancel");

        // And the call lives on: the same socket answers with the model's audio, which the client
        // reads exactly as it does in its frame loop.
        let mut spoke = false;
        while !spoke {
            let message = tokio::time::timeout(Duration::from_secs(5), from_call.recv())
                .await
                .expect("the call stopped reporting after the cancel")
                .expect("the call's channel");
            assert!(
                !matches!(message, ToClient::Event(Event::Live { active: false })),
                "a cancel is not an ending: {message:?}"
            );
            spoke = matches!(message, ToClient::Audio(Frame::Speech { .. }));
            client.on_message(&to_server, message);
        }
        assert!(client.call.is_some(), "the call is still the client's to end");
    }
}
