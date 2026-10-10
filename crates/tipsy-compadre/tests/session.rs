use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tipsy_compadre::audio::pcm::Pcm;
use tipsy_compadre::brain::BrainConfig;
use tipsy_compadre::config::Config;
use tipsy_compadre::config::store;
use tipsy_compadre::providers::*;
use tipsy_compadre::session::spawn_session;
use tipsy_protocol::{AssistantState, Command, Event, Frame, ToClient, ToServer, UiLanguage};
use tokio::sync::mpsc;

// ---------- fakes (copied from `tests/brain.rs`) ----------

/// Chat fake: each call pops the next scripted outcome.
struct FakeChat {
    script: Mutex<Vec<Result<Vec<&'static str>, ProviderError>>>,
    requests: Mutex<Vec<ChatRequest>>,
    chunk_delay: Duration,
}

impl FakeChat {
    fn new(script: Vec<Result<Vec<&'static str>, ProviderError>>) -> Arc<Self> {
        Self::slow(script, Duration::ZERO)
    }
    /// The same fake, sleeping `chunk_delay` before each streamed chunk.
    fn slow(script: Vec<Result<Vec<&'static str>, ProviderError>>, chunk_delay: Duration) -> Arc<Self> {
        Arc::new(Self { script: Mutex::new(script), requests: Mutex::new(Vec::new()), chunk_delay })
    }
    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl ChatProvider for FakeChat {
    async fn stream(&self, req: ChatRequest) -> Result<TextStream, ProviderError> {
        self.requests.lock().unwrap().push(req);
        let next = self.script.lock().unwrap().remove(0);
        let chunks = next?;
        let delay = self.chunk_delay;
        Ok(futures::stream::iter(chunks)
            .then(move |c| async move {
                tokio::time::sleep(delay).await;
                Ok(c.to_string())
            })
            .boxed())
    }
}

/// Speech fake: returns one sample per character at 24 kHz; per-text delays and failures.
struct FakeSpeech {
    delays: HashMap<&'static str, Duration>,
    fail_on: Option<&'static str>,
    requests: Mutex<Vec<SpeechRequest>>,
}

impl FakeSpeech {
    fn new() -> Arc<Self> {
        Arc::new(Self { delays: HashMap::new(), fail_on: None, requests: Mutex::new(Vec::new()) })
    }
}

#[async_trait]
impl SpeechProvider for FakeSpeech {
    async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError> {
        self.requests.lock().unwrap().push(req.clone());
        for (prefix, d) in &self.delays {
            if req.text.starts_with(prefix) {
                tokio::time::sleep(*d).await;
            }
        }
        if self.fail_on.is_some_and(|p| req.text.starts_with(p)) {
            return Err(ProviderError::BadResponse("tts exploded".into()));
        }
        Ok(Pcm { samples: vec![0.25; req.text.chars().count()], sample_rate: 24000 })
    }
}

struct FakeTranscribe {
    text: &'static str,
    languages: Mutex<Vec<String>>,
    /// The audio handed to each call, so the hold-to-talk turn can be checked, not just exercised.
    pcms: Mutex<Vec<Pcm>>,
}

#[async_trait]
impl TranscribeProvider for FakeTranscribe {
    async fn transcribe(&self, pcm: Pcm, language: &str) -> Result<String, ProviderError> {
        self.languages.lock().unwrap().push(language.to_string());
        self.pcms.lock().unwrap().push(pcm);
        Ok(self.text.to_string())
    }
}

// ---------- helpers ----------

const S1: &str = "Ozú, quillo, esto es la primera frase.";
const S2: &str = "Y esta es la segunda frase, compadre.";
const S3: &str = "Y la tercera, mi arma.";

/// A real config, so the built-in `compae` persona (and its greeting) is present.
fn config() -> Config {
    let dir = tempfile::tempdir().unwrap();
    store::load_or_create(&dir.path().join("config.toml")).unwrap().config
}

/// Runs a session on fake providers, handing back its ends and the transcribe fake.
fn session(
    chat: Arc<FakeChat>,
    speech: Arc<FakeSpeech>,
    transcript: &'static str,
) -> (mpsc::UnboundedSender<ToServer>, mpsc::UnboundedReceiver<ToClient>, Arc<FakeTranscribe>) {
    let transcribe = Arc::new(FakeTranscribe {
        text: transcript,
        languages: Mutex::new(Vec::new()),
        pcms: Mutex::new(Vec::new()),
    });
    let providers = Providers { chat, speech, transcribe: transcribe.clone() };
    let (tx, rx) = spawn_session(config(), providers, BrainConfig::default());
    (tx, rx, transcribe)
}

fn send(tx: &mpsc::UnboundedSender<ToServer>, command: Command) {
    tx.send(ToServer::Command(command)).expect("the session must still be listening");
}

fn send_mic(tx: &mpsc::UnboundedSender<ToServer>, samples: Vec<i16>) {
    tx.send(ToServer::Audio(Frame::Mic(samples))).expect("the session must still be listening");
}

/// The next message that matches `pred`, panicking with everything received if the timeout expires.
async fn next_matching(
    rx: &mut mpsc::UnboundedReceiver<ToClient>,
    pred: impl Fn(&ToClient) -> bool,
    timeout_ms: u64,
) -> ToClient {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            panic!("no message matched within {timeout_ms} ms; received {seen:#?}");
        }
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(m)) => {
                if pred(&m) {
                    return m;
                }
                seen.push(m);
            }
            Ok(None) => panic!("the session closed the channel; received {seen:#?}"),
            Err(_) => panic!("no message matched within {timeout_ms} ms; received {seen:#?}"),
        }
    }
}

fn is_speech(m: &ToClient) -> bool {
    matches!(m, ToClient::Audio(Frame::Speech { .. }))
}

fn is_done(m: &ToClient) -> bool {
    matches!(m, ToClient::Event(Event::Done { .. }))
}

fn is_idle(m: &ToClient) -> bool {
    matches!(m, ToClient::Event(Event::Status { state: AssistantState::Idle, .. }))
}

/// Everything received until `wait_ms` has passed without a message, or the session goes quiet.
async fn drain(rx: &mut mpsc::UnboundedReceiver<ToClient>, wait_ms: u64) -> Vec<ToClient> {
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

// ---------- tests ----------

#[tokio::test]
async fn welcome_then_greeting() {
    let config = config();
    let chat = FakeChat::new(vec![]);
    let speech = FakeSpeech::new();
    let (_tx, mut rx, _) = session(chat.clone(), speech.clone(), "");

    let first = next_matching(&mut rx, |_: &ToClient| true, 500).await;
    match first {
        ToClient::Event(Event::Welcome { persona, ui_language, listening_mode, web_bundle, .. }) => {
            assert_eq!(persona.id, "compae");
            assert_eq!(ui_language, UiLanguage::Es);
            assert_eq!(listening_mode, config.listening.mode);
            assert!(!web_bundle);
        }
        other => panic!("the first message must be Welcome, got {other:?}"),
    }

    next_matching(&mut rx, is_speech, 500).await;
    assert_eq!(chat.calls(), 0, "the greeting must not ask the chat provider");
    let spoken = speech.requests.lock().unwrap().clone();
    assert_eq!(spoken.len(), 1, "the greeting is the only thing spoken so far: {spoken:#?}");
    assert_eq!(spoken[0].text, config.active().greeting);
}

#[tokio::test]
async fn ask_produces_reply_and_done() {
    let chat = FakeChat::new(vec![Ok(vec![S1])]);
    let (tx, mut rx, _) = session(chat.clone(), FakeSpeech::new(), "");

    // The greeting is a `Say` turn: its `Done` means the session is ready for the next command.
    next_matching(&mut rx, is_done, 500).await;

    send(&tx, Command::Ask { text: "¿qué tal, compadre?".into() });
    let reply = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Reply { .. })), 500).await;
    assert_eq!(reply, ToClient::Event(Event::Reply { text: S1.into() }));
    next_matching(&mut rx, is_done, 500).await;
    assert_eq!(chat.calls(), 1);
}

#[tokio::test]
async fn listen_stop_transcribes_buffered_mic_audio() {
    let chat = FakeChat::new(vec![Ok(vec![S1])]);
    let (tx, mut rx, transcribe) = session(chat, FakeSpeech::new(), "¿y el ticket?");

    next_matching(&mut rx, is_done, 500).await; // the greeting

    send(&tx, Command::ListenStart);
    for _ in 0..10 {
        send_mic(&tx, vec![100; 320]);
    }
    send(&tx, Command::ListenStop);

    let heard = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Heard { .. })), 1000).await;
    assert_eq!(heard, ToClient::Event(Event::Heard { text: "¿y el ticket?".into() }));
    assert_eq!(transcribe.languages.lock().unwrap().as_slice(), ["es-ES".to_string()]);

    let pcms = transcribe.pcms.lock().unwrap();
    assert_eq!(pcms.len(), 1, "ListenStop must transcribe once, and only once: {pcms:#?}");
    assert_eq!(
        pcms[0].samples.len(),
        3200,
        "the ten 320-sample mic frames must reach the transcriber: {pcms:#?}"
    );
    assert_eq!(
        pcms[0].sample_rate,
        16000,
        "mic audio must be resampled to 16 kHz before transcription: {pcms:#?}"
    );
}

#[tokio::test]
async fn reset_clears_history() {
    let chat = FakeChat::new(vec![Ok(vec![S1]), Ok(vec![S2])]);
    let (tx, mut rx, _) = session(chat.clone(), FakeSpeech::new(), "");

    next_matching(&mut rx, is_done, 500).await; // the greeting
    send(&tx, Command::Ask { text: "primera pregunta".into() });
    next_matching(&mut rx, is_done, 500).await;

    send(&tx, Command::Reset);
    send(&tx, Command::Ask { text: "segunda pregunta".into() });
    next_matching(&mut rx, is_done, 500).await;

    let requests = chat.requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "one chat call per Ask");
    let second = requests[1].messages.clone();
    assert_eq!(second.len(), 2, "Reset must drop the earlier turn: {second:#?}");
    assert_eq!(second[0].role, Role::System);
    assert_eq!(second[1], ChatMessage::user("segunda pregunta"));
    assert!(!second.iter().any(|m| m.content == "primera pregunta" || m.content == S1));
}

#[tokio::test]
async fn barge_in_cancels_running_turn() {
    let chat = FakeChat::slow(vec![Ok(vec![S1, " ", S2, " ", S3])], Duration::from_millis(60));
    let (tx, mut rx, _) = session(chat, FakeSpeech::new(), "");

    next_matching(&mut rx, is_done, 500).await; // the greeting
    send(&tx, Command::Ask { text: "una pregunta larga".into() });
    next_matching(&mut rx, is_speech, 1000).await; // the first sentence is being spoken
    send(&tx, Command::Cancel);

    next_matching(&mut rx, is_idle, 500).await;
    let late = drain(&mut rx, 500).await;
    let audio: Vec<&ToClient> = late.iter().filter(|m| is_speech(m)).collect();
    assert!(audio.is_empty(), "no audio may play after Cancel: {audio:#?}");
}
