//! The server side of a session: one conversation, one provider stack, one running turn.
//!
//! The session owns the history and the microphone buffer. Each turn runs in its own task with a
//! fresh [`CancellationToken`], and hands the history it produced back to the session through a
//! oneshot — so the history has exactly one owner and a cancelled turn can never write into it.

use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tipsy_protocol::{
    AssistantState, Command, ErrorCode, Event, Frame, ListeningMode, LoginView, PROTOCOL_VERSION, ProfileView,
    ToClient, ToServer, UiLanguage,
};
use tokio::sync::Notify;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::audio::device::MIC_SAMPLE_RATE;
use crate::audio::pcm::{Pcm, i16_to_f32};
use crate::brain::{Brain, BrainConfig, TurnInput, TurnProfile};
use crate::config::Config;
use crate::providers::{ChatMessage, ProviderSlot, Role};
use crate::server::Hub;
use crate::vad::{Scorer, VadEvent, VadState};

/// The session ids this process has handed out. One session per connection means the id can no
/// longer be a constant (spec §5.2): two clients connected at once have to be told apart, and
/// `Welcome.session_id` is what tells them apart. The first session is `1`, so a single-client run
/// reports what it always did.
static SESSION_IDS: AtomicU64 = AtomicU64::new(0);

/// The next session id: unique within the process, and never reused.
fn next_session_id() -> u64 {
    SESSION_IDS.fetch_add(1, Ordering::Relaxed) + 1
}

/// How long a session waits for its first message before it refuses the client that never spoke.
///
/// **This is the ceiling the route's pre-upgrade token check used to be.** `/ws` demanded `?token=…`
/// before the upgrade, which is 401 for a socket that says nothing — and it is also what made the
/// documented remote attach impossible (the client sends the token in `Hello`, never in the URL).
/// With that check gone the session is the only gate, so a peer that opens a socket and never says
/// anything must be bounded here or it holds a session — a cloned configuration, a `Brain` and a
/// detector — for as long as it stays connected. Ten seconds is far longer than a real handshake:
/// both clients put their `Hello` on the wire before the terminal is even entered, and a browser has
/// one round trip to make. It is short enough that a silent peer cannot accumulate sessions.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What this session requires of a client before it serves one (spec §4.1).
///
/// **The server's exposure is not read from the configuration.** `config.server.lan` is the user's
/// intent, and the only thing that knows whether that intent survived the bind is the listener:
/// `bind = "0.0.0.0:7070"` with `lan = false` is exposed all the same. The listener therefore says
/// so explicitly — `SessionPolicy::local()` for the in-process client, `SessionPolicy::exposed(token)`
/// for anything that is not loopback — and this type carries the answer rather than re-deriving it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPolicy {
    /// True when the server is reachable from somewhere other than this machine's loopback.
    pub require_token: bool,
    /// The token a client must present. Empty when none has been generated; a server that requires a
    /// token and has none refuses every client rather than accepting every one.
    pub token: String,
    /// True when the connection reaches this server only over loopback. `GetConfig`'s token rule
    /// (spec §4.3, Task 1.3) and `SetKey`'s refusal (spec §9.4, Task 1.8) are both decided from it.
    pub loopback: bool,
    /// How long the prelude waits for the first message before refusing the connection
    /// ([`HANDSHAKE_TIMEOUT`]). It is part of the policy because it is one more thing this session
    /// requires of a client before it serves one, and because a test can then pin the refusal in
    /// milliseconds instead of holding a ten-second timer.
    pub handshake_timeout: Duration,
}

impl SessionPolicy {
    /// A session reached over the in-process channel: no token, none asked for, and the token still
    /// crosses to it, because it is the client's own machine (spec §4.3).
    pub fn local() -> Self {
        Self { require_token: false, token: String::new(), loopback: true, handshake_timeout: HANDSHAKE_TIMEOUT }
    }

    /// A session behind a listener that is not loopback.
    pub fn exposed(token: impl Into<String>) -> Self {
        Self { require_token: true, token: token.into(), loopback: false, handshake_timeout: HANDSHAKE_TIMEOUT }
    }

    /// The one place "may this client be served?" is decided, so the version check, the missing token
    /// and the wrong token cannot drift apart. `Err` names the refusal with the `ErrorCode` that
    /// branches it on the wire (Task 1.1's three variants); the sentence a client shows is the
    /// session's own, because this type is not where an interface string belongs.
    ///
    /// A `Command` that does not parse is `ProtocolVersion` and not `BadMessage`: `Hello` is the only
    /// message allowed to be first, and a first message that is not `Hello` means the client is not
    /// speaking this protocol. `BadMessage` is for a later malformed frame on an open connection
    /// (spec §10), which is Task 1.6's path, not this one.
    fn admits(&self, hello: &Command) -> Result<(), ErrorCode> {
        let Command::Hello { token, protocol_version, .. } = hello else {
            return Err(ErrorCode::ProtocolVersion);
        };
        if *protocol_version != PROTOCOL_VERSION {
            return Err(ErrorCode::ProtocolVersion);
        }
        if !self.require_token {
            return Ok(());
        }
        match token.as_deref() {
            None => Err(ErrorCode::TokenRequired),
            Some(presented) if presented == self.token => Ok(()),
            Some(_) => Err(ErrorCode::TokenInvalid),
        }
    }
}

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

/// Samples of microphone audio one utterance may hold, in every mode that buffers without a detector
/// to end it. **This is a bound and not a policy**: a client that never stops streaming — the default
/// `push` mode's microphone, or `auto`'s detector held open by continuous noise — would otherwise grow
/// the session's buffer for as long as it stayed connected, and a `Frame` says nothing about how much
/// audio it may carry (`tipsy-protocol/src/lib.rs:465-480` gives the frame, not a ceiling).
///
/// 16 kHz mono i16, so this is **30 seconds**, about 960 KB. A push-to-talk sentence is a few seconds;
/// thirty is longer than anyone speaks into a walkie-talkie key, and it is far below the read ceiling
/// the socket enforces (`server::conn::READ_CEILING`, 256 KiB per message but unbounded across
/// messages). Past it the **oldest** audio goes, so what the user just said is what is transcribed:
/// dropping the newest would lose the question and keeping the first thirty seconds would answer the
/// wrong one.
const UTTERANCE_MAX_SAMPLES: usize = MIC_SAMPLE_RATE as usize * 30;

/// Appends one frame to an utterance buffer, keeping at most `max` samples by dropping the oldest.
///
/// One function for both buffers (`mic` in `push`/`hands_free`, `utterance` in `auto`) because the
/// rule is the same one and a second copy of it is a second place to forget. `Vec::drain` moves what
/// is left, which is at most 960 KB per frame once the ceiling is reached — a memmove of well under a
/// millisecond, and the alternative (a ring buffer) would rewrite both call sites for it.
fn push_utterance(buffer: &mut Vec<i16>, samples: &[i16], max: usize) {
    buffer.extend_from_slice(samples);
    if buffer.len() > max {
        let excess = buffer.len() - max;
        buffer.drain(..excess);
    }
}

/// Starts a session on its own task and returns the two ends of its channels.
pub fn spawn_session(
    config: Config,
    providers: ProviderSlot,
    brain_config: BrainConfig,
    policy: SessionPolicy,
    hub: Hub,
) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>) {
    let session = spawn_with_events(config, providers, brain_config, policy, hub);
    (session.to_server, session.from_server)
}

/// The four ends the client keeps, exactly as a session hands them over.
pub struct SessionEnds {
    /// The id this session reports in `Welcome.session_id`: unique per session in this process.
    pub session_id: u64,
    pub to_server: UnboundedSender<ToServer>,
    pub from_server: UnboundedReceiver<ToClient>,
    /// A second sender for the same client channel, for producers that live beside the session, such
    /// as the system monitor.
    pub events: UnboundedSender<ToClient>,
}

/// Like [`spawn_session`], but also hands back the sender the session reports to, so a caller can put
/// other producers of [`ToClient`] on the same channel, and the session's own id.
pub fn spawn_with_events(
    config: Config,
    providers: ProviderSlot,
    brain_config: BrainConfig,
    policy: SessionPolicy,
    hub: Hub,
) -> SessionEnds {
    let session_id = next_session_id();
    let (to_server_tx, to_server_rx) = mpsc::unbounded_channel();
    let (to_client_tx, to_client_rx) = mpsc::unbounded_channel();
    // Held so the client end only closes once the session task itself has ended.
    let keep_alive = to_client_tx.clone();

    let profile = config.active().clone();
    let listening_mode = config.listening.mode;
    let listening_silence_ms = config.listening.silence_ms;
    let brain = Arc::new(Brain::new(providers.clone(), turn_profile(&config), brain_config.clone()));
    // Read before `config` moves into the session: with a login screen up, the greeting is owed rather
    // than spoken (spec §10).
    let greeting_withheld = crate::login::applies(&config.login);

    let mut session = Session {
        session_id,
        policy,
        token_presented: false,
        config,
        hub,
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
        greeting_withheld,
        greeting_signal: Arc::new(Notify::new()),
    };

    tokio::spawn(async move {
        session.run(to_server_rx).await;
        // The task ends with the session; the client end closes with it.
        drop(keep_alive);
    });

    SessionEnds { session_id, to_server: to_server_tx, from_server: to_client_rx, events: to_client_tx }
}

/// What the brain needs from the configuration: the active profile's texts and the voice the speech
/// connector recommends for it. Built when the session starts and again on every `SetProfile`.
///
/// `turn_profile` composes the voice instructions, the prompt and the language from the active
/// profile in the configuration. It is called on every turn, so the profile data is always fresh.
/// The function takes `&Config`, so the configuration that was built is the same that is reloaded,
/// and the floor is never written into the file.
pub fn turn_profile(config: &Config) -> TurnProfile {
    let profile = config.active();
    let floor = if profile.tone_floor.trim().is_empty() {
        if profile.language.split('-').next().is_some_and(|language| language.eq_ignore_ascii_case("es")) {
            tipsy_ui::tone::TONE_FLOOR_ES
        } else {
            tipsy_ui::tone::TONE_FLOOR_EN
        }
    } else {
        &profile.tone_floor
    };
    TurnProfile {
        prompt: tipsy_ui::tone::prompt_with_floor(&profile.prompt, floor),
        voice_instructions: tipsy_ui::tone::brief_with_floor(&profile.voice_instructions, floor),
        voice: profile.voice_for(&config.providers.speech),
        language: profile.language.clone(),
    }
}

/// A turn that is running right now, and the only handle the session may cancel.
struct Turn {
    cancel: CancellationToken,
    /// Where the turn's task leaves the history it produced. The loop's first branch awaits this
    /// rather than asking a task handle whether it finished: the channel is what says both that the
    /// turn is over and what it produced, so there is nothing else to keep.
    done: tokio::sync::oneshot::Receiver<Vec<ChatMessage>>,
    /// Bumped by `Reset`, so a turn that outlives one cannot restore the history it cleared.
    generation: u64,
}

struct Session {
    /// What `Welcome` reports, and what Task 1.3's `Hub` keys this session by.
    session_id: u64,
    /// The handshake this session insists on before it serves anything.
    policy: SessionPolicy,
    /// Whether this client is entitled to the server's token in a `GetConfig` answer (spec §4.3):
    /// true for a loopback connection, and for one that presented the token. Recorded once, where the
    /// handshake is judged, because it is a fact about the handshake and not about the request.
    token_presented: bool,
    /// The configuration the session was built from. A `SetProfile` re-derives the brain from it, so
    /// a switch takes effect on the next turn.
    config: Config,
    /// The registry this session is one of. The three configuration commands of Task 1.1 are answered
    /// *through* it, so that a change one client makes reaches the others (spec §5.2) instead of
    /// mutating this session alone. A handle like every other field here, which is why it is cloned
    /// rather than shared.
    hub: Hub,
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
    /// True while the greeting is still owed: a login screen applies and no verdict has opened yet, so
    /// nothing may be said behind the widget (spec `2026-10-11-login-screen-design.md` §10).
    greeting_withheld: bool,
    /// How the login transcription task asks the session for that greeting: only the session owns the
    /// turn machinery, and the task that transcribes is not the session.
    greeting_signal: Arc<Notify>,
}

impl Session {
    async fn run(&mut self, mut rx: UnboundedReceiver<ToServer>) {
        // The handshake is judged before the first byte the session says. `Hello` is sent by every
        // client as its first message (spec §4.1), so "refused, then closed" is exactly this: no
        // `Welcome`, one `Error`, and a channel that closes when this function returns. Judging it
        // here rather than in `command` is what removes the race between a refusal and the greeting
        // turn — a `Welcome` already sent could not be taken back, and the three refusals would be
        // indistinguishable from a session that was served and then died.
        //
        // `admits` is asked of *any* first message, not only of a `Hello`: `Hello` is the only
        // message allowed to be first, so a first message that is something else is exactly the
        // client `admits` answers `ProtocolVersion` to. Asking it only when the message already
        // parsed as `Hello` would serve an exposed session an `Ask` that never presented a token.
        //
        // **And the wait itself is bounded** (`HANDSHAKE_TIMEOUT`): a peer that says nothing is not
        // refused by `admits` because there is nothing to refuse, so it would hold this session — and
        // its configuration, brain and detector — for ever. The timeout is what makes "the socket
        // must say `Hello` first" a requirement rather than an invitation.
        let first = match tokio::time::timeout(self.policy.handshake_timeout, rx.recv()).await {
            Ok(Some(message)) => message,
            // The client vanished before saying anything: nothing to serve and nobody to tell.
            Ok(None) => return,
            // Nobody said anything at all. A peer that never greets is not speaking this protocol,
            // which is the same judgement a first message that is not a `Hello` gets, and the
            // channel closes when this function returns.
            Err(_) => {
                self.refuse(ErrorCode::ProtocolVersion);
                return;
            }
        };
        let admitted = match &first {
            ToServer::Command(hello) => self.policy.admits(hello),
            // Not a command at all, so it is not the `Hello` that has to come first. `Reconfigure` is
            // server-internal and can never be a connection's first message, but a session that
            // answered `Ok` to it would be admitting a handshake nobody sent.
            ToServer::Audio(_) | ToServer::Reconfigure => Err(ErrorCode::ProtocolVersion),
        };
        self.token_presented = self.policy.loopback || admitted.is_ok();
        if let Err(code) = admitted {
            self.refuse(code);
            return;
        }

        self.welcome();
        // The greeting is an ordinary turn: while it plays, `Cancel` still works. While a login screen
        // applies it is withheld instead and spoken by the verdict that opens the door (spec §10) — no
        // speech is spent on a visitor who is refused, and nothing talks over the widget.
        if !self.greeting_withheld {
            self.greet();
        }

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
                    // A refused `Hello` that arrives after the handshake ends the session here: the
                    // break is the whole difference, and nothing is settled for a session that is over.
                    if self.handle(message, &mut rx).is_break() {
                        break;
                    }
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
                // The login transcription runs in a task of its own, so the greeting its verdict earns
                // has to wake the session: this is the only branch that can start a turn no client
                // asked for, and `greet_if_withheld` is what keeps it to one.
                () = self.greeting_signal.notified() => {
                    self.greet_if_withheld();
                }
            }
        }
        // Nothing may keep speaking after the client hung up.
        self.cancel_running();
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

    /// Starts the greeting turn: at construction when no login screen applies, and at the verdict that
    /// opens the door otherwise (spec §10). The flag is cleared *before* the turn starts, which is what
    /// makes a greeting owed exactly once even if two verdicts open — a typed answer can land while a
    /// transcription is still in flight.
    fn greet(&mut self) {
        self.greeting_withheld = false;
        let greeting = self.greeting.clone();
        self.current = Some(self.start_turn(TurnInput::Say(greeting)));
    }

    /// Speaks the greeting if it is still owed, and does nothing otherwise.
    fn greet_if_withheld(&mut self) {
        if self.greeting_withheld {
            self.greet();
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
                session_id: self.session_id,
                profile: self.profile.clone(),
                ui_language: self.ui_language,
                listening_mode: self.listening_mode,
                // The truth for the binary that is running, not the constant `false` this used to
                // be: `cargo xtask dist` embeds the bundle and a plain `cargo build` does not
                // (spec §5.4; `architecture.md:83-86`).
                web_bundle: crate::server::assets::web_bundle(),
                // The one authority on whether a client shows the login screen. A native client never
                // reads `[login]` itself, so it cannot disagree with the server that will judge it.
                // Sent on every `Welcome`; the client shows the screen once per run and ignores the
                // later ones.
                login: crate::login::applies(&self.config.login)
                    .then_some(LoginView { voice: self.config.login.voice }),
            }),
        );
    }

    /// Handles one message and everything already queued behind it, so a `ListenStop` never leaves the
    /// tail of an utterance waiting behind it. `Break` means the session is over: a `Hello` refused
    /// here was refused at a message that is not the first one.
    fn handle(&mut self, first: ToServer, rx: &mut UnboundedReceiver<ToServer>) -> ControlFlow<()> {
        let mut message = first;
        loop {
            let flow = self.dispatch(message);
            if flow.is_break() {
                return flow;
            }
            match rx.try_recv() {
                Ok(next) => message = next,
                Err(_) => return ControlFlow::Continue(()),
            }
        }
    }

    fn dispatch(&mut self, message: ToServer) -> ControlFlow<()> {
        match message {
            ToServer::Command(command) => self.command(command),
            ToServer::Audio(Frame::Mic(samples)) => {
                // An open login capture owns the microphone: its frames are the answer being spoken,
                // and feeding them to the listening path as well would transcribe the login twice —
                // once as an answer and once as a question (spec §4.2).
                if self.login_active {
                    self.login_frame(&samples);
                    return ControlFlow::Continue(());
                }
                // Frames arriving before `ListenStart` — or after `ListenStop` — belong to no turn.
                if !self.listening {
                    return ControlFlow::Continue(());
                }
                match self.listening_mode {
                    // `auto`: the detector decides what is an utterance (spec §6), and only what it
                    // confirmed is ever transcribed.
                    ListeningMode::Auto => self.detect(&samples),
                    // `push`, and `hands_free` until Plan 4 implements it: the client's frames are
                    // the utterance, and `ListenStop` transcribes them. Capped, because nothing else
                    // ends it: a client that streams for ever would otherwise grow this buffer for
                    // ever (see [`UTTERANCE_MAX_SAMPLES`]).
                    ListeningMode::Push | ListeningMode::HandsFree => {
                        push_utterance(&mut self.mic, &samples, UTTERANCE_MAX_SAMPLES);
                    }
                }
                ControlFlow::Continue(())
            }
            ToServer::Audio(Frame::Speech { .. }) => ControlFlow::Continue(()),
            // The hub's fan-out: the configuration it has just persisted is now the one this session
            // runs, and the session reads it from the hub rather than from the message — the message
            // carries nothing, which is what keeps a client's view (no provider address, no key name,
            // no token) from ever being mistaken for the whole configuration.
            ToServer::Reconfigure => {
                let config = self.hub.config();
                self.reconfigure(config);
                ControlFlow::Continue(())
            }
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
                push_utterance(&mut self.utterance, samples, UTTERANCE_MAX_SAMPLES);
            }
            VadEvent::SpeechEnded => {
                self.heard = false;
                // The frame that reports the end is the last of the hangover: it is part of the
                // utterance, which is why it is pushed before the turn starts.
                push_utterance(&mut self.utterance, samples, UTTERANCE_MAX_SAMPLES);
                self.utterance_ended();
            }
            VadEvent::None => {
                if self.heard {
                    // Capped like `push` mode's buffer: a detector held open by continuous noise
                    // never ends this utterance, so nothing else would bound it.
                    push_utterance(&mut self.utterance, samples, UTTERANCE_MAX_SAMPLES);
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
        self.start(TurnInput::Utterance(Pcm { samples: i16_to_f32(&samples), sample_rate: MIC_SAMPLE_RATE }));
    }

    fn command(&mut self, command: Command) -> ControlFlow<()> {
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
            return ControlFlow::Continue(());
        }
        match command {
            Command::Ask { text } => {
                self.ask(text);
                ControlFlow::Continue(())
            }
            Command::Say { text } => {
                self.say(text);
                ControlFlow::Continue(())
            }
            Command::ListenStart => {
                self.listen_start();
                ControlFlow::Continue(())
            }
            Command::ListenStop => {
                self.listen_stop();
                ControlFlow::Continue(())
            }
            Command::Cancel => {
                self.cancel();
                ControlFlow::Continue(())
            }
            Command::Reset => {
                self.reset();
                ControlFlow::Continue(())
            }
            Command::SetProfile { id } => {
                // The profile is global (spec §5.2), so the command is the hub's to apply: it persists
                // the choice, re-derives every session and announces `ProfileChanged` to every client,
                // this one included. A session that mutated only its own copy would be the bug the
                // spec's §4.2 broadcast exists to prevent.
                let _ = self.hub.set_profile(&id);
                ControlFlow::Continue(())
            }
            Command::SetListeningMode { mode } => {
                // A mode the session is already in is the hub's own re-announcement — `Hub::resend_welcome`
                // sends this very command back to every session — so the whole of this arm is the fresh
                // `Welcome` the header reads (spec §5.2: the mode is not configuration, so it is not a
                // `ConfigChanged`).
                if self.listening_mode == mode {
                    self.set_listening_mode(mode);
                    return ControlFlow::Continue(());
                }
                // A genuine change, so it is the hub's to persist and to announce to everyone. The
                // local field is moved **first**, before the hub fans the same mode back out: the
                // echo then arrives at a session that is already in it and stops here, where taking the
                // hub's answer first would send it back to the hub and round again for ever. The
                // `Welcome` this sends is the client's own immediate answer, and the hub's
                // announcement adds the same one for every other connection.
                self.set_listening_mode(mode);
                let _ = self.hub.set_listening_mode(mode);
                ControlFlow::Continue(())
            }
            // The first `Hello` was judged in `run`, before the `Welcome`; this arm is both the
            // idempotent second pass `handle` makes for a queued message and the one place a *late*
            // `Hello` — after the handshake, with the channel still open — is refused. A client that
            // got this far was already admitted, so a failure here means a second hello with
            // different credentials: the session ends rather than changing its mind.
            Command::Hello { token, client, protocol_version } => {
                let hello = Command::Hello { token, client, protocol_version };
                match self.policy.admits(&hello) {
                    Ok(()) => ControlFlow::Continue(()),
                    Err(code) => {
                        self.refuse(code);
                        ControlFlow::Break(())
                    }
                }
            }
            // The Test probe asks a provider a question, not the session: Plan 2B Task 7 runs it in
            // the native client and Plan 3's server answers it over the wire. Accepted and ignored,
            // so the match stays exhaustive.
            Command::TestProvider { .. } => ControlFlow::Continue(()),
            Command::LoginStart => {
                self.login_start();
                ControlFlow::Continue(())
            }
            Command::LoginStop => {
                self.login_stop();
                ControlFlow::Continue(())
            }
            Command::LoginAnswer { text } => {
                self.login_answer(text);
                ControlFlow::Continue(())
            }
            Command::GetConfig => {
                // §4.3: the token goes to a client that already presented it, or to a loopback one.
                // The session is the only thing that knows which it is, so it is the thing that says.
                send(
                    &self.out,
                    ToClient::Event(Event::Config { config: Box::new(self.hub.config_view(self.token_presented)) }),
                );
                ControlFlow::Continue(())
            }
            Command::UpdateConfig { config } => {
                // **The client's save, and the one route a configuration change takes in.** The
                // session's own loop is behind the handshake, so this arm is unreachable until the
                // client was admitted — which is why the bridge hands this command here instead of
                // writing the hub itself (an unadmitted socket could otherwise persist a
                // configuration while its session was still refusing it).
                //
                // The hub is the only writer (Shared interface contract): it replaces the value,
                // merges the three fields a client's view does not carry from its own configuration,
                // writes the file and fans the change out to every session as
                // `ToServer::Reconfigure`. A view that does not describe a configuration is refused
                // there with nothing written, exactly as a local save is.
                let _ = self.hub.update_config_from_view(*config);
                ControlFlow::Continue(())
            }
            Command::SetKey { name, value } => {
                // Spec §9.4's refusal — `Error { code: ErrorCode::InsecureTransport, recoverable:
                // true }` unless the connection is loopback or TLS — is Task 1.8's, and this is the
                // point it plugs into: `self.policy.loopback` is the fact it needs, and
                // `self.hub.set_key` is the call it guards. Implementing the refusal here would put
                // the rule where TLS is not visible.
                let _ = self.hub.set_key(&name, &value);
                ControlFlow::Continue(())
            }
        }
    }

    /// Answers a refused handshake: the `code` a client branches on, and a sentence it shows as-is.
    ///
    /// The message is deliberately **not** the code spelled out. v1 spec §4.2 makes
    /// `Event::Error.message` "already localized to the profile's language", so no client may parse
    /// it; the machine-readable half is `code`. Both halves of that rule are load-bearing: a client
    /// that branched on this string would break the day somebody translated it.
    fn refuse(&self, code: ErrorCode) {
        let strings = tipsy_ui::strings(self.ui_language);
        let message = match code {
            ErrorCode::ProtocolVersion => strings.err_protocol_version,
            ErrorCode::TokenRequired => strings.err_token_required,
            ErrorCode::TokenInvalid => strings.err_token_invalid,
            // `refuse` is called from the handshake only, where those three are the only endings; the
            // fallback is what keeps it total rather than pretending a fourth case cannot be reached.
            _ => strings.err_handshake,
        };
        send(&self.out, ToClient::Event(Event::Error { code, message: message.to_string(), recoverable: false }));
    }

    /// Replaces the configuration this session is running and re-derives what comes from it: the
    /// `Brain` is rebuilt, and the profile the header shows and the interface language follow the newly
    /// active profile. The provider stack is unchanged — a save that changed it swaps the slot in
    /// place, which `client.rs:702-705` and `providers/mod.rs:175` already do. This is
    /// `UpdateConfig`'s session half (spec §5.2): no restart, and a turn already running keeps the
    /// brain it started with, because the rebuild happens here and the turn holds its own `Arc`.
    ///
    /// **The listening mode is deliberately not read here.** It is the one fact whose announcement is
    /// this very command, and `Hub::resend_welcome` re-announces it by sending `SetListeningMode` to
    /// every session: a `reconfigure` that re-read the mode would undo what that arm had just applied
    /// when a `Command::UpdateConfig` for the same save arrived behind it, the session would see a
    /// change again the next time the hub announced, and a client that changed its mode would spin the
    /// pair of them for ever. The field belongs to `SetListeningMode` alone.
    fn reconfigure(&mut self, config: Config) {
        self.config = config;
        let profile = self.config.active();
        self.profile = profile.view();
        self.ui_language = UiLanguage::from_bcp47(&profile.language);
        // **The greeting is the one fact a profile owns that `turn_profile` does not carry.** The
        // voice, the prompt, the voice instructions and the language all reach the brain through
        // `turn_profile`, which this rebuilds; the greeting is this struct's own field, read once when
        // the session was built. A profile switch that skipped it left a session introducing itself in
        // the *previous* profile's words while the header already showed the new name — which is what
        // "the profile does not change" looked like from the screen.
        self.greeting = profile.greeting.clone();
        self.brain =
            Arc::new(Brain::new(self.providers.clone(), turn_profile(&self.config), self.brain_config.clone()));
    }

    /// Applies the listening mode and announces it again, which is what moves the header's indicator:
    /// the mode lives in `Welcome` and nowhere else.
    ///
    /// The `SetListeningMode` arm is its only caller, in both of the shapes that arm has: a mode a
    /// client really changed, which the hub then persists, and the hub's own re-announcement, which
    /// arrives here for a session that is already in the mode.
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
        self.start(TurnInput::Utterance(Pcm { samples: i16_to_f32(&samples), sample_rate: MIC_SAMPLE_RATE }));
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
        let ok = verdict.ok;
        send(&self.out, ToClient::Event(Event::LoginVerdict { ok, name: verdict.name }));
        if ok {
            // The withheld greeting lands as the HUD appears — after the verdict that opened the door
            // (spec §10).
            self.greet_if_withheld();
        }
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
        let greeting = Arc::clone(&self.greeting_signal);
        tokio::spawn(async move {
            send(&out, status(AssistantState::Transcribing));
            let pcm = Pcm { samples: i16_to_f32(&samples), sample_rate: MIC_SAMPLE_RATE };
            let mut opened = false;
            if let Some(text) = brain.transcribe_once(pcm, &out, &cancel).await {
                send(&out, ToClient::Event(Event::LoginHeard { text: text.clone() }));
                let verdict = crate::login::judge(&login, &text);
                opened = verdict.ok;
                send(&out, ToClient::Event(Event::LoginVerdict { ok: verdict.ok, name: verdict.name }));
            }
            send(&out, status(AssistantState::Idle));
            if opened {
                // Only the session owns the turn machinery, so the task that heard the answer wakes it
                // for the greeting the screen withheld: the verdict and the idle are already on the
                // wire, so the greeting lands after them (spec §10).
                greeting.notify_one();
            }
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
        tokio::spawn(async move {
            brain.run_turn(&mut history, input, out, token).await;
            // Only the session may record the history, and only if it still wants it.
            let _ = done_tx.send(history);
        });
        Turn { cancel, done: done_rx, generation }
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
    use crate::keys::KeyResolver;
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
        [ChatMessage::user(format!("pregunta {n}")), ChatMessage::assistant(format!("respuesta {n}"))]
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

    /// **A profile switch has to move everything the profile owns, and the greeting was the one it
    /// left behind.** The voice, the prompt, the voice instructions and the language travel to the
    /// brain through `turn_profile`, which `reconfigure` rebuilds; the greeting is this struct's own
    /// field, read once when the session was built — so switching profiles mid-run left a session
    /// introducing itself in the *previous* profile's words while the header already showed the new
    /// name.
    #[test]
    fn reconfiguring_to_another_profile_moves_its_greeting() {
        let config = Config { active_profile: "compae".into(), ..Config::default() };
        let (mut session, _rx, _calls) = login_session(config.clone(), "", Vec::new());
        assert_eq!(session.profile.id, "compae");
        let compae_greeting = session.greeting.clone();
        assert_eq!(compae_greeting, session.config.active().greeting);

        session.reconfigure(Config { active_profile: "bloke".into(), ..config });

        assert_eq!(session.profile.id, "bloke", "the header's own fact");
        assert_ne!(session.greeting, compae_greeting, "not the previous profile's greeting");
        assert_eq!(session.greeting, session.config.active().greeting, "but the active profile's");
    }

    /// **The floor is composed on the way to the request and stored nowhere.**
    ///
    /// The first half is the "do not corrupt the user's copy" rule: `turn_profile` takes `&Config`, so
    /// after it has run the configuration still holds the card the user wrote. The second half is what
    /// makes a stored floor impossible to miss — a floor written into the file would be floored again
    /// by the next `turn_profile`, so the texts would grow by one sentence per save→load cycle. One
    /// round trip must not change a character.
    #[test]
    fn the_floor_is_never_written_into_the_configuration() {
        let config = Config { active_profile: "compae".into(), ..Config::default() };
        let card = config.active().clone();
        let composed = turn_profile(&config);

        assert_eq!(config.active(), &card, "the configuration is the user's copy, untouched");
        assert!(
            config.active().voice_instructions.starts_with("Voz masculina adulta"),
            "the card still holds the brief the built-in profile ships"
        );
        assert!(composed.voice_instructions.starts_with(&card.tone_floor), "and the request carries the floor first");
        assert!(composed.prompt.ends_with(&card.tone_floor), "the prompt carries the floor last");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        crate::config::store::save(&path, &config).unwrap();
        let reloaded = crate::config::store::load_or_create(&path).unwrap().config;
        let again = turn_profile(&reloaded);

        assert_eq!(
            again.voice_instructions, composed.voice_instructions,
            "one save→load cycle must not grow the brief"
        );
        assert_eq!(again.prompt, composed.prompt, "nor the prompt");
    }

    /// **The floor is baked into the profile's instruction when it is built.**
    ///
    /// `turn_profile` takes `&Config`, so the configuration that was built is the same that is
    /// reloaded, and the floor is never written into the file.
    #[test]
    fn the_built_in_profile_ships_its_tone_floor() {
        let config = Config { active_profile: "compae".into(), ..Config::default() };
        let composed = turn_profile(&config);
        let style = tipsy_ui::tone::first_sentence(&composed.voice_instructions);
        assert_eq!(style, config.active().tone_floor);
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
        let greeting_withheld = crate::login::applies(&config.login);
        // The login tests never reach the configuration commands, so this session's registry is a hub
        // of its own. The tempdir is leaked rather than dropped: a `Hub` whose configuration file sat
        // in a directory that had already gone away would turn any later save into a refusal.
        let dir = tempfile::tempdir().unwrap().keep();
        let hub = Hub::new(
            config.clone(),
            dir.join("config.toml"),
            KeyResolver::new(Box::new(crate::keys::MemoryStore::default()), Box::new(|_| None)),
            providers.clone(),
        );
        let session = Session {
            // A directly built session never runs the handshake, so its id is the first one and its
            // policy is the in-process one; both fields are what `run`'s prelude fills in for a
            // session that really serves a client.
            session_id: 1,
            policy: SessionPolicy::local(),
            token_presented: true,
            config,
            hub,
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
            greeting_withheld,
            greeting_signal: Arc::new(Notify::new()),
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
            // A microphone frame can never break the session: only a refused `Hello` can.
            let _ = session.dispatch(ToServer::Audio(Frame::Mic(vec![0i16; MIC_FRAME_SAMPLES])));
        }
    }

    /// F5, the session half: a `push` client that opens the microphone and never closes it must not
    /// grow the session's buffer without bound. The bound is the ceiling, and what it keeps is the
    /// **newest** audio — the question the user just finished asking, not the first thirty seconds.
    #[test]
    fn a_push_utterance_is_capped_and_keeps_the_newest_audio() {
        let (mut session, _rx, _) = login_session(Config::default(), "unused", Vec::new());
        let _ = session.command(Command::ListenStart);
        assert!(session.listening, "the microphone is open");

        let frames = UTTERANCE_MAX_SAMPLES / MIC_FRAME_SAMPLES + 8;
        speak(&mut session, frames);
        assert_eq!(
            session.mic.len(),
            UTTERANCE_MAX_SAMPLES,
            "the buffer must stop at its ceiling, held {} samples after {frames} frames",
            session.mic.len()
        );

        // And it is the newest audio that survived: the last frame is in there in full.
        let _ = session.dispatch(ToServer::Audio(Frame::Mic(vec![7i16; MIC_FRAME_SAMPLES])));
        let tail = &session.mic[session.mic.len() - MIC_FRAME_SAMPLES..];
        assert!(tail.iter().all(|sample| *sample == 7), "the frame just sent must still be in the buffer");
    }

    /// The same cap for `auto` mode's confirmed utterance. This drives the buffer the detector fills
    /// directly: the session's listening detector is the real one, and no amount of zeros opens it.
    #[test]
    fn an_auto_utterance_that_never_ends_is_capped_too() {
        let (mut session, _rx, _) = login_session(Config::default(), "unused", Vec::new());
        session.heard = true; // as if the detector had confirmed an onset and never confirmed an end
        let frames = UTTERANCE_MAX_SAMPLES / MIC_FRAME_SAMPLES + 4;
        for _ in 0..frames {
            push_utterance(&mut session.utterance, &vec![0i16; MIC_FRAME_SAMPLES], UTTERANCE_MAX_SAMPLES);
        }
        assert_eq!(session.utterance.len(), UTTERANCE_MAX_SAMPLES);

        // The helper itself, which is the rule both callers share: at most `max`, newest kept.
        let mut buffer = vec![0i16; 4];
        push_utterance(&mut buffer, &[1, 2, 3], 5);
        assert_eq!(buffer, vec![0, 0, 1, 2, 3], "appending below the ceiling changes nothing");
        push_utterance(&mut buffer, &[4, 5, 6], 5);
        assert_eq!(buffer, vec![2, 3, 4, 5, 6], "and past it the oldest samples go");
        let mut exact = vec![0i16; 5];
        push_utterance(&mut exact, &[9], 5);
        assert_eq!(exact, vec![0, 0, 0, 0, 9]);
    }

    /// Everything the session sent until the verdict (or until two seconds pass, which fails the test
    /// rather than hanging it).
    async fn events_until_verdict(rx: &mut UnboundedReceiver<ToClient>) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let next = timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("the verdict must arrive")
                .expect("the channel stays open");
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
        LoginConfig {
            enabled: true,
            voice: true,
            allowed: vec![LoginEntry { name: name.into(), keywords: keywords.iter().map(|k| (*k).into()).collect() }],
        }
    }

    /// The whole voice path, and the three things that must *not* happen on it: no chat call, no
    /// speech, no turn. `transcribe_once` is what keeps the login out of the conversation.
    #[tokio::test]
    async fn a_login_utterance_becomes_a_verdict_without_a_turn() {
        let config = Config { login: allowed("Huésped", &["huesped", "core"]), ..Config::default() };
        let (mut session, mut rx, chat_calls) =
            login_session(config, "yo soy Huésped con acceso autorizado y core", one_utterance(1000));

        let _ = session.command(Command::LoginStart);
        assert!(session.login_active, "the capture opens");
        speak(&mut session, 70);

        let events = events_until_verdict(&mut rx).await;
        assert!(
            events.iter().any(|e| matches!(e, Event::Status { state: AssistantState::Transcribing, .. })),
            "{events:?}"
        );
        assert!(
            events.iter().any(
                |e| matches!(e, Event::LoginHeard { text } if text == "yo soy Huésped con acceso autorizado y core")
            ),
            "{events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(e, Event::LoginVerdict { ok: true, name: Some(name) } if name == "Huésped")),
            "{events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::Reply { .. } | Event::ReplyDelta { .. } | Event::SpeechChunk { .. })),
            "the login path never replies: {events:?}"
        );
        assert_eq!(chat_calls.load(Ordering::SeqCst), 0, "the chat provider is never asked");
        assert!(session.history.is_empty(), "and the conversation is untouched");
    }

    /// The same capture, refusing: the keywords are not there, so the verdict says no and the capture
    /// stays open for the next attempt (spec §6.2 — the automatic re-arm).
    #[tokio::test]
    async fn a_login_utterance_without_the_keywords_is_refused_and_the_capture_stays_open() {
        let config = Config { login: allowed("Huésped", &["huesped", "core"]), ..Config::default() };
        let (mut session, mut rx, _) = login_session(config, "soy Huésped del score", one_utterance(1000));

        let _ = session.command(Command::LoginStart);
        speak(&mut session, 70);

        let events = events_until_verdict(&mut rx).await;
        assert!(events.iter().any(|e| matches!(e, Event::LoginVerdict { ok: false, .. })), "{events:?}");
        assert!(session.login_active, "the next utterance is the next attempt");
    }

    /// Silence is not an answer: no transcription request, `NothingHeard`, and no verdict at all — a
    /// cough must never be counted as a wrong answer (spec §4.1).
    #[tokio::test]
    async fn silence_asks_for_no_transcription_and_no_verdict() {
        let config = Config { login: allowed("Huésped", &["huesped", "core"]), ..Config::default() };
        let (mut session, mut rx, _) = login_session(config, "unused", vec![0.0; 80]);

        let _ = session.command(Command::LoginStart);
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
        let config = Config { login: allowed("Huésped", &["huesped", "core"]), ..Config::default() };
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
