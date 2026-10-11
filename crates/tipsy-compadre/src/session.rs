//! The server side of a session: one conversation, one provider stack, one running turn.
//!
//! The session owns the history and the microphone buffer. Each turn runs in its own task with a
//! fresh [`CancellationToken`], and hands the history it produced back to the session through a
//! oneshot — so the history has exactly one owner and a cancelled turn can never write into it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tipsy_protocol::{
    AssistantState, Command, Event, Frame, ListeningMode, LoginView, ProfileView, ToClient, ToServer, UiLanguage,
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::audio::device::MIC_SAMPLE_RATE;
use crate::audio::pcm::{i16_to_f32, Pcm};
use crate::brain::{Brain, BrainConfig, TurnInput, TurnProfile};
use crate::config::Config;
use crate::providers::{ChatMessage, ProviderSlot, Role};
use crate::vad::{Scorer, VadEvent, VadState};

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

/// Samples of continuous speech after which a login utterance is closed whether or not the detector
/// has ended it: a login answer is a sentence, and a transcription bill is not a joke (spec §4.1).
const LOGIN_MAX_UTTERANCE_SAMPLES: usize = MIC_SAMPLE_RATE as usize * 15;

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
        login_active: false,
        login_mic: Vec::new(),
        login_heard: false,
        login_vad: VadState::with_scorer(
            Box::new(crate::vad::Earshot::default()) as Box<dyn Scorer + Send>,
            listening_silence_ms,
        ),
        login_turn: None,
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
    /// True between `LoginStart` and `LoginStop`: the microphone's frames belong to the login screen
    /// and to nothing else (spec §4.2).
    login_active: bool,
    /// The login capture's own audio, and the detector that decides what in it is speech. Both are
    /// separate from `mic`/`vad` on purpose: a capture and a listening session must never share a
    /// carry buffer, and they can never be open at the same time.
    login_mic: Vec<i16>,
    login_heard: bool,
    /// Boxed rather than `Earshot` so a test can hand it a script: `earshot` cannot be asked to call
    /// synthetic audio speech, and the capture's logic is what needs testing.
    login_vad: VadState<Box<dyn Scorer + Send>>,
    /// The login transcription in flight, if any: a newer utterance replaces it (spec §4.1).
    login_turn: Option<CancellationToken>,
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
                // The one authority on whether a client shows the login screen. A native client never
                // reads `[login]` itself, so it cannot disagree with the server that will judge it.
                // Sent on every `Welcome`; the client shows the screen once per run and ignores the
                // later ones.
                login: crate::login::applies(&self.config.login)
                    .then_some(LoginView { voice: self.config.login.voice }),
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
                // An open login capture owns the microphone: its frames are the answer being spoken,
                // and feeding them to the listening path as well would transcribe the login twice —
                // once as an answer and once as a question (spec §4.2).
                if self.login_active {
                    self.login_frame(&samples);
                    return;
                }
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
        // While the door is shut, the commands that belong to the other side of it are ignored (spec
        // §4.2): no client of ours can send them — the screen is modal and has no other action — and
        // inventing an error for a state no user can reach would be wire surface with no user behind
        // it. `Hello` is unaffected: the handshake happens before the screen exists.
        //
        // Spec §4.2 asks for a `tracing::debug!` here and this crate has no `tracing` (and the plan's
        // Global Constraints forbid a new dependency), so the ignore is documented rather than
        // logged: there is no subscriber in this process for a debug line to reach.
        if self.login_active
            && !matches!(
                command,
                Command::LoginStart | Command::LoginStop | Command::LoginAnswer { .. } | Command::Hello { .. }
            )
        {
            return;
        }
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
            Command::LoginStart => self.login_start(),
            Command::LoginStop => self.login_stop(),
            Command::LoginAnswer { text } => self.login_answer(text),
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

    /// `LoginStart`: open the capture. Ignored when there is no screen to serve, and when a capture is
    /// already open — a repeated start must not eat an utterance in flight (spec §4.1).
    fn login_start(&mut self) {
        if !crate::login::applies(&self.config.login) || self.login_active {
            return;
        }
        self.login_active = true;
        self.login_mic.clear();
        self.login_heard = false;
        // A fresh detector, without throwing away the scorer this session was built with: `resume`
        // leaves exactly the state `VadState::new` starts in — the counters at zero and the carry
        // dropped — and keeping the scorer is what lets a test hand the capture its own script. The
        // plan rebuilt the box here, which replaced the scripted scorer with earshot and left the
        // capture unable to hear synthetic audio. Production builds the session with earshot, so the
        // server's own behaviour is the same either way.
        self.login_vad.resume();
    }

    /// `LoginStop`: close the capture, drop what it held, and cancel the transcription in flight with
    /// it — a verdict that arrives after the screen is gone has nothing to answer.
    fn login_stop(&mut self) {
        self.login_active = false;
        self.login_mic.clear();
        self.login_heard = false;
        self.cancel_login_turn();
    }

    /// Stops the login transcription in flight, if any.
    ///
    /// A `CancellationToken` is cancelled by `cancel()` and by nothing else: replacing or dropping the
    /// handle leaves the task running to its own end, so both places the capture lets go of one — a
    /// newer utterance (spec §4.1: the last thing said is the thing that counts) and `LoginStop` —
    /// call this instead of just forgetting the token.
    fn cancel_login_turn(&mut self) {
        if let Some(in_flight) = self.login_turn.take() {
            in_flight.cancel();
        }
    }

    /// `LoginAnswer`: a typed answer, judged here and now. Judged whether or not a capture is open, so
    /// a typing-only client never has to send `LoginStart` (spec §4.1). When the screen does not apply
    /// the ruling opens: there is nothing to check, so nobody is refused.
    fn login_answer(&mut self, text: String) {
        let verdict = crate::login::judge(&self.config.login, &text);
        send(&self.out, ToClient::Event(Event::LoginVerdict { ok: verdict.ok, name: verdict.name }));
    }

    /// One microphone frame of an open capture: the same detector discipline `auto` mode uses, with
    /// its own buffer — nothing before a confirmed onset is kept, and only confirmed speech is ever
    /// transcribed.
    fn login_frame(&mut self, samples: &[i16]) {
        match self.login_vad.push(samples) {
            VadEvent::SpeechStarted => {
                self.login_heard = true;
                self.login_mic.clear();
                self.login_mic.extend_from_slice(samples);
            }
            VadEvent::SpeechEnded => {
                self.login_heard = false;
                self.login_mic.extend_from_slice(samples);
                self.login_utterance_ended();
                return;
            }
            VadEvent::None => {
                if self.login_heard {
                    self.login_mic.extend_from_slice(samples);
                }
            }
        }
        // A monologue is not an answer: the cap closes the utterance the detector has not.
        if self.login_heard && self.login_mic.len() >= LOGIN_MAX_UTTERANCE_SAMPLES {
            self.login_heard = false;
            self.login_utterance_ended();
        }
    }

    /// The capture's detector says the answer is over: transcribe it in a task of its own, exactly as a
    /// turn runs, and let a newer utterance replace this one.
    fn login_utterance_ended(&mut self) {
        let samples = std::mem::take(&mut self.login_mic);
        if samples.is_empty() {
            return;
        }
        // Spec §4.1: a newer utterance *cancels* the transcription in flight, so a stale verdict can
        // never land after a fresh one.
        self.cancel_login_turn();
        let cancel = CancellationToken::new();
        self.login_turn = Some(cancel.clone());
        let brain = Arc::clone(&self.brain);
        let out = self.out.clone();
        let login = self.config.login.clone();
        tokio::spawn(async move {
            send(&out, status(AssistantState::Transcribing));
            let pcm = Pcm { samples: i16_to_f32(&samples), sample_rate: MIC_SAMPLE_RATE };
            if let Some(text) = brain.transcribe_once(pcm, &out, &cancel).await {
                send(&out, ToClient::Event(Event::LoginHeard { text: text.clone() }));
                let verdict = crate::login::judge(&login, &text);
                send(&out, ToClient::Event(Event::LoginVerdict { ok: verdict.ok, name: verdict.name }));
            }
            send(&out, status(AssistantState::Idle));
        });
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

    use crate::audio::device::MIC_FRAME_SAMPLES;
    use crate::config::{LoginConfig, LoginEntry};
    use crate::providers::{
        ChatProvider, ChatRequest, ProviderError, Providers, SpeechProvider, SpeechRequest, TextStream,
        TranscribeProvider,
    };
    use crate::vad::{Scorer, scripted::Scripted};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::timeout;

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

    /// A chat provider that must never be asked anything. It counts the calls instead of panicking
    /// because panicking inside the spawned turn would be reported as a task failure, not as the
    /// honest assertion this test wants.
    struct NeverChat(Arc<AtomicUsize>);

    #[async_trait]
    impl ChatProvider for NeverChat {
        async fn stream(&self, _req: ChatRequest) -> Result<TextStream, ProviderError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(ProviderError::BadResponse("the login path must not ask for a reply".into()))
        }
    }

    /// A speech provider that must never be asked anything either.
    struct NeverSpeech;

    #[async_trait]
    impl SpeechProvider for NeverSpeech {
        async fn synthesize(&self, _req: SpeechRequest) -> Result<Pcm, ProviderError> {
            Err(ProviderError::BadResponse("the login path must not speak".into()))
        }
    }

    struct FakeTranscribe(&'static str);

    #[async_trait]
    impl TranscribeProvider for FakeTranscribe {
        async fn transcribe(&self, _pcm: Pcm, _language: &str) -> Result<String, ProviderError> {
            Ok(self.0.to_string())
        }
    }

    /// A session whose transcriber answers `transcript`, whose chat and speech providers count their
    /// calls, and whose login detector answers `scores` in order. The listening detector is the real
    /// one: these tests never open it.
    fn login_session(
        config: Config,
        transcript: &'static str,
        scores: Vec<f32>,
    ) -> (Session, UnboundedReceiver<ToClient>, Arc<AtomicUsize>) {
        let chat_calls = Arc::new(AtomicUsize::new(0));
        let providers = ProviderSlot::new(Providers {
            chat: Arc::new(NeverChat(Arc::clone(&chat_calls))),
            speech: Arc::new(NeverSpeech),
            transcribe: Arc::new(FakeTranscribe(transcript)),
        });
        let brain_config = BrainConfig::default();
        let brain = Arc::new(Brain::new(providers.clone(), turn_profile(&config), brain_config.clone()));
        let (out, rx) = mpsc::unbounded_channel();
        let silence_ms = config.listening.silence_ms;
        let profile = config.active().clone();
        let session = Session {
            config,
            brain,
            brain_config,
            providers,
            out,
            history: Vec::new(),
            current: None,
            generation: 0,
            listening: false,
            mic: Vec::new(),
            vad: VadState::new(silence_ms),
            heard: false,
            utterance: Vec::new(),
            guard: None,
            profile: profile.view(),
            ui_language: UiLanguage::from_bcp47(&profile.language),
            listening_mode: ListeningMode::Push,
            greeting: profile.greeting.clone(),
            login_active: false,
            login_mic: Vec::new(),
            login_heard: false,
            login_vad: VadState::with_scorer(Box::new(Scripted::new(scores)) as Box<dyn Scorer + Send>, silence_ms),
            login_turn: None,
        };
        (session, rx, chat_calls)
    }

    /// The script of one confirmed utterance: three voiced windows to open it (`ONSET_FRAMES`), then
    /// the silence budget to close it, then zeros for whatever the detector asks after that.
    fn one_utterance(silence_ms: u32) -> Vec<f32> {
        let mut scores = vec![0.9; 3];
        scores.extend(std::iter::repeat_n(0.0, silence_ms.div_ceil(16) as usize + 1));
        scores
    }

    /// Drives `frames` microphone frames of 20 ms into the session, as a client would.
    fn speak(session: &mut Session, frames: usize) {
        for _ in 0..frames {
            session.dispatch(ToServer::Audio(Frame::Mic(vec![0i16; MIC_FRAME_SAMPLES])));
        }
    }

    /// Everything the session sent until the verdict (or until two seconds pass, which fails the test
    /// rather than hanging it).
    async fn events_until_verdict(rx: &mut UnboundedReceiver<ToClient>) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let next = timeout(Duration::from_secs(2), rx.recv()).await.expect("the verdict must arrive").expect("the channel stays open");
            if let ToClient::Event(event) = next {
                let verdict = matches!(event, Event::LoginVerdict { .. });
                seen.push(event);
                if verdict {
                    return seen;
                }
            }
        }
    }

    fn allowed(name: &str, keywords: &[&str]) -> LoginConfig {
        LoginConfig { enabled: true, voice: true, allowed: vec![LoginEntry { name: name.into(), keywords: keywords.iter().map(|k| (*k).into()).collect() }] }
    }

    /// The whole voice path, and the three things that must *not* happen on it: no chat call, no
    /// speech, no turn. `transcribe_once` is what keeps the login out of the conversation.
    #[tokio::test]
    async fn a_login_utterance_becomes_a_verdict_without_a_turn() {
        let config = Config { login: allowed("Andrés", &["andres", "core"]), ..Config::default() };
        let (mut session, mut rx, chat_calls) = login_session(config, "yo soy Andrés del equipo del core", one_utterance(1000));

        session.command(Command::LoginStart);
        assert!(session.login_active, "the capture opens");
        speak(&mut session, 70);

        let events = events_until_verdict(&mut rx).await;
        assert!(events.iter().any(|e| matches!(e, Event::Status { state: AssistantState::Transcribing, .. })), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, Event::LoginHeard { text } if text == "yo soy Andrés del equipo del core")), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, Event::LoginVerdict { ok: true, name: Some(name) } if name == "Andrés")), "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, Event::Reply { .. } | Event::ReplyDelta { .. } | Event::SpeechChunk { .. })), "the login path never replies: {events:?}");
        assert_eq!(chat_calls.load(Ordering::SeqCst), 0, "the chat provider is never asked");
        assert!(session.history.is_empty(), "and the conversation is untouched");
    }

    /// The same capture, refusing: the keywords are not there, so the verdict says no and the capture
    /// stays open for the next attempt (spec §6.2 — the automatic re-arm).
    #[tokio::test]
    async fn a_login_utterance_without_the_keywords_is_refused_and_the_capture_stays_open() {
        let config = Config { login: allowed("Andrés", &["andres", "core"]), ..Config::default() };
        let (mut session, mut rx, _) = login_session(config, "soy Andrés del score", one_utterance(1000));

        session.command(Command::LoginStart);
        speak(&mut session, 70);

        let events = events_until_verdict(&mut rx).await;
        assert!(events.iter().any(|e| matches!(e, Event::LoginVerdict { ok: false, .. })), "{events:?}");
        assert!(session.login_active, "the next utterance is the next attempt");
    }

    /// Silence is not an answer: no transcription request, `NothingHeard`, and no verdict at all — a
    /// cough must never be counted as a wrong answer (spec §4.1).
    #[tokio::test]
    async fn silence_asks_for_no_transcription_and_no_verdict() {
        let config = Config { login: allowed("Andrés", &["andres", "core"]), ..Config::default() };
        let (mut session, mut rx, _) = login_session(config, "unused", vec![0.0; 80]);

        session.command(Command::LoginStart);
        speak(&mut session, 70);

        let next = timeout(Duration::from_millis(200), rx.recv()).await;
        assert!(next.is_err(), "nothing is sent for silence, got {:?}", next.map(|m| m.map(|m| format!("{m:?}"))));
    }

    /// Spec §4.1: a newer utterance cancels the transcription in flight, so the last thing said is the
    /// thing that counts. The plan replaced the token without cancelling it, and a replaced
    /// `CancellationToken` stops nothing — the older transcription would have run to its own end and
    /// sent its verdict after the newer one's. The detector is not part of this rule, so the two
    /// utterances are closed directly.
    #[tokio::test]
    async fn a_new_utterance_cancels_the_transcription_in_flight() {
        let config = Config { login: allowed("Andrés", &["andres", "core"]), ..Config::default() };
        let (mut session, _rx, _) = login_session(config, "unused", Vec::new());

        session.login_mic = vec![0i16; MIC_FRAME_SAMPLES];
        session.login_utterance_ended();
        let first = session.login_turn.clone().expect("the first transcription is in flight");

        session.login_mic = vec![0i16; MIC_FRAME_SAMPLES];
        session.login_utterance_ended();
        let second = session.login_turn.clone().expect("the second transcription is in flight");

        assert!(first.is_cancelled(), "the newer utterance cancels the one in flight");
        assert!(!second.is_cancelled(), "and the newer one is left alone to answer");
    }
}
