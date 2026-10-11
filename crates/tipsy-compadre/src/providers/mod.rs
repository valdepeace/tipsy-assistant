//! Cloud provider abstractions: one trait per capability.

pub mod elevenlabs;
pub mod gemini;
pub mod openai_compat;
pub mod sse;

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
// `Capability` is defined in `tipsy-protocol` (it is part of the wire vocabulary Plan 2B's
// `Command::TestProvider` and `Event::TestResult` use) and consumed here by `BuildError`.
use tipsy_protocol::{Capability, ErrorCode};

use crate::audio::pcm::Pcm;
use crate::config::{ProviderConfig, ProviderKind, ProvidersConfig};
use crate::keys::{env_var_name, KeyError, KeyResolver};
use elevenlabs::{ElevenLabsSpeech, ElevenLabsTranscribe};
use gemini::{GeminiChat, GeminiSpeech};
use openai_compat::OpenAiCompat;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeechRequest {
    pub text: String,
    /// Delivery instructions (the profile's `voice_instructions`); ignored by providers without support.
    pub instructions: String,
    pub voice: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProviderError {
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("timed out")]
    Timeout,
    #[error("network error: {0}")]
    Network(String),
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("cancelled")]
    Cancelled,
}

impl ProviderError {
    /// Maps an HTTP error status and its body to an error, extracting `error.message` from a JSON
    /// body when present. ElevenLabs reports the array form `detail[]` instead of the string
    /// `/detail/message` the other providers use, and the entry it puts there names its field
    /// `msg` — the key the plan's recorded fixture carries — so both spellings are read.
    pub fn from_status(status: u16, body: &str) -> Self {
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .or_else(|| v.pointer("/message"))
                    .or_else(|| v.pointer("/detail/message"))
                    .or_else(|| v.pointer("/detail/0/message"))
                    .or_else(|| v.pointer("/detail/0/msg"))
                    .and_then(|m| m.as_str().map(str::to_string))
                    .filter(|m| !m.is_empty())
            })
            .unwrap_or_else(|| {
                let trimmed: String = body.chars().take(300).collect();
                if trimmed.trim().is_empty() { format!("HTTP {status}") } else { trimmed }
            });
        match status {
            401 | 403 => ProviderError::Auth(message),
            429 => ProviderError::RateLimited(message),
            408 | 504 => ProviderError::Timeout,
            _ => ProviderError::BadResponse(format!("HTTP {status}: {message}")),
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            ProviderError::Auth(_) => ErrorCode::Auth,
            ProviderError::RateLimited(_) => ErrorCode::RateLimited,
            ProviderError::Timeout => ErrorCode::Timeout,
            ProviderError::Network(_) => ErrorCode::Network,
            ProviderError::BadResponse(_) => ErrorCode::BadResponse,
            ProviderError::Cancelled => ErrorCode::Internal,
        }
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            ProviderError::Timeout
        } else {
            ProviderError::Network(e.to_string())
        }
    }
}

pub type TextStream = futures::stream::BoxStream<'static, Result<String, ProviderError>>;

#[async_trait]
pub trait ChatProvider: Send + Sync {
    /// Streams the reply text in pieces as the model produces it.
    async fn stream(&self, req: ChatRequest) -> Result<TextStream, ProviderError>;
}

#[async_trait]
pub trait SpeechProvider: Send + Sync {
    async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError>;
    /// How many sentences may be synthesised at the same time.
    fn max_parallel(&self) -> usize {
        4
    }
}

#[async_trait]
pub trait TranscribeProvider: Send + Sync {
    /// `language` is a BCP 47 tag such as `es-ES`; providers may use only the primary subtag.
    async fn transcribe(&self, pcm: Pcm, language: &str) -> Result<String, ProviderError>;
}

#[derive(Clone)]
pub struct Providers {
    pub chat: Arc<dyn ChatProvider>,
    pub speech: Arc<dyn SpeechProvider>,
    pub transcribe: Arc<dyn TranscribeProvider>,
}

/// A handle to the provider stack a session is running on.
///
/// The client and the session share one, which is what makes a save live: the client builds the
/// connectors from the configuration it just wrote and swaps them in, and every turn started after
/// that reads the new stack. Nothing is restarted, and no turn is ever half on the old stack — the
/// stack is read once, at the top of `Brain::run_turn`.
#[derive(Clone)]
pub struct ProviderSlot(std::sync::Arc<std::sync::RwLock<Providers>>);

impl ProviderSlot {
    pub fn new(providers: Providers) -> Self {
        Self(std::sync::Arc::new(std::sync::RwLock::new(providers)))
    }

    /// Swaps the whole stack. A turn already running keeps the stack it started on.
    pub fn replace(&self, providers: Providers) {
        // Poisoning cannot happen: the guard is dropped before any provider call is made, so
        // nothing that can panic ever runs while it is held.
        *self.0.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = providers;
    }

    /// The stack the next turn uses: three `Arc`s, and the guard is gone before the call is made.
    pub fn current(&self) -> Providers {
        self.0.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("no API key named `{name}`: run `tipsy-compadre key set {name}` or set {env}")]
    MissingKey { name: String, env: String },
    /// A connector that exists, asked for a capability it does not offer (spec §7: ElevenLabs has
    /// no chat, Gemini has no transcription). Replaces Plan 1's `Unsupported`, whose message
    /// ("not available yet") stopped being true when these connectors landed.
    #[error("{capability} is not supported by the `{kind:?}` connector")]
    UnsupportedCapability { capability: Capability, kind: ProviderKind },
    #[error(transparent)]
    Key(#[from] KeyError),
}

fn key_for(cfg: &ProviderConfig, keys: &KeyResolver) -> Result<Option<String>, BuildError> {
    if cfg.key.trim().is_empty() {
        return Ok(None);
    }
    match keys.resolve(&cfg.key)? {
        Some((value, _source)) => Ok(Some(value)),
        None => Err(BuildError::MissingKey { name: cfg.key.clone(), env: env_var_name(&cfg.key) }),
    }
}

/// Whether `kind` offers `capability` at all (spec §7's connector table).
const fn supports(kind: ProviderKind, capability: Capability) -> bool {
    match capability {
        Capability::Chat => matches!(kind, ProviderKind::OpenaiCompat | ProviderKind::Gemini),
        Capability::Speech => true,
        Capability::Transcribe => matches!(kind, ProviderKind::OpenaiCompat | ProviderKind::Elevenlabs),
    }
}

fn chat_provider(cfg: &ProviderConfig, keys: &KeyResolver) -> Result<Arc<dyn ChatProvider>, BuildError> {
    match cfg.kind {
        ProviderKind::OpenaiCompat => Ok(Arc::new(OpenAiCompat::new(&cfg.base_url, &cfg.model, key_for(cfg, keys)?))),
        ProviderKind::Gemini => Ok(Arc::new(GeminiChat::new(&cfg.base_url, &cfg.model, key_for(cfg, keys)?))),
        ProviderKind::Elevenlabs => {
            Err(BuildError::UnsupportedCapability { capability: Capability::Chat, kind: cfg.kind })
        }
    }
}

fn speech_provider(cfg: &ProviderConfig, keys: &KeyResolver) -> Result<Arc<dyn SpeechProvider>, BuildError> {
    match cfg.kind {
        ProviderKind::OpenaiCompat => Ok(Arc::new(OpenAiCompat::new(&cfg.base_url, &cfg.model, key_for(cfg, keys)?))),
        ProviderKind::Gemini => Ok(Arc::new(GeminiSpeech::new(&cfg.base_url, &cfg.model, key_for(cfg, keys)?))),
        ProviderKind::Elevenlabs => {
            Ok(Arc::new(ElevenLabsSpeech::new(&cfg.base_url, &cfg.model, key_for(cfg, keys)?)))
        }
    }
}

fn transcribe_provider(cfg: &ProviderConfig, keys: &KeyResolver) -> Result<Arc<dyn TranscribeProvider>, BuildError> {
    match cfg.kind {
        ProviderKind::OpenaiCompat => Ok(Arc::new(OpenAiCompat::new(&cfg.base_url, &cfg.model, key_for(cfg, keys)?))),
        ProviderKind::Gemini => {
            Err(BuildError::UnsupportedCapability { capability: Capability::Transcribe, kind: cfg.kind })
        }
        ProviderKind::Elevenlabs => {
            Ok(Arc::new(ElevenLabsTranscribe::new(&cfg.base_url, &cfg.model, key_for(cfg, keys)?)))
        }
    }
}

// Counts the provider stacks built **on this thread**, for Plan 3 Task 1.7's audit: the `--connect`
// client must never build one, and `client.rs`'s `the_remote_path_builds_no_provider_and_asks_the_server_instead`
// is what reads it. Thread-local rather than global because `cargo test` runs this binary's other
// tests — which build providers on purpose — on other threads at the same time.
#[cfg(test)]
thread_local! {
    static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread has built a provider stack or a single connector.
#[cfg(test)]
pub fn builds_on_this_thread() -> usize {
    BUILDS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn count_a_build() {
    BUILDS.with(|built| built.set(built.get() + 1));
}

/// A release build counts nothing: the audit exists only in the test binary.
#[cfg(not(test))]
fn count_a_build() {}

pub fn build_providers(config: &ProvidersConfig, keys: &KeyResolver) -> Result<Providers, BuildError> {
    count_a_build();
    // The capabilities are decided for all three connectors *before* any key is resolved: a
    // capability that cannot exist is not a missing-key problem, whoever else in the file has no
    // key. The builders below repeat the check so they stay correct on their own.
    for (capability, cfg) in [
        (Capability::Chat, &config.chat),
        (Capability::Speech, &config.speech),
        (Capability::Transcribe, &config.transcribe),
    ] {
        if !supports(cfg.kind, capability) {
            return Err(BuildError::UnsupportedCapability { capability, kind: cfg.kind });
        }
    }
    Ok(Providers {
        chat: chat_provider(&config.chat, keys)?,
        speech: speech_provider(&config.speech, keys)?,
        transcribe: transcribe_provider(&config.transcribe, keys)?,
    })
}

/// One capability's connector, built from one row of the configuration screen's Providers tab.
///
/// The Test button tests the values *on screen*, before they are saved, so it builds one connector
/// from one `ProviderConfig` — never the whole [`Providers`] stack, whose other two slots could
/// fail the test for a key this capability never needed.
pub enum Connector {
    Chat(Arc<dyn ChatProvider>),
    Speech(Arc<dyn SpeechProvider>),
    Transcribe(Arc<dyn TranscribeProvider>),
}

/// Builds the connector `capability` names, from one provider's configuration.
///
/// The capability is checked before any key is resolved, exactly as [`build_providers`] does it: a
/// capability that cannot exist is not a missing-key problem, whoever else in the file has no key.
pub fn build_one(cfg: &ProviderConfig, capability: Capability, keys: &KeyResolver) -> Result<Connector, BuildError> {
    count_a_build();
    if !supports(cfg.kind, capability) {
        return Err(BuildError::UnsupportedCapability { capability, kind: cfg.kind });
    }
    Ok(match capability {
        Capability::Chat => Connector::Chat(chat_provider(cfg, keys)?),
        Capability::Speech => Connector::Speech(speech_provider(cfg, keys)?),
        Capability::Transcribe => Connector::Transcribe(transcribe_provider(cfg, keys)?),
    })
}

/// What one Test found out: whether a real call answered, how long it took, and words a person can
/// read — the provider's own on failure, a short summary of what came back on success.
///
/// The three fields are one to one with `Event::TestResult`, so the client only copies them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub ok: bool,
    pub latency_ms: u64,
    pub message: String,
}

impl Probe {
    fn answered(latency_ms: u64, message: impl Into<String>) -> Self {
        Self { ok: true, latency_ms, message: message.into() }
    }

    fn refused(latency_ms: u64, message: impl Into<String>) -> Self {
        Self { ok: false, latency_ms, message: message.into() }
    }
}

/// The one word a chat probe asks for. `ChatRequest` carries no token cap, so the size of the reply
/// is bounded by reading a single delta and by the connector's own defaults — not by asking for
/// fewer tokens.
const PROBE_CHAT_WORD: &str = "ping";
/// The one word a speech probe speaks. It is Spanish because the product's shipped profiles are,
/// and the voice is whatever the row's model defaults to.
const PROBE_SPEECH_WORD: &str = "hola";
/// Half a second of silence at 16 kHz: the shortest buffer every transcription endpoint accepts.
const PROBE_SILENCE_SAMPLES: usize = 8_000;
const PROBE_SILENCE_RATE: u32 = 16_000;
/// What an empty 200 says. It is the probe's own sentence rather than a wrapped
/// `ProviderError::BadResponse`, whose `Display` would prefix it with "bad response: " — the plan
/// pins this exact wording, and nothing came back from the provider to quote instead.
const NO_TEXT: &str = "the provider answered with no text";

impl Connector {
    /// The smallest real call this connector allows (spec §9's Test button).
    ///
    /// * chat: one request whose only message is a single word, of which exactly one delta is read
    ///   and the stream then dropped. A 200 that streams no text at all — and a 200 that streams an
    ///   error payload — is *not* a success: a test that believed the status code would lie to the
    ///   user.
    /// * speech: one word, with the connector's own default voice and no instructions.
    /// * transcription: half a second of silence, which is the shortest buffer every connector's
    ///   endpoint accepts. An empty transcript is an answer — silence has no words.
    ///
    /// `language` is the BCP 47 tag the transcription probe sends, the way every other
    /// transcription does; the other two capabilities ignore it.
    pub async fn probe(self, language: &str) -> Probe {
        match self {
            Connector::Chat(chat) => probe_chat(chat.as_ref()).await,
            Connector::Speech(speech) => probe_speech(speech.as_ref()).await,
            Connector::Transcribe(transcribe) => probe_transcribe(transcribe.as_ref(), language).await,
        }
    }
}

/// The clock starts before the request and stops when the provider has answered, so `latency_ms` is
/// what the call took rather than a constant.
async fn probe_chat(chat: &dyn ChatProvider) -> Probe {
    let started = std::time::Instant::now();
    let request = ChatRequest { messages: vec![ChatMessage::user(PROBE_CHAT_WORD)] };
    // Every refusal below is a `String` and not a `ProviderError`, because one of them — a 200 that
    // says nothing — is not a provider error at all; the probe reports all three the same way, in
    // words a person can read.
    let outcome = match chat.stream(request).await {
        Err(e) => Err(e.to_string()),
        Ok(mut stream) => match stream.next().await {
            Some(Ok(text)) => Ok(format!("{} bytes", text.len())),
            Some(Err(e)) => Err(e.to_string()),
            None => Err(NO_TEXT.to_string()),
        },
    };
    let latency_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(message) => Probe::answered(latency_ms, message),
        Err(message) => Probe::refused(latency_ms, message),
    }
}

async fn probe_speech(speech: &dyn SpeechProvider) -> Probe {
    let started = std::time::Instant::now();
    // The connector's own default voice (`None`) and no delivery instructions: a Test asks whether
    // this row can speak at all. The profile is not on screen in this tab, and nothing here plays
    // what comes back — the samples are counted and dropped.
    let request = SpeechRequest { text: PROBE_SPEECH_WORD.into(), instructions: String::new(), voice: None };
    let outcome = speech.synthesize(request).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(pcm) if pcm.samples.is_empty() => {
            // A 200 that synthesised nothing is a refusal: there is nothing to play from it.
            Probe::refused(latency_ms, "the provider returned no audio")
        }
        Ok(pcm) => {
            Probe::answered(latency_ms, format!("{} samples at {} Hz", pcm.samples.len(), pcm.sample_rate))
        }
        Err(e) => Probe::refused(latency_ms, e.to_string()),
    }
}

async fn probe_transcribe(transcribe: &dyn TranscribeProvider, language: &str) -> Probe {
    let started = std::time::Instant::now();
    let pcm = Pcm { samples: vec![0.0; PROBE_SILENCE_SAMPLES], sample_rate: PROBE_SILENCE_RATE };
    let outcome = transcribe.transcribe(pcm, language).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match outcome {
        // Silence has no words: an empty transcript is what a working connector answers with.
        Ok(text) if text.trim().is_empty() => Probe::answered(latency_ms, "silence, no words"),
        Ok(text) => Probe::answered(latency_ms, format!("{} characters", text.chars().count())),
        Err(e) => Probe::refused(latency_ms, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_extracts_messages() {
        assert_eq!(
            ProviderError::from_status(401, r#"{"error":{"message":"Missing Authentication header","code":401}}"#),
            ProviderError::Auth("Missing Authentication header".into())
        );
        assert_eq!(
            ProviderError::from_status(429, r#"{"error":{"message":"Provider returned error"}}"#),
            ProviderError::RateLimited("Provider returned error".into())
        );
        assert_eq!(ProviderError::from_status(403, "forbidden"), ProviderError::Auth("forbidden".into()));
        assert_eq!(ProviderError::from_status(504, ""), ProviderError::Timeout);
        assert_eq!(
            ProviderError::from_status(500, ""),
            ProviderError::BadResponse("HTTP 500: HTTP 500".into())
        );
        assert_eq!(
            ProviderError::from_status(400, r#"{"message":"Gemini TTS only supports pcm"}"#),
            ProviderError::BadResponse("HTTP 400: Gemini TTS only supports pcm".into())
        );
    }

    /// ElevenLabs answers 422 with `detail` as an **array** of objects whose message field is
    /// `msg`, the key the plan's recorded fixture carries (`tests/elevenlabs.rs`), so the single
    /// string path (`/detail/message`) cannot read it and the first entry's `msg` is used instead.
    #[test]
    fn status_mapping_reads_the_first_detail_array_message() {
        assert_eq!(
            ProviderError::from_status(
                422,
                r#"{"detail":[{"type":"invalid_request","code":"voice_not_found","msg":"voice not found"}]}"#
            ),
            ProviderError::BadResponse("HTTP 422: voice not found".into())
        );
        // The `message` spelling is kept too: a provider that answers with it must not regress.
        assert_eq!(
            ProviderError::from_status(
                422,
                r#"{"detail":[{"type":"missing","loc":["body","text"],"message":"Field required","input":null}]}"#
            ),
            ProviderError::BadResponse("HTTP 422: Field required".into())
        );
    }

    /// The array form is used only when its first entry really carries a non-empty `msg` (or
    /// `message`): anything else falls back to the body, which is still never an empty message.
    #[test]
    fn detail_arrays_without_a_message_keep_the_body_fallback() {
        assert_eq!(
            ProviderError::from_status(422, r#"{"detail":[{"reason":"nope"}]}"#),
            ProviderError::BadResponse(r#"HTTP 422: {"detail":[{"reason":"nope"}]}"#.into())
        );
        assert_eq!(
            ProviderError::from_status(422, r#"{"detail":[{"msg":""}]}"#),
            ProviderError::BadResponse(r#"HTTP 422: {"detail":[{"msg":""}]}"#.into())
        );
        assert_eq!(
            ProviderError::from_status(500, r#"{"detail":[{"message":""}]}"#),
            ProviderError::BadResponse(r#"HTTP 500: {"detail":[{"message":""}]}"#.into())
        );
        assert_eq!(ProviderError::from_status(422, ""), ProviderError::BadResponse("HTTP 422: HTTP 422".into()));
    }

    #[test]
    fn long_non_json_bodies_are_truncated_on_char_boundaries() {
        let body = "ñ".repeat(1000);
        match ProviderError::from_status(502, &body) {
            ProviderError::BadResponse(m) => assert_eq!(m.chars().filter(|c| *c == 'ñ').count(), 300),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn error_codes() {
        assert_eq!(ProviderError::Timeout.code(), ErrorCode::Timeout);
        assert_eq!(ProviderError::Auth("x".into()).code(), ErrorCode::Auth);
    }
}

#[cfg(test)]
mod build_tests {
    use super::*;
    use crate::keys::MemoryStore;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn resolver_with(key: Option<&str>) -> KeyResolver {
        let store = MemoryStore::default();
        if let Some(k) = key {
            crate::keys::SecretStore::set(&store, "openrouter", k).unwrap();
        }
        KeyResolver::new(Box::new(store), Box::new(|_| None))
    }

    /// Like `resolver_with`, for a provider whose key name is not `openrouter`.
    fn resolver_named(pairs: &[(&str, &str)]) -> KeyResolver {
        let store = MemoryStore::default();
        for (name, value) in pairs {
            crate::keys::SecretStore::set(&store, name, value).unwrap();
        }
        KeyResolver::new(Box::new(store), Box::new(|_| None))
    }

    #[test]
    fn builds_with_key() {
        assert!(build_providers(&ProvidersConfig::default(), &resolver_with(Some("k"))).is_ok());
    }

    #[test]
    fn missing_key_explains_how_to_fix_it() {
        let err = build_providers(&ProvidersConfig::default(), &resolver_with(None)).err().unwrap();
        let msg = err.to_string();
        assert!(msg.contains("tipsy-compadre key set openrouter"), "{msg}");
        assert!(msg.contains("TIPSY_KEY_OPENROUTER"), "{msg}");
    }

    #[test]
    fn empty_key_name_means_no_key() {
        let mut cfg = ProvidersConfig::default();
        for p in [&mut cfg.chat, &mut cfg.speech, &mut cfg.transcribe] {
            p.key = String::new();
            p.base_url = "http://localhost:11434/v1".into();
        }
        assert!(build_providers(&cfg, &resolver_with(None)).is_ok());
    }

    /// Rejected capability, named: no panic, no silent fallback to another connector. The
    /// resolver is empty on purpose — the capability is checked before any key is resolved.
    #[test]
    fn gemini_transcription_is_rejected_by_name() {
        let mut cfg = ProvidersConfig::default();
        cfg.transcribe.kind = ProviderKind::Gemini;
        let err = build_providers(&cfg, &resolver_with(None)).err().unwrap();
        assert!(
            matches!(
                err,
                BuildError::UnsupportedCapability { capability: Capability::Transcribe, kind: ProviderKind::Gemini }
            ),
            "{err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("transcription") && msg.contains("Gemini"), "{msg}");
    }

    /// The note's spec §7 table has no chat row for ElevenLabs, and the old test that asserted
    /// `Unsupported` here is gone: ElevenLabs speech and transcription now build.
    #[test]
    fn elevenlabs_chat_is_rejected_by_name() {
        let mut cfg = ProvidersConfig::default();
        cfg.chat.kind = ProviderKind::Elevenlabs;
        let err = build_providers(&cfg, &resolver_with(None)).err().unwrap();
        assert!(
            matches!(
                err,
                BuildError::UnsupportedCapability { capability: Capability::Chat, kind: ProviderKind::Elevenlabs }
            ),
            "{err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("chat") && msg.contains("Elevenlabs"), "{msg}");
    }

    /// Both Gemini kinds build, and the requests they make land on Gemini's own endpoint — the
    /// mock answers 500, which the connector maps like any other error; the test reads the path,
    /// not the response.
    #[tokio::test]
    async fn gemini_builds_chat_and_speech() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(r#"{"error":{"message":"stop here"}}"#))
            .mount(&server)
            .await;

        let mut cfg = ProvidersConfig::default();
        cfg.chat.kind = ProviderKind::Gemini;
        cfg.chat.base_url = server.uri();
        cfg.chat.model = "gemini-2.5-flash".into();
        cfg.speech.kind = ProviderKind::Gemini;
        cfg.speech.base_url = server.uri();
        cfg.speech.model = "gemini-3.8-flash-tts".into();
        let providers = build_providers(&cfg, &resolver_with(Some("k"))).unwrap();

        let _ = providers.chat.stream(ChatRequest { messages: vec![ChatMessage::user("hola")] }).await;
        let _ = providers
            .speech
            .synthesize(SpeechRequest { text: "hola".into(), instructions: "andaluz".into(), voice: Some("Charon".into()) })
            .await;

        let paths: Vec<String> =
            server.received_requests().await.unwrap().iter().map(|r| r.url.path().to_string()).collect();
        assert_eq!(paths.len(), 2, "{paths:?}");
        for path in &paths {
            assert!(path.ends_with("/v1beta/interactions"), "{path} is not the Gemini connector's endpoint");
        }
    }

    /// Both ElevenLabs kinds build, and the connector answers on its own paths. El Compae's
    /// shipped `elevenlabs = "george"` is a voice *name*: the speech request must reach
    /// `/v1/text-to-speech/<resolved id>`, which is Task 3's resolution seen through the wiring.
    #[tokio::test]
    async fn elevenlabs_builds_speech_and_transcription() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/voices"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"voices":[{"voice_id":"fixture-voice-one","name":"Fixture One"},{"voice_id":"fixture-voice-george","name":"George"}]}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(r#"{"detail":{"message":"stop here"}}"#))
            .mount(&server)
            .await;

        let mut cfg = ProvidersConfig::default();
        cfg.speech.kind = ProviderKind::Elevenlabs;
        cfg.speech.base_url = server.uri();
        cfg.speech.model = "eleven_flash_v2_5".into();
        cfg.transcribe.kind = ProviderKind::Elevenlabs;
        cfg.transcribe.base_url = server.uri();
        cfg.transcribe.model = "scribe_v2".into();
        let providers = build_providers(&cfg, &resolver_with(Some("k"))).unwrap();

        let _ = providers
            .speech
            .synthesize(SpeechRequest { text: "hola".into(), instructions: String::new(), voice: Some("george".into()) })
            .await;
        let _ = providers.transcribe.transcribe(Pcm { samples: vec![0.0; 1600], sample_rate: 16_000 }, "es-ES").await;

        let paths: Vec<String> =
            server.received_requests().await.unwrap().iter().map(|r| r.url.path().to_string()).collect();
        assert!(paths.contains(&"/v1/voices".to_string()), "{paths:?}");
        assert!(paths.contains(&"/v1/text-to-speech/fixture-voice-george".to_string()), "{paths:?}");
        assert!(paths.contains(&"/v1/speech-to-text".to_string()), "{paths:?}");
    }

    /// The capability outranks the missing key even when the missing key belongs to *another*
    /// connector: the chat key is absent here, but ElevenLabs can never chat, so the answer is the
    /// capability error and not a complaint about a key that would not have helped.
    #[test]
    fn a_rejected_capability_outranks_a_missing_key_elsewhere() {
        let mut cfg = ProvidersConfig::default();
        cfg.chat.key = "openrouter".into();
        cfg.chat.kind = ProviderKind::Elevenlabs;
        let err = build_providers(&cfg, &resolver_named(&[])).err().unwrap();
        assert!(
            matches!(
                &err,
                BuildError::UnsupportedCapability { capability: Capability::Chat, kind: ProviderKind::Elevenlabs }
            ),
            "{err:?}"
        );
    }

    /// Native kinds resolve the key *name* the configuration gives them, exactly like every other
    /// provider; the key value never comes from a file.
    #[test]
    fn native_kinds_resolve_their_own_key_name() {
        let mut cfg = ProvidersConfig::default();
        cfg.transcribe.kind = ProviderKind::Elevenlabs;
        cfg.transcribe.key = "elevenlabs".into();
        // Chat and speech stay `openai_compat`, whose default key name is `openrouter`: they are
        // given no key so this test isolates the transcription connector's own resolution rather
        // than tripping over a key the resolver was never asked to hold.
        cfg.chat.key = String::new();
        cfg.speech.key = String::new();
        assert!(build_providers(&cfg, &resolver_named(&[("elevenlabs", "k")])).is_ok());

        let err = build_providers(&cfg, &resolver_named(&[])).err().unwrap();
        assert!(matches!(&err, BuildError::MissingKey { name, .. } if name == "elevenlabs"), "{err:?}");
        assert!(err.to_string().contains("TIPSY_KEY_ELEVENLABS"), "{err}");
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use crate::keys::{KeyResolver, MemoryStore, SecretStore};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A resolver that knows one key name and nothing else.
    fn resolver_with(name: &str) -> KeyResolver {
        let store = MemoryStore::default();
        SecretStore::set(&store, name, "k-test").unwrap();
        KeyResolver::new(Box::new(store), Box::new(|_| None))
    }

    /// A resolver that holds nothing at all: the "no key" state.
    fn empty_resolver() -> KeyResolver {
        KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None))
    }

    /// One OpenAI-compatible row, pointed at a mock.
    fn row(base_url: &str, model: &str) -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::OpenaiCompat,
            base_url: format!("{base_url}/v1"),
            model: model.into(),
            key: "openrouter".into(),
        }
    }

    /// Review Focus 5: a real answer, a latency that was really measured, and a request that is as
    /// small as the fixed `ChatRequest` shape allows — one word, and only the first delta read.
    #[tokio::test]
    async fn chat_probe_reports_the_measured_latency_of_a_real_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(30))
                    .set_body_raw(
                        "data: {\"choices\":[{\"delta\":{\"content\":\"pong\"}}]}\n\ndata: [DONE]\n\n",
                        "text/event-stream",
                    ),
            )
            .mount(&server)
            .await;

        let connector =
            build_one(&row(&server.uri(), "m"), Capability::Chat, &resolver_with("openrouter")).unwrap();
        let probe = connector.probe("es-ES").await;

        assert!(probe.ok, "{probe:?}");
        assert!(probe.latency_ms >= 20, "the latency is the measured one, got {}", probe.latency_ms);
        assert_eq!(probe.message, "4 bytes");

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["messages"], serde_json::json!([{"role": "user", "content": "ping"}]));
        assert_eq!(body["stream"], serde_json::json!(true));
    }

    /// The clock is a real one: the same probe against the same route, held for two different
    /// delays, reports two different times. A constant would pass the `>= 20` assertion above and
    /// still be a lie.
    #[tokio::test]
    async fn the_chat_probe_measures_the_delay_it_was_given() {
        async fn latency_against(delay_ms: u64) -> u64 {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(
                    ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(delay_ms)).set_body_raw(
                        "data: {\"choices\":[{\"delta\":{\"content\":\"pong\"}}]}\n\n",
                        "text/event-stream",
                    ),
                )
                .mount(&server)
                .await;
            let connector =
                build_one(&row(&server.uri(), "m"), Capability::Chat, &resolver_with("openrouter")).unwrap();
            connector.probe("es-ES").await.latency_ms
        }

        let quick = latency_against(5).await;
        let slow = latency_against(120).await;
        assert!(slow >= 100, "a 120 ms route cannot have answered in {slow} ms");
        assert!(slow > quick, "the latency follows the delay it was given: {quick} ms vs {slow} ms");
    }

    /// An error body inside a 200 is a refusal, in the provider's own words — the case OpenRouter
    /// really produces, and the one a status-code test would get wrong.
    #[tokio::test]
    async fn chat_probe_refuses_an_error_the_provider_sent_inside_the_stream() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"error\":{\"message\":\"insufficient credits\"}}\n\n",
                "text/event-stream",
            ))
            .mount(&server)
            .await;

        let connector =
            build_one(&row(&server.uri(), "m"), Capability::Chat, &resolver_with("openrouter")).unwrap();
        let probe = connector.probe("es-ES").await;

        assert!(!probe.ok, "a 200 carrying an error is not an answer: {probe:?}");
        assert!(probe.message.contains("insufficient credits"), "the provider's own words: {}", probe.message);
    }

    /// A 200 whose body says nothing at all is a refusal too: `ok` is `true` only when the provider
    /// really answered, so the probe never dresses up an empty body as a working connector.
    #[tokio::test]
    async fn chat_probe_never_calls_an_empty_answer_a_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"error":{"message":"no credit"}}"#,
            ))
            .mount(&server)
            .await;

        let connector =
            build_one(&row(&server.uri(), "m"), Capability::Chat, &resolver_with("openrouter")).unwrap();
        let probe = connector.probe("es-ES").await;

        assert!(!probe.ok, "{probe:?}");
        assert_eq!(probe.message, "the provider answered with no text");
    }

    /// A one-word synthesis that really produced audio, in samples and at the rate the connector
    /// announced; and the same connector refusing with the provider's own words.
    #[tokio::test]
    async fn speech_probe_reads_the_samples_and_the_providers_words() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x00, 0x40, 0x00, 0xC0]))
            .mount(&server)
            .await;

        let connector = build_one(
            &row(&server.uri(), "google/gemini-3.8-flash-tts"),
            Capability::Speech,
            &resolver_with("openrouter"),
        )
        .unwrap();
        let probe = connector.probe("es-ES").await;
        assert!(probe.ok, "{probe:?}");
        assert_eq!(probe.message, "2 samples at 24000 Hz");

        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["input"], serde_json::json!("hola"), "the probe speaks one word");
        assert!(body.get("voice").is_none(), "the connector's own default voice, not a guess: {body}");

        let refused = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(429).set_body_string(r#"{"error":{"message":"slow down"}}"#))
            .mount(&refused)
            .await;
        let connector = build_one(
            &row(&refused.uri(), "google/gemini-3.8-flash-tts"),
            Capability::Speech,
            &resolver_with("openrouter"),
        )
        .unwrap();
        let probe = connector.probe("es-ES").await;
        assert!(!probe.ok, "{probe:?}");
        assert_eq!(probe.message, "rate limited: slow down");
    }

    /// Half a second of silence is the shortest buffer every connector's endpoint accepts, and an
    /// empty transcript is an *answer*: silence has no words. The request carries the profile's
    /// language, the way every other transcription does.
    #[tokio::test]
    async fn transcribe_probe_answers_a_silent_buffer_in_the_profiles_language() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"text": ""})))
            .mount(&server)
            .await;

        let connector = build_one(
            &row(&server.uri(), "openai/whisper-large-v3"),
            Capability::Transcribe,
            &resolver_with("openrouter"),
        )
        .unwrap();
        let probe = connector.probe("es-ES").await;

        assert!(probe.ok, "an empty transcript is still an answer: {probe:?}");
        assert_eq!(probe.message, "silence, no words");

        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(body.contains("filename=\"audio.wav\"") && body.contains("RIFF"), "{body}");
        assert!(body.contains("name=\"language\"") && body.contains("\r\n\r\nes\r\n"), "{body}");
    }

    /// The connector that cannot do this capability is refused by name, and nothing is called.
    /// `supports` is checked before any key is resolved, exactly as `build_providers` checks it.
    #[tokio::test]
    async fn a_capability_a_connector_does_not_offer_is_refused_without_a_call() {
        let mut config = ProviderConfig { kind: ProviderKind::Gemini, base_url: "http://127.0.0.1:1".into(), model: "gemini-2.5-flash".into(), key: "google".into() };
        let error = build_one(&config, Capability::Transcribe, &empty_resolver()).err().unwrap();
        assert!(
            matches!(
                error,
                BuildError::UnsupportedCapability { capability: Capability::Transcribe, kind: ProviderKind::Gemini }
            ),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("transcription") && message.contains("Gemini"), "{message}");

        // And chat is refused for the connector that has no chat at all.
        config.kind = ProviderKind::Elevenlabs;
        let error = build_one(&config, Capability::Chat, &empty_resolver()).err().unwrap();
        assert!(error.to_string().contains("chat"), "{error}");
    }

    /// Review Focus 5's third state: "no key" is not "the provider refused". It is answered before
    /// any request is made, and the message is `BuildError`'s own, which names the environment
    /// variable that would fix it.
    #[tokio::test]
    async fn a_missing_key_is_refused_before_any_call() {
        let config = row("http://127.0.0.1:1", "m");
        let error = build_one(&config, Capability::Chat, &empty_resolver()).err().unwrap();
        assert!(
            matches!(&error, BuildError::MissingKey { name, env } if name == "openrouter" && env == "TIPSY_KEY_OPENROUTER"),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("openrouter") && message.contains("TIPSY_KEY_OPENROUTER"), "{message}");
    }
}
