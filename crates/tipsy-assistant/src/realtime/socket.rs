//! The duplex socket: connect, configure, read, keep alive, reopen, close.
//!
//! Every wire fact here is quoted from `.superpowers/sdd/plan-2c-research/openai-realtime.md` — the
//! endpoint, the `session.update` shape, the 24 kHz PCM format and the 60-minute session cap. The
//! one thing this file adds is the client-side policy the API leaves to us: an explicit open
//! timeout, a ping, and a reopen *before* the cap, because the note documents no close code, no
//! reconnect semantics and no idle timeout for a Realtime socket.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tipsy_protocol::ErrorCode;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::config::RealtimeConfig;
use crate::keys::{env_var_name, KeyResolver};

/// The note's WebSocket endpoint, before the model query (its "Endpoints" section):
/// `wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1`. A native desktop client uses this
/// socket; WebRTC is for browsers and the ephemeral-secret exchange exists for clients that must not
/// hold the key.
pub const REALTIME_URL: &str = "wss://api.openai.com/v1/realtime";

/// The PCM rate in both directions. The note: `{"type":"audio/pcm","rate":24000}` is the default and
/// the PCM variant supports **only** 24 kHz, so the microphone's 16 kHz is resampled before it goes
/// up and the player is told 24 kHz for what comes down.
pub const PCM_RATE: u32 = 24_000;

/// How long a handshake, or the wait for `session.created`, may take. The note documents no idle
/// behaviour at all, so the client decides: a call that cannot open is a failure, not a hang.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

/// How long the socket may stay quiet before Task 4's loop pings it.
pub const KEEPALIVE: Duration = Duration::from_secs(30);

/// The API's own session cap: "The maximum duration of a Realtime session is 60 minutes", and
/// reconnection is app-managed. One socket is reopened before this elapses.
pub const SESSION_CAP: Duration = Duration::from_secs(60 * 60);

/// The ASR model the note's `session.update` example configures for input transcription. Without a
/// `transcription` block the API sends no `conversation.item.input_audio_transcription.*` events at
/// all (the note's event table), and `Event::Heard` would have nothing to carry.
pub const TRANSCRIBE_MODEL: &str = "gpt-4o-transcribe";

/// What one call takes from the configuration and the active profile, in one place so `connect`'s
/// signature stays readable. Task 4 builds it: `instructions` is the profile's `prompt`, `language`
/// its BCP 47 `language`, and `silence_ms` is `[listening] silence_ms` — the plan's Decision is that
/// turn detection reuses the listening silence instead of a `[realtime] silence_ms`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSpec {
    pub instructions: String,
    pub language: String,
    pub silence_ms: u32,
}

/// Where a call connects and the two deadlines the API leaves to us. The default is the note's
/// endpoint and its 60-minute cap; the tests build their own with a loopback URL, a cap measured in
/// milliseconds and an open timeout measured the same way, because the real ones take an hour and a
/// real key to observe.
#[derive(Debug, Clone)]
pub struct CallOptions {
    pub base_url: String,
    pub cap: Duration,
    pub open_timeout: Duration,
}

impl Default for CallOptions {
    fn default() -> Self {
        Self { base_url: REALTIME_URL.to_string(), cap: SESSION_CAP, open_timeout: OPEN_TIMEOUT }
    }
}

impl CallOptions {
    /// The defaults pointed at another endpoint — a loopback fake server, in the tests.
    pub fn at(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), ..Self::default() }
    }

    /// The same, with a cap a test can reach without waiting an hour.
    pub fn with_cap(mut self, cap: Duration) -> Self {
        self.cap = cap;
        self
    }

    /// The same, with an open timeout a test can reach without waiting fifteen seconds.
    pub fn with_open_timeout(mut self, open_timeout: Duration) -> Self {
        self.open_timeout = open_timeout;
        self
    }
}

/// What one server frame means to the socket: JSON text to hand to `events`, or the close that ends
/// the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketEvent {
    /// A JSON text frame, byte for byte as it arrived: the socket parses nothing, so the mapping —
    /// including the two payloads the research note marks inferred — lives in one place (Task 3).
    Text(String),
    /// The server closed the socket. `code` is `None` when the close frame carried none. The note
    /// does not document which codes the API uses, so neither the socket nor its caller interprets
    /// them: the 60-minute reopen is decided by `session_expired`, never by a code.
    Closed { code: Option<u16>, reason: String },
}

/// What a call can fail with. It mirrors the connectors' `ProviderError` — the same five failure
/// kinds, the same `ErrorCode` mapping — plus the two things only a socket has: a key that is
/// missing before the handshake, and a credential store that refuses to answer.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RealtimeError {
    #[error("no API key named `{name}`: run `tipsy-assistant key set {name}` or set {env}")]
    MissingKey { name: String, env: String },
    #[error("credential store error: {0}")]
    Store(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("timed out opening the realtime socket")]
    Timeout,
    #[error("network error: {0}")]
    Network(String),
    #[error("bad response: {0}")]
    BadResponse(String),
}

impl RealtimeError {
    /// The connectors' mapping, for a handshake the server refused: a rejected upgrade is an HTTP
    /// response, and its body carries the same `{"error":{"message":…}}` shape spec §12 wants shown.
    /// 401/403 → `Auth`, 429 → `RateLimited`, 408/504 → `Timeout`, anything else → `BadResponse`.
    pub fn from_status(status: u16, body: &str) -> Self {
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|value| {
                value.pointer("/error/message").and_then(serde_json::Value::as_str).map(str::to_string)
            })
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| {
                let trimmed: String = body.chars().take(300).collect();
                if trimmed.trim().is_empty() { format!("HTTP {status}") } else { trimmed }
            });
        match status {
            401 | 403 => RealtimeError::Auth(message),
            429 => RealtimeError::RateLimited(message),
            408 | 504 => RealtimeError::Timeout,
            _ => RealtimeError::BadResponse(format!("HTTP {status}: {message}")),
        }
    }

    /// The protocol code the HUD shows (`Event::Error.code`), mapped exactly as the connectors map
    /// theirs, so a realtime failure reads like every other provider failure in the log.
    pub fn code(&self) -> ErrorCode {
        match self {
            RealtimeError::MissingKey { .. } => ErrorCode::NoKey,
            RealtimeError::Store(_) => ErrorCode::Internal,
            RealtimeError::Auth(_) => ErrorCode::Auth,
            RealtimeError::RateLimited(_) => ErrorCode::RateLimited,
            RealtimeError::Timeout => ErrorCode::Timeout,
            RealtimeError::Network(_) => ErrorCode::Network,
            RealtimeError::BadResponse(_) => ErrorCode::BadResponse,
        }
    }
}

impl From<Error> for RealtimeError {
    fn from(error: Error) -> Self {
        match error {
            // A handshake the server refused: 401/403 lands as `Auth`, like every connector's.
            Error::Http(response) => {
                let status = response.status().as_u16();
                let body = response
                    .body()
                    .as_ref()
                    .map(|bytes| String::from_utf8_lossy(bytes).to_string())
                    .unwrap_or_default();
                RealtimeError::from_status(status, &body)
            }
            Error::Io(error) => RealtimeError::Network(error.to_string()),
            Error::Tls(error) => RealtimeError::Network(format!("tls error: {error}")),
            // A peer that vanishes without a close frame: `ProtocolError` is where tungstenite says
            // so, and the note's Uncertainty is exactly this — no code, no reason, no goodbye.
            Error::Protocol(error) => RealtimeError::Network(format!("websocket protocol error: {error}")),
            Error::ConnectionClosed | Error::AlreadyClosed => {
                RealtimeError::Network("the socket closed without a close frame".into())
            }
            other => RealtimeError::Network(other.to_string()),
        }
    }
}

/// One duplex session. It holds no key value at all: the key goes into the handshake and is dropped,
/// and `reopen` resolves it again, so there is nothing here for a log to leak.
///
/// The one thing this type does *not* protect is the wire itself: `tungstenite` logs the request line
/// — `Authorization: Bearer <key>` included — at `trace!` when it opens the socket. What keeps the key
/// out of a log today is therefore the *absence* of a logger: this workspace installs no `log` or
/// `tracing` subscriber, and this plan's one-dependency rule forbids adding one to filter that crate.
/// Whoever installs one must filter `tungstenite` above `trace!` — never enable `trace` for it —
/// before this header can be considered log-safe. The same invariant is in the README's release
/// checklist.
pub struct RealtimeSocket {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    options: CallOptions,
    /// The section as configured: model, voice and the key *name* — no value.
    config: RealtimeConfig,
    spec: CallSpec,
    url: String,
    opened_at: tokio::time::Instant,
    /// Frames that arrived while `configure` was waiting for `session.created`; `recv` hands them
    /// back in order before it reads the socket again, so the wait swallows nothing.
    pending: VecDeque<String>,
}

impl fmt::Debug for RealtimeSocket {
    /// The one thing this type must never print (Global Constraints: the `Authorization` header must
    /// not be logged even at debug level). The struct holds no key value, and this impl prints the
    /// URL, the model, the voice and the parked-frame count — never `spec.instructions`, which is the
    /// user's own text, and never a header.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RealtimeSocket")
            .field("url", &self.url)
            .field("model", &self.config.model)
            .field("voice", &self.config.voice)
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl RealtimeSocket {
    /// Opens a call against the note's endpoint with the production cap and open timeout. It does
    /// **not** read `RealtimeConfig::enabled`: whether a call may start is the client's decision
    /// (Review Focus 3), and a socket that refused to open would put that rule in two places.
    pub async fn connect(
        keys: &KeyResolver,
        config: &RealtimeConfig,
        spec: &CallSpec,
    ) -> Result<Self, RealtimeError> {
        Self::connect_with(CallOptions::default(), keys, config, spec).await
    }

    /// The same, with the endpoint and the two deadlines chosen by the caller.
    pub async fn connect_with(
        options: CallOptions,
        keys: &KeyResolver,
        config: &RealtimeConfig,
        spec: &CallSpec,
    ) -> Result<Self, RealtimeError> {
        let url = realtime_url(&options.base_url, &config.model);
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| RealtimeError::BadResponse(format!("the realtime URL is not valid: {error}")))?;
        // An empty key name means "no key", exactly as `providers::key_for` treats it. Otherwise the
        // resolved value becomes the note's `Authorization: Bearer` header and is then dropped.
        if !config.key.trim().is_empty() {
            let key = match keys.resolve(&config.key) {
                Ok(Some((value, _source))) => value,
                Ok(None) => {
                    return Err(RealtimeError::MissingKey {
                        name: config.key.clone(),
                        env: env_var_name(&config.key),
                    })
                }
                Err(error) => return Err(RealtimeError::Store(error.to_string())),
            };
            let header = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| {
                RealtimeError::BadResponse(
                    "the key for the realtime socket contains a character an HTTP header cannot carry".into(),
                )
            })?;
            request.headers_mut().insert("Authorization", header);
        }
        // GA needs no beta header (the note: the beta interface was removed on 2026-05-12), and the
        // optional `OpenAI-Safety-Identifier` is not sent because this process has no hashed user id.

        let (stream, _response) =
            tokio::time::timeout(options.open_timeout, tokio_tungstenite::connect_async(request))
                .await
                .map_err(|_| RealtimeError::Timeout)??;

        let mut socket = Self {
            stream,
            options,
            config: config.clone(),
            spec: spec.clone(),
            url,
            opened_at: tokio::time::Instant::now(),
            pending: VecDeque::new(),
        };
        socket.configure().await?;
        Ok(socket)
    }

    /// The URL this socket connected to, model query included.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// True once this socket has been open for the cap. The caller reopens rather than waiting for
    /// the server to drop a 60-minute session, because the note documents no close code for the cap
    /// and a client that waited for one would be waiting for something that may never arrive.
    pub fn session_expired(&self) -> bool {
        self.opened_at.elapsed() >= self.options.cap
    }

    /// Drops the old socket and opens a new one, re-resolving the key and re-sending `session.update`
    /// (the server's own `session.created` must be seen first, as on the first connection), so a
    /// reopened call is configured exactly like the one it replaces.
    pub async fn reopen(&mut self, keys: &KeyResolver) -> Result<(), RealtimeError> {
        let fresh = Self::connect_with(self.options.clone(), keys, &self.config, &self.spec).await?;
        *self = fresh;
        Ok(())
    }

    /// Sends one client event as a JSON text frame: `input_audio_buffer.append` (base64 audio),
    /// `conversation.item.truncate`, `response.cancel` and everything else Task 4 and Task 5 send.
    pub async fn send(&mut self, event: &serde_json::Value) -> Result<(), RealtimeError> {
        self.stream.send(Message::text(event.to_string())).await?;
        Ok(())
    }

    /// The keepalive: an empty ping. The note documents no idle close on the API's side either, so
    /// the client proves the socket is alive every `KEEPALIVE` (Task 4's loop ticks it).
    pub async fn ping(&mut self) -> Result<(), RealtimeError> {
        self.stream.send(Message::Ping(Vec::<u8>::new().into())).await?;
        Ok(())
    }

    /// The next thing the server said. Pings, pongs and binary frames are not events — the note says
    /// "All events are JSON text; no binary frames are documented" — so they are skipped here and a
    /// caller's loop only ever sees text or a close.
    pub async fn recv(&mut self) -> Result<SocketEvent, RealtimeError> {
        if let Some(text) = self.pending.pop_front() {
            return Ok(SocketEvent::Text(text));
        }
        loop {
            let message = match self.stream.next().await {
                Some(Ok(message)) => message,
                Some(Err(error)) => return Err(error.into()),
                // The stream ended without a close frame. A *clean* close arrives as a
                // `Message::Close` below, so reaching here means the peer never said goodbye.
                None => {
                    return Err(RealtimeError::Network("the socket closed without a close frame".into()))
                }
            };
            match message {
                Message::Text(text) => return Ok(SocketEvent::Text(text.as_str().to_string())),
                Message::Close(frame) => {
                    return Ok(match frame {
                        Some(frame) => SocketEvent::Closed {
                            code: Some(u16::from(frame.code)),
                            reason: frame.reason.as_str().to_string(),
                        },
                        None => SocketEvent::Closed { code: None, reason: String::new() },
                    })
                }
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            }
        }
    }

    /// Closes the socket with a close frame. A close on a socket the server already closed is not a
    /// failure — the call is over either way — so this is safe to call twice and safe to call after a
    /// `SocketEvent::Closed`.
    ///
    /// The three "already gone" spellings matter because the close handshake is asynchronous: the
    /// server's own close frame (read by `recv`) puts the stream in `Closing`, and tungstenite
    /// answers any further write with `ProtocolError::SendAfterClosing` rather than a silent success.
    /// Swallowing that here is what makes the second `close` of a finished call a no-op instead of an
    /// error the caller would have to special-case.
    pub async fn close(&mut self) -> Result<(), RealtimeError> {
        match self.stream.close(None).await {
            Ok(()) => Ok(()),
            Err(Error::AlreadyClosed) | Err(Error::ConnectionClosed) => Ok(()),
            Err(Error::Protocol(ProtocolError::SendAfterClosing)) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// The note's ordering: the client configures the session "immediately after `session.created`".
    /// Frames that arrive first are parked in `pending`, never discarded, and the whole wait is
    /// bounded by the open timeout, so a server that never opens a session is a `Timeout`.
    async fn configure(&mut self) -> Result<(), RealtimeError> {
        tokio::time::timeout(self.options.open_timeout, async {
            loop {
                match self.recv().await? {
                    SocketEvent::Text(text) if event_type(&text).as_deref() == Some("session.created") => {
                        return Ok(())
                    }
                    SocketEvent::Text(text) => self.pending.push_back(text),
                    SocketEvent::Closed { code, reason } => {
                        return Err(RealtimeError::Network(match code {
                            Some(code) => format!(
                                "the server closed the socket before the session opened (code {code} {reason})"
                            ),
                            None => "the server closed the socket before the session opened".to_string(),
                        }))
                    }
                }
            }
        })
        .await
        .map_err(|_| RealtimeError::Timeout)??;
        self.send(&session_update(&self.config, &self.spec)).await
    }
}

/// `wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1` — the note's endpoint with the model as
/// a query parameter, which is how its "Endpoints" line spells the socket a desktop client uses.
pub fn realtime_url(base_url: &str, model: &str) -> String {
    format!("{}?model={}", base_url.trim_end_matches('/'), model.trim())
}

/// The note's `session.update`: instructions, the voice, PCM16 24 kHz both ways, server-side turn
/// detection with the configured silence, and input transcription (without which the API sends no
/// input-transcript events and `Heard` would have nothing to carry).
///
/// Threshold, prefix padding, noise reduction and every other knob the note documents are left at
/// the API's defaults: nothing in this plan decides them, and an unverified value is worse than an
/// omitted one.
pub fn session_update(config: &RealtimeConfig, spec: &CallSpec) -> serde_json::Value {
    let mut transcription = serde_json::json!({ "model": TRANSCRIBE_MODEL });
    let language = primary_subtag(&spec.language);
    if !language.is_empty() {
        transcription["language"] = serde_json::json!(language);
    }
    serde_json::json!({
        "type": "session.update",
        "session": {
            "type": "realtime",
            "model": config.model.trim(),
            "instructions": spec.instructions.as_str(),
            "output_modalities": ["audio"],
            "audio": {
                "input": {
                    "format": { "type": "audio/pcm", "rate": PCM_RATE },
                    "transcription": transcription,
                    "turn_detection": {
                        "type": "server_vad",
                        "silence_duration_ms": spec.silence_ms,
                        "create_response": true,
                        "interrupt_response": true,
                    },
                },
                "output": {
                    "format": { "type": "audio/pcm", "rate": PCM_RATE },
                    "voice": config.voice.trim(),
                },
            },
        },
    })
}

/// `es-ES` → `es`. The note's transcription block takes a language, and spec §8.3 makes the profile's
/// BCP 47 tag the hint; the API wants the primary subtag, like every other connector.
fn primary_subtag(tag: &str) -> &str {
    tag.split(['-', '_']).next().unwrap_or("").trim()
}

/// The `type` of a raw frame, if it is JSON with a string `type`. The socket reads this one field —
/// and only while opening a session, where the note's ordering needs it; everything else about a
/// frame is the event mapping's business.
fn event_type(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("type").and_then(serde_json::Value::as_str).map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> CallSpec {
        CallSpec {
            instructions: "Habla como un compae andaluz, con cariño y sin prisa.".into(),
            language: "es-ES".into(),
            silence_ms: 1500,
        }
    }

    #[test]
    fn realtime_url_is_the_notes_endpoint_with_the_model_as_a_query() {
        assert_eq!(
            realtime_url(REALTIME_URL, "gpt-realtime-2.1"),
            "wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1"
        );
        assert_eq!(
            realtime_url("ws://127.0.0.1:9/v1/realtime/", " gpt-realtime-2.1-mini "),
            "ws://127.0.0.1:9/v1/realtime?model=gpt-realtime-2.1-mini"
        );
    }

    #[test]
    fn the_session_update_has_the_notes_shape() {
        let config = RealtimeConfig {
            model: "gpt-realtime-2.1-mini".into(),
            voice: "cedar".into(),
            ..RealtimeConfig::default()
        };
        let event = session_update(&config, &spec());

        assert_eq!(event["type"], "session.update");
        let session = &event["session"];
        assert_eq!(session["type"], "realtime");
        assert_eq!(session["model"], "gpt-realtime-2.1-mini");
        assert_eq!(session["instructions"], "Habla como un compae andaluz, con cariño y sin prisa.");
        assert_eq!(session["output_modalities"], serde_json::json!(["audio"]));
        // Both directions are PCM16 at 24 kHz: the note says the PCM variant supports only 24 kHz.
        assert_eq!(
            session["audio"]["input"]["format"],
            serde_json::json!({ "type": "audio/pcm", "rate": 24_000 })
        );
        assert_eq!(
            session["audio"]["output"]["format"],
            serde_json::json!({ "type": "audio/pcm", "rate": 24_000 })
        );
        assert_eq!(session["audio"]["output"]["voice"], "cedar");
        // Server-side turn detection, with `[listening] silence_ms` and not a `[realtime] silence_ms`.
        let turn = &session["audio"]["input"]["turn_detection"];
        assert_eq!(turn["type"], "server_vad");
        assert_eq!(turn["silence_duration_ms"], serde_json::json!(1500));
        assert_eq!(turn["create_response"], serde_json::json!(true));
        assert_eq!(turn["interrupt_response"], serde_json::json!(true));
        // Input transcription, in the profile's own primary subtag: without it there is no `Heard`.
        assert_eq!(session["audio"]["input"]["transcription"]["model"], "gpt-4o-transcribe");
        assert_eq!(session["audio"]["input"]["transcription"]["language"], "es");
        // Nothing this plan does not decide is set: the note's other knobs stay at API defaults.
        assert!(session["audio"]["input"].get("noise_reduction").is_none(), "{session}");
        assert!(turn.get("threshold").is_none(), "{session}");
        assert_eq!(
            session.as_object().unwrap().len(),
            5,
            "type, model, instructions, output_modalities and audio is all we send: {session}"
        );
    }

    #[test]
    fn a_profile_without_a_language_gets_no_language_field() {
        let config = RealtimeConfig::default();
        let spec = CallSpec { language: String::new(), ..spec() };
        let event = session_update(&config, &spec);
        let transcription = &event["session"]["audio"]["input"]["transcription"];
        assert!(transcription.get("language").is_none(), "no language hint is not an empty hint");
        assert_eq!(transcription["model"], "gpt-4o-transcribe");
    }

    #[test]
    fn primary_subtags() {
        assert_eq!(primary_subtag("es-ES"), "es");
        assert_eq!(primary_subtag("en_AU"), "en");
        assert_eq!(primary_subtag("es"), "es");
        assert_eq!(primary_subtag(""), "");
    }

    #[test]
    fn a_rejected_handshake_maps_like_the_connectors() {
        assert_eq!(
            RealtimeError::from_status(401, r#"{"error":{"message":"Incorrect API key provided"}}"#),
            RealtimeError::Auth("Incorrect API key provided".into())
        );
        assert_eq!(RealtimeError::from_status(403, "forbidden"), RealtimeError::Auth("forbidden".into()));
        assert_eq!(RealtimeError::from_status(429, ""), RealtimeError::RateLimited("HTTP 429".into()));
        assert_eq!(RealtimeError::from_status(504, ""), RealtimeError::Timeout);
        assert_eq!(
            RealtimeError::from_status(500, ""),
            RealtimeError::BadResponse("HTTP 500: HTTP 500".into())
        );
    }

    #[test]
    fn error_codes_mirror_the_connectors() {
        assert_eq!(
            RealtimeError::MissingKey { name: "openai".into(), env: "TIPSY_KEY_OPENAI".into() }.code(),
            ErrorCode::NoKey
        );
        assert_eq!(RealtimeError::Store("no store".into()).code(), ErrorCode::Internal);
        assert_eq!(RealtimeError::Auth("x".into()).code(), ErrorCode::Auth);
        assert_eq!(RealtimeError::RateLimited("x".into()).code(), ErrorCode::RateLimited);
        assert_eq!(RealtimeError::Timeout.code(), ErrorCode::Timeout);
        assert_eq!(RealtimeError::Network("x".into()).code(), ErrorCode::Network);
        assert_eq!(RealtimeError::BadResponse("x".into()).code(), ErrorCode::BadResponse);
    }
}
