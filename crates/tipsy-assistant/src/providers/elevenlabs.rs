//! Native ElevenLabs connector: speech synthesis over HTTP (not the WebSocket) and Scribe
//! transcription (Task 4).
//!
//! Every endpoint, header, field name and format string here is quoted from
//! `.superpowers/sdd/plan-2-research/elevenlabs-api.md`; the note's own caveats are repeated at
//! the lines they affect.

use std::time::Duration;

use async_trait::async_trait;

use super::{ProviderError, SpeechProvider, SpeechRequest, TranscribeProvider};
use crate::audio::pcm::{Pcm, encode_wav_16k, pcm_s16le_to_pcm};

/// 30 s per sentence of speech (Global Constraints).
const SPEECH_TIMEOUT: Duration = Duration::from_secs(30);

/// The note's raw-PCM choice, sent as the `output_format` query parameter: "raw PCM, no
/// container". Nothing is decoded from a container here.
const OUTPUT_FORMAT: &str = "pcm_24000";

/// The sample rate that arrives with `OUTPUT_FORMAT`.
const OUTPUT_SAMPLE_RATE: u32 = 24_000;

/// The HTTP core both capabilities share: base URL, model id and the `xi-api-key` header.
///
/// The connector sends the model id the configuration gives it and never substitutes one of its
/// own (the plan's decision); the research note's recommendation for synthesis is
/// `eleven_flash_v2_5`.
pub(crate) struct ElevenLabsHttp {
    client: reqwest::Client,
    base_url: String,
    model: String,
    key: Option<String>,
}

impl ElevenLabsHttp {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self {
        let base_url: String = base_url.into();
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.into(),
            key,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path)
    }

    /// The note: HTTP auth is the `xi-api-key` header on every request.
    pub fn authed(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.key {
            Some(k) => rb.header("xi-api-key", k),
            None => rb,
        }
    }

    /// A non-2xx response becomes a typed `ProviderError` carrying the API's own body. The note's
    /// error shape is `422` with `detail[]`; when that cannot be read as a message,
    /// `ProviderError::from_status` carries the body itself, so nothing is ever swallowed.
    pub async fn check(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(ProviderError::from_status(status.as_u16(), &body))
    }
}

/// One entry of the account's voice list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AccountVoice {
    pub id: String,
    pub name: String,
}

/// How long a **failed** `GET /v1/voices` is remembered before it is retried.
///
/// The list is a per-session lookup with no way for the user to refresh it, so a failure should not
/// be retried on every sentence: up to four sentences are synthesised in parallel and each would
/// re-list the voices behind the same mutex, turning one flaky call into four extra requests and
/// surfacing the brain's 30-second timeout instead of the list's own error. The window is short
/// relative to speech timeouts (30 s), so a transient failure still recovers on the next turn.
const VOICES_FAILURE_TTL: Duration = Duration::from_secs(10);

/// The connector's single `/v1/voices` lookup: the voice list when it was fetched, or the failure
/// that was seen `at` so concurrent and following sentences share it.
#[derive(Default)]
struct VoiceCache {
    voices: Option<Vec<AccountVoice>>,
    failure: Option<(std::time::Instant, ProviderError)>,
}

pub struct ElevenLabsSpeech {
    http: ElevenLabsHttp,
    /// The account's voice list, resolved once per connector (Review Focus 5).
    voices: tokio::sync::Mutex<VoiceCache>,
}

impl ElevenLabsSpeech {
    /// `model` is the synthesis model id from the configuration (`eleven_flash_v2_5` is the
    /// note's realtime recommendation); `key` is the resolved key **value**, never its name.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self {
        Self { http: ElevenLabsHttp::new(base_url, model, key), voices: tokio::sync::Mutex::new(VoiceCache::default()) }
    }

    /// The account's voices from `GET /v1/voices`, fetched once and kept for the connector's
    /// lifetime. The lock is held across the request on purpose: concurrent first sentences wait
    /// for the one fetch instead of each issuing their own.
    ///
    /// A failure is cached for [`VOICES_FAILURE_TTL`] for the same reason, so the sentences of one
    /// turn share a single failed attempt rather than each paying for their own.
    ///
    /// The note records the endpoint and its `xi-api-key` header but not the response body, so
    /// `voices[].voice_id` and `voices[].name` are the field names the tests pin; a body without a
    /// readable `voices` array is `BadResponse`, never a panic.
    async fn account_voices(&self) -> Result<Vec<AccountVoice>, ProviderError> {
        let mut cache = self.voices.lock().await;
        if let Some(voices) = cache.voices.as_ref() {
            return Ok(voices.clone());
        }
        let recent_failure = cache.failure.as_ref().filter(|(at, _)| at.elapsed() < VOICES_FAILURE_TTL);
        if let Some((_, failure)) = recent_failure {
            return Err(failure.clone());
        }
        let fetched = self.fetch_voices().await;
        match fetched {
            Ok(voices) => {
                cache.voices = Some(voices.clone());
                cache.failure = None;
                Ok(voices)
            }
            Err(failure) => {
                cache.failure = Some((std::time::Instant::now(), failure.clone()));
                Err(failure)
            }
        }
    }

    /// The one `GET /v1/voices` request. Kept separate so `account_voices` can cache either
    /// outcome without holding the lock across two error paths.
    async fn fetch_voices(&self) -> Result<Vec<AccountVoice>, ProviderError> {
        let resp =
            self.http.authed(self.http.client.get(self.http.url("v1/voices"))).timeout(SPEECH_TIMEOUT).send().await?;
        let value: serde_json::Value = ElevenLabsHttp::check(resp).await?.json().await?;
        value
            .get("voices")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|v| {
                        Some(AccountVoice {
                            id: v.get("voice_id")?.as_str()?.to_string(),
                            name: v.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .ok_or_else(|| ProviderError::BadResponse(format!("no voices in the ElevenLabs voice list: {value}")))
    }

    /// Turns the profile's `voices` entry into a voice id.
    ///
    /// The entry is written the way a human wrote it: El Compae ships `george`, a stock voice's
    /// *name*, and putting a name in the path would 404 every sentence. So the value is matched
    /// against `voice_id` first and then against `name`, case-insensitively; no entry at all, or
    /// an entry that matches nothing, uses the account's first voice rather than failing.
    async fn resolve_voice(&self, requested: Option<&str>) -> Result<String, ProviderError> {
        let voices = self.account_voices().await?;
        if let Some(wanted) = requested.map(str::trim).filter(|w| !w.is_empty()) {
            if let Some(voice) = voices.iter().find(|v| v.id == wanted) {
                return Ok(voice.id.clone());
            }
            if let Some(voice) = voices.iter().find(|v| v.name.eq_ignore_ascii_case(wanted)) {
                return Ok(voice.id.clone());
            }
        }
        voices
            .first()
            .map(|v| v.id.clone())
            .ok_or_else(|| ProviderError::BadResponse("the ElevenLabs account lists no voices".into()))
    }
}

/// Turns a 200 body into PCM, rejecting what is provably not raw `pcm_24000`.
///
/// The note marks the PCM bit depth, signedness and channel count for TTS **output**
/// unconfirmed — it documents them only for STT *input* — so this decodes the way
/// `pcm_s16le_to_pcm` does (signed 16-bit little-endian, mono, 24 kHz) and refuses a payload that
/// announces itself as JSON (an object, an array, or one of those behind a UTF-8 BOM), a body the
/// response calls JSON in its `content-type`, a WAV/RIFF container and a body too short to hold one
/// sample. Raw PCM cannot announce itself, so an MP3 or a wrong sample rate inside a well-formed
/// body is not detectable here; playing it as 24 kHz would be worse than an error.
fn audio_bytes_to_pcm(bytes: &[u8], content_type: Option<&str>) -> Result<Pcm, ProviderError> {
    // A JSON content-type is the API's own error shape served with 200, and it is checked before
    // the bytes: a body that happens to look like PCM must still not be played as noise.
    if content_type.is_some_and(|t| t.to_ascii_lowercase().contains("json")) {
        let preview: String = String::from_utf8_lossy(bytes).chars().take(200).collect();
        return Err(ProviderError::BadResponse(format!(
            "expected raw pcm_24000 audio but the response declares a JSON content-type \
             ({content_type:?}): {preview}"
        )));
    }
    // A BOM-prefixed JSON body would otherwise start with the BOM's bytes rather than the brace, so
    // the BOM is stepped over before looking for the first meaningful byte.
    let payload = bytes.strip_prefix(b"\xEF\xBB\xBF".as_slice()).unwrap_or(bytes);
    if matches!(payload.iter().copied().find(|b| !b.is_ascii_whitespace()), Some(b'{') | Some(b'[')) {
        // `chars().take(..)` keeps the preview on character boundaries (Review Focus 2).
        let preview: String = String::from_utf8_lossy(bytes).chars().take(200).collect();
        return Err(ProviderError::BadResponse(format!("expected raw pcm_24000 audio but got a JSON body: {preview}")));
    }
    if bytes.starts_with(b"RIFF") {
        return Err(ProviderError::BadResponse(
            "expected raw pcm_24000 audio but got a WAV container (RIFF header)".into(),
        ));
    }
    if bytes.len() < 2 {
        return Err(ProviderError::BadResponse(format!(
            "expected raw pcm_24000 audio but got {} byte(s)",
            bytes.len()
        )));
    }
    Ok(pcm_s16le_to_pcm(bytes, OUTPUT_SAMPLE_RATE))
}

#[async_trait]
impl SpeechProvider for ElevenLabsSpeech {
    async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError> {
        let voice_id = self.resolve_voice(req.voice.as_deref()).await?;
        let url = format!("{}?output_format={OUTPUT_FORMAT}", self.http.url(&format!("v1/text-to-speech/{voice_id}")));
        // `req.instructions` is deliberately not sent: the note records no general
        // instructions/style field, and `voice_settings.style` is V2+-only and raises latency.
        let body = serde_json::json!({ "text": req.text, "model_id": self.http.model });
        let resp = self.http.authed(self.http.client.post(url)).timeout(SPEECH_TIMEOUT).json(&body).send().await?;
        let resp = ElevenLabsHttp::check(resp).await?;
        // The header has to be read before `bytes()` consumes the response.
        let content_type =
            resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
        let bytes = resp.bytes().await?;
        audio_bytes_to_pcm(&bytes, content_type.as_deref())
    }
}

/// 60 s per transcription (Global Constraints).
const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(60);

pub struct ElevenLabsTranscribe {
    http: ElevenLabsHttp,
}

impl ElevenLabsTranscribe {
    /// `model` is the transcription model id from the configuration; the note names `scribe_v2`.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self {
        Self { http: ElevenLabsHttp::new(base_url, model, key) }
    }
}

/// `es-ES` → `es`: the API takes ISO-639-1 or ISO-639-3 (the note) while our profiles declare
/// BCP 47 tags (the plan's decision).
fn primary_subtag(language: &str) -> String {
    language.split(['-', '_']).next().unwrap_or("").trim().to_ascii_lowercase()
}

#[async_trait]
impl TranscribeProvider for ElevenLabsTranscribe {
    async fn transcribe(&self, pcm: Pcm, language: &str) -> Result<String, ProviderError> {
        let primary = primary_subtag(language);
        // A self-describing WAV rather than raw PCM plus `file_format` (the plan's decision).
        let file = reqwest::multipart::Part::bytes(encode_wav_16k(&pcm))
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| ProviderError::BadResponse(e.to_string()))?;
        let mut form = reqwest::multipart::Form::new().text("model_id", self.http.model.clone()).part("file", file);
        if !primary.is_empty() {
            form = form.text("language_code", primary);
        }
        let resp = self
            .http
            .authed(self.http.client.post(self.http.url("v1/speech-to-text")))
            .timeout(TRANSCRIBE_TIMEOUT)
            .multipart(form)
            .send()
            .await?;
        let value: serde_json::Value = ElevenLabsHttp::check(resp).await?.json().await?;
        value
            .get("text")
            .and_then(|t| t.as_str())
            .map(|t| t.trim().to_string())
            .ok_or_else(|| ProviderError::BadResponse(format!("no text in transcription response: {value}")))
    }
}
