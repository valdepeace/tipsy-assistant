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

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};

use super::{ChatProvider, ChatRequest, ProviderError, Role, TextStream};

/// Every Gemini request carries the key in this header. `Authorization: Bearer` is the
/// OpenAI-compatibility layer's style, and the legacy `?key=` query string leaks into logs.
const API_KEY_HEADER: &str = "x-goog-api-key";
const INTERACTIONS_PATH: &str = "v1beta/interactions";

/// 30 s for the response headers of a chat stream. The body is read *after* this future resolves,
/// so the timeout wraps `send()`; a request-builder timeout would cut the stream off mid-reply.
const CHAT_TIMEOUT: Duration = Duration::from_secs(30);

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
        let mut finished = false;
        let stream = resp
            .bytes_stream()
            .map(move |chunk| -> Vec<Result<String, ProviderError>> {
                if finished {
                    return Vec::new();
                }
                let bytes = match chunk {
                    Ok(bytes) => bytes,
                    Err(e) => return vec![Err(ProviderError::from(e))],
                };
                let mut out = Vec::new();
                for event in parser.push(&bytes) {
                    match event {
                        GeminiEvent::Delta(text) => out.push(Ok(text)),
                        // HTTP stayed 200: stop reading and surface the provider's message rather
                        // than letting the truncated stream look like a finished reply.
                        GeminiEvent::Error { code, message } => {
                            finished = true;
                            out.push(Err(stream_error(&code, &message)));
                            break;
                        }
                        GeminiEvent::Done => {
                            finished = true;
                            break;
                        }
                    }
                }
                out
            })
            .flat_map(futures::stream::iter);
        Ok(stream.boxed())
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
}
