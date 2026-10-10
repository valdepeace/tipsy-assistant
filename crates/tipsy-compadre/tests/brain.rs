use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tipsy_compadre::audio::pcm::Pcm;
use tipsy_compadre::brain::{Brain, BrainConfig, TurnInput, TurnProfile};
use tipsy_compadre::providers::*;
use tipsy_protocol::{AssistantState, ErrorCode, Event, Frame, ToClient};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

// ---------- fakes ----------

/// Chat fake: each call pops the next scripted outcome.
struct FakeChat {
    script: Mutex<Vec<Result<Vec<&'static str>, ProviderError>>>,
    requests: Mutex<Vec<ChatRequest>>,
    chunk_delay: Duration,
}

impl FakeChat {
    fn new(script: Vec<Result<Vec<&'static str>, ProviderError>>) -> Arc<Self> {
        Arc::new(Self { script: Mutex::new(script), requests: Mutex::new(Vec::new()), chunk_delay: Duration::ZERO })
    }
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
#[derive(Default)]
struct FakeSpeech {
    delays: HashMap<&'static str, Duration>,
    fail_on: Option<&'static str>,
    requests: Mutex<Vec<SpeechRequest>>,
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
}

#[async_trait]
impl TranscribeProvider for FakeTranscribe {
    async fn transcribe(&self, _pcm: Pcm, language: &str) -> Result<String, ProviderError> {
        self.languages.lock().unwrap().push(language.to_string());
        Ok(self.text.to_string())
    }
}

// ---------- helpers ----------

const S1: &str = "Ozú, quillo, esto es la primera frase.";
const S2: &str = "Y esta es la segunda frase, compadre.";

fn profile() -> TurnProfile {
    TurnProfile {
        prompt: "SYSTEM PROMPT".into(),
        voice_instructions: "andaluz".into(),
        voice: Some("Charon".into()),
        language: "es-ES".into(),
    }
}

fn config() -> BrainConfig {
    BrainConfig {
        chat_timeout: Duration::from_secs(5),
        speech_timeout: Duration::from_secs(5),
        transcribe_timeout: Duration::from_secs(5),
        retry_backoff: Duration::from_millis(10),
    }
}

fn brain(chat: Arc<FakeChat>, speech: Arc<FakeSpeech>, transcript: &'static str) -> (Brain, Arc<FakeTranscribe>) {
    let transcribe = Arc::new(FakeTranscribe { text: transcript, languages: Mutex::new(Vec::new()) });
    let providers = ProviderSlot::new(Providers { chat, speech, transcribe: transcribe.clone() });
    (Brain::new(providers, profile(), config()), transcribe)
}

async fn run(b: &Brain, history: &mut Vec<ChatMessage>, input: TurnInput) -> Vec<ToClient> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    b.run_turn(history, input, tx, CancellationToken::new()).await;
    let mut out = Vec::new();
    while let Ok(m) = rx.try_recv() {
        out.push(m);
    }
    out
}

fn events(out: &[ToClient]) -> Vec<Event> {
    out.iter().filter_map(|m| if let ToClient::Event(e) = m { Some(e.clone()) } else { None }).collect()
}

/// (seq, samples.len()) of every speech frame, in delivery order.
fn speech_frames(out: &[ToClient]) -> Vec<(u32, usize)> {
    out.iter()
        .filter_map(|m| if let ToClient::Audio(Frame::Speech { seq, samples }) = m { Some((*seq, samples.len())) } else { None })
        .collect()
}

// ---------- tests ----------

#[tokio::test]
async fn ask_streams_text_and_speaks_sentences_in_order() {
    let chat = FakeChat::new(vec![Ok(vec!["Ozú, quillo, esto es ", "la primera frase. Y esta es la segunda ", "frase, compadre."])]);
    // The first sentence is slow, so the second finishes synthesis first; delivery must stay in order.
    let speech = Arc::new(FakeSpeech { delays: HashMap::from([("Ozú", Duration::from_millis(80))]), ..Default::default() });
    let (b, _) = brain(chat.clone(), speech.clone(), "");
    let mut history = Vec::new();
    let out = run(&b, &mut history, TurnInput::Ask("¿qué tal?".into())).await;

    assert_eq!(speech_frames(&out), vec![(0, S1.chars().count()), (1, S2.chars().count())]);

    let ev = events(&out);
    assert_eq!(ev.first(), Some(&Event::Status { state: AssistantState::Thinking, detail: None }));
    assert_eq!(ev.last(), Some(&Event::Status { state: AssistantState::Idle, detail: None }));
    let deltas: String = ev.iter().filter_map(|e| if let Event::ReplyDelta { text } = e { Some(text.as_str()) } else { None }).collect();
    assert_eq!(deltas, format!("{S1} {S2}"));
    assert!(ev.contains(&Event::Reply { text: format!("{S1} {S2}") }));
    assert!(ev.iter().any(|e| matches!(e, Event::Done { first_audio_ms: Some(_), .. })));

    // Each SpeechChunk header is immediately followed by its frame.
    for (i, m) in out.iter().enumerate() {
        if let ToClient::Event(Event::SpeechChunk { seq, sample_rate }) = m {
            assert_eq!(*sample_rate, 24000);
            assert!(matches!(&out[i + 1], ToClient::Audio(Frame::Speech { seq: s, .. }) if s == seq));
        }
    }

    assert_eq!(history, vec![ChatMessage::user("¿qué tal?"), ChatMessage::assistant(format!("{S1} {S2}"))]);
}

#[tokio::test]
async fn chat_request_has_system_prompt_history_and_question() {
    let chat = FakeChat::new(vec![Ok(vec!["Una respuesta suficientemente larga."])]);
    let (b, _) = brain(chat.clone(), Arc::new(FakeSpeech::default()), "");
    let mut history = vec![ChatMessage::user("antes"), ChatMessage::assistant("respuesta de antes")];
    run(&b, &mut history, TurnInput::Ask("ahora".into())).await;
    let sent = chat.requests.lock().unwrap()[0].messages.clone();
    assert_eq!(sent, vec![
        ChatMessage::system("SYSTEM PROMPT"),
        ChatMessage::user("antes"),
        ChatMessage::assistant("respuesta de antes"),
        ChatMessage::user("ahora"),
    ]);
}

#[tokio::test]
async fn speech_requests_are_cleaned_and_carry_profile_voice() {
    let chat = FakeChat::new(vec![Ok(vec!["Eso es pa' ná', **compadre**, de verdad te lo digo."])]);
    let speech = Arc::new(FakeSpeech::default());
    let (b, _) = brain(chat, speech.clone(), "");
    run(&b, &mut Vec::new(), TurnInput::Ask("x".into())).await;
    let reqs = speech.requests.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].text, "Eso es pa ná, compadre, de verdad te lo digo.");
    assert_eq!(reqs[0].instructions, "andaluz");
    assert_eq!(reqs[0].voice.as_deref(), Some("Charon"));
}

#[tokio::test]
async fn sentences_that_clean_to_nothing_are_skipped_without_using_a_seq() {
    let chat = FakeChat::new(vec![Ok(vec!["🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺🍺. ", S2])]);
    let speech = Arc::new(FakeSpeech::default());
    let (b, _) = brain(chat, speech.clone(), "");
    let out = run(&b, &mut Vec::new(), TurnInput::Ask("x".into())).await;
    assert_eq!(speech.requests.lock().unwrap().len(), 1);
    assert_eq!(speech_frames(&out), vec![(0, S2.chars().count())]);
}

#[tokio::test]
async fn utterance_is_transcribed_with_profile_language_then_answered() {
    let chat = FakeChat::new(vec![Ok(vec![S1])]);
    let (b, transcribe) = brain(chat.clone(), Arc::new(FakeSpeech::default()), "¿y el ticket?");
    let mut history = Vec::new();
    let out = run(&b, &mut history, TurnInput::Utterance(Pcm { samples: vec![0.0; 16000], sample_rate: 16000 })).await;
    let ev = events(&out);
    assert_eq!(ev[0], Event::Status { state: AssistantState::Transcribing, detail: None });
    assert!(ev.contains(&Event::Heard { text: "¿y el ticket?".into() }));
    assert_eq!(transcribe.languages.lock().unwrap().as_slice(), ["es-ES".to_string()]);
    assert_eq!(history[0], ChatMessage::user("¿y el ticket?"));
}

#[tokio::test]
async fn blank_transcription_reports_nothing_heard() {
    let chat = FakeChat::new(vec![]);
    let (b, _) = brain(chat.clone(), Arc::new(FakeSpeech::default()), "   ");
    let mut history = Vec::new();
    let out = run(&b, &mut history, TurnInput::Utterance(Pcm { samples: vec![0.0; 160], sample_rate: 16000 })).await;
    let ev = events(&out);
    assert!(ev.iter().any(|e| matches!(e, Event::Error { code: ErrorCode::NothingHeard, recoverable: true, .. })));
    assert_eq!(ev.last(), Some(&Event::Status { state: AssistantState::Idle, detail: None }));
    assert_eq!(chat.calls(), 0);
    assert!(history.is_empty());
}

#[tokio::test]
async fn say_speaks_literally_without_chat_or_history() {
    let chat = FakeChat::new(vec![]);
    let speech = Arc::new(FakeSpeech::default());
    let (b, _) = brain(chat.clone(), speech.clone(), "");
    let mut history = Vec::new();
    let out = run(&b, &mut history, TurnInput::Say(format!("{S1} {S2}"))).await;
    assert_eq!(chat.calls(), 0);
    assert_eq!(speech_frames(&out).len(), 2);
    assert!(events(&out).contains(&Event::Reply { text: format!("{S1} {S2}") }));
    assert!(history.is_empty());
}

#[tokio::test]
async fn chat_auth_error_is_reported_and_history_untouched() {
    let chat = FakeChat::new(vec![Err(ProviderError::Auth("bad key".into()))]);
    let (b, _) = brain(chat.clone(), Arc::new(FakeSpeech::default()), "");
    let mut history = Vec::new();
    let out = run(&b, &mut history, TurnInput::Ask("x".into())).await;
    let ev = events(&out);
    assert!(ev.iter().any(|e| matches!(e, Event::Error { code: ErrorCode::Auth, recoverable: true, message } if message.contains("bad key"))));
    assert_eq!(ev.last(), Some(&Event::Status { state: AssistantState::Idle, detail: None }));
    assert_eq!(chat.calls(), 1, "auth errors are not retried");
    assert!(history.is_empty());
}

#[tokio::test]
async fn rate_limited_chat_is_retried_once() {
    let chat = FakeChat::new(vec![Err(ProviderError::RateLimited("busy".into())), Ok(vec![S1])]);
    let (b, _) = brain(chat.clone(), Arc::new(FakeSpeech::default()), "");
    let mut history = Vec::new();
    let out = run(&b, &mut history, TurnInput::Ask("x".into())).await;
    assert_eq!(chat.calls(), 2);
    assert_eq!(speech_frames(&out).len(), 1);
    assert_eq!(history.len(), 2);
}

#[tokio::test]
async fn speech_failure_stops_audio_but_keeps_text_and_history() {
    let s3 = "Y una tercera frase que no sonará nunca.";
    let chat = FakeChat::new(vec![Ok(vec![S1, " ", S2, " ", s3])]);
    let speech = Arc::new(FakeSpeech { fail_on: Some("Y esta"), ..Default::default() });
    let (b, _) = brain(chat, speech, "");
    let mut history = Vec::new();
    let out = run(&b, &mut history, TurnInput::Ask("x".into())).await;
    assert_eq!(speech_frames(&out), vec![(0, S1.chars().count())]);
    let ev = events(&out);
    assert!(ev.iter().any(|e| matches!(e, Event::Error { code: ErrorCode::BadResponse, recoverable: true, .. })));
    assert!(ev.contains(&Event::Reply { text: format!("{S1} {S2} {s3}") }));
    assert!(ev.iter().any(|e| matches!(e, Event::Done { .. })));
    assert_eq!(history[1], ChatMessage::assistant(format!("{S1} {S2} {s3}")));
}

#[tokio::test]
async fn cancel_stops_audio_and_keeps_only_spoken_sentences() {
    let chat = FakeChat::slow(vec![Ok(vec![S1, " ", S2, " ", "Una tercera frase bastante larga también."])], Duration::from_millis(60));
    let (b, _) = brain(chat, Arc::new(FakeSpeech::default()), "");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let mut history = Vec::new();
    let turn = b.run_turn(&mut history, TurnInput::Ask("x".into()), tx, cancel.clone());
    let watcher = async {
        let mut seen = Vec::new();
        while let Some(m) = rx.recv().await {
            let first_frame = matches!(m, ToClient::Audio(Frame::Speech { .. }));
            seen.push(m);
            if first_frame {
                cancel.cancel();
            }
        }
        seen
    };
    let ((), out) = tokio::join!(turn, watcher);
    assert_eq!(speech_frames(&out).len(), 1, "no audio after cancel");
    assert_eq!(events(&out).last(), Some(&Event::Status { state: AssistantState::Idle, detail: None }));
    assert_eq!(history, vec![ChatMessage::user("x"), ChatMessage::assistant(S1)]);
}

#[tokio::test]
async fn reply_without_final_punctuation_is_still_spoken() {
    let chat = FakeChat::new(vec![Ok(vec!["Un remate sin punto final y ya está"])]);
    let speech = Arc::new(FakeSpeech::default());
    let (b, _) = brain(chat, speech.clone(), "");
    let out = run(&b, &mut Vec::new(), TurnInput::Ask("x".into())).await;
    assert_eq!(speech_frames(&out).len(), 1);
    assert_eq!(speech.requests.lock().unwrap()[0].text, "Un remate sin punto final y ya está");
}
