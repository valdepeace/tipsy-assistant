//! The server side of a session: one conversation, one provider stack, one running turn.
//!
//! The session owns the history and the microphone buffer. Each turn runs in its own task with a
//! fresh [`CancellationToken`], and hands the history it produced back to the session through a
//! oneshot — so the history has exactly one owner and a cancelled turn can never write into it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tipsy_protocol::{AssistantState, Command, Event, Frame, ListeningMode, ProfileView, ToClient, ToServer, UiLanguage};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::audio::device::MIC_SAMPLE_RATE;
use crate::audio::pcm::{i16_to_f32, Pcm};
use crate::brain::{Brain, BrainConfig, TurnInput, TurnProfile};
use crate::config::Config;
use crate::providers::{ChatMessage, ProviderSlot, Role};
use crate::vad::{VadEvent, VadState};

/// The session id reported to clients. There is one session per process for now.
const SESSION_ID: u64 = 1;

/// How much conversation the session keeps. An exchange is a question and its reply, so this is 20
/// exchanges: deep enough that a follow-up can still refer to what was said a while ago, shallow
/// enough that the prompt — and what every turn costs — stops growing with the life of the session.
const HISTORY_MAX_MESSAGES: usize = 40;

/// The tail the detector stays paused for after the assistant stops speaking: spec §6's 300 ms.
/// The turn's `Done` arrives while the device is still speaking the last sentence, which is what
/// this is for.
const GUARD: Duration = Duration::from_millis(300);

/// Starts a session on its own task and returns the two ends of its channels.
pub fn spawn_session(
    config: Config,
    providers: ProviderSlot,
    brain_config: BrainConfig,
) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>) {
    let session = spawn_with_events(config, providers, brain_config);
    (session.to_server, session.from_server)
}

/// The three ends the client keeps, exactly as a session hands them over.
pub struct SessionEnds {
    pub to_server: UnboundedSender<ToServer>,
    pub from_server: UnboundedReceiver<ToClient>,
    /// A second sender for the same client channel, for producers that live beside the session,
    /// such as the system monitor.
    pub events: UnboundedSender<ToClient>,
}

/// Like [`spawn_session`], but also hands back the sender the session reports to, so a caller can
/// put other producers of [`ToClient`] on the same channel.
pub fn spawn_with_events(config: Config, providers: ProviderSlot, brain_config: BrainConfig) -> SessionEnds {
    let (to_server_tx, to_server_rx) = mpsc::unbounded_channel();
    let (to_client_tx, to_client_rx) = mpsc::unbounded_channel();
    // Held so the client end only closes once the session task itself has ended.
    let keep_alive = to_client_tx.clone();

    let profile = config.active().clone();
    let listening_mode = config.listening.mode;
    let listening_silence_ms = config.listening.silence_ms;
    let brain = Arc::new(Brain::new(providers.clone(), turn_profile(&config), brain_config.clone()));

    let mut session = Session {
        config,
        brain,
        brain_config,
        providers,
        out: to_client_tx.clone(),
        history: Vec::new(),
        current: None,
        generation: 0,
        listening: false,
        mic: Vec::new(),
        vad: VadState::new(listening_silence_ms),
        heard: false,
        utterance: Vec::new(),
        guard: None,
        profile: profile.view(),
        ui_language: UiLanguage::from_bcp47(&profile.language),
        listening_mode,
        greeting: profile.greeting.clone(),
    };

    tokio::spawn(async move {
        session.run(to_server_rx).await;
        // The task ends with the session; the client end closes with it.
        drop(keep_alive);
    });

    SessionEnds { to_server: to_server_tx, from_server: to_client_rx, events: to_client_tx }
}

/// What the brain needs from the configuration: the active profile's texts and the voice the speech
/// connector recommends for it. Built when the session starts and again on every `SetProfile`.
fn turn_profile(config: &Config) -> TurnProfile {
    let profile = config.active();
    TurnProfile {
        prompt: profile.prompt.clone(),
        voice_instructions: profile.voice_instructions.clone(),
        voice: profile.voice_for(&config.providers.speech),
        language: profile.language.clone(),
    }
}

/// A turn that is running right now, and the only handle the session may cancel.
struct Turn {
    cancel: CancellationToken,
    /// Where the turn's task leaves the history it produced.
    done: tokio::sync::oneshot::Receiver<Vec<ChatMessage>>,
    handle: JoinHandle<()>,
    /// Bumped by `Reset`, so a turn that outlives one cannot restore the history it cleared.
    generation: u64,
}

struct Session {
    /// The configuration the session was built from. A `SetProfile` re-derives the brain from it, so
    /// a switch takes effect on the next turn.
    config: Config,
    brain: Arc<Brain>,
    brain_config: BrainConfig,
    /// The live provider stack (Task 6): read once per turn, swapped in place by a save.
    providers: ProviderSlot,
    out: UnboundedSender<ToClient>,
    history: Vec<ChatMessage>,
    current: Option<Turn>,
    generation: u64,
    /// True between `ListenStart` and `ListenStop`.
    listening: bool,
    /// Mono 16 kHz samples gathered while listening, in `push` mode.
    mic: Vec<i16>,
    /// The `auto` mode detector, paused while the assistant speaks and for 300 ms after.
    vad: VadState,
    /// True from the frame that reports `SpeechStarted` until the one that reports `SpeechEnded`.
    heard: bool,
    /// The audio the detector confirmed: what `SpeechEnded` transcribes.
    utterance: Vec<i16>,
    /// When the 300 ms guard after the assistant's speech expires, if it is running.
    guard: Option<Instant>,
    profile: ProfileView,
    ui_language: UiLanguage,
    listening_mode: ListeningMode,
    greeting: String,
}

impl Session {
    async fn run(&mut self, mut rx: UnboundedReceiver<ToServer>) {
        self.welcome();
        // The greeting is an ordinary turn: while it plays, `Cancel` still works.
        let greeting = self.greeting.clone();
        self.current = Some(self.start_turn(TurnInput::Say(greeting)));

        loop {
            // The guard's deadline is read once per iteration, so the timer's branch borrows nothing
            // the turn's branch is already borrowing. A turn that ends below arms the next
            // iteration's deadline, and a turn that starts clears it: the deadline read here is
            // never one a later turn has already left behind, because the message branch wins the
            // iteration that handles the command and the next iteration reads the cleared guard.
            let guard = self.guard;
            tokio::select! {
                // The running turn is polled first, so a turn that finished applies its history
                // before the command that follows it is handled.
                biased;
                result = async {
                    match self.current.as_mut() {
                        Some(turn) => (&mut turn.done).await,
                        // No turn runs: this branch can never win.
                        None => std::future::pending().await,
                    }
                } => {
                    let history = result.ok();
                    if let Some(turn) = self.current.take() {
                        self.end_turn(turn, history);
                    }
                }
                message = rx.recv() => {
                    let Some(message) = message else {
                        break;
                    };
                    self.handle(message, &mut rx);
                }
                // The 300 ms the detector waits out after the assistant stops speaking. A command
                // arriving at the same moment is handled first, and the deadline has elapsed either
                // way: this branch wins on the next turn of the loop.
                () = async {
                    match guard {
                        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                        // No guard runs: this branch can never win.
                        None => std::future::pending().await,
                    }
                } => {
                    self.guard_elapsed();
                }
            }
            // A turn that ended is settled here, where the borrow the select held is over. A turn
            // the command just started has no history yet and is left alone.
            self.settle_finished_turn().await;
        }
        // Nothing may keep speaking after the client hung up.
        self.cancel_running();
    }

    /// Applies the history of a turn that has ended, if the running one has ended.
    async fn settle_finished_turn(&mut self) {
        if self.current.as_ref().is_some_and(|turn| turn.handle.is_finished()) {
            let mut turn = self.current.take().expect("checked just above");
            let history = (&mut turn.done).await.ok();
            self.end_turn(turn, history);
        }
    }

    /// Ends a turn: records the history it produced, and arms the 300 ms guard the detector stays
    /// paused for while the speaker drains what the turn has already sent it.
    ///
    /// Every path that ends a turn goes through here, because the guard is what lets the microphone
    /// back in: a turn whose end did not arm it would leave the detector paused for good.
    fn end_turn(&mut self, turn: Turn, produced: Option<Vec<ChatMessage>>) {
        self.finish_turn(turn, produced);
        self.guard = Some(Instant::now() + GUARD);
    }

    /// The guard elapsed: the speaker is quiet, so the microphone is the user's again.
    fn guard_elapsed(&mut self) {
        self.guard = None;
        self.vad.resume();
    }

    /// Applies the history of a turn that ended, unless a `Reset` has since cleared the history.
    fn finish_turn(&mut self, turn: Turn, produced: Option<Vec<ChatMessage>>) {
        if let Some(history) = produced {
            if turn.generation == self.generation {
                self.history = history;
                trim_history(&mut self.history);
            }
        }
    }

    fn welcome(&self) {
        send(
            &self.out,
            ToClient::Event(Event::Welcome {
                session_id: SESSION_ID,
                profile: self.profile.clone(),
                ui_language: self.ui_language,
                listening_mode: self.listening_mode,
                web_bundle: false,
                // The screen itself is the capture's to announce (Task 3), where
                // `login::applies(&config.login)` is the one authority on whether there is a screen
                // at all. `None` is this task's honest placeholder: no client reads the field until
                // the screen is wired, and inventing a second `applies` here is what the plan's
                // "one rule, in one place" forbids.
                login: None,
            }),
        );
    }

    /// Handles one message from the client and everything already queued behind it, so a
    /// `ListenStop` never leaves the tail of an utterance waiting behind it. A command that starts
    /// a turn also stops the draining, since the turn replaces whatever was queued.
    fn handle(&mut self, first: ToServer, rx: &mut UnboundedReceiver<ToServer>) {
        let mut message = first;
        loop {
            self.dispatch(message);
            match rx.try_recv() {
                Ok(next) => message = next,
                Err(_) => return,
            }
        }
    }

    fn dispatch(&mut self, message: ToServer) {
        match message {
            ToServer::Command(command) => self.command(command),
            ToServer::Audio(Frame::Mic(samples)) => {
                // Frames arriving before `ListenStart` — or after `ListenStop` — belong to no turn.
                if !self.listening {
                    return;
                }
                match self.listening_mode {
                    // `auto`: the detector decides what is an utterance (spec §6), and only what it
                    // confirmed is ever transcribed.
                    ListeningMode::Auto => self.detect(&samples),
                    // `push`, and `hands_free` until Plan 4 implements it: the client's frames are
                    // the utterance, and `ListenStop` transcribes them.
                    ListeningMode::Push | ListeningMode::HandsFree => self.mic.extend_from_slice(&samples),
                }
            }
            ToServer::Audio(Frame::Speech { .. }) => {}
        }
    }

    /// Feeds one microphone frame to the detector and keeps only what it confirms.
    ///
    /// This function is Review Focus 2: nothing before `SpeechStarted` is kept, so the silence and
    /// the room noise before the first syllable can never reach the transcriber; and `SpeechEnded`
    /// closes the utterance and starts it, so the assistant's own voice — paused away by
    /// `start_turn` — cannot open one either.
    ///
    /// The detector's own carry buffer and [`Session::utterance`] are not the same thing: the first
    /// holds samples waiting for a 256-sample window, the second holds audio confirmed as the
    /// user's.
    fn detect(&mut self, samples: &[i16]) {
        match self.vad.push(samples) {
            VadEvent::SpeechStarted => {
                self.heard = true;
                // Nothing was kept before this frame: not the room tone and not the silence the
                // user started speaking through.
                self.utterance.clear();
                self.utterance.extend_from_slice(samples);
            }
            VadEvent::SpeechEnded => {
                self.heard = false;
                // The frame that reports the end is the last of the hangover: it is part of the
                // utterance, which is why it is pushed before the turn starts.
                self.utterance.extend_from_slice(samples);
                self.utterance_ended();
            }
            VadEvent::None => {
                if self.heard {
                    self.utterance.extend_from_slice(samples);
                }
            }
        }
    }

    /// The detector says the user stopped talking: the audio it confirmed becomes the next turn.
    fn utterance_ended(&mut self) {
        let samples = std::mem::take(&mut self.utterance);
        if samples.is_empty() {
            // Nothing confirmed and nothing kept: no turn, and no transcription request for audio
            // nobody heard.
            return;
        }
        self.start(TurnInput::Utterance(Pcm {
            samples: i16_to_f32(&samples),
            sample_rate: MIC_SAMPLE_RATE,
        }));
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Ask { text } => self.ask(text),
            Command::Say { text } => self.say(text),
            Command::ListenStart => self.listen_start(),
            Command::ListenStop => self.listen_stop(),
            Command::Cancel => self.cancel(),
            Command::Reset => self.reset(),
            Command::SetProfile { id } => self.set_profile(id),
            Command::SetListeningMode { mode } => self.set_listening_mode(mode),
            Command::Hello { .. } => {}
            // The Test probe asks a provider a question, not the session: Plan 2B Task 7 runs it in
            // the native client and Plan 3's server answers it over the wire. Accepted and ignored,
            // so the match stays exhaustive.
            Command::TestProvider { .. } => {}
            // The login screen's own three commands drive a capture this task does not have yet
            // (Task 3 owns it, together with the `welcome` that announces the screen). Accepted and
            // ignored for now, so the match stays exhaustive and this task owns no behaviour here.
            Command::LoginStart | Command::LoginStop | Command::LoginAnswer { .. } => {}
        }
    }

    /// Applies the profile the configuration screen saved and tells the client about it, so the
    /// header, the interface language and the next turn all follow the switch.
    ///
    /// The edited *texts* of a profile cannot travel the channel (`Command` carries an id), so this
    /// re-derives the brain from the configuration the session holds: an id that is not in it is a
    /// no-op rather than a brain built around nothing.
    fn set_profile(&mut self, id: String) {
        if !self.config.profiles.iter().any(|profile| profile.id == id) {
            return;
        }
        self.config.active_profile = id;
        let profile = self.config.active();
        self.profile = profile.view();
        self.ui_language = UiLanguage::from_bcp47(&profile.language);
        self.brain = Arc::new(Brain::new(
            self.providers.clone(),
            turn_profile(&self.config),
            self.brain_config.clone(),
        ));
        self.welcome();
    }

    /// Applies the listening mode a save changed. The mode lives in `Welcome` and nowhere else, so
    /// the session announces it again — that is what moves the header's indicator.
    fn set_listening_mode(&mut self, mode: ListeningMode) {
        self.listening_mode = mode;
        self.welcome();
    }

    fn ask(&mut self, text: String) {
        if text.trim().is_empty() {
            // An empty question is not worth a provider call, and leaves a turn in flight alone.
            self.idle();
            return;
        }
        self.start(TurnInput::Ask(text));
    }

    fn say(&mut self, text: String) {
        if text.trim().is_empty() {
            // A bare `!` in the HUD sends an empty `Say`; skipping it keeps it free.
            self.idle();
            return;
        }
        self.start(TurnInput::Say(text));
    }

    fn listen_start(&mut self) {
        self.listening = true;
        self.mic.clear();
        // A fresh hold-to-talk: whatever the detector had confirmed but not ended is not this
        // utterance. The 300 ms guard is deliberately left alone — a `Tab` inside it opens the
        // microphone while the detector stays paused, so the assistant's own tail cannot be heard
        // as the user's first word.
        self.utterance.clear();
        self.heard = false;
        send(&self.out, status(AssistantState::Listening));
    }

    /// `ListenStop`: the only way `push` mode ever ends an utterance, and the way `auto` mode ends
    /// one of those the detector has not closed by itself.
    fn listen_stop(&mut self) {
        self.listening = false;
        // What the detector confirmed and has not ended yet is the utterance the user just closed
        // by hand. In `push` mode that is every frame the microphone sent since `ListenStart`: the
        // buffer is taken here, which is also what empties it. Clearing it first — as this did —
        // is what stopped `push` mode transcribing the audio it had gathered.
        let samples = if self.listening_mode == ListeningMode::Auto {
            self.heard = false;
            std::mem::take(&mut self.utterance)
        } else {
            std::mem::take(&mut self.mic)
        };
        if samples.is_empty() {
            // Nothing was heard: no turn, and no transcription request for silence. Pressing `Tab`
            // twice in `auto` mode is the case this keeps free.
            self.idle();
            return;
        }
        self.start(TurnInput::Utterance(Pcm {
            samples: i16_to_f32(&samples),
            sample_rate: MIC_SAMPLE_RATE,
        }));
    }

    /// `Cancel`: stop the running turn without starting another one.
    fn cancel(&mut self) {
        // The speaker keeps draining what the turn already sent it, so the detector stays paused for
        // the tail a finished turn would have left, even though no new turn follows this one. Losing
        // it would leave `auto` mode deaf until some later turn ended.
        let cancelled = self.current.is_some();
        self.cancel_running();
        if cancelled {
            self.guard = Some(Instant::now() + GUARD);
        }
        self.idle();
    }

    /// `Reset`: stop the running turn and forget the conversation.
    ///
    /// A reset is a hard reset, and the detector is part of it: it is resumed here rather than
    /// through a guard, because `resume` is reachable only from [`Session::guard_elapsed`] and a
    /// reset that merely dropped the guard would make `auto` mode deaf for good — `Tab` opens the
    /// microphone, frames arrive, `push` reports nothing and nothing is ever transcribed until some
    /// later turn ends. That is the common case, too: clearing a conversation is exactly when a user
    /// reaches for F4. Nothing is left playing to be heard, because the cancelled turn's audio is
    /// flushed with the rest.
    fn reset(&mut self) {
        self.cancel_running();
        self.history.clear();
        self.mic.clear();
        self.utterance.clear();
        self.heard = false;
        self.guard = None;
        self.vad.resume();
        self.listening = false;
        // A turn that ignored the cancellation must not restore what this just cleared.
        self.generation += 1;
        self.idle();
    }

    /// Barge-in: a new turn ends the running one, then takes its place.
    fn start(&mut self, input: TurnInput) {
        self.cancel_running();
        self.current = Some(self.start_turn(input));
    }

    /// Stops the running turn, if any. The history a cancelled turn produced is deliberately
    /// dropped: the session only ever applies the history of a turn that finished on its own.
    fn cancel_running(&mut self) {
        if let Some(turn) = self.current.take() {
            turn.cancel.cancel();
        }
    }

    fn idle(&self) {
        send(&self.out, status(AssistantState::Idle));
    }

    /// Spawns one turn with a fresh cancellation token and the history as it stands now.
    fn start_turn(&mut self, input: TurnInput) -> Turn {
        // A deadline left over from the turn before this one must not outlive it: the guard resumes
        // the detector when it elapses, and a new turn starting inside the previous turn's guard
        // would be un-paused mid-turn — over the assistant's own voice, which is the one thing spec
        // §6's pause and Review Focus 2 forbid. The detector is paused for the whole turn anyway,
        // and `end_turn` re-arms the guard when this turn really ends.
        self.guard = None;
        // The assistant is about to speak, and the speaker is what the detector would otherwise
        // hear. Pausing at the start of the turn also covers synthesis and transcription, which is
        // stricter than spec §6's "while the session is speaking" and never lets a leak through.
        self.vad.pause();
        self.heard = false;
        self.generation += 1;
        let generation = self.generation;
        let cancel = CancellationToken::new();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let mut history = self.history.clone();
        let brain = Arc::clone(&self.brain);
        let out = self.out.clone();
        let token = cancel.clone();
        let handle = tokio::spawn(async move {
            brain.run_turn(&mut history, input, out, token).await;
            // Only the session may record the history, and only if it still wants it.
            let _ = done_tx.send(history);
        });
        Turn { cancel, done: done_rx, handle, generation }
    }
}

/// Trims the stored conversation to its most recent [`HISTORY_MAX_MESSAGES`] messages, dropping the
/// oldest first.
///
/// The history is what every turn clones and what the brain prepends to the prompt, so without a cap
/// both the prompt and its cost would grow for the life of the session. The system message is not
/// part of what is stored here — the brain builds it from the profile prompt on every request — so
/// trimming the tail can never drop it.
///
/// A cancelled turn that never spoke leaves a question with no reply behind it, which can put the
/// cut on the reply that follows it. The window is then moved forward to the next question, so it
/// always opens with something the user asked rather than with an answer to a question that is gone.
fn trim_history(history: &mut Vec<ChatMessage>) {
    let excess = history.len().saturating_sub(HISTORY_MAX_MESSAGES);
    if excess > 0 {
        history.drain(..excess);
    }
    if history.first().is_some_and(|message| message.role == Role::Assistant) {
        history.remove(0);
    }
}

/// A status event with no detail.
fn status(state: AssistantState) -> ToClient {
    ToClient::Event(Event::Status { state, detail: None })
}

/// Sending to a closed client is not an error: the client simply left.
fn send(out: &UnboundedSender<ToClient>, message: ToClient) {
    let _ = out.send(message);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One question and its reply, numbered so a window can be checked message by message.
    fn exchange(n: usize) -> [ChatMessage; 2] {
        [
            ChatMessage::user(format!("pregunta {n}")),
            ChatMessage::assistant(format!("respuesta {n}")),
        ]
    }

    fn history_of(exchanges: usize) -> Vec<ChatMessage> {
        (0..exchanges).flat_map(exchange).collect()
    }

    #[test]
    fn a_history_under_the_cap_is_left_alone() {
        let mut history = history_of(3);
        let before = history.clone();
        trim_history(&mut history);
        assert_eq!(history, before);
    }

    #[test]
    fn the_oldest_exchanges_are_the_ones_dropped() {
        // Fifty exchanges against a forty-message cap: the newest twenty exchanges are what stays.
        let mut history = history_of(50);
        assert_eq!(history.len(), 100);

        trim_history(&mut history);

        assert_eq!(history.len(), HISTORY_MAX_MESSAGES);
        assert_eq!(history[0], ChatMessage::user("pregunta 30"));
        assert_eq!(history.last().unwrap(), &ChatMessage::assistant("respuesta 49"));
        assert!(!history.iter().any(|m| m.content == "pregunta 0"), "{history:#?}");
    }

    #[test]
    fn a_long_conversation_stops_growing_the_prompt() {
        // The cap is what keeps the prompt — and what every turn costs — flat for the life of the
        // session, however long the conversation runs.
        let mut history = Vec::new();
        for n in 0..500 {
            history.extend(exchange(n));
            trim_history(&mut history);
            assert!(history.len() <= HISTORY_MAX_MESSAGES, "{n}: {}", history.len());
        }
        assert_eq!(history.last().unwrap(), &ChatMessage::assistant("respuesta 499"));
    }

    #[test]
    fn the_window_opens_with_a_question() {
        // A cancelled turn that never spoke leaves a question with no reply behind it, so an odd
        // history can put the cut on a reply whose question fell off the front.
        let mut history = history_of(21);
        history.push(ChatMessage::user("pregunta sin respuesta"));
        assert_eq!(history.len(), 43);

        trim_history(&mut history);

        assert_eq!(history[0].role, Role::User, "the window must open with a question: {history:#?}");
        assert_eq!(history.last().unwrap().content, "pregunta sin respuesta");
        assert!(history.len() <= HISTORY_MAX_MESSAGES);
    }
}
