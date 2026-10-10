//! Interface state: everything the HUD draws, updated from server events and user input.

use std::collections::VecDeque;

use tipsy_protocol::{AssistantState, Command, Event, ListeningMode, PersonaView, UiLanguage};

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
}

pub struct UiState {
    pub persona: Option<PersonaView>,
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
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        Self {
            persona: None,
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
        }
    }

    pub fn strings(&self) -> &'static Strings {
        strings(self.lang)
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), NOTICE_MS));
    }

    fn pending_assistant(&mut self) -> Option<&mut LogEntry> {
        self.log.iter_mut().rev().find(|e| e.pending && e.who == Who::Assistant)
    }

    pub fn busy(&self) -> bool {
        self.log.iter().any(|e| e.pending)
    }

    pub fn apply(&mut self, ev: &Event) {
        match ev {
            Event::Welcome { persona, ui_language, listening_mode, .. } => {
                self.persona = Some(persona.clone());
                self.lang = *ui_language;
                self.listening_mode = *listening_mode;
            }
            Event::Status { state, .. } => self.status = *state,
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
                }
            }
            Event::SpeechChunk { .. } => {}
            Event::Done { first_audio_ms, .. } => {
                if let Some(e) = self.pending_assistant() {
                    e.pending = false;
                    e.first_audio_ms = *first_audio_ms;
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
                    e.error = Some(message.clone());
                }
                self.notify(message.clone());
            }
        }
    }

    pub fn on_key(&mut self, key: KeyInput) -> Vec<UiAction> {
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
        Event::Welcome {
            session_id: 1,
            persona: PersonaView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: lang,
            listening_mode: ListeningMode::Push,
            web_bundle: false,
        }
    }

    fn type_text(s: &mut UiState, text: &str) {
        for c in text.chars() {
            s.on_key(KeyInput::Char(c));
        }
    }

    #[test]
    fn welcome_sets_persona_and_language() {
        let mut s = UiState::new();
        s.apply(&welcome(UiLanguage::Es));
        assert_eq!(s.persona.as_ref().unwrap().name, "El Compae");
        assert_eq!(s.strings().core, "NÚCLEO");
        s.apply(&welcome(UiLanguage::En));
        assert_eq!(s.strings().core, "CORE");
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
}
