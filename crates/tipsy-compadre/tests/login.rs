//! The login capture end to end, over the channels a client uses.
//!
//! These tests build a real session (`spawn_session`) and drive it with `ToServer` messages, so what
//! they pin is the wire behaviour of Task 3: the `Welcome`'s verdict on whether a screen applies, a
//! typed answer judged without a chat call, an answer to a screen that is not there, and the ignore
//! rule of spec §4.2 with the capture's lifetime as its scope.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tipsy_compadre::audio::device::MIC_FRAME_SAMPLES;
use tipsy_compadre::audio::pcm::Pcm;
use tipsy_compadre::brain::BrainConfig;
use tipsy_compadre::config::{Config, LoginEntry};
use tipsy_compadre::providers::*;
use tipsy_compadre::session::spawn_session;
use tipsy_protocol::{AssistantState, Command, Event, Frame, LoginView, ToClient, ToServer};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

// ---------- fakes ----------

/// Chat fake, copied from `tests/brain.rs`: each call pops the next scripted outcome and is recorded,
/// so a test can prove the login path never asked for a reply. `chunk_delay` is not copied: nothing
/// here measures when a chunk arrives.
struct FakeChat {
    script: Mutex<Vec<Result<Vec<&'static str>, ProviderError>>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl FakeChat {
    fn new(script: Vec<Result<Vec<&'static str>, ProviderError>>) -> Arc<Self> {
        Arc::new(Self { script: Mutex::new(script), requests: Mutex::new(Vec::new()) })
    }

    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// What was asked of the model. `tests/brain.rs` keeps this list private and only counts it; the
    /// login tests want the list itself, so the getter is added here.
    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChatProvider for FakeChat {
    async fn stream(&self, req: ChatRequest) -> Result<TextStream, ProviderError> {
        self.requests.lock().unwrap().push(req);
        let next = self.script.lock().unwrap().remove(0);
        let chunks = next?;
        Ok(futures::stream::iter(chunks).map(|c| Ok(c.to_string())).boxed())
    }
}

/// Speech fake: one sample per character at 24 kHz, which is all the greeting needs to play.
struct FakeSpeech;

#[async_trait]
impl SpeechProvider for FakeSpeech {
    async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError> {
        Ok(Pcm { samples: vec![0.25; req.text.chars().count()], sample_rate: 24000 })
    }
}

/// Transcribe fake: the login tests type instead of speaking, so the transcriber only has to exist.
struct FakeTranscribe {
    text: &'static str,
}

#[async_trait]
impl TranscribeProvider for FakeTranscribe {
    async fn transcribe(&self, _pcm: Pcm, _language: &str) -> Result<String, ProviderError> {
        Ok(self.text.to_string())
    }
}

// ---------- helpers ----------

/// A session with one fake provider of each kind, and the handles the tests assert on.
fn session_with(config: Config) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>, Arc<FakeChat>) {
    let chat = FakeChat::new(vec![Ok(vec!["vale"]), Ok(vec!["vale"]), Ok(vec!["vale"])]);
    let providers = ProviderSlot::new(Providers {
        chat: chat.clone(),
        speech: Arc::new(FakeSpeech),
        transcribe: Arc::new(FakeTranscribe { text: "no se usa" }),
    });
    let (to_server, from_server) = spawn_session(config, providers, BrainConfig::default());
    (to_server, from_server, chat)
}

/// Wait for the next event, or fail rather than hang.
async fn next_event(rx: &mut UnboundedReceiver<ToClient>) -> Event {
    let next = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("an event must arrive")
        .expect("the channel stays open");
    match next {
        ToClient::Event(event) => event,
        other => panic!("expected an event, got {other:?}"),
    }
}

/// The next event matching `pred`, skipping everything else, panicking with what it saw if the
/// timeout expires.
///
/// Everything else is on this channel: the session opens with a greeting turn of its own, so its
/// `Status`, `SpeechChunk` and audio frames share the client end with whatever the test provokes.
/// `tests/session.rs` skips them the same way.
async fn next_event_matching(
    rx: &mut UnboundedReceiver<ToClient>,
    pred: impl Fn(&Event) -> bool,
    timeout_ms: u64,
) -> Event {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            panic!("no event matched within {timeout_ms} ms; received {seen:#?}");
        }
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(ToClient::Event(event))) if pred(&event) => return event,
            Ok(Some(other)) => seen.push(other),
            Ok(None) => panic!("the session closed the channel; received {seen:#?}"),
            Err(_) => panic!("no event matched within {timeout_ms} ms; received {seen:#?}"),
        }
    }
}

/// Everything received until `wait_ms` passes without a message, or the session goes quiet.
async fn drain(rx: &mut UnboundedReceiver<ToClient>, wait_ms: u64) -> Vec<ToClient> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return out;
        }
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(m)) => out.push(m),
            _ => return out,
        }
    }
}

/// Waits out the greeting turn the session starts with — its `Done`, then the tail behind it — so that
/// what the session sends after this is only what the test itself asked for. `tests/session.rs` waits
/// for the same `Done` before every one of its own commands.
async fn greeting_is_over(rx: &mut UnboundedReceiver<ToClient>) {
    let _ = next_event_matching(rx, |e| matches!(e, Event::Done { .. }), 5000).await;
    let _ = drain(rx, 50).await;
}

fn allowed() -> Vec<LoginEntry> {
    vec![LoginEntry { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] }]
}

// ---------- tests ----------

/// The `Welcome` every session sends on start carries the verdict on whether a screen applies at all.
#[tokio::test]
async fn welcome_carries_the_login_view_only_when_the_section_applies() {
    let mut config = Config::default();
    let (to_server, mut from_server, _chat) = session_with(config.clone());
    let _ = &to_server;
    match next_event(&mut from_server).await {
        Event::Welcome { login, .. } => assert_eq!(login, None, "no entries, no screen"),
        other => panic!("expected a Welcome, got {other:?}"),
    }

    config.login.allowed = allowed();
    let (_to_server, mut from_server, _chat) = session_with(config);
    match next_event(&mut from_server).await {
        Event::Welcome { login, .. } => assert_eq!(login, Some(LoginView { voice: true }), "and the voice switch travels with it"),
        other => panic!("expected a Welcome, got {other:?}"),
    }
}

/// A typed answer never touches the chat provider, opens at the right answer and refuses the rest. It
/// is the whole feature's contract in one test, and the reason `LoginAnswer` is judged whether or not
/// a capture is open: this session never sends `LoginStart`.
#[tokio::test]
async fn a_typed_answer_is_judged_without_ever_asking_the_chat_provider() {
    let mut config = Config::default();
    config.login.allowed = allowed();
    let (to_server, mut from_server, chat) = session_with(config);
    greeting_is_over(&mut from_server).await;

    to_server.send(ToServer::Command(Command::LoginAnswer { text: "soy Andrés del score".into() })).unwrap();
    let refused = next_event_matching(&mut from_server, |e| matches!(e, Event::LoginVerdict { .. }), 2000).await;
    assert_eq!(refused, Event::LoginVerdict { ok: false, name: None });

    to_server.send(ToServer::Command(Command::LoginAnswer { text: "yo soy Andrés del equipo del core".into() })).unwrap();
    let opened = next_event_matching(&mut from_server, |e| matches!(e, Event::LoginVerdict { .. }), 2000).await;
    assert_eq!(opened, Event::LoginVerdict { ok: true, name: Some("Andrés".into()) });

    assert_eq!(chat.calls(), 0, "a login is not a question");
    assert!(chat.requests().is_empty(), "and it is never asked for a reply");
}

/// With the screen switched off, every answer opens: a stray answer to a screen that is not there must
/// never refuse anybody (spec §4.1).
#[tokio::test]
async fn a_typed_answer_opens_when_the_screen_does_not_apply() {
    let mut config = Config::default();
    config.login.enabled = false;
    config.login.allowed = allowed();
    let (to_server, mut from_server, _chat) = session_with(config);
    greeting_is_over(&mut from_server).await;

    to_server.send(ToServer::Command(Command::LoginAnswer { text: "quien sea".into() })).unwrap();
    let verdict = next_event_matching(&mut from_server, |e| matches!(e, Event::LoginVerdict { .. }), 2000).await;
    assert_eq!(verdict, Event::LoginVerdict { ok: true, name: None });
}

/// Review Focus 4, the server's half: the commands of the other side of the door are ignored while the
/// capture is open and no reply is ever produced for them, and a microphone frame that arrives after
/// the stop belongs to nobody and is not a panic.
#[tokio::test]
async fn commands_outside_the_login_are_ignored_while_the_capture_is_open() {
    let mut config = Config::default();
    config.login.allowed = allowed();
    let (to_server, mut from_server, chat) = session_with(config);
    greeting_is_over(&mut from_server).await;

    to_server.send(ToServer::Command(Command::LoginStart)).unwrap();
    to_server.send(ToServer::Command(Command::Ask { text: "¿qué tal?".into() })).unwrap();
    to_server.send(ToServer::Command(Command::Say { text: "hola".into() })).unwrap();

    // Nothing at all comes back for the two of them: no Status, no Reply, no Error.
    let quiet = tokio::time::timeout(Duration::from_millis(200), from_server.recv()).await;
    assert!(quiet.is_err(), "an ignored command answers nothing, got {:?}", quiet.map(|m| m.map(|m| format!("{m:?}"))));
    assert_eq!(chat.calls(), 0);

    to_server.send(ToServer::Command(Command::LoginStop)).unwrap();
    to_server.send(ToServer::Audio(Frame::Mic(vec![0i16; MIC_FRAME_SAMPLES]))).unwrap();
    to_server.send(ToServer::Command(Command::Ask { text: "ahora sí".into() })).unwrap();
    // …and with the capture closed an `Ask` goes through again: the guard is the capture, not the
    // screen. The reply's `Status` is the proof it started.
    let started = next_event_matching(
        &mut from_server,
        |e| matches!(e, Event::Status { state: AssistantState::Thinking, .. }),
        2000,
    )
    .await;
    assert!(matches!(started, Event::Status { state: AssistantState::Thinking, .. }), "the capture's guard must lift with `LoginStop`");
}
