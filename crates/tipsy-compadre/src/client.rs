//! The native terminal client: keys and mouse in, HUD out, audio played and captured here.
//!
//! The client owns everything the protocol cannot carry: the terminal, the output device and the
//! microphone. It holds no conversation state beyond what it draws.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    self, EnableMouseCapture, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui::crossterm::execute;
use tipsy_protocol::{Capability, Command, Event, Frame, ToClient, ToServer};
use tipsy_ui::{ClockView, ConfigEffect, Hit, KeyInput, NoticeKind, UiAction, UiState, draw};
use tokio::sync::mpsc::UnboundedSender;

use crate::audio::device::{Mic, Player};
use crate::audio::pcm::{envelope, i16_to_f32, rms};
use crate::brain::BrainConfig;
use crate::config::form::{config_from, form_of, provider_config};
use crate::config::store;
use crate::config::Config;
use crate::keys::KeyResolver;
use crate::providers::{build_one, build_providers, ProviderSlot};
use crate::session::spawn_with_events;
use crate::sysmon;

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
    // `init` enters the alternate screen, enables raw mode and installs the panic hook that
    // restores the terminal; `restore` undoes all of it.
    let mut terminal = ratatui::init();
    // Mouse capture is not part of a bare terminal: without it the clickable titles do nothing.
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    if let Some(warning) = loaded.warning {
        client.state.notify(warning);
    }
    client.open_player();
    let outcome = client.frame_loop(&mut terminal);
    let _ = execute!(std::io::stdout(), event::DisableMouseCapture);
    ratatui::restore();
    outcome
}

/// Everything the client keeps from one frame to the next.
struct Client {
    state: UiState,
    /// The configuration the running session was built from: a save that worked replaces it, so the
    /// client always holds the configuration the file holds.
    config: Config,
    /// Where the screen writes, and where it reads every time it opens.
    config_path: PathBuf,
    /// The resolver the connectors are built with: the one the app started with.
    keys: KeyResolver,
    /// The live provider stack, shared with the session.
    providers: ProviderSlot,
    /// `None` when there is no output device: the HUD still works, silently.
    player: Option<Player>,
    /// The audio of the reply being spoken, kept for `Replay` until a new turn starts.
    reply: Vec<i16>,
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
            keys,
            providers,
            player: None,
            reply: Vec::new(),
            rate: 24000,
            mic_gain,
            mic: None,
            mic_frames,
            mic_tx,
        }
    }

    /// Opens the default output device. A machine without one keeps a silent HUD.
    fn open_player(&mut self) {
        match Player::new() {
            Ok(player) => self.player = Some(player),
            Err(e) => self.state.notify(e),
        }
    }

    /// The frame loop: input, messages from the session, audio and drawing, about 30 times a second.
    fn frame_loop(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<(), String> {
        let ends = spawn_with_events(self.config.clone(), self.providers.clone(), BrainConfig::default());
        let to_server = ends.to_server;
        let mut from_server = ends.from_server;
        // A second sender for the same client channel: the system monitor and the screen's Test
        // results (Task 7) both arrive with the session's own events.
        let events = ends.events.clone();
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
                self.on_message(message);
            }
            self.drain_mic(&to_server);
            self.sync_mic();

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
        Ok(())
    }

    fn on_message(&mut self, message: ToClient) {
        match message {
            ToClient::Event(event) => {
                if let Event::SpeechChunk { sample_rate, .. } = &event {
                    self.rate = *sample_rate;
                }
                // A Test result belongs to the screen's notice line, not to the HUD's log: the
                // answer stays beside the rows it is about. `UiState::apply` deliberately has
                // nothing to say about it.
                if let Event::TestResult { capability, ok, latency_ms, message } = &event
                    && let Some(form) = self.state.config_form.as_mut()
                {
                    form.test_result(*capability, *ok, *latency_ms, message.clone());
                }
                // No event ever touches the queued audio or the audio kept for `Replay`: the status
                // that ends a reply arrives while the device is still speaking it, so the only
                // interruption is one the user asks for, in `on_action`.
                self.state.apply(&event);
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
    fn flush_playback(&mut self) {
        if let Some(player) = &self.player {
            player.stop();
        }
    }

    /// Plays one spoken sentence, and shows the envelope of the reply it belongs to.
    fn on_speech(&mut self, samples: Vec<i16>) {
        if samples.is_empty() {
            return;
        }
        self.reply.extend_from_slice(&samples);
        if let Some(player) = &self.player {
            player.play(samples, self.rate);
        }
        // The oscilloscope shows the reply as it is heard, so it is rebuilt as it grows.
        self.state.start_voice(envelope(&i16_to_f32(&self.reply), self.rate));
    }

    /// Forwards every captured frame and shows its loudness in the oscilloscope.
    fn drain_mic(&mut self, to_server: &UnboundedSender<ToServer>) {
        while let Ok(samples) = self.mic_frames.try_recv() {
            let _ = to_server.send(ToServer::Audio(Frame::Mic(samples.clone())));
            self.state.push_mic_level(rms(&i16_to_f32(&samples)) * self.mic_gain);
        }
    }

    /// Opens or closes the microphone so it matches `state.mic_open`.
    fn sync_mic(&mut self) {
        match (self.state.mic_open, self.mic.is_some()) {
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
                        self.state.notify(e);
                    }
                }
            }
            (false, true) => self.mic = None,
            _ => {}
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
            // served out of a copy taken at startup.
            UiAction::OpenConfig => {
                self.open_screen();
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
            UiAction::Quit => true,
        }
    }

    /// Opens the configuration screen on a copy read from the file *now* (Review Focus 3): a
    /// `config.toml` edited while the app runs is what the screen shows. Opening changes nothing
    /// that is running — `Esc` discards the edits, and only a save applies them.
    ///
    /// The screen is put up filled only when the file was read: a read that fails leaves the
    /// placeholder form on screen, unmarked (finding G), and that form refuses `Ctrl+S` and `F4`
    /// rather than writing blank base URLs and models over the user's `config.toml`. The notice
    /// below is what tells the user why the screen is empty.
    fn open_screen(&mut self) {
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
            Err(e) => self.state.notify(format!("{}: {e}", self.config_path.display())),
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
            ConfigEffect::Test(capability) => self.test_provider(capability, events),
            ConfigEffect::RegenerateToken => self.regenerate_token(),
        }
    }

    /// Runs the screen's Test against the row on screen and reports the answer into the client's own
    /// event channel, so it is rendered like anything else the session says.
    ///
    /// The connector is built *here*, before the call: a row that names a key nobody holds is then
    /// answered at once, with no latency and without a single request — which is what makes "no key"
    /// a different answer from "the provider refused". The probe itself runs on a task of its own,
    /// so the HUD never waits for a provider.
    fn test_provider(&mut self, capability: Capability, events: &UnboundedSender<ToClient>) {
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
        let connector = match build_one(&cfg, capability, &self.keys) {
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
    fn save(&mut self, to_server: &UnboundedSender<ToServer>) {
        // 1. The form in the file's shape.
        let parsed = self.state.config_form.as_ref().map(config_from);
        let config = match parsed {
            None => return,
            Some(Err(message)) => return self.config_notice(NoticeKind::Error, message),
            Some(Ok(config)) => config,
        };
        // 2. The connectors that configuration names.
        let rebuilt = match build_providers(&config.providers, &self.keys) {
            Ok(providers) => providers,
            Err(e) => return self.config_notice(NoticeKind::Error, e.to_string()),
        };
        // 3. The file itself.
        if let Err(e) = store::save(&self.config_path, &config) {
            return self.config_notice(NoticeKind::Error, format!("{}: {e}", self.config_path.display()));
        }
        // 4. What is running: the stack in place, then the fields the channel carries.
        self.mic_gain = config.listening.mic_gain;
        self.providers.replace(rebuilt);
        let _ = to_server.send(ToServer::Command(Command::SetListeningMode { mode: config.listening.mode }));
        let _ = to_server.send(ToServer::Command(Command::SetProfile { id: config.active_profile.clone() }));
        self.config = config;
        if let Some(form) = self.state.config_form.as_mut() {
            form.dirty = false;
            form.notice = None;
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
    use tipsy_protocol::{AssistantState, ListeningMode};

    use crate::config::form::form_of;
    use crate::keys::{KeyResolver, MemoryStore, SecretStore};
    use crate::providers::ProviderSlot;
    use tipsy_ui::{ConfigAction, NoticeKind};
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

    #[test]
    fn a_finished_reply_keeps_its_audio_until_a_new_turn_starts() {
        let (to_server, _from_client) = tokio::sync::mpsc::unbounded_channel();
        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);

        client.on_message(ToClient::Event(Event::Status { state: AssistantState::Speaking, detail: None }));
        client.on_message(ToClient::Event(Event::SpeechChunk { seq: 0, sample_rate: 24000 }));
        client.on_message(ToClient::Audio(Frame::Speech { seq: 0, samples: vec![7, 8, 9] }));
        client.on_message(ToClient::Event(Event::Reply { text: "Aquí andamos.".into() }));
        client.on_message(ToClient::Event(Event::Done { first_audio_ms: Some(1), total_ms: 2 }));
        assert_eq!(client.reply, vec![7, 8, 9], "`F2` has the reply as soon as it is spoken");

        // The idle that ends every normal reply: the device is still speaking, so it must keep both
        // the queue and what `F2` replays.
        client.on_message(ToClient::Event(Event::Status { state: AssistantState::Idle, detail: None }));
        assert_eq!(client.reply, vec![7, 8, 9], "a normal `Speaking -> Idle` keeps the last reply");

        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Send(Command::Ask { text: "¿y eso?".into() }), &to_server, &events);
        assert!(client.reply.is_empty(), "the next turn speaks a reply of its own");
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

        client.open_screen();
        let first = client.state.config_form.as_ref().unwrap();
        assert_eq!(first.listening.silence_ms, "1000");
        assert!(first.server.token.is_empty());

        // The user edits the file in another window while the app keeps running.
        let mut edited = config.clone();
        edited.listening.silence_ms = 4321;
        edited.server.token = "pasted-by-hand".into();
        store::save(&path, &edited).unwrap();

        client.open_screen();
        let second = client.state.config_form.as_ref().unwrap();
        assert_eq!(second.listening.silence_ms, "4321", "the screen must read the file again");
        assert_eq!(second.server.token, "pasted-by-hand");
        assert_eq!(client.config, config, "opening the screen changes nothing that is running");
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
        client.open_screen();
        assert!(client.state.config_form.as_ref().unwrap().server.token.is_empty());

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
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

        client.on_message(message);
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

        client.on_message(message);
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
}
