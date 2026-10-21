//! Native Gemini connector over the Interactions API.
//!
//! `POST {base_url}/v1beta/interactions` serves chat (`"stream": true` → SSE) and speech. The host
//! is never hardcoded: `base_url` is whatever the configuration says, and a value that already
//! carries a version segment (`…/v1beta`) keeps it instead of gaining a second one.
//!
//! The native API takes **native** model ids — `gemini-3.8-flash` for chat, `gemini-3.8-flash-tts`
//! for speech — and **not** the OpenRouter slugs for the same models
//! (`google/gemini-3.8-flash-tts`). Switching `kind` in `config.toml` leaves the old model string
//! in place until it is changed by hand, so a slug reaching this connector is a configuration
//! mistake and not something to rewrite here; the configuration screen fixes the trap in Plan 2B.
//! This connector sends the id it is given, verbatim.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use super::{ChatProvider, ChatRequest, ProviderError, Role, SpeechProvider, SpeechRequest, TextStream};
use crate::audio::pcm::{Pcm, decode_audio, pcm_s16le_to_pcm};

/// Every Gemini request carries the key in this header. `Authorization: Bearer` is the
/// OpenAI-compatibility layer's style, and the legacy `?key=` query string leaks into logs.
const API_KEY_HEADER: &str = "x-goog-api-key";
const INTERACTIONS_PATH: &str = "v1beta/interactions";

/// 30 s for the response headers of a chat stream. The body is read *after* this future resolves,
/// so the timeout wraps `send()`; a request-builder timeout would cut the stream off mid-reply.
const CHAT_TIMEOUT: Duration = Duration::from_secs(30);

/// 30 s for a whole sentence of speech. Speech is a unary request, so this one goes on the request
/// builder rather than around `send()`.
const SPEECH_TIMEOUT: Duration = Duration::from_secs(30);
/// The voice sent when the profile's `voices` table has no `gemini` entry. `Kore` is the voice the
/// API documentation's own request example uses and the only prebuilt name the research note
/// confirms; the full list of 30 names is unconfirmed, and the note never shows a request without
/// `speech_config`, so this default is sent rather than omitting the field.
const DEFAULT_VOICE: &str = "Kore";

/// The parts of the connector chat and speech share: the endpoint, the API-key header, the
/// interaction envelope (`store: false` on every request) and the status → error mapping.
struct GeminiHttp {
    client: reqwest::Client,
    base_url: String,
    key: Option<String>,
}

impl GeminiHttp {
    fn new(base_url: impl Into<String>, key: Option<String>) -> Self {
        let base_url: String = base_url.into();
        Self { client: reqwest::Client::new(), base_url: base_url.trim_end_matches('/').to_string(), key }
    }

    /// The interactions endpoint. The reference page's path is `v1beta/interactions`, and the
    /// note's uncertainty list leaves the canonical version string open (the overview calls the
    /// API GA while the reference says beta, and migration examples show `v1beta2`), so a
    /// `base_url` that already carries a version segment — what the documentation writes and what
    /// a user may paste into `config.toml` — keeps its own segment instead of gaining a second.
    fn endpoint(&self) -> String {
        if Self::is_versioned(&self.base_url) {
            format!("{}/interactions", self.base_url)
        } else {
            format!("{}/{INTERACTIONS_PATH}", self.base_url)
        }
    }

    /// Does the last path segment of `base_url` look like a version prefix (`v1`, `v1beta`,
    /// `v1beta2`)? The dot check keeps a host named `v1.example.com` from being mistaken for one.
    fn is_versioned(base_url: &str) -> bool {
        let segment = base_url.rsplit('/').next().unwrap_or("");
        match segment.strip_prefix('v') {
            Some(rest) if !rest.is_empty() && !rest.contains('.') => {
                rest.chars().next().is_some_and(|c| c.is_ascii_digit())
            }
            _ => false,
        }
    }

    /// The interaction envelope every Gemini request shares. `store: false` keeps the request
    /// stateless: the API retains interactions by default (55 days on paid, 1 day on free) and the
    /// user's questions must not be left on someone else's server.
    fn body(model: &str, input: Value, stream: bool) -> Value {
        json!({ "model": model, "input": input, "stream": stream, "store": false })
    }

    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        let rb = self.client.post(self.endpoint()).json(body);
        match &self.key {
            Some(key) => rb.header(API_KEY_HEADER, key),
            None => rb,
        }
    }

    /// A non-2xx response becomes the typed error carrying the provider's own message
    /// (`{"error":{"code":"…","message":"…"}}`); `from_status` falls back to the body and finally
    /// to `HTTP <status>`, so the message is never empty.
    async fn check(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(ProviderError::from_status(status.as_u16(), &body))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GeminiEvent {
    /// A piece of reply text: `event_type == "step.delta"` with `delta.type == "text"`.
    Delta(String),
    /// `event_type: "error"` — the same `code`/`message` envelope, but on a 200 stream.
    Error { code: String, message: String },
    /// `data: [DONE]`
    Done,
}

/// Incremental SSE reader for the Interactions API. Only complete lines are parsed, so the tail
/// stays pending: the transport can hand over a chunk that ends inside a multi-byte character or
/// in the middle of a JSON token.
#[derive(Default)]
struct GeminiSse {
    pending: Vec<u8>,
}

impl GeminiSse {
    fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, bytes: &[u8]) -> Vec<GeminiEvent> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(pos) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            // `event:` lines only repeat the type the JSON already carries, so they are skipped.
            let Some(data) = line.trim_end_matches(['\r', '\n']).strip_prefix("data:") else { continue };
            let data = data.trim_start();
            if data == "[DONE]" {
                events.push(GeminiEvent::Done);
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(data) else { continue };
            if let Some(event) = classify(&value) {
                events.push(event);
            }
        }
        events
    }
}

/// Maps one SSE payload to an event, skipping every `event_type`/`delta.type` this connector does
/// not know: the API adds new ones over time, and `thought` steps and audio deltas are not chat.
fn classify(value: &Value) -> Option<GeminiEvent> {
    match value.get("event_type").and_then(Value::as_str)? {
        "step.delta" => {
            let delta = value.get("delta")?;
            if delta.get("type").and_then(Value::as_str) != Some("text") {
                return None;
            }
            let text = delta.get("text").and_then(Value::as_str)?;
            (!text.is_empty()).then(|| GeminiEvent::Delta(text.to_string()))
        }
        "error" => {
            let error = value.get("error")?;
            let code = error.get("code").and_then(Value::as_str).unwrap_or("").to_string();
            let message = error.get("message").and_then(Value::as_str).unwrap_or("").to_string();
            Some(GeminiEvent::Error { code, message })
        }
        _ => None,
    }
}

/// An in-stream `error` event becomes the same variant its HTTP status would have produced, and
/// never carries an empty message.
fn stream_error(code: &str, message: &str) -> ProviderError {
    let message = match (message.trim(), code.trim()) {
        ("", "") => "stream error".to_string(),
        ("", code) => code.to_string(),
        (message, _) => message.to_string(),
    };
    match code.trim() {
        "authentication" | "unauthenticated" | "permission_denied" => ProviderError::Auth(message),
        "rate_limit_exceeded" | "resource_exhausted" | "quota_exceeded" | "too_many_requests" => {
            ProviderError::RateLimited(message)
        }
        "deadline_exceeded" | "gateway_timeout" => ProviderError::Timeout,
        _ => ProviderError::BadResponse(message),
    }
}

/// Chat over the Interactions API: one request, one SSE stream of `step.delta` text.
pub struct GeminiChat {
    http: GeminiHttp,
    model: String,
}

impl GeminiChat {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self {
        Self { http: GeminiHttp::new(base_url, key), model: model.into() }
    }

    /// `system_instruction` is the interaction-scoped top-level **string** the API takes (not a
    /// `Content` object) and is omitted when the request has no system message; the remaining
    /// messages travel as inline history. A single user turn uses the documented string form.
    fn request_body(&self, req: ChatRequest) -> Value {
        let mut system: Vec<String> = Vec::new();
        let mut blocks: Vec<Value> = Vec::new();
        for message in req.messages {
            match message.role {
                Role::System => system.push(message.content),
                Role::User => blocks.push(user_block(&message.content)),
                Role::Assistant => blocks.push(model_block(&message.content)),
            }
        }
        let input = match blocks.as_slice() {
            [only] if only["type"] == "user_input" => only["content"][0]["text"].clone(),
            _ => Value::Array(blocks),
        };
        let mut body = GeminiHttp::body(&self.model, input, true);
        if !system.is_empty() {
            body["system_instruction"] = json!(system.join("\n\n"));
        }
        body
    }
}

fn user_block(text: &str) -> Value {
    json!({ "type": "user_input", "content": [{ "type": "text", "text": text }] })
}

fn model_block(text: &str) -> Value {
    json!({ "type": "model_output", "content": [{ "type": "text", "text": text }] })
}

#[async_trait]
impl ChatProvider for GeminiChat {
    async fn stream(&self, req: ChatRequest) -> Result<TextStream, ProviderError> {
        let body = self.request_body(req);
        // The timeout covers the response headers only; the body streams after this resolves.
        let send = self.http.post(&body).send();
        let resp = tokio::time::timeout(CHAT_TIMEOUT, send).await.map_err(|_| ProviderError::Timeout)??;
        let resp = GeminiHttp::check(resp).await?;

        let mut parser = GeminiSse::new();
        // Shared with the tail below: `flat_map` cannot see the end of the body, so completion is
        // recorded here and checked once the body has ended.
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let stream = resp
            .bytes_stream()
            .map(move |chunk| -> Vec<Result<String, ProviderError>> {
                if flag.load(Ordering::Relaxed) {
                    return Vec::new();
                }
                let bytes = match chunk {
                    Ok(bytes) => bytes,
                    // The transport failed mid-reply: stop and record it, so the tail below does not
                    // report a second, invented failure for the same truncation.
                    Err(e) => {
                        flag.store(true, Ordering::Relaxed);
                        return vec![Err(ProviderError::from(e))];
                    }
                };
                let mut out = Vec::new();
                for event in parser.push(&bytes) {
                    match event {
                        GeminiEvent::Delta(text) => out.push(Ok(text)),
                        // HTTP stayed 200: stop reading and surface the provider's message rather
                        // than letting the truncated stream look like a finished reply.
                        GeminiEvent::Error { code, message } => {
                            flag.store(true, Ordering::Relaxed);
                            out.push(Err(stream_error(&code, &message)));
                            break;
                        }
                        GeminiEvent::Done => {
                            flag.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
                out
            })
            .flat_map(futures::stream::iter);
        // Over HTTP/2 an early `END_STREAM` is a clean close, so "the body ended" says nothing by
        // itself: only the completion event does. A body that ends without one is a truncated
        // reply, and it must reach the brain as a failure rather than as a stream that finished
        // normally, which would speak the half-reply and record it in history.
        let tail = futures::stream::iter(
            std::iter::once_with(move || {
                if finished.load(Ordering::Relaxed) {
                    Vec::new()
                } else {
                    vec![Err(ProviderError::BadResponse("the reply stream ended before the completion event".into()))]
                }
            })
            .flatten(),
        );
        Ok(stream.chain(tail).boxed())
    }
}

/// The voice name the API takes (`Kore`, `Puck`, …). The profile's `voices["gemini"]` entry is
/// used verbatim: it is a *name*, not an id to look up. A Voice Library id or a designed
/// `voice_…` would also be accepted in this field, so a user's own entry keeps working.
fn voice_name(voice: Option<&str>) -> &str {
    match voice.map(str::trim) {
        Some(name) if !name.is_empty() => name,
        _ => DEFAULT_VOICE,
    }
}

/// The one-line `style`: the first sentence of the profile's `voice_instructions`. The API wants a
/// short style and Google's guidance says a long style paragraph is the commonest cause of voice
/// drift, so the voice-designer brief is condensed here instead of in the profile cards.
fn first_sentence(text: &str) -> &str {
    let trimmed = text.trim();
    trimmed
        .char_indices()
        .find(|(_, c)| matches!(c, '.' | '!' | '?' | '…'))
        .map(|(i, c)| &trimmed[..i + c.len_utf8()])
        .unwrap_or(trimmed)
}

/// Standard base64 (`A–Z a–z 0–9 + /`, `=` padding), written here because this plan adds no
/// dependency for it. Padding and line breaks are ignored; any other byte is an error.
///
/// `pub(crate)` because Plan 2C's realtime audio deltas are base64 too and that plan adds no base64
/// dependency either: one decoder, one set of tests, two call sites.
pub(crate) fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    fn sextet(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        if matches!(byte, b'=' | b'\n' | b'\r' | b' ' | b'\t') {
            continue;
        }
        let value = sextet(byte).ok_or_else(|| format!("invalid base64 byte {byte} in the audio data"))?;
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1; // keep only the bits that are still pending
        }
    }
    Ok(out)
}

/// The audio block of the response: the note's jq filter, in Rust — the **last** `model_output`
/// step's `audio` content.
fn audio_block(value: &Value) -> Result<&Value, ProviderError> {
    value
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|step| step.get("type").and_then(Value::as_str) == Some("model_output"))
        .filter_map(|step| step.get("content").and_then(Value::as_array))
        .flatten()
        .rfind(|content| content.get("type").and_then(Value::as_str) == Some("audio"))
        .ok_or_else(|| ProviderError::BadResponse(format!("no audio block in the synthesis response: {value}")))
}

/// Turns the audio block into PCM. The two containers the note documents are never mixed up: the
/// unary default `audio/wav` is self-describing and carries its own rate, while `audio/l16` is
/// headerless raw 16-bit signed LE that must be decoded at the rate the block declares.
fn audio_block_to_pcm(block: &Value, bytes: Vec<u8>) -> Result<Pcm, ProviderError> {
    let mime_type = block.get("mime_type").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    let declared = block.get("sample_rate").and_then(Value::as_u64).map(|rate| rate as u32);

    let pcm = match mime_type.as_str() {
        "audio/l16" => pcm_s16le_to_pcm(&bytes, declared.unwrap_or(24_000)),
        "audio/wav" | "audio/x-wav" | "audio/wave" => {
            decode_audio(bytes).map_err(|e| ProviderError::BadResponse(format!("undecodable audio/wav: {e}")))?
        }
        other => {
            return Err(ProviderError::BadResponse(format!(
                "unsupported audio container `{other}` in the synthesis response: the request asked \
                 for `{{\"type\":\"audio\"}}`, whose unary default is `audio/wav`"
            )));
        }
    };

    if let Some(declared) = declared
        && declared != pcm.sample_rate
    {
        return Err(ProviderError::BadResponse(format!(
            "the synthesis response declares {declared} Hz but the container is {} Hz",
            pcm.sample_rate
        )));
    }
    if pcm.samples.is_empty() {
        return Err(ProviderError::BadResponse("the synthesis response contained no samples".into()));
    }
    Ok(pcm)
}

/// Speech over the same endpoint. Unary (`"stream": false`): one request per sentence, one response
/// carrying the whole utterance, and the sample rate travels with the bytes.
pub struct GeminiSpeech {
    http: GeminiHttp,
    model: String,
}

impl GeminiSpeech {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self {
        Self { http: GeminiHttp::new(base_url, key), model: model.into() }
    }

    /// The TTS shape the note records: the verbatim transcript with a `speech_metadata` annotation
    /// carrying the one-line style, audio as the response format, and the voice name in
    /// `generation_config`. No `system_instruction`: whether a TTS interaction accepts one is
    /// unconfirmed.
    fn request_body(&self, req: &SpeechRequest) -> Value {
        // The tone floor is already prepended to req.instructions by turn_profile.
        // This is the operational meaning of "the tone floor prevails": it's baked into
        // the brief that the profile declares.
        let style = first_sentence(&req.instructions);
        let mut text = json!({ "type": "text", "text": req.text });
        if !style.is_empty() {
            text["annotations"] = json!([{ "type": "speech_metadata", "style": style }]);
        }
        let input = json!([{ "type": "user_input", "content": [text] }]);
        let mut body = GeminiHttp::body(&self.model, input, false);
        body["response_format"] = json!({ "type": "audio" });
        body["generation_config"] = json!({ "speech_config": [{ "voice": voice_name(req.voice.as_deref()) }] });
        body
    }
}

#[async_trait]
impl SpeechProvider for GeminiSpeech {
    async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError> {
        let body = self.request_body(&req);
        let resp = self.http.post(&body).timeout(SPEECH_TIMEOUT).send().await?;
        let value: Value = GeminiHttp::check(resp)
            .await?
            .json()
            .await
            .map_err(|e| ProviderError::BadResponse(format!("the synthesis response was not JSON: {e}")))?;

        let block = audio_block(&value)?;
        let data = block.get("data").and_then(Value::as_str).unwrap_or("");
        let bytes = decode_base64(data).map_err(ProviderError::BadResponse)?;
        audio_block_to_pcm(block, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recorded stream: a `model_output` step with two text deltas, an audio delta and an unknown
    /// event type (both ignored), then the terminator.
    const RECORDED: &str = concat!(
        "event: step.start\n",
        "data: {\"index\":1,\"step\":{\"type\":\"model_output\"},\"event_type\":\"step.start\"}\n\n",
        "event: step.delta\n",
        "data: {\"index\":1,\"delta\":{\"text\":\"1, 2, 3, 4, 5, 6, \",\"type\":\"text\"},\"event_type\":\"step.delta\"}\n\n",
        "event: step.delta\n",
        "data: {\"index\":1,\"delta\":{\"text\":\"¡Ozú, ñandú!\",\"type\":\"text\"},\"event_type\":\"step.delta\"}\n\n",
        "event: step.stop\n",
        "data: {\"index\":1,\"event_type\":\"step.stop\"}\n\n",
        "event: step.delta\n",
        "data: {\"index\":2,\"delta\":{\"data\":\"AAAA\",\"mime_type\":\"audio/l16\",\"type\":\"audio\"},\"event_type\":\"step.delta\"}\n\n",
        "event: interaction.created\n",
        "data: {\"interaction\":{\"id\":\"v1_abc\"},\"event_type\":\"interaction.created\"}\n\n",
        "event: done\n",
        "data: [DONE]\n",
    );

    fn expected() -> Vec<GeminiEvent> {
        vec![
            GeminiEvent::Delta("1, 2, 3, 4, 5, 6, ".into()),
            GeminiEvent::Delta("¡Ozú, ñandú!".into()),
            GeminiEvent::Done,
        ]
    }

    #[test]
    fn parses_step_deltas_and_ignores_everything_else() {
        let mut parser = GeminiSse::new();
        assert_eq!(parser.push(RECORDED.as_bytes()), expected());
    }

    /// Review Focus 2: a chunk may end inside a multi-byte character (`ñ` is two bytes) or in the
    /// middle of a JSON token. Neither may panic, lose the delta or corrupt the UTF-8.
    #[test]
    fn splits_inside_a_multibyte_character_and_inside_a_json_token() {
        let bytes = RECORDED.as_bytes();

        // Split inside the `ñ` of the second delta: only its lead byte goes in the first chunk.
        let lead = bytes.iter().position(|b| *b == 0xC3).expect("the fixture contains a `ñ`");
        let mut parser = GeminiSse::new();
        let mut events = parser.push(&bytes[..=lead]);
        events.extend(parser.push(&bytes[lead + 1..]));
        assert_eq!(events, expected());

        // Split inside the `event_type` key of the first data line, cutting the JSON token in half.
        let token = RECORDED.find("\"event_type\":\"step.delta\"").expect("the fixture has the key") + 6;
        let mut parser = GeminiSse::new();
        let mut events = parser.push(&bytes[..token]);
        events.extend(parser.push(&bytes[token..]));
        assert_eq!(events, expected());
    }

    /// Both real mid-stream fragments from the note, plus the never-empty-message rule.
    #[test]
    fn in_stream_errors_keep_the_provider_message_or_fall_back_to_the_code() {
        assert_eq!(
            stream_error("not_found", "Failed to get completed interaction: Result not found."),
            ProviderError::BadResponse("Failed to get completed interaction: Result not found.".into())
        );
        assert_eq!(
            stream_error("gateway_timeout", "Deadline expired before operation could complete."),
            ProviderError::Timeout
        );
        assert_eq!(stream_error("rate_limit_exceeded", ""), ProviderError::RateLimited("rate_limit_exceeded".into()));
        assert_eq!(stream_error("permission_denied", "denied"), ProviderError::Auth("denied".into()));
        assert_eq!(stream_error("", ""), ProviderError::BadResponse("stream error".into()));
    }

    const BRIEF: &str = concat!(
        "Voz masculina adulta, medio-grave y cálida, limpia y con apenas un leve toque rasposo, nada ronca. ",
        "Acento andaluz natural y bien marcado: seseo, eses aspiradas, finales relajados y musicalidad andaluza."
    );
    const STYLE: &str =
        "Voz masculina adulta, medio-grave y cálida, limpia y con apenas un leve toque rasposo, nada ronca.";

    #[test]
    fn decode_base64_decodes_standard_base64() {
        assert_eq!(decode_base64("AAECAw==").unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(decode_base64("T3rDug==").unwrap(), "Ozú".as_bytes()); // multi-byte UTF-8 bytes
        assert_eq!(decode_base64("").unwrap(), Vec::<u8>::new());
        assert_eq!(decode_base64("AA\nEC\r\nAw==").unwrap(), vec![0, 1, 2, 3]); // line breaks tolerated
        assert!(decode_base64("AAEC*").is_err());
    }

    #[test]
    fn condenses_the_voice_brief_to_its_first_sentence() {
        assert_eq!(first_sentence(BRIEF), STYLE);
        assert_eq!(first_sentence("  Ozú, quillo.  "), "Ozú, quillo.");
        assert_eq!(first_sentence("una sola frase sin punto"), "una sola frase sin punto");
        assert_eq!(first_sentence(""), "");
        assert_eq!(first_sentence("¿Qué pasa? Y ya está."), "¿Qué pasa?");
    }

    /// The same condenser over all nine built-in briefs, structurally: the style sent to the API
    /// must exist, must not end in whitespace (it is concatenated into a JSON annotation) and must
    /// be something the API would read as one short style rather than the whole paragraph. No
    /// profile's wording is asserted, so the copy stays the user's to reword.
    ///
    /// This is what catches a future brief whose first sentence lost its full stop (the style would
    /// swallow the whole paragraph) and one that is blank (the style would be empty, sending an
    /// annotation with no style at all).
    #[test]
    fn every_builtin_brief_condenses_to_a_single_non_empty_style() {
        for profile in crate::config::profiles::builtin_profiles() {
            let brief = profile.voice_instructions.as_str();
            assert!(!brief.trim().is_empty(), "{} has no voice brief", profile.id);
            let style = first_sentence(brief);
            assert!(!style.is_empty(), "{} condenses to an empty style", profile.id);
            assert_eq!(style, style.trim_end(), "{}'s style has trailing whitespace", profile.id);
            assert!(
                style.len() < brief.trim().len(),
                "{} condenses to the whole brief ({} characters): its first sentence has no terminator",
                profile.id,
                style.len()
            );
        }
    }

    /// The falsifiable half of the tone floor, and the reason this test lives in this module.
    ///
    /// `request_body` condenses the delivery brief with `first_sentence` — the same function this test
    /// calls. A test that asked whether the floor appears *somewhere* in the brief would pass for an
    /// implementation that appends it, and an appended clause never reaches the Gemini API at all:
    /// `first_sentence(BRIEF + " " + floor)` is `first_sentence(BRIEF)`. What is asserted here is the
    /// equality the API depends on — the style *is* the floor — and it is asserted through the real
    /// condenser and the real composition function, not through copies of either.
    ///
    /// Both halves of the requirement's own example are here: a profile written from nothing, and a
    /// profile whose brief contradicts the floor.
    #[test]
    fn every_brief_the_speech_provider_receives_opens_with_the_tone_floor() {
        for profile in crate::config::profiles::builtin_profiles() {
            let config = crate::config::Config { active_profile: profile.id.clone(), ..crate::config::Config::default() };
            let composed = crate::turn_profile(&config);
            assert_eq!(
                super::first_sentence(&composed.voice_instructions),
                tipsy_ui::tone::TONE_FLOOR_ES,
                "{}: the API must read the floor as the style",
                profile.id
            );
        }

        // The profile the requirement is about: one the user wrote from nothing, whose brief is empty,
        // blank, or says the opposite of the floor.
        for brief in ["", "   ", "Habla como un presentador de telediario, formal y sobrio."] {
            let mut config = crate::config::Config::default();
            config.active_profile = "inventado".into();
            config.profiles.push(crate::config::Profile {
                id: "inventado".into(),
                language: "es-ES".into(),
                voice_instructions: brief.into(),
                ..crate::config::Profile::default()
            });
            let composed = crate::turn_profile(&config);
            assert_eq!(
                super::first_sentence(&composed.voice_instructions),
                tipsy_ui::tone::TONE_FLOOR_ES,
                "a brief of {brief:?} must still open with the floor"
            );
        }
    }
}
