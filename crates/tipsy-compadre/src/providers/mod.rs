//! Cloud provider abstractions: one trait per capability.

pub mod elevenlabs;
pub mod gemini;
pub mod openai_compat;
pub mod sse;

use std::sync::Arc;

use async_trait::async_trait;
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
    /// Delivery instructions (the persona's `voice_instructions`); ignored by providers without support.
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
    /// `/detail/message` the other providers use, so the first entry's `message` is read too.
    pub fn from_status(status: u16, body: &str) -> Self {
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .or_else(|| v.pointer("/message"))
                    .or_else(|| v.pointer("/detail/message"))
                    .or_else(|| v.pointer("/detail/0/message"))
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

pub fn build_providers(config: &ProvidersConfig, keys: &KeyResolver) -> Result<Providers, BuildError> {
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

    /// ElevenLabs answers 422 with `detail` as an **array** of objects, so the single string path
    /// (`/detail/message`) cannot read it and the first entry's `message` is used instead.
    #[test]
    fn status_mapping_reads_the_first_detail_array_message() {
        assert_eq!(
            ProviderError::from_status(
                422,
                r#"{"detail":[{"type":"missing","loc":["body","text"],"message":"Field required","input":null}]}"#
            ),
            ProviderError::BadResponse("HTTP 422: Field required".into())
        );
    }

    /// The array form is used only when its first entry really carries a non-empty `message`:
    /// anything else falls back to the body, which is still never an empty message.
    #[test]
    fn detail_arrays_without_a_message_keep_the_body_fallback() {
        assert_eq!(
            ProviderError::from_status(422, r#"{"detail":[{"msg":"nope"}]}"#),
            ProviderError::BadResponse(r#"HTTP 422: {"detail":[{"msg":"nope"}]}"#.into())
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
