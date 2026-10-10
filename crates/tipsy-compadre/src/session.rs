//! The server side of a session: one conversation, one provider stack, one running turn.
//!
//! The session owns the history and the microphone buffer. Each turn runs in its own task with a
//! fresh [`CancellationToken`], and hands the history it produced back to the session through a
//! oneshot — so the history has exactly one owner and a cancelled turn can never write into it.

use std::sync::Arc;

use tipsy_protocol::{AssistantState, Command, Event, Frame, ListeningMode, ProfileView, ToClient, ToServer, UiLanguage};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::audio::device::MIC_SAMPLE_RATE;
use crate::audio::pcm::{i16_to_f32, Pcm};
use crate::brain::{Brain, BrainConfig, TurnInput, TurnProfile};
use crate::config::Config;
use crate::providers::{ChatMessage, Providers, Role};

/// The session id reported to clients. There is one session per process for now.
const SESSION_ID: u64 = 1;

/// How much conversation the session keeps. An exchange is a question and its reply, so this is 20
/// exchanges: deep enough that a follow-up can still refer to what was said a while ago, shallow
/// enough that the prompt — and what every turn costs — stops growing with the life of the session.
const HISTORY_MAX_MESSAGES: usize = 40;

/// Starts a session on its own task and returns the two ends of its channels.
pub fn spawn_session(
    config: Config,
    providers: Providers,
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
pub fn spawn_with_events(config: Config, providers: Providers, brain_config: BrainConfig) -> SessionEnds {
    let (to_server_tx, to_server_rx) = mpsc::unbounded_channel();
    let (to_client_tx, to_client_rx) = mpsc::unbounded_channel();
    // Held so the client end only closes once the session task itself has ended.
    let keep_alive = to_client_tx.clone();

    let profile = config.active().clone();
    let listening_mode = config.listening.mode;
    let brain = Arc::new(Brain::new(
        providers,
        TurnProfile {
            prompt: profile.prompt.clone(),
            voice_instructions: profile.voice_instructions.clone(),
            voice: profile.voice_for(&config.providers.speech),
            language: profile.language.clone(),
        },
        brain_config,
    ));

    let mut session = Session {
        brain,
        out: to_client_tx.clone(),
        history: Vec::new(),
        current: None,
        generation: 0,
        listening: false,
        mic: Vec::new(),
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
    brain: Arc<Brain>,
    out: UnboundedSender<ToClient>,
    history: Vec<ChatMessage>,
    current: Option<Turn>,
    generation: u64,
    /// True between `ListenStart` and `ListenStop`.
    listening: bool,
    /// Mono 16 kHz samples gathered while listening.
    mic: Vec<i16>,
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
                        self.finish_turn(turn, history);
                    }
                }
                message = rx.recv() => {
                    let Some(message) = message else {
                        break;
                    };
                    self.handle(message, &mut rx);
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
            self.finish_turn(turn, history);
        }
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
                if self.listening {
                    self.mic.extend_from_slice(&samples);
                }
            }
            ToServer::Audio(Frame::Speech { .. }) => {}
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Ask { text } => self.ask(text),
            Command::Say { text } => self.say(text),
            Command::ListenStart => self.listen_start(),
            Command::ListenStop => self.listen_stop(),
            Command::Cancel => self.cancel(),
            Command::Reset => self.reset(),
            Command::SetProfile { id } => {
                self.profile.id = id;
            }
            Command::SetListeningMode { mode } => {
                self.listening_mode = mode;
            }
            Command::Hello { .. } => {}
            // The Test probe asks a provider a question, not the session: Plan 2B Task 7 runs it in
            // the native client and Plan 3's server answers it over the wire. Accepted and ignored,
            // so the match stays exhaustive.
            Command::TestProvider { .. } => {}
        }
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
        send(&self.out, status(AssistantState::Listening));
    }

    fn listen_stop(&mut self) {
        self.listening = false;
        let samples = std::mem::take(&mut self.mic);
        self.start(TurnInput::Utterance(Pcm {
            samples: i16_to_f32(&samples),
            sample_rate: MIC_SAMPLE_RATE,
        }));
    }

    /// `Cancel`: stop the running turn without starting another one.
    fn cancel(&mut self) {
        self.cancel_running();
        self.idle();
    }

    /// `Reset`: stop the running turn and forget the conversation.
    fn reset(&mut self) {
        self.cancel_running();
        self.history.clear();
        self.mic.clear();
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
