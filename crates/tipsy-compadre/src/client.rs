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
use tipsy_protocol::{AssistantState, Command, Event, Frame, ToClient, ToServer};
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
    /// The audio of the reply being spoken, kept for `Replay`.
    reply: Vec<i16>,
    /// Sample rate announced by the last `SpeechChunk`.
    rate: u32,
    /// The assistant state the previous server event left behind, for terminal transitions.
    last_status: AssistantState,
    /// True while the turn the user just interrupted is still winding down: its audio is flushed
    /// as soon as the session reports that it stopped.
    cancel_pending: bool,
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
            last_status: AssistantState::Idle,
            cancel_pending: false,
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
                if matches!(event, Event::Done { .. }) {
                    // The next sentence spoken starts a new reply.
                    self.reply.clear();
                }
                self.state.apply(&event);
                if let Event::Status { state, .. } = event {
                    let flush = flush_on_terminal(self.last_status, state, self.cancel_pending);
                    if state == AssistantState::Idle {
                        // The idle status arrived: nothing is still winding down.
                        self.cancel_pending = false;
                    }
                    self.last_status = state;
                    if flush {
                        self.flush_playback();
                    }
                }
            }
            ToClient::Audio(Frame::Speech { samples, .. }) => self.on_speech(samples),
            ToClient::Audio(Frame::Mic(_)) => {}
        }
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
                if command == Command::Cancel {
                    // The user ended the turn: its audio stops as soon as the session says so.
                    self.cancel_pending = true;
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

/// Whether the queued audio must be dropped on this status change.
///
/// The session sends no more frames once a turn is over, but sentences handed to rodio earlier are
/// still queued in the device, and the session reports the end of the turn before the device has
/// played them. Flushing is therefore reserved for the two terminal transitions: the speaking
/// status giving way to idle (a cancelled turn, or a failure that stopped the audio), and the
/// moment the cancel the user asked for reaches the idle status. A normal reply ends with its own
/// `Done`, which never flushes, so its last sentence is heard in full.
fn flush_on_terminal(previous: AssistantState, current: AssistantState, cancelled: bool) -> bool {
    if current != AssistantState::Idle {
        return false;
    }
    previous == AssistantState::Speaking || cancelled
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

    #[test]
    fn speaking_giving_way_to_idle_flushes_the_queued_audio() {
        // A cancelled turn: the session stops emitting frames, but rodio still holds a sentence.
        assert!(flush_on_terminal(AssistantState::Speaking, AssistantState::Idle, false));
        // A failure that stopped the audio on the way out.
        assert!(flush_on_terminal(AssistantState::Speaking, AssistantState::Idle, true));
    }

    #[test]
    fn the_cancel_the_user_asked_for_flushes_even_before_any_audio() {
        assert!(flush_on_terminal(AssistantState::Thinking, AssistantState::Idle, true));
        assert!(flush_on_terminal(AssistantState::Transcribing, AssistantState::Idle, true));
    }

    #[test]
    fn only_the_idle_status_is_terminal() {
        // Reaching idle is the only transition that drops audio; every other status is the turn
        // still going on, and `Done` itself never flushes, so a normal reply is heard in full.
        for previous in [
            AssistantState::Idle,
            AssistantState::Listening,
            AssistantState::Transcribing,
            AssistantState::Thinking,
            AssistantState::Speaking,
        ] {
            assert!(!flush_on_terminal(previous, AssistantState::Listening, false), "{previous:?}");
            assert!(!flush_on_terminal(previous, AssistantState::Transcribing, false), "{previous:?}");
            assert!(!flush_on_terminal(previous, AssistantState::Thinking, false), "{previous:?}");
            assert!(!flush_on_terminal(previous, AssistantState::Speaking, false), "{previous:?}");
        }
    }

    #[test]
    fn an_idle_that_follows_silence_has_nothing_to_flush() {
        // Nothing was queued: the greeting of a silent session, a repeated idle, or the idle that
        // follows a turn which never spoke.
        assert!(!flush_on_terminal(AssistantState::Idle, AssistantState::Idle, false));
        assert!(!flush_on_terminal(AssistantState::Listening, AssistantState::Idle, false));
        assert!(!flush_on_terminal(AssistantState::Transcribing, AssistantState::Idle, false));
        assert!(!flush_on_terminal(AssistantState::Thinking, AssistantState::Idle, false));
    }
}
