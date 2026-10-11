//! Interface state: everything the HUD draws, updated from server events and user input.

use std::collections::VecDeque;

use tipsy_protocol::{AssistantState, Command, Event, ListeningMode, ProfileView, UiLanguage};

use crate::config_form::{ConfigAction, ConfigEffect, ConfigForm};
use crate::strings::{strings, Strings};

const NOTICE_MS: u64 = 3000;
const HISTORY_LEN: usize = 120;
const MIC_LEVELS_MAX: usize = 400;
const ENVELOPE_HOP_MS: u64 = 50;

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
}

pub struct UiState {
    pub profile: Option<ProfileView>,
    pub lang: UiLanguage,
    pub listening_mode: ListeningMode,
    pub status: AssistantState,
    pub mic_open: bool,
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
            mic_open: false,
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
    pub fn show_config_form(&mut self, form: ConfigForm) {
        self.config_form = Some(form);
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
        let outcome = form.key(key);
        let mut actions = Vec::new();
        for effect in outcome.effects {
            match effect {
                ConfigEffect::None => {}
                ConfigEffect::Save => actions.push(UiAction::Config(ConfigAction::Save)),
                ConfigEffect::Test(_) => actions.push(UiAction::Config(ConfigAction::Test)),
                ConfigEffect::RegenerateToken => {
                    actions.push(UiAction::Config(ConfigAction::RegenerateToken))
                }
            }
        }
        actions
    }

    /// The assistant entry the next reply belongs to: the last one that has not been said yet.
    fn pending_assistant(&mut self) -> Option<&mut LogEntry> {
        self.log.iter_mut().rev().find(|e| reply_pending(e))
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
            Event::Welcome { profile, ui_language, listening_mode, .. } => {
                self.profile = Some(profile.clone());
                self.lang = *ui_language;
                self.listening_mode = *listening_mode;
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
                    // conversation — is not this announcement's business and stays.
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
            }
            Event::Heard { text } => {
                let at = self
                    .log
                    .iter()
                    .rposition(|e| e.pending && e.who == Who::Assistant)
                    .unwrap_or(self.log.len());
                self.log.insert(at, LogEntry::user(text.clone()));
            }
            Event::ReplyDelta { text } => {
                if let Some(e) = self.pending_assistant() {
                    if e.text == "…" {
                        e.text.clear();
                    }
                    e.text.push_str(text);
                }
            }
            Event::Reply { text } => {
                if let Some(e) = self.pending_assistant() {
                    e.text = text.clone();
                    // A recoverable error is advisory: the reply it sat in front of has arrived,
                    // and the entry is the reply, not the warning that preceded it.
                    e.error = None;
                }
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
            Event::Error { message, .. } => {
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
                self.mic_open = !self.mic_open;
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
                Some(i) if self.log.get(i).is_some_and(|e| e.who == Who::Assistant && !e.pending) => vec![UiAction::Replay(i)],
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::*;

    fn welcome(lang: UiLanguage) -> Event {
        welcome_mode(lang, ListeningMode::Push)
    }

    /// The same announcement for any mode: the mode is an argument because what the HUD says about
    /// it is what the tests below are about.
    fn welcome_mode(lang: UiLanguage, mode: ListeningMode) -> Event {
        Event::Welcome {
            session_id: 1,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: lang,
            listening_mode: mode,
            web_bundle: false,
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
        assert_eq!(
            s.on_key(KeyInput::Char('\u{12}')),
            vec![UiAction::Config(ConfigAction::RegenerateToken)]
        );

        // A key that raises no effect is still nobody's business but the form's: typing inside a
        // buffer changes the buffer, and asks the client for nothing.
        let form = s.config_form.as_mut().unwrap();
        form.field = 1; // the Base URL row, the first editable one
        form.apply(ConfigAction::BeginEdit);
        assert_eq!(s.on_key(KeyInput::Char('x')), Vec::new());
        assert_eq!(s.config_form.as_ref().unwrap().editing.as_deref(), Some("x"));
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
}

