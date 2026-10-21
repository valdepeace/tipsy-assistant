//! Interface state: everything the HUD draws, updated from server events and user input.

use std::collections::VecDeque;

use tipsy_protocol::{AssistantState, Command, ErrorCode, Event, ListeningMode, ProfileView, UiLanguage};

use crate::config_form::{ConfigAction, ConfigEffect, ConfigForm};
use crate::render::Hit;
use crate::strings::{Strings, strings};

const NOTICE_MS: u64 = 3000;
const HISTORY_LEN: usize = 120;
const MIC_LEVELS_MAX: usize = 400;
const ENVELOPE_HOP_MS: u64 = 50;
const MIC_TRANSITION_MS: u64 = 180;

/// The three effects with a native side, as the actions the client has to perform. The file write,
/// the connector probe and the token generator all live on the native side, which is the only thing
/// that may touch any of them; [`ConfigEffect::None`] is a no-op and reports nothing.
fn effects_to_actions(effects: Vec<ConfigEffect>) -> Vec<UiAction> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            ConfigEffect::None => None,
            ConfigEffect::Save => Some(UiAction::Config(ConfigAction::Save)),
            ConfigEffect::Test(_) => Some(UiAction::Config(ConfigAction::Test)),
            ConfigEffect::RegenerateToken => Some(UiAction::Config(ConfigAction::RegenerateToken)),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub who: Who,
    pub text: String,
    pub first_audio_ms: Option<u64>,
    pub error: Option<String>,
    pub pending: bool,
}

impl LogEntry {
    fn user(text: impl Into<String>) -> Self {
        Self { who: Who::User, text: text.into(), first_audio_ms: None, error: None, pending: false }
    }
    fn pending_assistant(text: impl Into<String>) -> Self {
        Self { who: Who::Assistant, text: text.into(), first_audio_ms: None, error: None, pending: true }
    }
}

/// True for an assistant entry that has not been said yet: what `Reply` and `Done` fill in. It is
/// still pending while the turn runs, and may carry an advisory error — a recoverable failure the
/// session is already retrying — which the reply must clear once it arrives.
fn reply_pending(entry: &LogEntry) -> bool {
    entry.who == Who::Assistant && entry.first_audio_ms.is_none()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Chat,
    Network,
}

/// Keys the HUD understands, independent of any terminal library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Esc,
    Up,
    Down,
    Left,
    Right,
    F(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    Send(Command),
    Quit,
    /// Replay the audio of the last completed reply (handled by the client, which kept it).
    Replay(usize),
    /// Open the configuration screen: the client loads `config.toml`, converts it to a
    /// [`ConfigForm`] and calls [`UiState::show_config_form`]. The HUD holds a default form until
    /// then, so the screen is usable the moment the key is pressed.
    OpenConfig,
    /// One action the screen's form raised a *native* effect for: the client applies it to the form
    /// it holds and performs the effect — a save writes the file, a Test probes a connector, a
    /// Regenerate mints a token. Applying it twice is harmless because those three actions only set
    /// the form dirty, which is what the form has already done here.
    Config(ConfigAction),
    /// Start a live call, or end the one that is running (Plan 2C). The client owns the socket and
    /// the HUD only asks: the indicator turns on when the socket says it is live, never here.
    ToggleLive,
    /// The door opened and the user pressed `Enter`: the client closes the screen and enters the HUD.
    LoginDone,
}

/// How many characters the login's box takes. A login answer is a sentence, and this is what keeps a
/// pasted novel out of the transcription path (spec §6.2).
pub const LOGIN_INPUT_MAX: usize = 200;

/// How many refusals in a row the screen keeps listening through before it waits for a key (spec L8).
pub const LOGIN_AUTO_REARM: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginPhase {
    /// No capture: the microphone is off or has not been opened yet.
    Waiting,
    /// The capture is open and nothing is in flight.
    Listening,
    /// An answer is with the server: typed or spoken, and a verdict is coming.
    Judging,
    Refused,
    Passed,
}

/// Which sentence the widget's one line shows (spec §6.1), in this precedence: `Passed`, `Refused`,
/// `Heard`, a failure, the transcription, and the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginLine {
    Prompt,
    Heard(String),
    Refused,
    /// An answer is with the transcriber. The server sends `Status { Transcribing }` and that is what
    /// sets this: the wait before a verdict is the one thing §6.1 keeps `strings.transcribing` for,
    /// and the login screen is the only screen where nothing the user does ends it.
    Transcribing,
    /// `NothingHeard`, whose wording is the UI's: the server's message for it is English.
    NothingHeard,
    /// Any other `Event::Error`: the provider's own words.
    Failed(String),
    Passed,
}

/// The login screen's own state (spec §6). `open` is the capture **the client reconciles the
/// microphone to**: the reducer cannot return an action from `apply`, so a refusal that stops listening
/// is expressed as a state the client reads, never as a command nobody could send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginState {
    pub input: String,
    /// The `Welcome.login.voice` switch: with it off there is no capture and `Tab` does nothing.
    pub voice: bool,
    pub open: bool,
    pub phase: LoginPhase,
    pub line: LoginLine,
    /// The matched entry's name, once the door opens.
    pub name: Option<String>,
    /// Refusals in a row: the streak `LOGIN_AUTO_REARM` guards.
    pub refusals: u32,
}

impl LoginState {
    /// True once the streak has stopped the automatic re-arm: the border swaps to the escape hint, and
    /// the microphone stays shut until `Tab` — which is why a locked-out owner is never stuck (spec §6).
    pub fn needs_a_key(&self) -> bool {
        self.refusals >= LOGIN_AUTO_REARM
    }

    /// Whether the door has opened. **`Passed` is terminal for the phase** (spec §6.2: the screen stays
    /// until `Enter`), so every writer of `phase` asks this first. Three of them could otherwise put the
    /// user back outside a door they are already through, and each is reachable: `Tab` would reopen the
    /// capture and reset the phase, a refusal arriving late would flip the border back to a refusal, and
    /// an `Event::Error` — including the `cancelled` that the pass's own `LoginStop` provokes by ending
    /// a login transcription in flight — would set the phase to `Waiting`, which is what made `Enter`
    /// stop entering.
    pub fn passed(&self) -> bool {
        self.phase == LoginPhase::Passed
    }
}

pub struct UiState {
    pub profile: Option<ProfileView>,
    pub lang: UiLanguage,
    pub listening_mode: ListeningMode,
    pub status: AssistantState,
    /// True for as long as a live call lasts (Plan 2C). The header says so for the whole call — the
    /// mode bills while it is open — and the flag is set by `Event::Live`, never by the key.
    pub live: bool,
    pub mic_open: bool,
    /// Client-local player/timeline reports queued, scheduled, or ongoing output.
    pub local_output: bool,
    /// Elapsed time for the one mic-on transition; advanced only by the client's frame delta.
    pub mic_transition_ms: Option<u64>,
    pub log: Vec<LogEntry>,
    pub selected: Option<usize>,
    pub input: String,
    pub tab: Tab,
    /// Text and remaining milliseconds.
    pub notice: Option<(String, u64)>,
    pub elapsed_ms: u64,
    pub turn_ms: u64,
    pub cpu: f32,
    pub ram: f32,
    pub net_in: f64,
    pub net_out: f64,
    pub cpu_history: VecDeque<f32>,
    pub net_history: VecDeque<(f64, f64)>,
    pub mic_levels: Vec<f32>,
    pub voice_envelope: Vec<f32>,
    pub voice_pos_ms: u64,
    /// The configuration screen's form while it is open: `None` is the closed HUD. Kept as plain
    /// values (Task 3) so this crate still knows nothing about `config.toml`.
    pub config_form: Option<ConfigForm>,
    /// The login screen while it is up: `None` is a client that has been let in, or one the server
    /// never asked (spec §3 — `Welcome.login` is the only authority).
    pub login: Option<LoginState>,
    /// Whether a login screen has been shown at all in this client's run. The login spec's rule is
    /// **once per client run** (login spec §3), and `login` alone cannot express it: the session
    /// re-sends `Welcome` on `SetProfile` and `SetListeningMode`, and when the door opens `login` goes
    /// back to `None` — which is exactly what it holds before the first screen is ever shown. This bit
    /// is the difference between "asked already and let in" and "never asked", and without it a save
    /// from the configuration screen reopens a door the user has already walked through.
    pub login_seen: bool,
    /// What the running server reported about its own binary (`Welcome.web_bundle`, spec §5.4):
    /// `false` means the web UI is not available in this build.
    pub web_bundle: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        Self {
            profile: None,
            lang: UiLanguage::Es,
            listening_mode: ListeningMode::Push,
            status: AssistantState::Idle,
            live: false,
            mic_open: false,
            local_output: false,
            mic_transition_ms: None,
            log: Vec::new(),
            selected: None,
            input: String::new(),
            tab: Tab::Chat,
            notice: None,
            elapsed_ms: 0,
            turn_ms: 0,
            cpu: 0.0,
            ram: 0.0,
            net_in: 0.0,
            net_out: 0.0,
            cpu_history: VecDeque::from(vec![0.0; HISTORY_LEN]),
            net_history: VecDeque::from(vec![(0.0, 0.0); HISTORY_LEN]),
            mic_levels: Vec::new(),
            voice_envelope: Vec::new(),
            voice_pos_ms: 0,
            config_form: None,
            login: None,
            login_seen: false,
            web_bundle: false,
        }
    }

    pub fn strings(&self) -> &'static Strings {
        strings(self.lang)
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), NOTICE_MS));
    }

    /// Opens the configuration screen, or closes it if it is already open.
    fn toggle_config_form(&mut self) -> Vec<UiAction> {
        match self.config_form.take() {
            // Closing leaves the HUD exactly as it was: the screen never touched the log, the input
            // or the microphone, and the half-typed edit goes with the form.
            Some(_) => Vec::new(),
            None => {
                self.config_form = Some(ConfigForm::default());
                vec![UiAction::OpenConfig]
            }
        }
    }

    /// Puts the loaded configuration on screen: the client's answer to [`UiAction::OpenConfig`].
    /// The form arrives from the client's own conversion of the file, so it is the one form that
    /// carries the user's configuration — and marking it filled is what lets it be saved. The
    /// placeholder the screen opens on before this call is not filled, and cannot be.
    pub fn show_config_form(&mut self, form: ConfigForm) {
        let mut form = form;
        form.mark_filled();
        self.config_form = Some(form);
    }

    /// Shows the login screen, the client's answer to `Welcome.login` — **once per client run**, which
    /// is what the login spec requires (§3) and what a bare assignment cannot deliver: the session
    /// re-sends `Welcome` on `SetProfile` and `SetListeningMode`, so a screen shown again would wipe a
    /// half-typed answer, reset the phase and restart the refusal streak on a keypress the user never
    /// made. Returns whether this call was the one that showed it, so a caller can run its first-show
    /// setup (the capture switch and the phase) without asking the state a second time.
    pub fn show_login(&mut self, voice: bool) -> bool {
        if self.login_seen {
            return false;
        }
        self.login_seen = true;
        self.login = Some(LoginState {
            input: String::new(),
            voice,
            open: false,
            phase: LoginPhase::Waiting,
            line: LoginLine::Prompt,
            name: None,
            refusals: 0,
        });
        true
    }

    /// A click, dispatched the way the control that reported it meant it.
    ///
    /// The screen's controls name their own target in `hit.action` — a tab, a row, a button — so a
    /// click is *not* turned into the keystroke of whatever happens to be focused. A tab or a row is
    /// handed to the form as the action it is; the two buttons and the token generator are the same
    /// actions their keys are, so they take the key path and the two cannot drift apart. `hit.key`
    /// is the fallback for the controls that are keys in the hand — the header's two, the HUD's own
    /// three — and it is read through [`UiState::on_key`], so the door into the screen answers a
    /// click exactly as it answers `F9`.
    pub fn on_hit(&mut self, hit: &Hit) -> Vec<UiAction> {
        let Some(action) = hit.action else {
            return self.on_key(hit.key);
        };
        if self.config_form.is_none() {
            // The HUD's own controls: a click is the keystroke they already answer.
            return self.on_key(hit.key);
        }
        match action {
            // The two buttons and the generator are the actions their keys raise; going through the
            // key keeps one translation for both paths.
            ConfigAction::Save => self.config_key(KeyInput::F(4)),
            ConfigAction::Test => self.config_key(KeyInput::F(2)),
            ConfigAction::RegenerateToken => self.config_key(KeyInput::Char(crate::config_form::REGENERATE_KEY)),
            // The header's two controls, and the screen's door: a click on either toggles the screen
            // the way `F9` does. The door is the state's, not the form's — `Toggle` names it here so
            // that a click on the gear cannot be read as the keystroke of a screen control.
            ConfigAction::Open | ConfigAction::Close | ConfigAction::Toggle => self.toggle_config_form(),
            // Everything else the screen draws names a row or a tab, and the form decides what that
            // row's own action is: a buffer, a switch, a Test, a profile.
            _ => {
                let effects = match self.config_form.as_mut() {
                    Some(form) => form.apply(action),
                    None => Vec::new(),
                };
                effects_to_actions(effects)
            }
        }
    }

    /// The screen's own keys: the form translates and reduces them — the buffer, the focus and the
    /// rows are the form's — and the effects it raises are what the client has to do about it.
    ///
    /// Only the three effects with a native side are reported as actions: the file, the connector
    /// probe and the token generator all live in the client, which is the only thing that may touch
    /// either. Everything else the key did is already in the form.
    fn config_key(&mut self, key: KeyInput) -> Vec<UiAction> {
        let Some(form) = self.config_form.as_mut() else {
            return Vec::new();
        };
        effects_to_actions(form.key(key).effects)
    }

    /// The login screen's keys (spec §6.2). It is the outermost screen, so it is asked before the
    /// configuration screen's: `F9` and the gear do nothing while the door is shut.
    ///
    /// `Tab` is the microphone because every letter is also a possible answer, and `Enter` is the only
    /// other key: no bare-letter shortcut can live here.
    fn login_key(&mut self, key: KeyInput) -> Vec<UiAction> {
        let Some(login) = self.login.as_mut() else {
            return Vec::new();
        };
        match key {
            KeyInput::Char(c) if !c.is_control() && login.input.chars().count() < LOGIN_INPUT_MAX => {
                login.input.push(c);
                Vec::new()
            }
            KeyInput::Char(_) => Vec::new(),
            KeyInput::Backspace => {
                login.input.pop();
                Vec::new()
            }
            KeyInput::Enter => {
                let text = login.input.trim().to_string();
                match (text.is_empty(), login.phase) {
                    (_, LoginPhase::Passed) => vec![UiAction::LoginDone],
                    (false, _) => {
                        login.input.clear();
                        login.phase = LoginPhase::Judging;
                        vec![UiAction::Send(Command::LoginAnswer { text })]
                    }
                    // An empty box after a refusal is the same re-arm `Tab` is.
                    (true, LoginPhase::Refused) => {
                        login.phase = LoginPhase::Listening;
                        login.open = true;
                        login.refusals = 0;
                        login.line = LoginLine::Prompt;
                        vec![UiAction::Send(Command::LoginStart)]
                    }
                    (true, _) => Vec::new(),
                }
            }
            // `Tab` re-arms a shut capture, and on a passed screen the capture is shut *because the door
            // opened*: toggling it there would reopen the microphone onto a session that has finished
            // judging, and would destroy the phase `Enter` needs to let the user in.
            KeyInput::Tab if login.voice && !login.passed() => {
                let was_open = login.open;
                login.open = !was_open;
                login.phase = if was_open { LoginPhase::Waiting } else { LoginPhase::Listening };
                login.line = LoginLine::Prompt;
                if !was_open {
                    // An explicit key is consent, and the guard's job is to stop *automatic* listening.
                    login.refusals = 0;
                }
                vec![UiAction::Send(if was_open { Command::LoginStop } else { Command::LoginStart })]
            }
            KeyInput::Esc => vec![UiAction::Quit],
            _ => Vec::new(),
        }
    }

    /// The assistant entry the next reply belongs to: the last one that has not been said yet.
    fn pending_assistant(&mut self) -> Option<&mut LogEntry> {
        self.log.iter_mut().rev().find(|e| reply_pending(e))
    }

    /// The entry a streamed reply fills: the pending one if there is one, and a fresh placeholder if
    /// there is not.
    ///
    /// The session's own path always has a placeholder by the time a delta arrives — `submit` and the
    /// microphone key both push it — but a live call has no key press to push it with, because the
    /// socket starts and ends its turns on its own. Without this the first line of a call's reply
    /// would be dropped on the floor, and a reply nobody pressed anything for still has to be
    /// readable.
    fn reply_entry(&mut self) -> &mut LogEntry {
        if !self.log.iter().any(reply_pending) {
            self.log.push(LogEntry::pending_assistant("…"));
        }
        self.pending_assistant().expect("an entry was just ensured")
    }

    /// Marks every assistant entry that is still waiting for a reply as no longer pending.
    ///
    /// The session emits `Done` before it reports `Idle` for every reply it completed, so an entry
    /// that still has no audio when the turn goes idle — a turn cancelled mid-sentence, a question
    /// nothing ever answered — will never be completed, and leaving it pending is what kept the HUD
    /// in "Pensando…" for ever.
    ///
    /// The entries themselves are kept rather than dropped: the last one may belong to the turn a
    /// barge-in has just started, whose reply still has to land somewhere, and the ones carrying a
    /// failure must stay on screen for the user to read.
    fn end_pending_replies(&mut self) {
        for entry in self.log.iter_mut().filter(|e| e.who == Who::Assistant) {
            entry.pending = false;
        }
    }

    pub fn busy(&self) -> bool {
        self.log.iter().any(|e| e.pending)
    }

    pub fn apply(&mut self, ev: &Event) {
        match ev {
            // The `..` covers `session_id` and `login`, neither of which this reducer reads: the id is
            // the bridge's and the login screen is shown by the client from the same event. The
            // earlier `..` that hid `web_bundle` is gone, because the flag is read now.
            Event::Welcome { profile, ui_language, listening_mode, web_bundle, .. } => {
                self.profile = Some(profile.clone());
                self.lang = *ui_language;
                self.listening_mode = *listening_mode;
                self.web_bundle = *web_bundle;
                // `architecture.md:85-86`: a binary without the bundle "reports that the web UI is
                // not available". The notice slot holds one message, so this only ever replaces a
                // notice it left itself — a warning from the session keeps the screen.
                if !self.web_bundle
                    && self.notice.as_ref().is_none_or(|(text, _)| text == self.strings().web_unavailable)
                {
                    self.notify(self.strings().web_unavailable);
                }
                // The configuration accepts three modes and this plan implements two, so the third
                // one says so instead of going quiet: the wake word arrives in a later plan, and
                // until then a `hands_free` session behaves like `push` — the microphone is opened
                // by hand, which is why this only speaks and never touches `mic_open`.
                let unsupported = self.strings().hands_free_unimplemented;
                match *listening_mode {
                    ListeningMode::HandsFree => self.notify(unsupported),
                    // A mode that is implemented has nothing to explain, and it takes a notice the
                    // unimplemented one left behind with it: a stale warning over a working `push`
                    // mode would be its own lie. Any other notice — a failure, a cleared
                    // conversation, the missing-bundle message — is not this announcement's business
                    // and stays.
                    ListeningMode::Push | ListeningMode::Auto => {
                        if self.notice.as_ref().is_some_and(|(text, _)| text == unsupported) {
                            self.notice = None;
                        }
                    }
                }
            }
            Event::Status { state, .. } => {
                self.status = *state;
                if *state == AssistantState::Idle {
                    // The turn is over: nothing is mid-reply any more.
                    self.end_pending_replies();
                }
                // While the door is shut, the server's `Transcribing` is the login's own wait: the row
                // says so, because the alternative is a refused screen that looks frozen and invites
                // the user to repeat the answer — which cancels the transcription in flight (§4.1,
                // review finding 2). Only a capture that is open transcribes anything.
                if *state == AssistantState::Transcribing
                    && let Some(login) = self.login.as_mut()
                    && login.open
                {
                    login.line = LoginLine::Transcribing;
                }
            }
            Event::Heard { text } => {
                let at = self.log.iter().rposition(|e| e.pending && e.who == Who::Assistant).unwrap_or(self.log.len());
                self.log.insert(at, LogEntry::user(text.clone()));
            }
            Event::ReplyDelta { text } => {
                let e = self.reply_entry();
                if e.text == "…" {
                    e.text.clear();
                }
                e.text.push_str(text);
            }
            Event::Reply { text } => {
                let e = self.reply_entry();
                e.text = text.clone();
                // A recoverable error is advisory: the reply it sat in front of has arrived,
                // and the entry is the reply, not the warning that preceded it.
                e.error = None;
            }
            Event::SpeechChunk { .. } => {}
            // A Test result belongs to the configuration screen's notice line (`ConfigForm::notice`,
            // Plan 2B Task 7), not to the HUD's log or its status rings: nothing to record here.
            Event::TestResult { .. } => {}
            Event::Done { first_audio_ms, .. } => {
                if let Some(e) = self.pending_assistant() {
                    e.pending = false;
                    e.first_audio_ms = *first_audio_ms;
                    e.error = None;
                }
            }
            Event::Stats { cpu, ram, net_in, net_out } => {
                self.cpu = *cpu;
                self.ram = *ram;
                self.net_in = *net_in;
                self.net_out = *net_out;
                self.cpu_history.push_back(*cpu);
                self.net_history.push_back((*net_in, *net_out));
                while self.cpu_history.len() > HISTORY_LEN {
                    self.cpu_history.pop_front();
                }
                while self.net_history.len() > HISTORY_LEN {
                    self.net_history.pop_front();
                }
            }
            // A live-call event is raised by the call, not by a turn: the status rings and the log
            // say nothing about it. What it *does* own is the header's indicator, which only this
            // event may raise or lower (Plan 2C Task 6).
            Event::Live { active } => {
                self.set_live(*active);
                if !self.live {
                    // A call that ended is not still thinking: its turns were started by the socket,
                    // so a reply that was cut off mid-answer never gets its `Done` and would leave
                    // the spinner running for ever — the same rule the session's `Idle` applies
                    // above.
                    self.end_pending_replies();
                }
            }
            // A login utterance and its verdict belong to the login screen's own state (spec §6), and
            // are data about nothing while no screen is up: the client shows the screen once per run,
            // from `Welcome`.
            Event::LoginHeard { text } => {
                if let Some(login) = self.login.as_mut() {
                    login.line = LoginLine::Heard(text.clone());
                }
            }
            Event::LoginVerdict { ok, name } => {
                if let Some(login) = self.login.as_mut() {
                    if *ok {
                        login.phase = LoginPhase::Passed;
                        login.line = LoginLine::Passed;
                        login.name = name.clone();
                        // The door opens on this verdict, so a capture it found open ends here (spec
                        // §6.2: "if a capture is open the client sends `LoginStop` and closes the
                        // stream"). The client's reconciliation reads `open` on its edges and sends
                        // that `LoginStop`; leaving it open would keep frames arriving that the
                        // session still judges, so a word said after "Pasa, Huésped." could reach the
                        // refusal streak and shut a door the user had already opened.
                        login.open = false;
                    } else if !login.passed() {
                        // A refusal that arrives after the door opened is stale — the pass's own
                        // `LoginStop` can provoke one — and it must not count against a user who is
                        // already inside.
                        login.phase = LoginPhase::Refused;
                        login.line = LoginLine::Refused;
                        login.refusals += 1;
                        if login.needs_a_key() {
                            login.open = false;
                        }
                        // A capture that is still open has another attempt in front of it, and the
                        // refusal has been read: a verdict is not a state to carry into the next
                        // utterance, because the row would then read "Anda, que te peinen…" — or a
                        // stale "Transcribiendo…" — for the whole of the wait that follows it, which
                        // is what made a working screen look deaf (review finding 2). The row's own
                        // next marks are the server's `Transcribing` and its `LoginHeard`.
                        if login.open {
                            login.line = LoginLine::Prompt;
                        }
                    }
                }
            }
            // The configuration's own three events. Where they are *shown* is a later task's: the
            // screen is `ConfigForm`'s to draw and the HUD is `UiState`'s, and neither has a home for
            // them here. They are here to keep the match exhaustive, and this task behaviour-free.
            Event::Config { .. } => {}
            Event::ProfileChanged { .. } => {}
            Event::ConfigChanged => {}
            Event::Error { code, message, .. } => {
                // While the door is shut, a failure belongs to the screen and not to the log: the
                // wording of "nothing heard" is the UI's, and any other failure stops the voice.
                if let Some(login) = self.login.as_mut() {
                    // Once the door is open nothing an error says can matter: the screen is waiting for
                    // `Enter`, and after it the HUD is in front. This arm used to set the phase back to
                    // `Waiting`, which the pass's own `LoginStop` can reach by cancelling a login
                    // transcription in flight — and that erased `Passed`, so `Enter` stopped entering.
                    if login.passed() {
                        return;
                    }
                    if *code == ErrorCode::NothingHeard {
                        login.line = LoginLine::NothingHeard;
                    } else {
                        login.line = LoginLine::Failed(message.clone());
                        login.open = false;
                        login.phase = LoginPhase::Waiting;
                    }
                    return;
                }
                if let Some(e) = self.pending_assistant() {
                    e.pending = false;
                    // The warning is advisory: the session is already retrying a recoverable one,
                    // and the reply that lands clears it. A failure that never gets a reply keeps
                    // its marker, so it stays visible on screen.
                    e.error = Some(message.clone());
                }
                self.notify(message.clone());
            }
        }
    }

    pub fn on_key(&mut self, key: KeyInput) -> Vec<UiAction> {
        if self.login.is_some() {
            // The door is the outermost screen: nothing behind it answers a key while it is up.
            return self.login_key(key);
        }
        if self.config_form.is_some() {
            // F9 and Esc are the screen's door: everything else — Tab, Enter, characters, F4 —
            // belongs to the form while it is on screen.
            return if matches!(key, KeyInput::Esc | KeyInput::F(9)) {
                self.toggle_config_form()
            } else {
                self.config_key(key)
            };
        }
        if key == KeyInput::F(9) {
            return self.toggle_config_form();
        }
        match key {
            KeyInput::Char(c) => {
                self.input.push(c);
                Vec::new()
            }
            KeyInput::Backspace => {
                self.input.pop();
                Vec::new()
            }
            KeyInput::Enter => self.submit(),
            KeyInput::Tab | KeyInput::F(3) => {
                self.set_mic_open(!self.mic_open);
                if self.mic_open {
                    vec![UiAction::Send(Command::ListenStart)]
                } else {
                    self.turn_ms = 0;
                    self.log.push(LogEntry::pending_assistant("…"));
                    self.select_last();
                    vec![UiAction::Send(Command::ListenStop)]
                }
            }
            KeyInput::F(4) => {
                self.log.clear();
                self.selected = None;
                self.notify(self.strings().cleared);
                vec![UiAction::Send(Command::Reset)]
            }
            KeyInput::F(2) => match self.selected {
                Some(i) if self.log.get(i).is_some_and(|e| e.who == Who::Assistant && !e.pending) => {
                    vec![UiAction::Replay(i)]
                }
                _ => Vec::new(),
            },
            KeyInput::Left | KeyInput::Right | KeyInput::F(6) => {
                self.tab = if self.tab == Tab::Chat { Tab::Network } else { Tab::Chat };
                Vec::new()
            }
            KeyInput::F(7) => {
                self.tab = Tab::Chat;
                Vec::new()
            }
            KeyInput::F(8) => {
                self.tab = Tab::Network;
                Vec::new()
            }
            KeyInput::Up => {
                self.selected = match self.selected {
                    _ if self.log.is_empty() => None,
                    None => Some(self.log.len() - 1),
                    Some(i) => Some(i.saturating_sub(1)),
                };
                Vec::new()
            }
            KeyInput::Down => {
                self.selected = match self.selected {
                    _ if self.log.is_empty() => None,
                    None => Some(0),
                    Some(i) => Some((i + 1).min(self.log.len() - 1)),
                };
                Vec::new()
            }
            KeyInput::Esc => vec![UiAction::Quit],
            KeyInput::F(10) => vec![UiAction::ToggleLive],
            KeyInput::F(_) => Vec::new(),
        }
    }

    fn select_last(&mut self) {
        self.selected = self.log.len().checked_sub(1);
    }

    fn submit(&mut self) -> Vec<UiAction> {
        let text = self.input.trim().to_string();
        self.input.clear();
        if text.is_empty() {
            return Vec::new();
        }
        self.turn_ms = 0;
        let action = if let Some(literal) = text.strip_prefix('!') {
            let literal = literal.trim().to_string();
            self.log.push(LogEntry::pending_assistant(literal.clone()));
            UiAction::Send(Command::Say { text: literal })
        } else {
            self.log.push(LogEntry::user(text.clone()));
            self.log.push(LogEntry::pending_assistant("…"));
            UiAction::Send(Command::Ask { text })
        };
        self.select_last();
        vec![action]
    }

    /// Advances clocks. Call once per frame with the milliseconds since the previous call.
    pub fn tick(&mut self, dt_ms: u64) {
        self.elapsed_ms += dt_ms;
        self.turn_ms += dt_ms;
        self.voice_pos_ms += dt_ms;
        if let Some(elapsed) = self.mic_transition_ms {
            let elapsed = elapsed.saturating_add(dt_ms);
            self.mic_transition_ms = (elapsed < MIC_TRANSITION_MS).then_some(elapsed);
        }
        if let Some((_, left)) = &mut self.notice {
            *left = left.saturating_sub(dt_ms);
            if *left == 0 {
                self.notice = None;
            }
        }
    }

    pub fn push_mic_level(&mut self, level: f32) {
        self.mic_levels.push(level.clamp(0.0, 1.0));
        if self.mic_levels.len() > MIC_LEVELS_MAX {
            self.mic_levels.drain(..MIC_LEVELS_MAX / 2);
        }
    }

    /// Starts the voice visualiser with the envelope (one value per 50 ms) of audio that just began playing.
    pub fn start_voice(&mut self, envelope: Vec<f32>) {
        self.voice_envelope = envelope;
        self.voice_pos_ms = 0;
    }

    pub fn voice_level(&self) -> f32 {
        let i = (self.voice_pos_ms / ENVELOPE_HOP_MS) as usize;
        self.voice_envelope.get(i).copied().unwrap_or(0.0)
    }

    pub fn mic_enabled(&self) -> bool {
        self.mic_open || self.live
    }

    pub fn set_mic_open(&mut self, open: bool) {
        let was_enabled = self.mic_enabled();
        self.mic_open = open;
        self.mic_changed(was_enabled);
    }

    fn set_live(&mut self, active: bool) {
        let was_enabled = self.mic_enabled();
        self.live = active;
        self.mic_changed(was_enabled);
    }

    fn mic_changed(&mut self, was_enabled: bool) {
        match (was_enabled, self.mic_enabled()) {
            (false, true) => self.mic_transition_ms = Some(0),
            (true, false) => self.mic_transition_ms = None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The lib code never names a tab — the form's own actions carry it — so the tab enum is a test
    // import only, and the click test below is what needs it.
    use crate::config_form::ConfigTab;
    use ratatui::layout::Rect;

    fn welcome(lang: UiLanguage) -> Event {
        welcome_mode(lang, ListeningMode::Push)
    }

    /// The same announcement for any mode: the mode is an argument because what the HUD says about
    /// it is what the tests below are about. The bundle is reported as available here so these tests
    /// stay about the mode; §5.4's message has its own test below.
    fn welcome_mode(lang: UiLanguage, mode: ListeningMode) -> Event {
        welcome_bundle(lang, mode, true)
    }

    /// The same announcement with `Welcome.web_bundle` under the caller's control.
    fn welcome_bundle(lang: UiLanguage, mode: ListeningMode, web_bundle: bool) -> Event {
        Event::Welcome {
            session_id: 1,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: lang,
            listening_mode: mode,
            web_bundle,
            // No login screen: the HUD tests below are about the HUD.
            login: None,
        }
    }

    fn type_text(s: &mut UiState, text: &str) {
        for c in text.chars() {
            s.on_key(KeyInput::Char(c));
        }
    }

    #[test]
    fn welcome_sets_profile_and_language() {
        let mut s = UiState::new();
        s.apply(&welcome(UiLanguage::Es));
        assert_eq!(s.profile.as_ref().unwrap().name, "El Compae");
        assert_eq!(s.strings().core, "NÚCLEO");
        s.apply(&welcome(UiLanguage::En));
        assert_eq!(s.strings().core, "CORE");
    }

    /// The configuration accepts three modes and this plan implements two: `hands_free` has no wake
    /// word yet, so the HUD says so — in the interface's own language, and without pretending the
    /// microphone is open, because in that mode it is still the user who opens it.
    #[test]
    fn hands_free_says_it_is_not_implemented_instead_of_going_quiet() {
        for (lang, word) in [(UiLanguage::Es, "manos libres"), (UiLanguage::En, "hands free")] {
            let mut s = UiState::new();
            s.apply(&welcome_mode(lang, ListeningMode::HandsFree));
            let (notice, left) = s.notice.as_ref().expect("an unimplemented mode must say so");
            assert!(notice.contains(word), "{lang:?}: {notice}");
            // The copy comes from the table, so both tables have to carry it: the same key, read
            // through the language the announcement brought.
            assert_eq!(notice.as_str(), s.strings().hands_free_unimplemented, "{lang:?}");
            assert!(*left > 0, "and the notice is a live one, not a spent one");
            assert!(!s.mic_open, "and it must not pretend to be listening");
        }
    }

    #[test]
    fn the_other_two_modes_raise_no_notice() {
        for mode in [ListeningMode::Push, ListeningMode::Auto] {
            let mut s = UiState::new();
            s.apply(&welcome_mode(UiLanguage::En, mode));
            assert!(s.notice.is_none(), "{mode:?} is implemented and must stay quiet");
        }
    }

    /// `architecture.md:85-86` promises that a binary built without the web bundle "reports that
    /// the web UI is not available". The flag is the server's truth, so it reaches the state — and
    /// the HUD says so, in the interface's own language, from the same two static tables every
    /// other string comes from.
    #[test]
    fn a_binary_without_the_bundle_says_so_in_both_languages() {
        for (lang, word) in [(UiLanguage::Es, "no está disponible"), (UiLanguage::En, "not available")] {
            let mut s = UiState::new();
            s.apply(&welcome_bundle(lang, ListeningMode::Push, false));
            assert!(!s.web_bundle, "the flag reaches the state: {lang:?}");
            let (notice, left) = s.notice.as_ref().expect("a binary without the bundle must say so");
            assert!(notice.contains(word), "{lang:?}: {notice}");
            assert_eq!(notice.as_str(), s.strings().web_unavailable, "{lang:?}");
            assert!(*left > 0, "and the notice is a live one, not a spent one");
        }
    }

    /// A server that *has* the bundle says nothing: a message about a missing page over a server
    /// that serves it would be its own lie.
    #[test]
    fn a_binary_with_the_bundle_stays_quiet() {
        let mut s = UiState::new();
        s.apply(&welcome_bundle(UiLanguage::En, ListeningMode::Push, true));
        assert!(s.web_bundle, "the flag reaches the state");
        assert!(s.notice.is_none(), "an available bundle has nothing to explain");
    }

    /// The notice belongs to the announcement that raised it: a session that re-announces an
    /// implemented mode — what a save does when it switches the mode, Task 6 — must not leave "not
    /// implemented yet" over a mode that works, and must not sweep away a notice that was never its
    /// own.
    #[test]
    fn a_mode_that_is_implemented_takes_the_unimplemented_notice_away() {
        let mut s = UiState::new();
        s.apply(&welcome_mode(UiLanguage::Es, ListeningMode::HandsFree));
        assert!(s.notice.is_some());

        // A failure, or anything else the HUD is saying, is not the announcement's to drop.
        s.notify("otra cosa");
        s.apply(&welcome_mode(UiLanguage::Es, ListeningMode::Auto));
        assert_eq!(s.notice.as_ref().map(|n| n.0.as_str()), Some("otra cosa"));

        // Its own warning, though, goes with the mode it was about.
        s.apply(&welcome_mode(UiLanguage::Es, ListeningMode::HandsFree));
        assert!(s.notice.is_some());
        s.apply(&welcome_mode(UiLanguage::Es, ListeningMode::Push));
        assert!(s.notice.is_none(), "`push` works, so the warning has nothing left to say");
    }

    #[test]
    fn enter_sends_ask_and_adds_pending_entries() {
        let mut s = UiState::new();
        type_text(&mut s, "¿qué pasa?");
        let actions = s.on_key(KeyInput::Enter);
        assert_eq!(actions, vec![UiAction::Send(Command::Ask { text: "¿qué pasa?".into() })]);
        assert!(s.input.is_empty());
        assert_eq!(s.log.len(), 2);
        assert_eq!(s.log[0].who, Who::User);
        assert!(s.log[1].pending && s.log[1].who == Who::Assistant);
    }

    #[test]
    fn bang_prefix_sends_say() {
        let mut s = UiState::new();
        type_text(&mut s, "!  hola compadre");
        assert_eq!(s.on_key(KeyInput::Enter), vec![UiAction::Send(Command::Say { text: "hola compadre".into() })]);
        assert_eq!(s.log.len(), 1);
        assert!(s.log[0].pending);
    }

    #[test]
    fn blank_enter_does_nothing() {
        let mut s = UiState::new();
        type_text(&mut s, "   ");
        assert!(s.on_key(KeyInput::Enter).is_empty());
        assert!(s.log.is_empty());
    }

    #[test]
    fn a_transcript_with_no_placeholder_still_lands_in_the_log() {
        let mut s = UiState::new();
        // A call: the socket starts and ends its own turns, so no key press pushed the placeholder
        // the session path pushes in `submit` and in the microphone key.
        s.apply(&Event::Heard { text: "¿y el ticket?".into() });
        s.apply(&Event::ReplyDelta { text: "Aquí".into() });
        s.apply(&Event::ReplyDelta { text: " andamos.".into() });
        assert_eq!(s.log.len(), 2, "{:#?}", s.log);
        assert_eq!(s.log[0].who, Who::User);
        assert_eq!(s.log[1].who, Who::Assistant);
        assert_eq!(s.log[1].text, "Aquí andamos.");
        assert!(s.log[1].pending, "the reply is still being said");

        s.apply(&Event::Done { first_audio_ms: Some(300), total_ms: 1200 });
        assert!(!s.log[1].pending);
        assert_eq!(s.log[1].first_audio_ms, Some(300));
        assert!(!s.busy());
    }

    #[test]
    fn streamed_reply_fills_pending_entry() {
        let mut s = UiState::new();
        type_text(&mut s, "x");
        s.on_key(KeyInput::Enter);
        s.apply(&Event::ReplyDelta { text: "Ozú".into() });
        s.apply(&Event::ReplyDelta { text: ", quillo".into() });
        assert_eq!(s.log[1].text, "Ozú, quillo");
        s.apply(&Event::Reply { text: "Ozú, quillo.".into() });
        s.apply(&Event::Done { first_audio_ms: Some(1200), total_ms: 3000 });
        assert_eq!(s.log[1].text, "Ozú, quillo.");
        assert!(!s.log[1].pending);
        assert_eq!(s.log[1].first_audio_ms, Some(1200));
    }

    #[test]
    fn heard_inserts_user_entry_before_pending_reply() {
        let mut s = UiState::new();
        s.on_key(KeyInput::Tab); // ListenStart
        let actions = s.on_key(KeyInput::Tab); // ListenStop adds a pending reply
        assert_eq!(actions, vec![UiAction::Send(Command::ListenStop)]);
        s.apply(&Event::Heard { text: "¿y el ticket?".into() });
        assert_eq!(s.log[0].who, Who::User);
        assert_eq!(s.log[0].text, "¿y el ticket?");
        assert!(s.log[1].pending);
    }

    #[test]
    fn tab_toggles_listening() {
        let mut s = UiState::new();
        assert_eq!(s.on_key(KeyInput::Tab), vec![UiAction::Send(Command::ListenStart)]);
        assert!(s.mic_open);
        assert_eq!(s.on_key(KeyInput::Tab), vec![UiAction::Send(Command::ListenStop)]);
        assert!(!s.mic_open);
    }

    #[test]
    fn error_marks_pending_entry_and_notifies() {
        let mut s = UiState::new();
        type_text(&mut s, "x");
        s.on_key(KeyInput::Enter);
        s.apply(&Event::Error { code: ErrorCode::Auth, message: "bad key".into(), recoverable: true });
        assert!(!s.log[1].pending);
        assert_eq!(s.log[1].error.as_deref(), Some("bad key"));
        assert_eq!(s.notice.as_ref().map(|n| n.0.as_str()), Some("bad key"));
    }

    #[test]
    fn f4_clears_and_sends_reset() {
        let mut s = UiState::new();
        type_text(&mut s, "x");
        s.on_key(KeyInput::Enter);
        s.apply(&Event::Done { first_audio_ms: None, total_ms: 1 });
        assert_eq!(s.on_key(KeyInput::F(4)), vec![UiAction::Send(Command::Reset)]);
        assert!(s.log.is_empty());
        assert!(s.notice.is_some());
    }

    #[test]
    fn arrows_switch_tabs_and_esc_quits() {
        let mut s = UiState::new();
        assert_eq!(s.tab, Tab::Chat);
        s.on_key(KeyInput::Right);
        assert_eq!(s.tab, Tab::Network);
        s.on_key(KeyInput::Left);
        assert_eq!(s.tab, Tab::Chat);
        assert_eq!(s.on_key(KeyInput::Esc), vec![UiAction::Quit]);
    }

    #[test]
    fn f2_replays_selected_finished_assistant_entry() {
        let mut s = UiState::new();
        type_text(&mut s, "x");
        s.on_key(KeyInput::Enter);
        s.apply(&Event::Done { first_audio_ms: None, total_ms: 1 });
        s.on_key(KeyInput::Down);
        s.on_key(KeyInput::Down);
        assert_eq!(s.selected, Some(1));
        assert_eq!(s.on_key(KeyInput::F(2)), vec![UiAction::Replay(1)]);
    }

    #[test]
    fn notices_expire_with_tick() {
        let mut s = UiState::new();
        s.notify("hola");
        s.tick(2999);
        assert!(s.notice.is_some());
        s.tick(2);
        assert!(s.notice.is_none());
    }

    #[test]
    fn mic_transition_uses_frame_deltas_and_live_keeps_the_switch_enabled() {
        let mut s = UiState::new();
        s.set_mic_open(true);
        assert_eq!(s.mic_transition_ms, Some(0));
        s.tick(75);
        assert_eq!(s.mic_transition_ms, Some(75));
        s.set_live(true);
        s.set_mic_open(false);
        assert!(s.mic_enabled(), "a live call also enables the microphone indicator");
        assert_eq!(s.mic_transition_ms, Some(75), "changing one source must not cancel the other");
        s.tick(200);
        assert_eq!(s.mic_transition_ms, None, "the completed transition releases its temporary state");
        assert!(s.mic_enabled(), "the mic stays enabled after its animation completes");
        s.set_live(false);
        assert!(!s.mic_enabled());
        assert_eq!(s.mic_transition_ms, None, "the transition is discarded when the effective signal ends");
    }

    #[test]
    fn voice_envelope_advances_with_tick() {
        let mut s = UiState::new();
        s.start_voice(vec![0.2, 0.9, 0.4]);
        assert!((s.voice_level() - 0.2).abs() < 1e-6);
        s.tick(50);
        assert!((s.voice_level() - 0.9).abs() < 1e-6);
        s.tick(200);
        assert_eq!(s.voice_level(), 0.0);
    }

    #[test]
    fn stats_feed_history_capped_at_120() {
        let mut s = UiState::new();
        for i in 0..130 {
            s.apply(&Event::Stats { cpu: i as f32, ram: 50.0, net_in: 10.0, net_out: 1.0 });
        }
        assert_eq!(s.cpu_history.len(), 120);
        assert_eq!(s.net_history.len(), 120);
        assert_eq!(*s.cpu_history.back().unwrap(), 129.0);
    }

    #[test]
    fn mic_levels_are_capped() {
        let mut s = UiState::new();
        for _ in 0..500 {
            s.push_mic_level(0.5);
        }
        assert!(s.mic_levels.len() <= 400);
    }
    #[test]
    fn a_turn_that_heard_nothing_leaves_the_hud_idle() {
        let mut s = UiState::new();
        s.apply(&welcome(UiLanguage::Es));
        // The mic closes: the HUD pushes the assistant placeholder before the turn is even run.
        s.on_key(KeyInput::Tab);
        s.on_key(KeyInput::Tab);
        assert!(s.busy());

        // Nothing intelligible was captured: no `Heard`, no `Reply`, no `Done` — only an error and
        // the idle status that follows it.
        s.apply(&Event::Error {
            code: ErrorCode::NothingHeard,
            message: "nothing was heard".into(),
            recoverable: true,
        });
        s.apply(&Event::Status { state: AssistantState::Idle, detail: None });

        assert!(!s.busy(), "the spinner must stop: {:#?}", s.log);
        // The failure is still on screen, in the entry itself and in the notice: the user is told
        // why the assistant said nothing.
        assert_eq!(s.log.len(), 1);
        assert_eq!(s.log[0].error.as_deref(), Some("nothing was heard"));
        assert_eq!(s.notice.as_ref().map(|n| n.0.as_str()), Some("nothing was heard"));
    }

    #[test]
    fn an_idle_status_stops_the_spinner_on_a_cancelled_turn() {
        let mut s = UiState::new();
        // The mic closes and the turn is cancelled before the assistant says anything: no `Reply`,
        // no `Done`, no `Error` — only the idle status that ends the turn.
        s.on_key(KeyInput::Tab);
        s.on_key(KeyInput::Tab);
        assert!(s.busy());

        s.apply(&Event::Status { state: AssistantState::Idle, detail: None });

        assert!(!s.busy(), "the spinner must stop: {:#?}", s.log);
        assert!(s.log.iter().all(|e| !e.pending), "nothing may still be waiting: {:#?}", s.log);
    }

    #[test]
    fn an_idle_from_a_cancelled_turn_does_not_swallow_the_next_reply() {
        let mut s = UiState::new();
        type_text(&mut s, "primera");
        s.on_key(KeyInput::Enter);
        // Barge-in: a second question is asked while the first turn is still running, so the
        // placeholder for it already exists when the cancelled turn reports that it is idle.
        type_text(&mut s, "segunda");
        s.on_key(KeyInput::Enter);
        s.apply(&Event::Status { state: AssistantState::Idle, detail: None });
        assert!(!s.busy());

        // The turn that is now running still has to answer the question that was just asked.
        s.apply(&Event::Reply { text: "Aquí andamos.".into() });
        s.apply(&Event::Done { first_audio_ms: Some(300), total_ms: 900 });

        assert_eq!(s.log.last().unwrap().text, "Aquí andamos.", "the reply must land: {:#?}", s.log);
        assert_eq!(s.log.last().unwrap().first_audio_ms, Some(300));
        assert!(!s.busy());
    }

    #[test]
    fn an_idle_status_keeps_a_failure_visible_and_stops_the_spinner() {
        let mut s = UiState::new();
        type_text(&mut s, "x");
        s.on_key(KeyInput::Enter);
        s.apply(&Event::Error { code: ErrorCode::Auth, message: "bad key".into(), recoverable: false });
        // The failure is still on screen while the turn winds down.
        assert_eq!(s.log[1].error.as_deref(), Some("bad key"));
        s.apply(&Event::Status { state: AssistantState::Idle, detail: None });
        assert!(!s.busy(), "the error ended the turn, so nothing is still running");
        // A failure that never got a reply keeps its row: the user has to see why nothing was said.
        assert_eq!(s.log[1].error.as_deref(), Some("bad key"));
        // The failure itself is still reported, in the notice too.
        assert_eq!(s.notice.as_ref().map(|n| n.0.as_str()), Some("bad key"));
    }

    #[test]
    fn f9_opens_the_screen_on_a_default_form_and_closes_it_again() {
        let mut s = UiState::new();
        assert!(s.config_form.is_none());
        // Opening is the one key that asks the client for anything: it owns `config.toml`, and the
        // request is the whole of what the HUD can do about it (the test below is what pins it).
        assert_eq!(s.on_key(KeyInput::F(9)), vec![UiAction::OpenConfig]);
        let form = s.config_form.as_ref().expect("the screen is open");
        assert_eq!(form.tab, crate::config_form::ConfigTab::Providers);
        assert_eq!(form.field, 0);
        assert!(form.editing.is_none());
        assert!(!form.dirty);

        // While it is open, nothing reaches the log or the microphone.
        s.on_key(KeyInput::Tab);
        s.on_key(KeyInput::Char('x'));
        s.on_key(KeyInput::Enter);
        assert!(!s.mic_open && s.input.is_empty() && s.log.is_empty(), "the screen has the keyboard");

        // Esc closes it instead of quitting, and F9 does the same. Closing asks for nothing: the
        // form the HUD holds is the default one, and re-reading the file is not what Esc means.
        assert!(s.on_key(KeyInput::Esc).is_empty());
        assert!(s.config_form.is_none());
        assert_eq!(s.on_key(KeyInput::Esc), vec![UiAction::Quit], "and then Esc is the HUD's again");
        s.on_key(KeyInput::F(9));
        s.on_key(KeyInput::F(9));
        assert!(s.config_form.is_none(), "F9 is not a one-way door");
    }

    #[test]
    fn opening_the_screen_asks_the_client_for_the_configuration() {
        let mut s = UiState::new();
        // The client owns the file, so the only thing the HUD can do is ask for the conversion. The
        // action is the whole request; the form above is what is shown until it answers.
        assert_eq!(s.on_key(KeyInput::F(9)), vec![UiAction::OpenConfig]);
    }

    #[test]
    fn a_screen_effect_the_client_has_to_perform_becomes_an_action() {
        let mut s = UiState::new();
        s.apply(&welcome(UiLanguage::En));
        s.on_key(KeyInput::F(9));
        // The form this test is about is the one the client answered with, not the placeholder the
        // screen opens on: a placeholder refuses to save (finding G), and refusing is a different
        // test's subject. Marking it filled here is what a client's `form_of` does in the app.
        s.config_form.as_mut().unwrap().mark_filled();
        // Ctrl+S reaches the form as the control character no text row wants. The client owns the
        // file, so the key is reported as the action to perform rather than left as a notice the
        // user can do nothing about.
        assert_eq!(
            s.on_key(KeyInput::Char('\u{13}')),
            vec![UiAction::Config(ConfigAction::Save)],
            "a wired key reports the action instead of doing nothing"
        );
        assert!(s.config_form.as_ref().is_some_and(|f| f.dirty), "and the form knows there is something to save");

        // The token generator lives on the native side too: the screen can only ask for it.
        assert_eq!(s.on_key(KeyInput::Char('\u{12}')), vec![UiAction::Config(ConfigAction::RegenerateToken)]);

        // A key that raises no effect is still nobody's business but the form's: typing inside a
        // buffer changes the buffer, and asks the client for nothing.
        let form = s.config_form.as_mut().unwrap();
        form.field = 1; // the Base URL row, the first editable one
        form.apply(ConfigAction::BeginEdit);
        assert_eq!(s.on_key(KeyInput::Char('x')), Vec::new());
        assert_eq!(s.config_form.as_ref().unwrap().editing.as_deref(), Some("x"));
    }

    /// The click path: a control's hit carries the action it means, and `on_hit` dispatches that
    /// action rather than the keystroke of whatever happens to be focused. This is the test that
    /// would have caught a click on the Profiles tab arriving as `F9` and closing the screen.
    #[test]
    fn a_click_dispatches_the_action_the_control_names() {
        let mut s = UiState::new();
        s.apply(&welcome(UiLanguage::Es));
        s.on_key(KeyInput::F(9));
        let mut form = ConfigForm {
            active_profile: "compae".into(),
            profiles: vec![
                crate::config_form::ProfileForm {
                    id: "compae".into(),
                    name: "El Compae".into(),
                    language: "es-ES".into(),
                    ..Default::default()
                },
                crate::config_form::ProfileForm {
                    id: "meigo".into(),
                    name: "O Meigo".into(),
                    language: "es-ES".into(),
                    ..Default::default()
                },
            ],
            ..ConfigForm::default()
        };
        form.tab = ConfigTab::Profiles;
        s.show_config_form(form);
        assert!(s.config_form.as_ref().is_some_and(|f| f.filled()), "a client's form can be saved");

        // A click on the Profiles tab shows the tab: it does not close the screen.
        let tab = Hit {
            area: Rect::new(1, 1, 8, 1),
            key: KeyInput::F(8),
            action: Some(ConfigAction::SelectTab(ConfigTab::Profiles)),
        };
        assert_eq!(s.on_hit(&tab), Vec::new(), "showing a tab asks the client for nothing");
        assert_eq!(s.config_form.as_ref().unwrap().tab, ConfigTab::Profiles);
        assert!(s.config_form.is_some(), "the screen is still open");

        // A click on the second profile row makes it the active one, which no key could do before.
        let row = Hit { area: Rect::new(4, 4, 20, 1), key: KeyInput::Enter, action: Some(ConfigAction::FocusRow(1)) };
        assert_eq!(s.on_hit(&row), Vec::new());
        assert_eq!(s.config_form.as_ref().unwrap().active_profile, "meigo");

        // And the Save button means what Save means: the effect comes back as an action for the
        // client, exactly as it does from `Ctrl+S`.
        let save = Hit { area: Rect::new(1, 20, 4, 1), key: KeyInput::F(4), action: Some(ConfigAction::Save) };
        assert_eq!(s.on_hit(&save), vec![UiAction::Config(ConfigAction::Save)]);
    }

    #[test]
    fn a_second_welcome_does_not_reset_the_form_the_user_is_filling_in() {
        let mut s = UiState::new();
        s.apply(&welcome(UiLanguage::Es));
        // Work already done on the HUD, and work in progress in the screen: the two things a fresh
        // session would drop.
        type_text(&mut s, "a medias");
        s.on_key(KeyInput::Enter);
        assert_eq!(s.log.len(), 2);
        type_text(&mut s, "sin enviar");
        s.on_key(KeyInput::F(9));
        s.config_form.as_mut().unwrap().profiles[0].name = "Otro Compae".into();

        // Task 6 makes `SetProfile` and `SetListeningMode` re-announce the session. A reducer that
        // treated the second `Welcome` as a fresh one would drop everything the user has typed.
        s.apply(&Event::Welcome {
            session_id: 1,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: UiLanguage::Es,
            listening_mode: ListeningMode::Auto,
            web_bundle: false,
            login: None,
        });

        assert_eq!(s.listening_mode, ListeningMode::Auto, "the re-announcement still says what the mode is");
        assert_eq!(
            s.config_form.as_ref().map(|f| f.profiles[0].name.as_str()),
            Some("Otro Compae"),
            "the half-edited form is the user's work, and it survives the announcement"
        );
        assert_eq!(s.log.len(), 2, "and so does the log behind the screen");
        assert_eq!(s.input, "sin enviar", "and the question box");
    }

    #[test]
    fn a_recoverable_error_does_not_outlive_the_reply() {
        let mut s = UiState::new();
        s.apply(&welcome(UiLanguage::Es));
        type_text(&mut s, "¿qué pasa?");
        s.on_key(KeyInput::Enter);

        // The rate-limited warning arrives before the retry; the reply the user heard follows it.
        s.apply(&Event::Error {
            code: ErrorCode::RateLimited,
            message: "rate limited: busy".into(),
            recoverable: true,
        });
        assert_eq!(s.log[1].error.as_deref(), Some("rate limited: busy"));

        // The retry succeeded: the entry the warning sat on is the one the reply fills.
        s.apply(&Event::Reply { text: "Nada, aquí andamos.".into() });
        s.apply(&Event::Done { first_audio_ms: Some(900), total_ms: 2000 });

        assert_eq!(s.log[1].text, "Nada, aquí andamos.");
        assert_eq!(s.log[1].error, None, "the entry shows the reply, not the warning it survived");
        assert!(!s.log[1].pending);
        assert_eq!(s.log[1].first_audio_ms, Some(900));
        assert!(!s.busy());
    }

    #[test]
    fn f10_asks_for_the_call_and_the_socket_answers() {
        let mut s = UiState::new();
        assert_eq!(s.on_key(KeyInput::F(10)), vec![UiAction::ToggleLive]);
        // The key asks; the flag is the socket's to set, so it is untouched here.
        assert!(!s.live, "a key press is not a call: only `Event::Live` turns the indicator on");
        s.apply(&Event::Live { active: true });
        assert!(s.live);
        // And the same key again is the way out, asked for the same way.
        assert_eq!(s.on_key(KeyInput::F(10)), vec![UiAction::ToggleLive]);
        assert!(s.live, "the indicator goes down when the socket says so, not when the key is pressed");
    }

    #[test]
    fn the_end_of_a_call_takes_the_indicator_down_and_stops_the_spinner() {
        let mut s = UiState::new();
        s.apply(&Event::Live { active: true });
        // A call's turns are started by the socket: `Heard` and the deltas open the reply entry
        // (`reply_entry`), and no `Done` ever arrives for a call that dies mid-answer.
        s.apply(&Event::Heard { text: "¿y el ticket?".into() });
        s.apply(&Event::ReplyDelta { text: "Aquí andamos".into() });
        assert!(s.busy(), "the reply is being said");

        s.apply(&Event::Live { active: false });
        assert!(!s.live, "the indicator is the first thing to go");
        assert!(!s.busy(), "a call that ended is not still thinking: {:#?}", s.log);
        assert_eq!(s.log.last().unwrap().text, "Aquí andamos", "the partial reply stays readable");
    }

    // ---- the login screen (spec 2026-10-11-login-screen-design.md, §6) ----

    fn login_state(voice: bool) -> UiState {
        let mut s = UiState::new();
        s.show_login(voice);
        s
    }

    fn refused(s: &mut UiState) {
        s.apply(&Event::LoginVerdict { ok: false, name: None });
    }

    /// The screen is the outermost thing on it: while the door is shut there is no second screen, and
    /// `esc` is still the way out (spec §6.2).
    #[test]
    fn the_login_screen_is_asked_first_and_f9_does_nothing() {
        let mut s = login_state(true);
        assert!(s.on_key(KeyInput::F(9)).is_empty(), "F9 is not a door out of the login");
        assert!(s.config_form.is_none(), "and it does not open the configuration screen");
        assert_eq!(s.on_key(KeyInput::Esc), vec![UiAction::Quit]);
    }

    /// The screen has its own buffer, so the HUD's box behind the veil stays empty, and 200 characters
    /// is where it stops (spec §6.2).
    #[test]
    fn typing_goes_to_the_screen_and_is_capped() {
        let mut s = login_state(true);
        type_text(&mut s, "huesped");
        assert_eq!(s.login.as_ref().unwrap().input, "huesped");
        assert!(s.input.is_empty(), "the HUD's own box is not the screen's");
        type_text(&mut s, &"x".repeat(250));
        assert_eq!(s.login.as_ref().unwrap().input.chars().count(), LOGIN_INPUT_MAX);
        let _ = s.on_key(KeyInput::Backspace);
        assert_eq!(s.login.as_ref().unwrap().input.chars().count(), LOGIN_INPUT_MAX - 1);
    }

    /// The screen is shown **once per client run**, and the state owns that rule rather than each
    /// caller: a second `Welcome` — the one `SetProfile` and `SetListeningMode` make the session send —
    /// must neither wipe a half-typed answer nor reopen a door that has already been walked through
    /// (login spec §3). The second case is the one a bare `login.is_none()` guard cannot tell apart
    /// from "never asked", because the door coming down puts `login` back to `None`.
    #[test]
    fn the_screen_is_shown_once_per_run() {
        let mut s = UiState::new();
        assert!(s.show_login(true), "the first view shows the screen");
        type_text(&mut s, "yo soy huesped");
        assert!(!s.show_login(true), "a second view while it is up shows nothing");
        assert_eq!(s.login.as_ref().unwrap().input, "yo soy huesped", "and wipes nothing");

        // The door opens: the client takes the screen down, and the run's own bit is what remembers.
        s.login = None;
        assert!(!s.show_login(true), "a view after the door opened does not reopen it");
        assert!(s.login.is_none(), "the HUD stays in front");
    }

    /// A pass closes the capture it found open (spec §6.2). Without this the microphone stays open
    /// after the door opens, the client never sends its `LoginStop`, and the session keeps judging
    /// frames — so a word said after "Pasa, Huésped." reaches the refusal streak and the door the user
    /// had just walked through shuts again.
    #[test]
    fn a_pass_closes_the_capture_it_found_open() {
        let mut s = login_state(true);
        s.login.as_mut().unwrap().open = true;

        s.apply(&Event::LoginVerdict { ok: true, name: Some("Huésped".into()) });

        let login = s.login.as_ref().expect("the screen stays until Enter");
        assert_eq!(login.phase, LoginPhase::Passed);
        assert_eq!(login.name.as_deref(), Some("Huésped"));
        assert!(!login.open, "the capture is over the moment the door opens");
    }

    /// **`Passed` is terminal for the phase** (spec §6.2: the screen stays until `Enter`). Three
    /// writers could put the user back outside a door they are already through, and every one of them
    /// is reachable — this is the whole of review finding 2 plus the new trigger the pass's own
    /// `LoginStop` introduced.
    #[test]
    fn a_passed_screen_is_not_put_back_outside() {
        let mut s = login_state(true);
        s.login.as_mut().unwrap().open = true;
        s.apply(&Event::LoginVerdict { ok: true, name: Some("Huésped".into()) });
        assert!(s.login.as_ref().unwrap().passed());

        // 1. `Tab` re-armed the capture and reset the phase the border advertises `Enter` for.
        assert!(s.on_key(KeyInput::Tab).is_empty(), "a passed screen re-arms nothing");
        let login = s.login.as_ref().unwrap();
        assert!(login.passed(), "the phase stands");
        assert!(!login.open, "and the capture stays over");

        // 2. A refusal verdict that arrives after the door opened.
        s.apply(&Event::LoginVerdict { ok: false, name: None });
        let login = s.login.as_ref().unwrap();
        assert!(login.passed(), "a stale refusal is not a refusal");
        assert_eq!(login.refusals, 0, "and it does not count against the user");

        // 3. The `Event::Error` the pass itself can provoke: `transcribe_once` reports the `cancelled`
        //    its own `LoginStop` causes as an error, and this arm used to set the phase to `Waiting`.
        s.apply(&Event::Error { code: ErrorCode::Internal, message: "cancelled".into(), recoverable: true });
        let login = s.login.as_ref().unwrap();
        assert!(login.passed(), "an error does not put the user back outside");
        assert_eq!(login.line, LoginLine::Passed, "and the line still says they are in");

        // And the key the screen has been advertising still works.
        assert_eq!(s.on_key(KeyInput::Enter), vec![UiAction::LoginDone]);
    }

    /// A typed answer is a `LoginAnswer` the server judges; the box empties and the screen says it is
    /// waiting (spec §6.2).
    #[test]
    fn enter_sends_the_typed_answer() {
        let mut s = login_state(true);
        type_text(&mut s, "  yo soy huesped del core  ");
        assert_eq!(
            s.on_key(KeyInput::Enter),
            vec![UiAction::Send(Command::LoginAnswer { text: "yo soy huesped del core".into() })]
        );
        let login = s.login.as_ref().unwrap();
        assert_eq!(login.input, "", "the box empties");
        assert_eq!(login.phase, LoginPhase::Judging);
    }

    /// A refusal is a line and a counter, and the microphone stays open for the next thing said — that
    /// is the automatic re-arm; the third refusal in a row is what stops it (spec §6.2, L8).
    #[test]
    fn the_third_refusal_in_a_row_closes_the_microphone() {
        let mut s = login_state(true);
        let _ = s.on_key(KeyInput::Tab);
        assert!(s.login.as_ref().unwrap().open);
        refused(&mut s);
        assert_eq!(
            s.login.as_ref().unwrap().line,
            LoginLine::Prompt,
            "the refusal has been read: the row is ready for the next attempt (review finding 2)"
        );
        assert!(s.login.as_ref().unwrap().open, "attempt 1 leaves the capture open");
        refused(&mut s);
        assert!(s.login.as_ref().unwrap().open, "and so does attempt 2");
        refused(&mut s);
        let login = s.login.as_ref().unwrap();
        assert_eq!(login.refusals, 3);
        assert!(!login.open, "the third stops the automatic re-arm");
        assert!(login.needs_a_key());
    }

    /// `Tab` is the microphone: on, off, and on again. An explicit re-arm is consent, so it resets the
    /// streak — the guard's whole job is to stop *automatic* listening (spec §6.2).
    #[test]
    fn tab_toggles_the_capture_and_an_explicit_re_arm_resets_the_streak() {
        let mut s = login_state(true);
        assert_eq!(s.on_key(KeyInput::Tab), vec![UiAction::Send(Command::LoginStart)]);
        assert!(s.login.as_ref().unwrap().open);
        assert_eq!(s.on_key(KeyInput::Tab), vec![UiAction::Send(Command::LoginStop)]);
        assert!(!s.login.as_ref().unwrap().open);
        let _ = s.on_key(KeyInput::Tab);
        for _ in 0..3 {
            refused(&mut s);
        }
        assert!(s.login.as_ref().unwrap().needs_a_key());
        assert_eq!(s.on_key(KeyInput::Tab), vec![UiAction::Send(Command::LoginStart)]);
        assert_eq!(s.login.as_ref().unwrap().refusals, 0, "an explicit re-arm starts the count over");
    }

    /// With the voice switch off there is no capture to toggle and no hint to draw (spec §6).
    #[test]
    fn tab_does_nothing_when_the_voice_is_off() {
        let mut s = login_state(false);
        assert!(s.on_key(KeyInput::Tab).is_empty());
        assert!(!s.login.as_ref().unwrap().open);
    }

    /// The greeting stays until `Enter`, which is the client's cue to close the screen and enter the
    /// HUD (spec §6.2).
    #[test]
    fn a_passed_screen_waits_for_enter_and_then_finishes() {
        let mut s = login_state(true);
        s.apply(&Event::LoginHeard { text: "yo soy Huésped del core".into() });
        assert_eq!(
            s.login.as_ref().unwrap().line,
            LoginLine::Heard("yo soy Huésped del core".into()),
            "a misheard accent is visible rather than mysterious"
        );
        s.apply(&Event::LoginVerdict { ok: true, name: Some("Huésped".into()) });
        let login = s.login.as_ref().unwrap();
        assert_eq!(login.phase, LoginPhase::Passed);
        assert_eq!(login.line, LoginLine::Passed);
        assert_eq!(login.name.as_deref(), Some("Huésped"));
        assert_eq!(s.on_key(KeyInput::Enter), vec![UiAction::LoginDone]);
    }

    /// "Nothing heard" is the UI's wording, not the server's, and it counts no attempt: a cough is not
    /// a wrong answer (spec §4.1, §6.2).
    #[test]
    fn nothing_heard_counts_no_attempt_and_leaves_the_capture_open() {
        let mut s = login_state(true);
        let _ = s.on_key(KeyInput::Tab);
        s.apply(&Event::Error {
            code: ErrorCode::NothingHeard,
            message: "nothing was heard".into(),
            recoverable: true,
        });
        let login = s.login.as_ref().unwrap();
        assert_eq!(login.line, LoginLine::NothingHeard);
        assert_eq!(login.refusals, 0);
        assert!(login.open, "the capture keeps listening");
    }

    /// Any other failure turns the microphone off and shows the provider's own words: retrying the
    /// voice would only repeat it, and typing still works (spec §6.2).
    #[test]
    fn another_error_turns_the_microphone_off_and_shows_its_message() {
        let mut s = login_state(true);
        let _ = s.on_key(KeyInput::Tab);
        s.apply(&Event::Error {
            code: ErrorCode::NoKey,
            message: "no key named `openrouter`".into(),
            recoverable: true,
        });
        let login = s.login.as_ref().unwrap();
        assert_eq!(login.line, LoginLine::Failed("no key named `openrouter`".into()));
        assert!(!login.open);
        assert!(s.log.is_empty(), "a login failure is not a conversation entry");
    }

    /// An empty `Enter` after a refusal re-arms the capture, and does nothing before one: the prompt is
    /// not a button (spec §6.2). The case is the third refusal in a row, which is the one that closes
    /// the capture by itself — a manual `Tab` moves the phase out of `Refused`, and re-arming a refused
    /// screen is *that* key's job, not a general "open the microphone" key: that one is `Tab`.
    #[test]
    fn an_empty_enter_re_arms_only_after_a_refusal() {
        let mut s = login_state(true);
        assert!(s.on_key(KeyInput::Enter).is_empty(), "nothing to say, nothing to re-arm");
        let _ = s.on_key(KeyInput::Tab);
        for _ in 0..LOGIN_AUTO_REARM {
            refused(&mut s);
        }
        assert!(!s.login.as_ref().unwrap().open, "the third refusal stopped the automatic re-arm");
        assert_eq!(s.on_key(KeyInput::Enter), vec![UiAction::Send(Command::LoginStart)]);
        assert!(s.login.as_ref().unwrap().open);
        assert_eq!(s.login.as_ref().unwrap().refusals, 0);
    }

    /// With no screen up, a login event is data about nothing and must not touch the HUD (spec §3: the
    /// client shows the screen once per run, from `Welcome`).
    #[test]
    fn login_events_without_a_screen_are_ignored() {
        let mut s = UiState::new();
        s.apply(&Event::LoginHeard { text: "huesped".into() });
        s.apply(&Event::LoginVerdict { ok: true, name: Some("Huésped".into()) });
        assert!(s.login.is_none());
        assert!(s.log.is_empty());
        assert!(s.notice.is_none());
    }

    /// Review finding 2: while an answer is being transcribed the row must say so — the server sends
    /// `Status { Transcribing }` and that is the honest trigger — and a verdict must not be carried
    /// into the next attempt. In that window the spoken answer is in flight, `● rec` is lit and the row
    /// read "Anda, que te peinen para el lado": a user reads that as "it did not hear me", says it
    /// again, and cancels the transcription (§4.1).
    #[test]
    fn a_login_capture_in_flight_says_it_is_transcribing() {
        let mut s = login_state(true);
        let _ = s.on_key(KeyInput::Tab);
        refused(&mut s);
        // The refusal has been read: a capture that is still open is three seconds from the next
        // attempt, and the row says so rather than repeating a verdict.
        assert_eq!(s.login.as_ref().unwrap().line, LoginLine::Prompt);

        // The next utterance ends and the server starts transcribing: the row is the wait.
        s.apply(&Event::Status { state: AssistantState::Transcribing, detail: None });
        assert_eq!(s.login.as_ref().unwrap().line, LoginLine::Transcribing, "the wait has a face");
        assert!(s.login.as_ref().unwrap().open, "and the capture is still open");

        // What the transcriber heard replaces it, and the verdict replaces that — and clears it again.
        s.apply(&Event::LoginHeard { text: "soy huesped".into() });
        assert_eq!(s.login.as_ref().unwrap().line, LoginLine::Heard("soy huesped".into()));
        s.apply(&Event::LoginVerdict { ok: false, name: None });
        assert_eq!(s.login.as_ref().unwrap().line, LoginLine::Prompt);
        assert_eq!(s.login.as_ref().unwrap().refusals, 2);

        // The third refusal in a row is the one that stops the capture, and there the refusal is what
        // the row keeps: there is no next attempt for it to be stale in, and it is read beside the
        // escape hint. Nothing in flight may wear the transcription line either.
        for _ in 0..LOGIN_AUTO_REARM {
            refused(&mut s);
        }
        assert!(!s.login.as_ref().unwrap().open);
        assert_eq!(s.login.as_ref().unwrap().line, LoginLine::Refused);
        s.apply(&Event::Status { state: AssistantState::Transcribing, detail: None });
        assert_eq!(s.login.as_ref().unwrap().line, LoginLine::Refused, "a shut capture transcribes nothing");

        // And a door that has opened is never covered by it (spec §6.1's precedence).
        let mut passed = login_state(true);
        passed.apply(&Event::LoginVerdict { ok: true, name: Some("Huésped".into()) });
        passed.apply(&Event::Status { state: AssistantState::Transcribing, detail: None });
        assert_eq!(passed.login.as_ref().unwrap().line, LoginLine::Passed);
    }
}
