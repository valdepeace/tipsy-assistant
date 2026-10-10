//! The native terminal client: keys and mouse in, HUD out, audio played and captured here.
//!
//! The client owns everything the protocol cannot carry: the terminal, the output device and the
//! microphone. It holds no conversation state beyond what it draws.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    self, EnableMouseCapture, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, MouseEventKind,
};
use ratatui::crossterm::execute;
use tipsy_protocol::{Command, Event, Frame, ToClient, ToServer};
use tipsy_ui::{ClockView, Hit, KeyInput, UiAction, UiState, draw};
use tokio::sync::mpsc::UnboundedSender;

use crate::audio::device::{Mic, Player};
use crate::audio::pcm::{envelope, i16_to_f32, rms};
use crate::brain::BrainConfig;
use crate::config::store;
use crate::config::Config;
use crate::keys::KeyResolver;
use crate::providers::{build_providers, Providers};
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
    let providers = build_providers(&config.providers, &KeyResolver::system()).map_err(|e| e.to_string())?;

    let mut client = Client::new(config);
    // `init` enters the alternate screen, enables raw mode and installs the panic hook that
    // restores the terminal; `restore` undoes all of it.
    let mut terminal = ratatui::init();
    // Mouse capture is not part of a bare terminal: without it the clickable titles do nothing.
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    if let Some(warning) = loaded.warning {
        client.state.notify(warning);
    }
    client.open_player();
    let outcome = client.frame_loop(&mut terminal, providers);
    let _ = execute!(std::io::stdout(), event::DisableMouseCapture);
    ratatui::restore();
    outcome
}

/// Everything the client keeps from one frame to the next.
struct Client {
    state: UiState,
    /// The configuration the session was built from, kept for its provider settings.
    config: Config,
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
    fn new(config: Config) -> Self {
        let (mic_tx, mic_frames) = std::sync::mpsc::sync_channel(MIC_QUEUE);
        let mic_gain = config.listening.mic_gain;
        Self {
            state: UiState::new(),
            config,
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
    fn frame_loop(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        providers: Providers,
    ) -> Result<(), String> {
        let ends = spawn_with_events(self.config.clone(), providers, BrainConfig::default());
        let to_server = ends.to_server;
        let mut from_server = ends.from_server;
        sysmon::spawn(ends.events);

        let mut drawn = Instant::now();
        loop {
            let hits = draw_frame(terminal, &self.state)?;

            let waited = FRAME.saturating_sub(drawn.elapsed());
            let mut keys = Vec::new();
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
                                keys.push(hit.key);
                            }
                        }
                        _ => {}
                    }
                    if keys.len() >= MAX_INPUT_PER_FRAME {
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
                    quit |= self.on_action(action, &to_server);
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
    fn on_action(&mut self, action: UiAction, to_server: &UnboundedSender<ToServer>) -> bool {
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
            UiAction::Quit => true,
        }
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
        Command::Hello { .. }
        | Command::ListenStart
        | Command::Cancel
        | Command::SetPersona { .. }
        | Command::SetListeningMode { .. } => false,
    }
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
    let title = state.persona.as_ref().map(|p| p.name.clone()).unwrap_or_default();
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
    use tipsy_protocol::AssistantState;

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
            Command::SetPersona { id: "compae".into() },
        ] {
            assert!(!interrupts_playback(&command), "{command:?} leaves the reply alone");
        }
    }

    #[test]
    fn a_finished_reply_keeps_its_audio_until_a_new_turn_starts() {
        let (to_server, _from_client) = tokio::sync::mpsc::unbounded_channel();
        let mut client = Client::new(Config::default());

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

        client.on_action(UiAction::Send(Command::Ask { text: "¿y eso?".into() }), &to_server);
        assert!(client.reply.is_empty(), "the next turn speaks a reply of its own");
    }
}
