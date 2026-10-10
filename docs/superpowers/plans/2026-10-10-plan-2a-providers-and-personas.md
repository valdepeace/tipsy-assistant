# Plan 2A — Providers and personas Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the native `gemini` (chat + speech) and `elevenlabs` (speech + transcription) connectors and ship all nine built-in personas, so that either connector and any persona can be selected in `config.toml` and used end to end in the terminal.

**Architecture:** Both connectors live beside `openai_compat` in `crates/tipsy-compadre/src/providers/`, implement the same traits (`ChatProvider`, `SpeechProvider`, `TranscribeProvider`) that Task 7 of Plan 1 defined, and are returned by the existing `build_providers` for the `ProviderKind` variants that today answer `BuildError::Unsupported`. The nine personas are data inside the binary (`crates/tipsy-compadre/src/config/personas.rs`); a persona with the same `id` in the user's `config.toml` keeps overriding the built-in, exactly as Plan 1's Task 5 established.

**Tech Stack:** the workspace as Plan 1 left it — Rust 1.96.1 (edition 2024), tokio 1.53, reqwest 0.13 (`json`, `stream`, `multipart`), async-trait 0.1, serde/serde_json, `wiremock` 0.6 for connector tests. Gemini needs no new dependency; the ElevenLabs task names whatever its research note requires.

**Spec:** `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md` — §7 (the connector table, the traits and `ProviderError`), §8.1–8.3 (keys, personas and the four prompt rules), §13 (connector tests with `wiremock`).

## Research inputs

Three research notes were produced while writing this plan, and every endpoint, field name, model id and voice id quoted in the tasks below comes from them:

- `.superpowers/sdd/plan-2-research/gemini-native-api.md`
- `.superpowers/sdd/plan-2-research/elevenlabs-api.md`

Where a note marks something as unconfirmed, the task quotes the note's caveat and pins the behaviour with a test rather than guessing.

## Decisions this plan makes, and why

- **Gemini is built against the Interactions API**, not `generateContent`/`streamGenerateContent`: the research note found Interactions generally available since mid-2026 and recommended, and it is the only surface with documented speech. A single endpoint, `POST /v1beta/interactions`, serves chat (`"stream": true` → SSE) and speech, so the connector is interactions-shaped and Tasks 1 and 2 share its request builder, its SSE delta filter and its error mapping.
- **`store: false` on every Gemini request.** The API retains interactions by default (55 days on paid, 1 day on free); a desktop assistant must not leave the user's questions on someone else's server.
- **Gemini's delivery instruction is the first sentence of the persona's `voice_instructions`.** The API takes a short `speech_metadata.style`, and Google's own guidance says a long style paragraph causes voice drift; our persona cards carry a voice-designer brief whose first sentence is exactly the short description wanted ("Voz masculina adulta, medio-grave y cálida…"). The connector condenses it instead of the cards changing format, and Task 2 pins it with a test.
- **ElevenLabs synthesis uses the HTTP endpoint, not the WebSocket.** The socket is real and generally available, but it costs a WebSocket client dependency, a base64 frame protocol and its own test surface, for a first-audio gain the brain's per-sentence parallelism already covers (up to four sentences in flight, spec §5.1). The HTTP endpoint honours `output_format=pcm_24000`, which arrives as raw PCM the existing `audio::pcm` helpers already handle. **This is a deviation from spec §7** and is recorded below.
- **ElevenLabs' synthesis model is `eleven_flash_v2_5`**, the documentation's explicit realtime recommendation, and the right trade for speech read aloud one sentence at a time.
- **A persona's `voices` entry is a human-written name or id, resolved by the connector.** El Compae ships `elevenlabs = "george"` and `gemini = "Charon"` — stock voice *names*, not ids — because that is what a person can read and write in `config.toml`. ElevenLabs needs a `voice_id` in the path, so its connector resolves the value against the account's voices (`GET /v1/voices`, once, cached): match `voice_id` first, then `name` case-insensitively, and fall back to the account's first voice when the persona has no entry or nothing matches. Gemini takes the name as it is. Both connectors therefore still speak when the table is empty — that is Review Focus 5 and each task owns its test.
- **Gemini's fallback voice is `Kore`**, the voice in the API documentation's own example, cited from the research note: the note could not confirm that `speech_config` is optional, so the connector sends a documented voice rather than omitting the field. The full current voice list is an acknowledged gap in the note, which the task says out loud.
- **Scribe's language hint is the primary subtag** of the persona's BCP 47 language (`es-ES` → `es`), because the API takes ISO-639-1 or ISO-639-3 and our personas declare BCP 47.
- **Transcription uploads a WAV built with `encode_wav_16k`**, not raw PCM with `file_format=pcm_s16le_16`: a self-describing container is one less way for a provider to misread the audio. The research note's `file_format` option is recorded in the task for whoever wants to shave the upload.
- **Native model ids are the configuration's business, not the connector's.** Gemini's native speech model is `gemini-3.8-flash-tts` (research-confirmed) while the OpenRouter slug for the same thing is `google/gemini-3.8-flash-tts`; the connector sends the id it is given and its doc comment says so. No live model id is baked into a test — connector tests use `wiremock`.
- **`BuildError::Unsupported` becomes a typed capability error.** A connector that exists is not "unsupported"; what can be missing is a *capability* (Gemini has no transcription, ElevenLabs has no chat). Task 5 replaces the variant with one naming both the capability and the kind, and updates Plan 1's `native_kinds_are_not_available_yet` test accordingly. Nothing else in the workspace pattern-matches that error — only its `Display` is used, in `client.rs` and `main.rs` — and the task confirms that before deleting the variant.
- **Switching `kind` does not switch the model, and this plan says so instead of guessing.** With `kind = "elevenlabs"` the configuration still carries the model the connector before it used, so a user must also set `model = "eleven_flash_v2_5"` and `transcribe.model = "scribe_v2"`. The connectors' doc comments state the model ids they expect, and the configuration screen of Plan 2B will apply a connector's default model when its kind changes. In 2A the trap is documented rather than papered over with a heuristic about slashes in slugs.
- **The `GET /v1/voices` body shape is pinned by fixtures, not by documentation.** The research note records the endpoint and its header but not the body, so Task 3's `wiremock` fixtures fix `voices[].voice_id` and `voices[].name`, and the task says the shape is documented-by-test: if the live body differs, that one test is where it shows.
- **No new dependency in this plan.** `earshot` arrives with Plan 2B's VAD task.

## Deviations from the spec

- **§7 ElevenLabs speech**: the spec says "streaming synthesis (WebSocket if available, else HTTP)". This plan takes the HTTP branch and records why above. The WebSocket stays as a documented future option if measurements ever ask for it.
- **§8.3 `voice_instructions`**: the field stays a long voice-designer brief, and the Gemini connector condenses it to the one-line style the API wants. The persona cards' format is unchanged.

## Fidelity of the task bodies

Every task gives the **complete tests** (verbatim — they are the contract), every type and signature verbatim, the exact wire shapes (paths, headers, JSON fields, status mappings) quoted from the research notes, and the decisive code fragments. Where the remaining implementation is mechanical — an HTTP call wrapped around shapes already given — the step states the behaviour instead of repeating a whole module. Plan 1 wrote every line of every task; this plan trades some of that verbosity to stay affordable to author, and the research notes travel with it so an implementer never has to recall an endpoint.

## Model routing

This harness cannot select a model per subagent — Plan 1's ledger records the ruling (`progress.md`, "the plan's `[haiku]`/`[sonnet]` model tags cannot be honored"). Every task therefore runs on the session model, and the `[tier]` tags in the task headers are **hints about a task's size**, not model selections.

## Global Constraints

- Toolchain pinned by `rust-toolchain.toml` to `1.96.1`; every crate uses `edition = "2024"`.
- All code, identifiers, comments, docs and commit messages are in **English**. Persona texts are the exception: each is written in the language and variety its persona declares.
- `tipsy-protocol` and `tipsy-ui` must NOT depend on `tokio`, `cpal`, `rodio`, `keyring`, `sysinfo`, `reqwest` or `crossterm`; the new connectors live in `tipsy-compadre` only.
- API keys never appear in `config.toml`, logs, events or test fixtures. Config stores only a key **name**; the resolution order stays environment variable `TIPSY_KEY_<NAME>` first, then the OS credential store (service `tipsy-compadre`, account `<name>`).
- Timeouts: 30 s per chat request, 30 s per sentence of speech, 60 s per transcription. Exactly one retry with backoff on `RateLimited`, and it lives in the brain (`chat_with_retry`), never in a connector.
- Mic audio is mono 16 kHz; speech handed to clients is mono `i16` PCM at the provider's own sample rate, and the sample rate travels with it.
- Personas: regional humour is affectionate, never mocking; never a slur; never the voice of a real person; never a quote from a film, book or other work. Every persona prompt states the four rules of spec §8.3 **in the persona's own language**.
- No new dependency without a line in this plan naming it and the reason.
- Every commit message ends with the trailer line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A provider error body must reach the user.** Every new request path (Gemini's stream, ElevenLabs' synthesis and Scribe) must map a non-2xx response to the typed `ProviderError` variants carrying the provider's own message — never an empty string, never a panic. Task 1, Task 3 and Task 4 each own the test for their path.
2. **A response split at arbitrary byte boundaries must not panic or corrupt UTF-8.** Gemini's stream and ElevenLabs' audio both arrive in pieces; the parser must survive a chunk boundary inside a multi-byte character and inside a JSON token. Task 1 and Task 3 own those tests.
3. **Audio in an unexpected container or sample rate is reported, not played.** A synthesis response whose bytes are not decodable, or whose format is not the one requested, must be `BadResponse` with a readable message — a wrong sample rate played as if it were right is worse than an error. Task 2 and Task 3 own those tests.
4. **Adding eight built-ins must not break the override semantics.** A user persona with the same `id` still wins, every other built-in still appears, and a config written before this plan (with a single persona) still loads. Task 8 owns those tests.
5. **A persona with no entry for the active connector must still speak.** `voices` is a recommendation table: when it has no entry for the connector in use, the connector falls back to a documented default — Gemini sends `Kore`, ElevenLabs takes the account's first voice — rather than failing or inventing a value. Task 2, Task 3 and Task 8 cover it.

## File Structure

```
crates/tipsy-compadre/src/providers/
├── mod.rs            MODIFY: build_providers returns the new connectors; per-capability validation
├── openai_compat.rs  unchanged
├── sse.rs            unchanged
├── gemini.rs         NEW: native Gemini — streamed chat and speech with delivery instructions
└── elevenlabs.rs     NEW: native ElevenLabs — speech synthesis and Scribe transcription
crates/tipsy-compadre/src/config/
└── personas.rs       MODIFY: nine built-in personas, every prompt carrying the four §8.3 rules
crates/tipsy-compadre/tests/
├── gemini.rs         NEW: wiremock tests for the Gemini connector
└── elevenlabs.rs     NEW: wiremock tests for the ElevenLabs connector
```

## Tasks

| # | Task `[tier]` | Deliverable | Files | Depends on |
|---|---|---|---|---|
| 1 | Gemini chat streaming `[mid]` | `GeminiChat` implements `ChatProvider` over the native streaming endpoint | `providers/gemini.rs`, `tests/gemini.rs` | — |
| 2 | Gemini speech `[mid]` | `GeminiSpeech` implements `SpeechProvider`, honouring `voice_instructions` and the persona's `voices` entry | `providers/gemini.rs`, `tests/gemini.rs` | 1 (same file) |
| 3 | ElevenLabs speech `[mid]` | `ElevenLabsSpeech` implements `SpeechProvider` with the streaming decision the research note fixes | `providers/elevenlabs.rs`, `tests/elevenlabs.rs` | — |
| 4 | ElevenLabs transcription `[mid]` | `ElevenLabsTranscribe` implements `TranscribeProvider` (Scribe) with the language hint | `providers/elevenlabs.rs`, `tests/elevenlabs.rs` | 3 (same file) |
| 5 | Wiring and capability validation `[small]` | `build_providers` returns the new connectors; a capability a connector does not offer is a typed `BuildError` | `providers/mod.rs` | 1–4 |
| 6 | Spanish personas `[mid]` | `meigo`, `chulapo`, `senyor` — data, prompts, voices, tests | `config/personas.rs` | — |
| 7 | English personas `[mid]` | `good_ol_boy`, `hustler`, `mate`, `bloke`, `polite_one` — data, prompts, voices, tests | `config/personas.rs` | — |
| 8 | Persona rules and override semantics `[small]` | The four §8.3 rules in every prompt, the voices tables reconciled with the connector research, and the override/regression tests | `config/personas.rs`, `config/store.rs` tests | 6, 7 |

---

### Task 1: Gemini chat streaming `[mid]`

**Files:**
- Create: `crates/tipsy-compadre/src/providers/gemini.rs`
- Modify: `crates/tipsy-compadre/src/providers/mod.rs` (declare `pub mod gemini;`; Task 5 changes the `build_providers` body, not this line)
- Test: `crates/tipsy-compadre/tests/gemini.rs`

**Interfaces:**
- Consumes (exact, from the real code):
  - `ChatProvider::stream(&self, req: ChatRequest) -> Result<TextStream, ProviderError>`; `pub type TextStream = futures::stream::BoxStream<'static, Result<String, ProviderError>>`
  - `pub struct ChatRequest { pub messages: Vec<ChatMessage> }`, `pub struct ChatMessage { pub role: Role, pub content: String }`, `pub enum Role { System, User, Assistant }`
  - `ProviderError::{Auth(String), RateLimited(String), Timeout, Network(String), BadResponse(String), Cancelled}`, `ProviderError::from_status(status: u16, body: &str) -> Self` (reads `/error/message`, else the first 300 chars of the body, else `HTTP {status}` — never an empty message), `impl From<reqwest::Error> for ProviderError`
  - `ProviderConfig { kind: ProviderKind, base_url: String, model: String, key: String }` — `key` is a **name**; `key_for(cfg, keys)` in `providers/mod.rs` resolves it to `Option<String>`
- Consumes (from the research note `.superpowers/sdd/plan-2-research/gemini-native-api.md`, quoted, never recalled):
  - `POST {host}/v1beta/interactions` for chat **and** speech; chat sets `"stream": true` → SSE
  - Auth header `x-goog-api-key: <key>`. `Authorization: Bearer` is the OpenAI-compat layer's style and must not be sent; the query-string form (`?key=`) is legacy and leaks into logs
  - Request body: `model`, `system_instruction` (top-level **string**, not a `Content` object), `input` (string, or inline history array), `stream: true`, `store: false`
  - Framing: `event: <event_type>` line plus a `data:` line whose JSON repeats `event_type`; ends with `event: done` / `data: [DONE]`. Text is `delta.text` **only** when `delta.type == "text"`, inside a `step.delta`; `thought` steps and `audio` deltas are skipped; unrecognised `event_type`/`delta.type` values are skipped
  - Mid-stream failures arrive as `event_type: "error"` with `{"error":{"code":...,"message":...}}` **on HTTP 200** — the status never changes, so the reader must stop and surface it
  - Errors otherwise: HTTP status plus `{"error":{"code":"snake_case","message":"..."}}`
- Produces (Task 2 consumes these names verbatim; Task 5 consumes only `GeminiChat::new`):

```rust
const API_KEY_HEADER: &str = "x-goog-api-key";
const INTERACTIONS_PATH: &str = "v1beta/interactions";
const CHAT_TIMEOUT: Duration = Duration::from_secs(30);   // wraps send(), never the request builder

struct GeminiHttp { client: reqwest::Client, base_url: String, key: Option<String> }
impl GeminiHttp {
    fn new(base_url: impl Into<String>, key: Option<String>) -> Self;
    fn endpoint(&self) -> String;                                    // base_url + /v1beta/interactions
    fn is_versioned(base_url: &str) -> bool;                         // base_url already ends in a version segment
    fn body(model: &str, input: serde_json::Value, stream: bool) -> serde_json::Value;
    fn post(&self, body: &serde_json::Value) -> reqwest::RequestBuilder;
    async fn check(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GeminiEvent { Delta(String), Error { code: String, message: String }, Done }
struct GeminiSse { pending: Vec<u8> }
impl GeminiSse { fn new() -> Self; fn push(&mut self, bytes: &[u8]) -> Vec<GeminiEvent>; }
fn classify(value: &serde_json::Value) -> Option<GeminiEvent>;
fn stream_error(code: &str, message: &str) -> ProviderError;

pub struct GeminiChat { http: GeminiHttp, model: String }
impl GeminiChat { pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self }
impl ChatProvider for GeminiChat
```

- `GeminiHttp`, `GeminiEvent`, `GeminiSse`, `classify`, `stream_error` and the consts are **private to `gemini.rs`**: Task 2 is the only consumer, and nothing outside should reach past `GeminiChat`/`GeminiSpeech`. Every item in the block above is used inside this task, so the file adds no `dead_code` warning for `cargo clippy -D warnings` before Task 2 lands; Task 2 adds `SPEECH_TIMEOUT` and `DEFAULT_VOICE` next to `CHAT_TIMEOUT` when it needs them.
- The request path is `base_url` + `/v1beta/interactions`, with the host never hardcoded. A `base_url` that already carries a version segment (`…/v1beta`) keeps it and gains only `/interactions`, so both spellings reach `…/v1beta/interactions`; Task 5's consuming test asserts exactly that suffix, and the test below pins the already-versioned form. This is also how the note's version-prefix uncertainty (its "Uncertainty" section: the reference page says beta, the overview says GA, migration examples show `v1beta2`) is contained: one predicate, one place to change.
- **Native model ids only.** The connector sends `model` verbatim; the native API takes `gemini-3.8-flash` / `gemini-3.8-flash-tts`, not the OpenRouter slugs (`google/gemini-3.8-flash-tts`). Switching `kind` in `config.toml` keeps the old model string until it is edited, which is a trap the configuration screen fixes in Plan 2B — this connector never rewrites or strips a model id, and `config/model.rs` is not touched by this plan.
- `store: false` is set by `GeminiHttp::body` for **every** request, so Task 2 inherits the decision by construction. The API retains interactions by default (55 days paid / 1 day free) and this is a desktop assistant.
- No retry anywhere in this file. The note records that per-model limits are unpublished, that a rolling 10-minute spend cap returns 429 and that no `Retry-After` header is documented; the single retry with backoff lives in the brain (`chat_with_retry`). Test 4 below pins it by asserting exactly one request reaches the server on a 429.
- The 30 s timeouts are constants, not test assertions: a test that waits 30 s to prove a timeout would slow CI for no added information.
- Model ids are the configuration's business. `gemini-3.8-flash` (research-confirmed) appears only in this task's fixtures, never in the connector.

- [ ] **Step 1: Write the failing integration test** — `crates/tipsy-compadre/tests/gemini.rs`:

```rust
//! Connector tests for the native Gemini API (spec §13: `wiremock` with recorded responses).

use futures::StreamExt;
use serde_json::{json, Value};
use tipsy_compadre::providers::gemini::GeminiChat;
use tipsy_compadre::providers::{ChatMessage, ChatProvider, ChatRequest, ProviderError};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A recorded Interactions stream: a `model_output` step with two text deltas, an audio delta and
/// an unknown event type (both ignored by the reader), then the terminator. Every `data:` line is
/// preceded by the `event:` line that repeats its type, exactly as the API frames it.
const STREAM: &str = concat!(
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
    "event: interaction.completed\n",
    "data: {\"interaction\":{\"id\":\"v1_abc\",\"status\":\"completed\"},\"event_type\":\"interaction.completed\"}\n\n",
    "event: done\n",
    "data: [DONE]\n",
);

fn simple() -> ChatRequest {
    ChatRequest { messages: vec![ChatMessage::system("sys"), ChatMessage::user("hola")] }
}

async fn items(provider: &GeminiChat, req: ChatRequest) -> Vec<Result<String, ProviderError>> {
    let mut stream = provider.stream(req).await.expect("the request must be accepted");
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        out.push(item);
    }
    out
}

async fn collect(provider: &GeminiChat, req: ChatRequest) -> Result<Vec<String>, ProviderError> {
    let mut out = Vec::new();
    for item in items(provider, req).await {
        out.push(item?);
    }
    Ok(out)
}

async fn stream_error(provider: &GeminiChat, req: ChatRequest) -> ProviderError {
    match provider.stream(req).await {
        Ok(_) => panic!("expected the request to fail"),
        Err(e) => e,
    }
}

#[tokio::test]
async fn chat_streams_step_deltas_and_sends_the_documented_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .and(header("x-goog-api-key", "k123"))
        .and(header("content-type", "application/json"))
        .and(body_partial_json(json!({
            "model": "gemini-3.8-flash",
            "system_instruction": "sys",
            "input": "hola",
            "stream": true,
            "store": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(STREAM, "text/event-stream"))
        .mount(&server)
        .await;
    // A trailing slash is what a user pastes into `config.toml`; the endpoint must not double it.
    let p = GeminiChat::new(format!("{}/", server.uri()), "gemini-3.8-flash", Some("k123".into()));
    assert_eq!(
        collect(&p, simple()).await.unwrap(),
        vec!["1, 2, 3, 4, 5, 6, ".to_string(), "¡Ozú, ñandú!".to_string()]
    );
    let sent = &server.received_requests().await.unwrap()[0];
    // Native Gemini authenticates with the API-key header only; `Bearer` is the OpenAI-compat
    // layer's style and would be wrong here.
    assert!(sent.headers.get("authorization").is_none(), "unexpected authorization header");
}

#[tokio::test]
async fn chat_sends_the_full_history_as_inline_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(STREAM, "text/event-stream"))
        .mount(&server)
        .await;
    let p = GeminiChat::new(server.uri(), "gemini-3.8-flash", None);
    let req = ChatRequest {
        messages: vec![
            ChatMessage::system("sys"),
            ChatMessage::user("pregunta"),
            ChatMessage::assistant("respuesta"),
            ChatMessage::user("y ahora"),
        ],
    };
    collect(&p, req).await.unwrap();

    let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["system_instruction"], "sys");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    // The note prints the `user_input` block verbatim and says history travels inline as
    // `[{type:"user_input",...},{type:"model_output",...}, ...]`, but it prints no `model_output`
    // block. This pins the symmetric shape; if the API rejects it, this fixture is what changes.
    assert_eq!(
        body["input"],
        json!([
            {"type": "user_input", "content": [{"type": "text", "text": "pregunta"}]},
            {"type": "model_output", "content": [{"type": "text", "text": "respuesta"}]},
            {"type": "user_input", "content": [{"type": "text", "text": "y ahora"}]},
        ])
    );
}

#[tokio::test]
async fn chat_omits_system_instruction_when_there_is_none() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(STREAM, "text/event-stream"))
        .mount(&server)
        .await;
    let p = GeminiChat::new(server.uri(), "m", None);
    let req = ChatRequest { messages: vec![ChatMessage::user("hola")] };
    collect(&p, req).await.unwrap();

    let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    // A single user turn uses the documented string form, and an absent system prompt is an
    // absent field, never an empty string.
    assert_eq!(body["input"], "hola");
    assert!(body.get("system_instruction").is_none(), "no system prompt must send no field: {body}");
}

/// Review Focus 1: 401, 429 and 5xx with a JSON body keep the provider's own message (never
/// empty). The note records no message text for these three statuses, so the fixture strings are
/// arbitrary; only the envelope `{"error":{"code","message"}}` is quoted from it.
#[tokio::test]
async fn chat_maps_auth_rate_limit_and_server_errors_with_the_provider_message() {
    let cases = [
        (
            401u16,
            "authentication",
            "API key not valid. Please pass a valid API key.",
            ProviderError::Auth("API key not valid. Please pass a valid API key.".to_string()),
        ),
        (
            429u16,
            "rate_limit_exceeded",
            "Quota exceeded for quota metric 'generate requests'.",
            ProviderError::RateLimited("Quota exceeded for quota metric 'generate requests'.".to_string()),
        ),
        (
            500u16,
            "api_error",
            "Internal error encountered.",
            ProviderError::BadResponse("HTTP 500: Internal error encountered.".to_string()),
        ),
    ];
    for (status, code, message, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/interactions"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string(json!({"error": {"code": code, "message": message}}).to_string()),
            )
            .mount(&server)
            .await;
        let p = GeminiChat::new(server.uri(), "gemini-3.8-flash", Some("nope".into()));
        let err = stream_error(&p, simple()).await;
        assert_eq!(err, expected, "status {status}");

        let carried = match &err {
            ProviderError::Auth(m) | ProviderError::RateLimited(m) | ProviderError::BadResponse(m) => m,
            other => panic!("status {status} produced {other:?}"),
        };
        assert!(!carried.trim().is_empty(), "status {status} carried an empty message");

        // The note publishes no `Retry-After` and no per-model limits; the connector must hand the
        // 429 straight up, because the single retry with backoff lives in the brain.
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "status {status} was retried");
    }
}

/// The consuming task (Task 5) asserts the request path ends with `/v1beta/interactions`, and a
/// user may paste `base_url` either with or without the version segment. Both forms must reach the
/// same path: never `/v1beta/v1beta/interactions`.
#[tokio::test]
async fn chat_accepts_a_base_url_that_already_carries_the_version() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(STREAM, "text/event-stream"))
        .mount(&server)
        .await;
    // If the path doubled, wiremock matches nothing and answers 404, so `collect` fails here.
    let p = GeminiChat::new(format!("{}/v1beta", server.uri()), "gemini-3.8-flash", None);
    assert_eq!(collect(&p, simple()).await.unwrap().len(), 2);
}

/// The mid-stream failure the note records: HTTP stays 200 and the stream simply carries an
/// `error` event, so a truncated stream must not be mistaken for a finished reply.
#[tokio::test]
async fn chat_surfaces_a_mid_stream_error_that_arrived_with_http_200() {
    let server = MockServer::start().await;
    let body = concat!(
        "event: step.delta\n",
        "data: {\"index\":1,\"delta\":{\"text\":\"Ozú\",\"type\":\"text\"},\"event_type\":\"step.delta\"}\n\n",
        "event: error\n",
        "data: {\"event_type\":\"error\",\"error\":{\"code\":\"not_found\",\"message\":\"Failed to get completed interaction: Result not found.\"}}\n\n",
        "event: done\n",
        "data: [DONE]\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;
    let p = GeminiChat::new(server.uri(), "gemini-3.8-flash", None);

    let out = items(&p, simple()).await;
    assert_eq!(
        out,
        vec![
            Ok("Ozú".to_string()),
            Err(ProviderError::BadResponse(
                "Failed to get completed interaction: Result not found.".to_string()
            )),
        ]
    );
    assert!(out.last().unwrap().is_err(), "the error must end the reply, not complete it");
}
```

Run: `cargo test -p tipsy-compadre --test gemini`
Expected: compile error — `error[E0432]: unresolved import tipsy_compadre::providers::gemini` (the module does not exist yet).

- [ ] **Step 2: Write the shared pieces and the SSE reader** — `crates/tipsy-compadre/src/providers/gemini.rs`:

```rust
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
```

The unit tests in that block cannot run yet: the module is not declared until Step 3, which is what makes `cargo test -p tipsy-compadre providers::gemini` reachable. `async_trait`, `ChatProvider`, `ChatRequest`, `Role` and `TextStream` are also unused until Step 3 adds their users — that is the only warning this intermediate state produces, and it is gone by the time Step 4 runs.

- [ ] **Step 3: Add `GeminiChat` and declare the module** — append to `crates/tipsy-compadre/src/providers/gemini.rs`:

```rust
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
```

In `crates/tipsy-compadre/src/providers/mod.rs`, add the module to the existing list (keep it alphabetical):

```rust
pub mod gemini;
pub mod openai_compat;
pub mod sse;
```

Do **not** touch `build_providers` in this task: returning the connector for `ProviderKind::Gemini` is Task 5, which also fills the `Unsupported` arm's tests.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p tipsy-compadre providers::gemini`
Expected: `test result: ok. 3 passed`

Run: `cargo test -p tipsy-compadre --test gemini`
Expected: `test result: ok. 6 passed`

If `chat_streams_step_deltas_and_sends_the_documented_request` fails on the mock not matching, the request body is wrong, not the matcher: the fixture asserts exactly the note's `model`/`system_instruction`/`input`/`stream`/`store` shape.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/providers/gemini.rs crates/tipsy-compadre/src/providers/mod.rs crates/tipsy-compadre/tests/gemini.rs
git commit -m "feat(providers): stream chat from the native gemini interactions api

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Gemini speech `[mid]`

**Files:**
- Modify: `crates/tipsy-compadre/src/providers/gemini.rs` (Task 1's file: adds `GeminiSpeech`, the base64 decoder, the style condenser and the audio decoder)
- Test: `crates/tipsy-compadre/tests/gemini.rs` (same file Task 1 created)

**Interfaces:**
- Consumes (Task 1's names, verbatim, private to the module — this task is their only consumer):
  - `GeminiHttp::{new, endpoint, body(model, input, stream), post(&self, body) -> reqwest::RequestBuilder, check(resp)}`
  - consts `API_KEY_HEADER`, `INTERACTIONS_PATH`, `CHAT_TIMEOUT` (untouched), and `GeminiHttp::body`'s `stream` flag, which this task passes as `false`
  - `GeminiEvent`, `GeminiSse`, `classify`, `stream_error` — unchanged; the unary speech path never reaches them
  - `GeminiHttp::body` already applies `store: false`, so the speech request inherits that decision
  - The `#[cfg(test)] mod tests` at the bottom of `gemini.rs`, to which this task adds two tests
- Consumes (exact, from the real code):
  - `SpeechProvider::synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError>`; the trait's default `max_parallel() -> usize` returns `4`, which this task leaves untouched
  - `pub struct SpeechRequest { pub text: String, pub instructions: String, pub voice: Option<String> }` — `instructions` is the persona's `voice_instructions`, `voice` is already resolved by the caller (`session.rs`: `persona.voice_for(&config.providers.speech)`)
  - `pub struct Pcm { pub samples: Vec<f32>, pub sample_rate: u32 }`
  - `crate::audio::pcm::{decode_audio(bytes: Vec<u8>) -> Result<Pcm, String>, pcm_s16le_to_pcm(bytes: &[u8], sample_rate: u32) -> Pcm}`
- Consumes (from the research note, quoted):
  - Same endpoint, `"stream": false` (unary) → `{"id":"v1_…","status":"completed","steps":[{"type":"model_output","content":[{"type":"audio","mime_type":"audio/wav","data":"<base64>","channels":1,"sample_rate":24000}]}]}`; the note's own jq filter takes the **last** `model_output` → `content` → `audio` → `data`
  - Request: `input: [{"type":"user_input","content":[{"type":"text","text":"<verbatim transcript>","annotations":[{"type":"speech_metadata","style":"…"}]}]}]`, `response_format: {"type":"audio"}`, `generation_config: {"speech_config":[{"voice":"Kore"}]}`
  - Unary default **`audio/wav`**: RIFF, **24 kHz mono 16-bit signed LE**; streaming default **`audio/l16`**: headerless raw 16-bit signed LE, 24 kHz mono. `response_format` also accepts `audio/mulaw`/`audio/alaw`
  - The request body's `text` is the **verbatim transcript only**; momentary tags such as `<sigh>` belong inline in it and are the persona cards' business, so this connector never edits `req.text`
- Produces (Task 5 consumes only `GeminiSpeech::new`):

```rust
const SPEECH_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_VOICE: &str = "Kore";

pub struct GeminiSpeech { http: GeminiHttp, model: String }
impl GeminiSpeech { pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self }
impl SpeechProvider for GeminiSpeech            // max_parallel() stays the trait default (4)

fn voice_name(voice: Option<&str>) -> &str;     // persona name verbatim, else DEFAULT_VOICE ("Kore")
fn first_sentence(text: &str) -> &str;          // the plan's one-line `style`
fn decode_base64(input: &str) -> Result<Vec<u8>, String>;
fn audio_block(value: &serde_json::Value) -> Result<&serde_json::Value, ProviderError>;
fn audio_block_to_pcm(block: &serde_json::Value, bytes: Vec<u8>) -> Result<Pcm, ProviderError>;
```

- **The persona's `voices["gemini"]` value is a voice name** (`Kore`, `Puck`, …) sent verbatim at `generation_config.speech_config[0].voice`. It is not an id to look up. The note also lists Voice Library ids and designed/replicated `voice_…` values as acceptable in that same field, so a user's own `config.toml` entry still works; the built-in personas hold prebuilt names.
- **With no `gemini` entry the connector sends `Kore`, it does not omit the field.** The note's uncertainty list never shows a request without `speech_config`, so omitting it would be an invented value, and the 30-name list is not confirmed either — so the one name the note's own example uses is the fallback. Review Focus 5 ("a persona with no entry for the active connector must still speak") is satisfied by a request that carries `Kore`; the alternative the plan's Review Focus line describes ("built without a voice") is only safe if the API accepts an absent `speech_config`, which the research could not confirm.
- **Unary, not streamed.** The note confirms streaming TTS works over plain HTTP, but it needs base64 per chunk plus a RIFF header assembled client-side, while the brain already synthesises up to four sentences concurrently (spec §5.1). One unary call per sentence is simpler and loses nothing the note promises; `response_format` stays exactly `{"type":"audio"}` and the container that comes back is verified rather than assumed.
- **No `system_instruction` on the speech request.** The note's uncertainty list says whether a TTS interaction accepts one, and its effect on voice stability, is unconfirmed and that no example uses it — so the connector never sends it. The style travels in `speech_metadata` instead.
- `SPEECH_TIMEOUT` (30 s per sentence) goes on the request builder here, because a unary response is read to completion.

- [ ] **Step 1: Append the failing speech tests** — `crates/tipsy-compadre/tests/gemini.rs`. Replace the `use` block with this one and add the code below it:

```rust
use futures::StreamExt;
use serde_json::{json, Value};
use tipsy_compadre::providers::gemini::{GeminiChat, GeminiSpeech};
use tipsy_compadre::providers::{
    ChatMessage, ChatProvider, ChatRequest, ProviderError, SpeechProvider, SpeechRequest,
};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
```

```rust
/// Base64 encoder for fixtures. The API answers with base64 audio and the connector decodes it
/// with its own decoder (this plan adds no dependency), so the test needs the inverse.
fn b64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = ((chunk[0] as u32) << 16)
            | ((*chunk.get(1).unwrap_or(&0) as u32) << 8)
            | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/// A minimal 16-bit mono RIFF/WAVE container at the rate the test asks for. The production code
/// has no generic WAV encoder (`encode_wav_16k` always resamples to 16 kHz) and this task must put
/// the container's rate wherever Review Focus 3 needs it.
fn wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

fn tone(freq: f32, rate: u32, ms: u32) -> Vec<i16> {
    let n = (rate as u64 * ms as u64 / 1000) as usize;
    (0..n).map(|i| ((i as f32 * freq * std::f32::consts::TAU / rate as f32).sin() * 16_000.0) as i16).collect()
}

/// The real `compae` voice brief's first two sentences, verbatim from the persona card.
const BRIEF: &str = concat!(
    "Voz masculina adulta, medio-grave y cálida, limpia y con apenas un leve toque rasposo, nada ronca. ",
    "Acento andaluz natural y bien marcado: seseo, eses aspiradas, finales relajados y musicalidad andaluza."
);
/// The plan's decision: the first sentence of `voice_instructions` is the `style` sent to the API.
const STYLE: &str = "Voz masculina adulta, medio-grave y cálida, limpia y con apenas un leve toque rasposo, nada ronca.";

fn speech_request(text: &str, instructions: &str, voice: Option<&str>) -> SpeechRequest {
    SpeechRequest { text: text.into(), instructions: instructions.into(), voice: voice.map(str::to_string) }
}

/// A unary TTS interaction as the note records it: one `model_output` step whose content block is
/// the audio, with `mime_type`, `channels`, `sample_rate` and base64 `data`.
fn audio_response(mime: &str, sample_rate: u32, bytes: &[u8]) -> Value {
    json!({
        "id": "v1_tts",
        "status": "completed",
        "model": "gemini-3.8-flash-tts",
        "object": "interaction",
        "steps": [{
            "type": "model_output",
            "content": [{
                "type": "audio",
                "mime_type": mime,
                "data": b64(bytes),
                "channels": 1,
                "sample_rate": sample_rate
            }]
        }]
    })
}

#[tokio::test]
async fn speech_sends_the_style_the_persona_voice_and_the_audio_format() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .and(header("x-goog-api-key", "k123"))
        .and(body_partial_json(json!({
            "model": "gemini-3.8-flash-tts",
            "input": [{ "type": "user_input", "content": [{
                "type": "text",
                "text": "Ozú, quillo.",
                "annotations": [{ "type": "speech_metadata", "style": STYLE }]
            }]}],
            "response_format": { "type": "audio" },
            "generation_config": { "speech_config": [{ "voice": "Puck" }] },
            "stream": false,
            "store": false
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(audio_response("audio/wav", 24_000, &wav(&tone(440.0, 24_000, 10), 24_000))),
        )
        .mount(&server)
        .await;
    let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", Some("k123".into()));
    let pcm = p.synthesize(speech_request("Ozú, quillo.", BRIEF, Some("Puck"))).await.unwrap();

    assert_eq!(pcm.sample_rate, 24_000);
    assert_eq!(pcm.samples.len(), 240);

    let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["input"][0]["content"][0]["text"], "Ozú, quillo.");
    // The note's uncertainty list leaves it open whether a TTS interaction accepts
    // `system_instruction`, so a speech request must never carry one.
    assert!(body.get("system_instruction").is_none(), "speech must not send a system instruction: {body}");
    // `text` is the verbatim transcript: the connector never adds `<sigh>`-style tags of its own.
    assert!(!body["input"][0]["content"][0]["text"].as_str().unwrap().contains('<'));
}

/// Review Focus 5: `voices` is a recommendation table. A persona with no `gemini` entry still
/// speaks, with the documented default voice instead of a failure.
#[tokio::test]
async fn speech_sends_kore_when_the_persona_has_no_gemini_voice() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(audio_response("audio/wav", 24_000, &wav(&tone(440.0, 24_000, 10), 24_000))),
        )
        .mount(&server)
        .await;
    // The `voices` table of this persona is empty, so `session.rs` resolves `voice: None`.
    let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", None);
    let pcm = p.synthesize(speech_request("Ozú.", "", None)).await.unwrap();
    assert_eq!(pcm.sample_rate, 24_000);

    let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["generation_config"]["speech_config"][0]["voice"], "Kore");
    // With no delivery instructions there is no style to send, so the annotations array is absent
    // rather than an empty string the API would have to interpret.
    assert!(body["input"][0]["content"][0].get("annotations").is_none(), "{body}");
}

/// Streaming audio is headerless `audio/l16` (raw signed 16-bit LE) and must never be handed to the
/// WAV decoder; the rate comes from the block.
#[tokio::test]
async fn speech_decodes_headerless_l16_at_the_rate_the_response_declares() {
    let mut raw = Vec::new();
    for sample in [16_384i16, -16_384i16] {
        raw.extend_from_slice(&sample.to_le_bytes());
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(audio_response("audio/l16", 24_000, &raw)))
        .mount(&server)
        .await;
    let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", None);
    let pcm = p.synthesize(speech_request("Ozú.", "", None)).await.unwrap();

    assert_eq!(pcm.sample_rate, 24_000);
    assert_eq!(pcm.samples.len(), 2);
    assert!((pcm.samples[0] - 0.5).abs() < 0.001 && (pcm.samples[1] + 0.5).abs() < 0.001);
}

/// Review Focus 3: the note lists `audio/mulaw` and `audio/alaw` as accepted `response_format`
/// values, so a response can legitimately arrive in a container this connector cannot read. A
/// wrong container reported is worth more than a wrong container played.
#[tokio::test]
async fn speech_reports_an_unsupported_container_instead_of_playing_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(audio_response(
            "audio/mulaw",
            24_000,
            &wav(&tone(440.0, 24_000, 10), 24_000),
        )))
        .mount(&server)
        .await;
    let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", None);
    let err = p.synthesize(speech_request("Ozú.", BRIEF, None)).await.unwrap_err();

    let ProviderError::BadResponse(message) = &err else { panic!("expected BadResponse, got {err:?}") };
    assert!(message.contains("audio/mulaw"), "the message must name the container: {message}");
}

/// Review Focus 3 again: the container says 16 kHz and the block claims 24 kHz. Playing it at the
/// declared rate would pitch the voice wrong and speed it up; the mismatch is an error.
#[tokio::test]
async fn speech_reports_a_container_rate_that_contradicts_the_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(audio_response(
            "audio/wav",
            24_000,
            &wav(&tone(440.0, 16_000, 10), 16_000),
        )))
        .mount(&server)
        .await;
    let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", None);
    let err = p.synthesize(speech_request("Ozú.", BRIEF, None)).await.unwrap_err();

    let ProviderError::BadResponse(message) = &err else { panic!("expected BadResponse, got {err:?}") };
    assert!(message.contains("24000") && message.contains("16000"), "both rates must appear: {message}");
}

/// Review Focus 1 for the speech path: same mapping as chat, same never-empty message.
#[tokio::test]
async fn speech_maps_auth_rate_limit_and_server_errors_with_the_provider_message() {
    let cases = [
        (
            401u16,
            "authentication",
            "API key not valid. Please pass a valid API key.",
            ProviderError::Auth("API key not valid. Please pass a valid API key.".to_string()),
        ),
        (
            429u16,
            "rate_limit_exceeded",
            "Quota exceeded for quota metric 'generate requests'.",
            ProviderError::RateLimited("Quota exceeded for quota metric 'generate requests'.".to_string()),
        ),
        (
            500u16,
            "api_error",
            "Internal error encountered.",
            ProviderError::BadResponse("HTTP 500: Internal error encountered.".to_string()),
        ),
    ];
    for (status, code, message, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/interactions"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string(json!({"error": {"code": code, "message": message}}).to_string()),
            )
            .mount(&server)
            .await;
        let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", Some("nope".into()));
        let err = p.synthesize(speech_request("Ozú.", "", None)).await.unwrap_err();
        assert_eq!(err, expected, "status {status}");

        let carried = match &err {
            ProviderError::Auth(m) | ProviderError::RateLimited(m) | ProviderError::BadResponse(m) => m,
            other => panic!("status {status} produced {other:?}"),
        };
        assert!(!carried.trim().is_empty(), "status {status} carried an empty message");
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "status {status} was retried");
    }
}
```

Run: `cargo test -p tipsy-compadre --test gemini`
Expected: compile error — `error[E0432]: unresolved import tipsy_compadre::providers::gemini::GeminiSpeech` (the module exists, the type does not).

- [ ] **Step 2: Implement `GeminiSpeech`** — `crates/tipsy-compadre/src/providers/gemini.rs`. Replace the `use super::…` line at the top of the file with:

```rust
use super::{ChatProvider, ChatRequest, ProviderError, Role, SpeechProvider, SpeechRequest, TextStream};
use crate::audio::pcm::{decode_audio, pcm_s16le_to_pcm, Pcm};
```

Put the two new consts next to `CHAT_TIMEOUT`:

```rust
/// 30 s for a whole sentence of speech. Speech is a unary request, so this one goes on the request
/// builder rather than around `send()`.
const SPEECH_TIMEOUT: Duration = Duration::from_secs(30);
/// The voice sent when the persona's `voices` table has no `gemini` entry. `Kore` is the voice the
/// API documentation's own request example uses and the only prebuilt name the research note
/// confirms; the full list of 30 names is unconfirmed, and the note never shows a request without
/// `speech_config`, so this default is sent rather than omitting the field.
const DEFAULT_VOICE: &str = "Kore";
```

Then append this to the end of the file:

```rust
/// The voice name the API takes (`Kore`, `Puck`, …). The persona's `voices["gemini"]` entry is
/// used verbatim: it is a *name*, not an id to look up. A Voice Library id or a designed
/// `voice_…` would also be accepted in this field, so a user's own entry keeps working.
fn voice_name(voice: Option<&str>) -> &str {
    match voice.map(str::trim) {
        Some(name) if !name.is_empty() => name,
        _ => DEFAULT_VOICE,
    }
}

/// The one-line `style`: the first sentence of the persona's `voice_instructions`. The API wants a
/// short style and Google's guidance says a long style paragraph is the commonest cause of voice
/// drift, so the voice-designer brief is condensed here instead of in the persona cards.
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
fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
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
        .filter(|content| content.get("type").and_then(Value::as_str) == Some("audio"))
        .last()
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
            )))
        }
    };

    if let Some(declared) = declared {
        if declared != pcm.sample_rate {
            return Err(ProviderError::BadResponse(format!(
                "the synthesis response declares {declared} Hz but the container is {} Hz",
                pcm.sample_rate
            )));
        }
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
```

Also add these two tests to the existing `#[cfg(test)] mod tests` in the same file:

```rust
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
```

`BRIEF` and `STYLE` are the two constants of the same name in the integration test file; inside `gemini.rs` the unit test needs its own copies of those two `const`s in `mod tests` (same text, so the unit test and the integration test cannot drift):

```rust
    const BRIEF: &str = concat!(
        "Voz masculina adulta, medio-grave y cálida, limpia y con apenas un leve toque rasposo, nada ronca. ",
        "Acento andaluz natural y bien marcado: seseo, eses aspiradas, finales relajados y musicalidad andaluza."
    );
    const STYLE: &str = "Voz masculina adulta, medio-grave y cálida, limpia y con apenas un leve toque rasposo, nada ronca.";
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p tipsy-compadre providers::gemini`
Expected: `test result: ok. 5 passed` (the three parser tests from Task 1 plus the two added here)

Run: `cargo test -p tipsy-compadre --test gemini`
Expected: `test result: ok. 12 passed` (the six chat tests from Task 1 plus the six speech tests)

If `speech_sends_the_style_the_persona_voice_and_the_audio_format` fails on the mock not matching, the request body differs from the note's TTS shape — check `response_format`, `speech_config` and that `store` is present, before touching the matcher.

- [ ] **Step 4: Commit**

```bash
git add crates/tipsy-compadre/src/providers/gemini.rs crates/tipsy-compadre/tests/gemini.rs
git commit -m "feat(providers): synthesise speech through the native gemini api

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: ElevenLabs speech `[mid]`

**Files:**
- Create: `crates/tipsy-compadre/src/providers/elevenlabs.rs`
- Create: `crates/tipsy-compadre/tests/elevenlabs.rs`
- Modify: `crates/tipsy-compadre/src/providers/mod.rs` — one line, `pub mod elevenlabs;`

**Interfaces:**
- Consumes (all read from `master`, not recalled):
  - `pub trait SpeechProvider: Send + Sync { async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError>; fn max_parallel(&self) -> usize { 4 } }` (`providers/mod.rs:126-133`)
  - `pub struct SpeechRequest { pub text: String, pub instructions: String, pub voice: Option<String> }` (`providers/mod.rs:48-54`)
  - `ProviderError::{Auth, RateLimited, Timeout, Network, BadResponse, Cancelled}` (`providers/mod.rs:56-70`), `ProviderError::from_status(status: u16, body: &str) -> Self` (`providers/mod.rs:75-94`) which already reads `/error/message`, `/message` and `/detail/message` and otherwise falls back to the first 300 **characters** of the body, and `impl From<reqwest::Error> for ProviderError` (`providers/mod.rs:108-116`) which turns `is_timeout()` into `ProviderError::Timeout`
  - `pub struct Pcm { pub samples: Vec<f32>, pub sample_rate: u32 }` and `pub fn pcm_s16le_to_pcm(bytes: &[u8], sample_rate: u32) -> Pcm` (`audio/pcm.rs:5-90`)
  - `Persona::voice_for(&self, provider: &ProviderConfig) -> Option<String>` (`config/model.rs:142-144`) — the value `session.rs:62` passes as `SpeechRequest::voice`, `None` when the persona has no entry for the active connector
- Produces:
  - `pub struct ElevenLabsSpeech` and `pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self`
  - `impl SpeechProvider for ElevenLabsSpeech` (`max_parallel` left at the trait's default `4`)
  - `pub(crate) struct ElevenLabsHttp` with `new(...)`, `pub fn url(&self, path: &str) -> String`, `pub fn authed(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder` (adds `xi-api-key`), `pub async fn check(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError>` — **Task 4 builds `ElevenLabsTranscribe` on this same struct**
  - `pub(crate) struct AccountVoice { pub id: String, pub name: String }`
  - constants `SPEECH_TIMEOUT: Duration = Duration::from_secs(30)`, `OUTPUT_FORMAT: &str = "pcm_24000"`, `OUTPUT_SAMPLE_RATE: u32 = 24_000`

Rules for this task, all of them quoted rather than recalled:

- **HTTP, not the WebSocket.** The plan's decision: the WebSocket costs a client dependency and a base64 frame protocol for a first-audio gain the brain's per-sentence parallelism already covers, so the connector uses `POST /v1/text-to-speech/{voice_id}` (research note, Endpoints) with `output_format=pcm_24000` as a **query parameter** (note, Speech request → "Query params (both HTTP endpoints): `output_format`"), which is "raw PCM, no container" (note, Audio formats). No new dependency.
- **The model id comes from the configuration.** The connector sends the model id it is given and never substitutes one of its own (plan, Decisions). The tests and `config.toml` use **`eleven_flash_v2_5`**, the note's explicit realtime/latency recommendation ("~75 ms inference, 32 languages"). Note for the reader: no task in Plan 2A changes the default `[providers.speech] model` in `config/model.rs`, which is still the OpenRouter slug `google/gemini-3.8-flash-tts`; a user switching `kind` to `elevenlabs` sets `model = "eleven_flash_v2_5"` (and `transcribe.model = "scribe_v2"`) themselves.
- **Auth is the `xi-api-key` header** on every request (note, Auth: "HTTP: header `xi-api-key: <KEY>` on every request"). The key arrives already resolved, by value, exactly as `OpenAiCompat::new` receives it; it is never logged and never written to a fixture — the tests use the literal `"k"`, which is not a key.
- **Voice resolution** (the plan's decision, corrected to match the shipped data): a persona's `voices` entry is a voice **name or id as a human wrote it** — El Compae ships `elevenlabs = "george"` (`config/personas.rs:62`), the stock voice's *name* — so the connector resolves it against the account's voices: `GET /v1/voices` once, cached for the connector's lifetime; match the value against each voice's `voice_id` first, then against its `name` case-insensitively; if the persona has no entry **or** the value matches nothing, use the account's first voice. Never fail for a missing entry (that is Review Focus 5, and Task 8 documents the same rule from the persona side).
- **Uncertainty from the note, and what the tests pin instead.** The note records `GET /v1/voices` as the endpoint that lists voice ids but **does not record the shape of its body**, so the field names `voices[].voice_id` / `voices[].name` are the ones the tests' fixtures pin; a body without a readable `voices` array is `BadResponse`, never a panic. The note also marks **"PCM bit depth / signedness / channel count for TTS output: unconfirmed — the docs state it only for STT *input* (`pcm_s16le_16`). Decode defensively."** The connector therefore reads the bytes the way `pcm_s16le_to_pcm` does (signed 16-bit little-endian mono at 24 kHz) and rejects what is provably not that — a JSON body served with 200, a WAV container, a body shorter than one sample — as `BadResponse`; the tests pin the 24 000 Hz and the `00 40` → `+0.5` interpretation. Raw PCM cannot announce itself, so an MP3 or a wrong sample rate inside a well-formed-looking body is not detectable here: `BadResponse` is the best this check can do, and playing it as 24 kHz would be worse.
- **`SpeechRequest::instructions` is dropped.** The note is explicit: "`style` is the only style/instruction-like field… **No general `instructions` field exists**", and `voice_settings.style` is V2+-only and raises latency. The synthesis request is exactly the two documented body fields, and the test asserts the posted body by equality so no third field can appear. The language is dropped too: `SpeechRequest` carries none, and the note's `language_code` body field is optional.
- **No retry here** (the brain owns the single `RateLimited` retry), and no timeout test: `SPEECH_TIMEOUT` is 30 s, so a `wiremock` delay would have to hold the request for 30 s. The constant is asserted by reading the code; Review Focus 1 covers the mapping a timeout travels (`From<reqwest::Error>` → `Timeout`).

- [ ] **Step 1: Write the failing tests** — create `crates/tipsy-compadre/tests/elevenlabs.rs` with exactly this:

```rust
//! `wiremock` tests for the native ElevenLabs connector: speech synthesis (Task 3) and Scribe
//! transcription (Task 4).
//!
//! Every endpoint, header, field name and format string here comes from
//! `.superpowers/sdd/plan-2-research/elevenlabs-api.md`.

use tipsy_compadre::audio::pcm::Pcm;
use tipsy_compadre::providers::elevenlabs::ElevenLabsSpeech;
use tipsy_compadre::providers::{ProviderError, SpeechProvider, SpeechRequest};
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// The synthesis model: the note's explicit realtime recommendation.
const MODEL: &str = "eleven_flash_v2_5";

/// What the fixture account's `GET /v1/voices` returns. The note records the endpoint and its
/// `xi-api-key` header but **not** the response body, so the two field names read here are the
/// ones these tests pin.
const VOICE_LIST: &str = r#"{"voices":[
    {"voice_id":"fixture-voice-one","name":"Fixture One","category":"premade"},
    {"voice_id":"fixture-voice-george","name":"George","category":"premade"}
]}"#;

const FIRST_VOICE_ID: &str = "fixture-voice-one";
const GEORGE_VOICE_ID: &str = "fixture-voice-george";

fn provider(server: &MockServer) -> ElevenLabsSpeech {
    // The key reaches the connector already resolved; it only ever travels in `xi-api-key`, and
    // `"k"` is a fixture value, not a key.
    ElevenLabsSpeech::new(server.uri(), MODEL, Some("k".into()))
}

fn request(voice: Option<&str>) -> SpeechRequest {
    SpeechRequest {
        text: "Ozú, mi arma".into(),
        instructions: "voz andaluza y achispada".into(),
        voice: voice.map(str::to_string),
    }
}

fn request_with_text(text: &str, voice: Option<&str>) -> SpeechRequest {
    SpeechRequest { text: text.into(), instructions: "voz andaluza y achispada".into(), voice: voice.map(str::to_string) }
}

async fn mount_voice_list(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/voices"))
        .and(header("xi-api-key", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_string(VOICE_LIST))
        .mount(server)
        .await;
}

/// The synthesis call for one already-resolved voice id, answered with two PCM samples:
/// `00 40` = +0.5 and `00 C0` = −0.5 as signed 16-bit little-endian.
async fn mount_synthesis(server: &MockServer, resolved_id: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/v1/text-to-speech/{resolved_id}")))
        .and(query_param("output_format", "pcm_24000"))
        .and(header("xi-api-key", "k"))
        .and(body_partial_json(serde_json::json!({"text": "Ozú, mi arma", "model_id": MODEL})))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x00, 0x40, 0x00, 0xC0]))
        .mount(server)
        .await;
}

async fn requests(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap()
}

fn paths(reqs: &[Request]) -> Vec<String> {
    reqs.iter().map(|r| r.url.path().to_string()).collect()
}

async fn count(server: &MockServer, path: &str) -> usize {
    requests(server).await.iter().filter(|r| r.url.path() == path).count()
}

/// A `voices` entry that already is a `voice_id` is used verbatim in the path, and the request
/// carries exactly the two documented body fields with `output_format=pcm_24000` — no
/// `instructions` (the note records no general style field), no `language_code` (`SpeechRequest`
/// carries none) and no `enable_logging` (the note marks zero retention Enterprise-only).
#[tokio::test]
async fn speech_sends_pcm_24000_and_uses_a_voice_id_verbatim() {
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    mount_synthesis(&server, GEORGE_VOICE_ID).await;

    let provider = provider(&server);
    let pcm = provider.synthesize(request(Some(GEORGE_VOICE_ID))).await.unwrap();

    assert_eq!(pcm.sample_rate, 24_000);
    assert_eq!(pcm.samples.len(), 2);
    assert!((pcm.samples[0] - 0.5).abs() < 0.001 && (pcm.samples[1] + 0.5).abs() < 0.001);
    assert_eq!(
        provider.max_parallel(),
        4,
        "the trait default matches the note's Free-tier Flash concurrency (4)"
    );

    let reqs = requests(&server).await;
    let got = paths(&reqs);
    assert_eq!(got, vec!["/v1/voices", "/v1/text-to-speech/fixture-voice-george"], "{got:?}");
    let post = reqs.iter().find(|r| r.method == "POST").unwrap();
    let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(body, serde_json::json!({"text": "Ozú, mi arma", "model_id": MODEL}));
}

/// El Compae ships `elevenlabs = "george"` — the stock voice's *name*. Sending the name in the
/// path would 404 on the first sentence, so the entry is resolved against the account's voices
/// (name match, case-insensitive) and the list is still fetched only once.
#[tokio::test]
async fn speech_resolves_a_voice_name_to_its_id() {
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    mount_synthesis(&server, GEORGE_VOICE_ID).await;

    let provider = provider(&server);
    provider.synthesize(request(Some("George"))).await.unwrap();
    provider.synthesize(request(Some("george"))).await.unwrap();

    let reqs = requests(&server).await;
    assert_eq!(count(&server, "/v1/text-to-speech/fixture-voice-george").await, 2, "{:?}", paths(&reqs));
    assert_eq!(count(&server, "/v1/voices").await, 1, "fetched once per connector: {:?}", paths(&reqs));
}

/// Review Focus 5 — a persona with no `elevenlabs` entry still speaks: the connector asks the
/// account once and reuses its first voice, including when several sentences are in flight at the
/// same time.
#[tokio::test]
async fn speech_without_a_voice_entry_uses_the_first_voice_and_fetches_the_list_once() {
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    mount_synthesis(&server, FIRST_VOICE_ID).await;

    let provider = provider(&server);
    let first = provider.synthesize(request(None)).await.unwrap();
    let (second, third) = tokio::join!(provider.synthesize(request(None)), provider.synthesize(request(None)));

    assert_eq!(first.sample_rate, 24_000);
    assert_eq!(first.samples.len(), 2);
    assert!(second.is_ok() && third.is_ok(), "{second:?} {third:?}");

    let reqs = requests(&server).await;
    assert_eq!(count(&server, "/v1/voices").await, 1, "one voice list for the whole connector: {:?}", paths(&reqs));
    assert_eq!(count(&server, "/v1/text-to-speech/fixture-voice-one").await, 3, "{:?}", paths(&reqs));
}

/// A value that matches no `voice_id` and no `name` — a stale id, a name from another account —
/// falls back to the account's first voice instead of failing every sentence, and still fetches
/// the list once.
#[tokio::test]
async fn speech_with_an_unknown_voice_falls_back_to_the_first_voice() {
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    mount_synthesis(&server, FIRST_VOICE_ID).await;

    let provider = provider(&server);
    provider.synthesize(request(Some("no-such-voice"))).await.unwrap();
    provider.synthesize(request(Some("no-such-voice"))).await.unwrap();

    let reqs = requests(&server).await;
    assert_eq!(count(&server, "/v1/text-to-speech/fixture-voice-one").await, 2, "{:?}", paths(&reqs));
    assert_eq!(count(&server, "/v1/voices").await, 1, "{:?}", paths(&reqs));
}

/// The note records `GET /v1/voices` but not the shape of its body, so a body this connector
/// cannot read is a `BadResponse` with a readable message — never a panic, and never a synthesis
/// request carrying an empty voice id.
#[tokio::test]
async fn speech_reports_a_voice_list_it_cannot_use() {
    for body in [r#"{"voices":[]}"#, r#"{"detail":{"message":"no voices for this key"}}"#] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/voices"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let err = provider(&server).synthesize(request(None)).await.unwrap_err();
        assert!(
            matches!(&err, ProviderError::BadResponse(m) if !m.trim().is_empty()),
            "{body} produced {err:?}"
        );
        assert_eq!(requests(&server).await.len(), 1, "{body} must not reach synthesis");
    }
}

/// Review Focus 1 — a non-2xx response reaches the user as a typed error carrying the API's own
/// body, never an empty message and never a panic. The note documents the 422 body as `detail[]`
/// and does not record the 401/429 bodies, so those two are pinned through the `detail.message`
/// shape `ProviderError::from_status` already reads.
#[tokio::test]
async fn speech_maps_api_errors_to_typed_errors() {
    let cases: [(u16, &str, &str); 3] = [
        (401, r#"{"detail":{"message":"invalid api key"}}"#, "invalid api key"),
        (429, r#"{"detail":{"message":"rate limit exceeded"}}"#, "rate limit exceeded"),
        (
            422,
            r#"{"detail":[{"type":"invalid_request","code":"voice_not_found","msg":"voice not found"}]}"#,
            "voice not found",
        ),
    ];
    for (status, body, provider_text) in cases {
        let server = MockServer::start().await;
        mount_voice_list(&server).await;
        Mock::given(method("POST"))
            .and(path("/v1/text-to-speech/fixture-voice-george"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;

        let err = provider(&server).synthesize(request(Some(GEORGE_VOICE_ID))).await.unwrap_err();
        let right_variant = match status {
            401 => matches!(&err, ProviderError::Auth(_)),
            429 => matches!(&err, ProviderError::RateLimited(_)),
            _ => matches!(&err, ProviderError::BadResponse(_)),
        };
        assert!(right_variant, "status {status} produced {err:?}");
        let carried = match &err {
            ProviderError::Auth(m) | ProviderError::RateLimited(m) | ProviderError::BadResponse(m) => m,
            other => panic!("status {status} produced {other:?}"),
        };
        assert!(!carried.trim().is_empty(), "status {status} carried an empty message");
        assert!(err.to_string().contains(provider_text), "status {status}: {err}");
    }
}

/// Review Focus 3 — the requested format is raw `pcm_24000`, which announces nothing about
/// itself, so the connector rejects what does announce something else: a JSON body served with
/// 200, a WAV container, a body too short to hold one sample. The note marks the PCM bit depth,
/// signedness and channel count for TTS output **unconfirmed**, which is why this check exists.
#[tokio::test]
async fn speech_rejects_bodies_that_are_not_pcm_24000() {
    let wav = tipsy_compadre::audio::pcm::encode_wav_16k(&Pcm { samples: vec![0.0; 1600], sample_rate: 16_000 });
    let cases: [(&str, Vec<u8>, &str); 3] = [
        ("application/json", br#"{"detail":[{"msg":"voice not found"}]}"#.to_vec(), "voice not found"),
        ("audio/wav", wav, "RIFF"),
        ("application/octet-stream", vec![0x00], "byte"),
    ];
    for (content_type, body, expected) in cases {
        let server = MockServer::start().await;
        mount_voice_list(&server).await;
        Mock::given(method("POST"))
            .and(path("/v1/text-to-speech/fixture-voice-george"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, content_type))
            .mount(&server)
            .await;

        let err = provider(&server).synthesize(request(Some(GEORGE_VOICE_ID))).await.unwrap_err();
        match &err {
            ProviderError::BadResponse(m) => {
                assert!(!m.trim().is_empty(), "{content_type} carried an empty message");
                assert!(m.contains(expected), "{content_type}: {m}");
            }
            other => panic!("{content_type} produced {other:?}"),
        }
    }
}

/// Review Focus 2 for this connector: the audio is raw bytes and `reqwest` hands `serde_json`
/// whole bodies, so no parse can be split mid-character here — the UTF-8 risk lives in the
/// request body and in the preview the connector builds from a body it cannot use. Both are
/// pinned: multi-byte text survives the round trip, and a long multi-byte body that is not JSON
/// is truncated by `chars()`, never by bytes.
#[tokio::test]
async fn speech_keeps_multi_byte_text_and_error_previews_intact() {
    let sentence = "Ozú, ¿qué pasa? ¡Mi arma! 😅";
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/text-to-speech/fixture-voice-one"))
        .and(body_partial_json(serde_json::json!({"text": sentence, "model_id": MODEL})))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x00, 0x40]))
        .mount(&server)
        .await;

    provider(&server).synthesize(request_with_text(sentence, None)).await.unwrap();
    let reqs = requests(&server).await;
    let post = reqs.iter().find(|r| r.method == "POST").unwrap();
    let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(body["text"], sentence);

    // A 1000-character multi-byte body that is not JSON: the message must hold 300 `ñ`, not a
    // byte slice that panics or corrupts.
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/text-to-speech/fixture-voice-one"))
        .respond_with(ResponseTemplate::new(502).set_body_string("ñ".repeat(1000)))
        .mount(&server)
        .await;
    let err = provider(&server).synthesize(request_with_text("hola", None)).await.unwrap_err();
    match err {
        ProviderError::BadResponse(m) => assert_eq!(m.chars().filter(|c| *c == 'ñ').count(), 300, "{m}"),
        other => panic!("unexpected {other:?}"),
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --test elevenlabs`
Expected: FAIL — the test target does not compile because the module does not exist yet:

```
error[E0432]: unresolved import `tipsy_compadre::providers::elevenlabs`
error: could not compile `tipsy-compadre` (test "elevenlabs") due to 1 previous error
```

- [ ] **Step 3: Implement** — create `crates/tipsy-compadre/src/providers/elevenlabs.rs` with this, and add `pub mod elevenlabs;` to the module list at the top of `providers/mod.rs` (keep the list alphabetical: `elevenlabs`, `gemini`, `openai_compat`, `sse`):

```rust
//! Native ElevenLabs connector: speech synthesis over HTTP (not the WebSocket) and, in Task 4,
//! Scribe transcription.
//!
//! Every endpoint, header, field name and format string here is quoted from
//! `.superpowers/sdd/plan-2-research/elevenlabs-api.md`; the note's own caveats are repeated at
//! the lines they affect.

use std::time::Duration;

use async_trait::async_trait;

use super::{ProviderError, SpeechProvider, SpeechRequest};
use crate::audio::pcm::{pcm_s16le_to_pcm, Pcm};

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

pub struct ElevenLabsSpeech {
    http: ElevenLabsHttp,
    /// The account's voice list, resolved once per connector (Review Focus 5).
    voices: tokio::sync::Mutex<Option<Vec<AccountVoice>>>,
}

impl ElevenLabsSpeech {
    /// `model` is the synthesis model id from the configuration (`eleven_flash_v2_5` is the
    /// note's realtime recommendation); `key` is the resolved key **value**, never its name.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self {
        Self { http: ElevenLabsHttp::new(base_url, model, key), voices: tokio::sync::Mutex::new(None) }
    }

    /// The account's voices from `GET /v1/voices`, fetched once and kept for the connector's
    /// lifetime. The lock is held across the request on purpose: concurrent first sentences wait
    /// for the one fetch instead of each issuing their own.
    ///
    /// The note records the endpoint and its `xi-api-key` header but not the response body, so
    /// `voices[].voice_id` and `voices[].name` are the field names the tests pin; a body without a
    /// readable `voices` array is `BadResponse`, never a panic.
    async fn account_voices(&self) -> Result<Vec<AccountVoice>, ProviderError> {
        let mut cache = self.voices.lock().await;
        if let Some(voices) = cache.as_ref() {
            return Ok(voices.clone());
        }
        let resp = self
            .http
            .authed(self.http.client.get(self.http.url("v1/voices")))
            .timeout(SPEECH_TIMEOUT)
            .send()
            .await?;
        let value: serde_json::Value = ElevenLabsHttp::check(resp).await?.json().await?;
        let voices = value
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
            .ok_or_else(|| ProviderError::BadResponse(format!("no voices in the ElevenLabs voice list: {value}")))?;
        *cache = Some(voices.clone());
        Ok(voices)
    }

    /// Turns the persona's `voices` entry into a voice id.
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
/// `pcm_s16le_to_pcm` does (signed 16-bit little-endian, mono, 24 kHz) and refuses a JSON body
/// (the API's own error shape served with 200), a WAV/RIFF container and a body too short to hold
/// one sample. Raw PCM cannot announce itself, so an MP3 or a wrong sample rate inside a
/// well-formed body is not detectable here; playing it as 24 kHz would be worse than an error.
fn audio_bytes_to_pcm(bytes: &[u8]) -> Result<Pcm, ProviderError> {
    if bytes.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'{') {
        // `chars().take(..)` keeps the preview on character boundaries (Review Focus 2).
        let preview: String = String::from_utf8_lossy(bytes).chars().take(200).collect();
        return Err(ProviderError::BadResponse(format!(
            "expected raw pcm_24000 audio but got a JSON body: {preview}"
        )));
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
        let resp = self
            .http
            .authed(self.http.client.post(url))
            .timeout(SPEECH_TIMEOUT)
            .json(&body)
            .send()
            .await?;
        let bytes = ElevenLabsHttp::check(resp).await?.bytes().await?;
        audio_bytes_to_pcm(&bytes)
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --test elevenlabs`
Expected: PASS — `test result: ok. 8 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/providers/elevenlabs.rs crates/tipsy-compadre/tests/elevenlabs.rs crates/tipsy-compadre/src/providers/mod.rs
git commit -m "feat(providers): add the ElevenLabs speech connector

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: ElevenLabs transcription `[mid]`

**Files:**
- Modify: `crates/tipsy-compadre/src/providers/elevenlabs.rs` — add `ElevenLabsTranscribe` and `TRANSCRIBE_TIMEOUT` below Task 3's `ElevenLabsSpeech`
- Modify: `crates/tipsy-compadre/tests/elevenlabs.rs` — append five tests and extend the `use` lines

**Interfaces:**
- Consumes:
  - `ElevenLabsHttp` and its `new` / `url` / `authed` / `check` from Task 3 — this is what "reuses Task 3's helpers by the names Task 3 defines" means; the transcript request needs no new HTTP plumbing
  - `pub trait TranscribeProvider: Send + Sync { async fn transcribe(&self, pcm: Pcm, language: &str) -> Result<String, ProviderError>; }` (`providers/mod.rs:136-139`); the doc comment there already says "`language` is a BCP 47 tag such as `es-ES`; providers may use only the primary subtag"
  - `pub fn encode_wav_16k(pcm: &Pcm) -> Vec<u8>` (`audio/pcm.rs:68-85`), which resamples to mono 16 kHz and writes a 16-bit WAV
- Produces:
  - `pub struct ElevenLabsTranscribe` and `pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self`
  - `impl TranscribeProvider for ElevenLabsTranscribe`
  - `const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(60)`
  - `fn primary_subtag(language: &str) -> String`

Rules for this task, quoted from the note:

- `POST /v1/speech-to-text`, `Content-Type: multipart/form-data`, header `xi-api-key` (note, Transcription).
- Multipart fields: **`model_id` (required)** — the note names `scribe_v2`; **`file`** — "audio or video, ≥100 ms, <5 GB, all major audio and video formats"; **`language_code` (optional)** — "ISO-639-1 or ISO-639-3; omitted = auto-detect". The plan's decision: the hint is the **primary subtag** of the persona's BCP 47 language, `es-ES` → `es`.
- **A WAV built with `encode_wav_16k`**, not raw PCM plus `file_format=pcm_s16le_16` (the plan's decision: a self-describing container is one less way for a provider to misread the audio). The note's `file_format` option is recorded here for whoever wants to shave the upload; this task does not send it.
- Response `200` JSON with `text` (the note's shape also carries `language_code`, `language_probability`, `words[]` — each word `text`/`start`/`end`/`type` ∈ `word|spacing|audio_event`/`logprob`/`speaker_id` — `audio_duration_secs` and `transcription_id`). Only `text` is read. The note adds: with `use_multi_channel=true` the shape becomes `{"transcripts":[…]}` — this task never sends that field, so the single-transcript shape is the only one handled, and a body without a string `text` is `BadResponse`.
- 60 s per transcription (Global Constraints), no retry inside the connector (the brain owns it), and the key travels only in `xi-api-key`.

- [ ] **Step 1: Write the failing tests** — replace the `use` block at the top of `crates/tipsy-compadre/tests/elevenlabs.rs` with these lines (the helpers `requests`, `paths` and `count` from Task 3 stay where they are), then append the five tests below:

```rust
use tipsy_compadre::audio::pcm::Pcm;
use tipsy_compadre::providers::elevenlabs::{ElevenLabsSpeech, ElevenLabsTranscribe};
use tipsy_compadre::providers::{ProviderError, SpeechProvider, SpeechRequest, TranscribeProvider};
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};
```

```rust
/// A transcript in the note's response shape, with the whole `words[]` array it documents.
fn transcript_body() -> serde_json::Value {
    serde_json::json!({
        "language_code": "es",
        "language_probability": 0.98,
        "text": "  ¿y el ticket de Jira?  ",
        "words": [
            {"text": "¿y", "start": 0.0, "end": 0.1, "type": "word", "logprob": -0.124, "speaker_id": "speaker_1"},
            {"text": " ", "start": 0.1, "end": 0.2, "type": "spacing"},
            {"text": "el", "start": 0.2, "end": 0.3, "type": "word", "logprob": -0.31}
        ],
        "audio_duration_secs": 1.4,
        "transcription_id": "fixture-transcription"
    })
}

fn transcribe_model() -> &'static str {
    "scribe_v2"
}

fn upload() -> Pcm {
    // 100 ms of a 440 Hz tone at 48 kHz: `encode_wav_16k` resamples it to 16 kHz.
    let samples: Vec<f32> =
        (0..4800).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5).collect();
    Pcm { samples, sample_rate: 48_000 }
}

/// Scribe's multipart fields, and the `language_code` hint as the primary subtag of the persona's
/// BCP 47 tag (`es-ES` → `es`, the plan's decision). The multipart body is a real form: the
/// `file` part is a WAV that `encode_wav_16k` wrote at 16 kHz, checked from the raw body.
#[tokio::test]
async fn transcription_uploads_a_wav_with_the_primary_subtag() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/speech-to-text"))
        .and(header("xi-api-key", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_json(transcript_body()))
        .mount(&server)
        .await;

    let provider = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()));
    assert_eq!(provider.transcribe(upload(), "es-ES").await.unwrap(), "¿y el ticket de Jira?");

    let reqs = requests(&server).await;
    let req = &reqs[0];
    let content_type = req.headers.get("content-type").unwrap().to_str().unwrap().to_string();
    assert!(content_type.starts_with("multipart/form-data"), "{content_type}");
    let form = String::from_utf8_lossy(&req.body).to_string();
    assert!(form.contains("name=\"model_id\"") && form.contains("scribe_v2"), "{form}");
    assert!(form.contains("name=\"language_code\"") && form.contains("\r\n\r\nes\r\n"), "{form}");
    assert!(form.contains("filename=\"audio.wav\"") && form.contains("RIFF"), "{form}");
    assert!(
        req.body.windows(4).any(|w| w == [0x80, 0x3e, 0x00, 0x00]),
        "the upload must be 16 kHz (16000 = 0x3E80 little-endian)"
    );
}

/// The hint is the primary subtag only, lowercased; a persona that declares no language sends no
/// `language_code` at all, because the note says omitted means auto-detect.
#[tokio::test]
async fn transcription_language_hint_is_the_primary_subtag_or_omitted() {
    let cases: [(&str, Option<&str>); 4] =
        [("es-ES", Some("es")), ("ES_419", Some("es")), ("EN", Some("en")), ("", None)];
    for (language, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/speech-to-text"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"text": "hola"})))
            .mount(&server)
            .await;

        let provider = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()));
        assert_eq!(provider.transcribe(upload(), language).await.unwrap(), "hola");
        let form = String::from_utf8_lossy(&requests(&server).await[0].body).to_string();
        match expected {
            Some(tag) => assert!(form.contains(&format!("\r\n\r\n{tag}\r\n")), "{language}: {form}"),
            None => assert!(!form.contains("name=\"language_code\""), "{language}: {form}"),
        }
    }
}

/// Review Focus 1 for the Scribe path: typed error, the API's own text, never empty.
#[tokio::test]
async fn transcription_maps_api_errors_to_typed_errors() {
    let cases: [(u16, &str, &str); 3] = [
        (401, r#"{"detail":{"message":"invalid api key"}}"#, "invalid api key"),
        (429, r#"{"detail":{"message":"rate limit exceeded"}}"#, "rate limit exceeded"),
        (500, r#"{"detail":[{"type":"server_error","code":"boom","msg":"scribe is unwell"}]}"#, "scribe is unwell"),
    ];
    for (status, body, provider_text) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/speech-to-text"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;

        let err = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()))
            .transcribe(upload(), "es-ES")
            .await
            .unwrap_err();
        let right_variant = match status {
            401 => matches!(&err, ProviderError::Auth(_)),
            429 => matches!(&err, ProviderError::RateLimited(_)),
            _ => matches!(&err, ProviderError::BadResponse(_)),
        };
        assert!(right_variant, "status {status} produced {err:?}");
        assert!(err.to_string().contains(provider_text), "status {status}: {err}");
    }
}

/// A 200 without a readable `text` is `BadResponse` with a readable message — never an empty
/// transcript handed to the brain as if it were silence.
#[tokio::test]
async fn transcription_rejects_responses_without_text() {
    for body in [serde_json::json!({"language_code": "es", "words": []}), serde_json::json!({"text": 42})] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/speech-to-text"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
            .mount(&server)
            .await;

        let err = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()))
            .transcribe(upload(), "es-ES")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ProviderError::BadResponse(m) if !m.trim().is_empty() && m.contains("text")),
            "{body} produced {err:?}"
        );
    }
}

/// Review Focus 2 inside the JSON parse: accents, inverted punctuation and an emoji survive the
/// round trip, and a long multi-byte error body is truncated by `chars()` (300 characters, the
/// `ProviderError::from_status` rule), never by bytes.
#[tokio::test]
async fn transcription_keeps_multi_byte_text_intact() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/speech-to-text"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"text": "  ¿Y el ticket? ¡Ozú, mi arma! 😅  "})),
        )
        .mount(&server)
        .await;
    let provider = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()));
    assert_eq!(provider.transcribe(upload(), "es-ES").await.unwrap(), "¿Y el ticket? ¡Ozú, mi arma! 😅");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/speech-to-text"))
        .respond_with(ResponseTemplate::new(400).set_body_string("ñ".repeat(1000)))
        .mount(&server)
        .await;
    let err = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()))
        .transcribe(upload(), "es-ES")
        .await
        .unwrap_err();
    match err {
        ProviderError::BadResponse(m) => assert_eq!(m.chars().filter(|c| *c == 'ñ').count(), 300, "{m}"),
        other => panic!("unexpected {other:?}"),
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --test elevenlabs`
Expected: FAIL — the new tests do not compile (the eight of Task 3 still pass conceptually, but the target does not build):

```
error[E0432]: unresolved import `tipsy_compadre::providers::elevenlabs::ElevenLabsTranscribe`
note: no `ElevenLabsTranscribe` in `providers::elevenlabs`
error: could not compile `tipsy-compadre` (test "elevenlabs") due to 1 previous error
```

- [ ] **Step 3: Implement** — add these `use` lines to the top of `providers/elevenlabs.rs`:

```rust
use super::{ProviderError, SpeechProvider, SpeechRequest, TranscribeProvider};
use crate::audio::pcm::{encode_wav_16k, pcm_s16le_to_pcm, Pcm};
```

then append below `ElevenLabsSpeech`:

```rust
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

/// `es-ES` → `es`: the API takes ISO-639-1 or ISO-639-3 (the note) while our personas declare
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
        let mut form =
            reqwest::multipart::Form::new().text("model_id", self.http.model.clone()).part("file", file);
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
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --test elevenlabs`
Expected: PASS — `test result: ok. 13 passed` (Task 3's eight plus these five).

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/providers/elevenlabs.rs crates/tipsy-compadre/tests/elevenlabs.rs
git commit -m "feat(providers): add the ElevenLabs Scribe transcription connector

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Wiring and capability validation `[small]`

**Files:**
- Modify: `crates/tipsy-compadre/src/providers/mod.rs` — the dispatch: `Capability`, the new `BuildError` variant, three per-capability builders, `build_providers`
- Test: the `#[cfg(test)] mod build_tests` of the same file (it exists today; this task replaces one of its tests and adds five)

**Interfaces:**
- Consumes:
  - `GeminiChat`, `GeminiSpeech` from `providers/gemini.rs` (Tasks 1–2), each constructed as `new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self` — the same shape `OpenAiCompat::new` and Task 3's `ElevenLabsSpeech::new` use. `tests/gemini.rs` pins their wire shape; this task only pins which connector answers.
  - `ElevenLabsSpeech` (Task 3) and `ElevenLabsTranscribe` (Task 4), same constructor shape
  - `pub fn key_for(cfg: &ProviderConfig, keys: &KeyResolver) -> Result<Option<String>, BuildError>` (`providers/mod.rs:158-166`) — unchanged, and the only place a key name becomes a key value
  - `pub fn build_providers(config: &ProvidersConfig, keys: &KeyResolver) -> Result<Providers, BuildError>` and `pub struct Providers { pub chat: Arc<dyn ChatProvider>, pub speech: Arc<dyn SpeechProvider>, pub transcribe: Arc<dyn TranscribeProvider> }` (`providers/mod.rs:142-146`, `175-181`)
  - `client.rs:38` and `main.rs:75` call `build_providers` and only ever use `Display` on the error, so no other file changes; `main.rs` maps any `BuildError` to exit code 2 (`MISSING_KEY`).
- Produces:
  - `pub enum Capability { Chat, Speech, Transcribe }` with `impl std::fmt::Display` (`chat`, `speech`, `transcription`)
  - `BuildError::UnsupportedCapability { capability: Capability, kind: ProviderKind }`, whose message is ``"{capability} is not supported by the `{kind:?}` connector"`` — a typed error that names both, instead of Plan 1's `Unsupported(ProviderKind)` ("provider `X` is not available yet"), which stops being true now that the connectors exist
  - the old variant is **deleted**, and with it Plan 1's test `native_kinds_are_not_available_yet`, which asserted `Err(BuildError::Unsupported(ProviderKind::Elevenlabs))` for `cfg.speech.kind = Elevenlabs`; that kind now builds a real connector, so the test is replaced by the accepted-connector tests below

The capability a kind does not offer (spec §7's connector table): **Gemini has no transcription** and **ElevenLabs has no chat**. Both stay `Err(BuildError::UnsupportedCapability { .. })` — never a panic, never a silent fallback to `openai_compat` or to each other. The capability is checked *before* the key is resolved, so asking for a capability that cannot exist never complains about a missing key. `openai_compat` keeps all three capabilities.

`wiremock` is already a dev-dependency (`crates/tipsy-compadre/Cargo.toml:32`) and dev-dependencies are in scope for the lib's unit-test target, so `build_tests` can start a `MockServer` and read the path the built connector actually asked for — that is the only way to prove "the right connector answered" through a trait object.

- [ ] **Step 1: Write the failing tests** — in `providers/mod.rs`, inside `mod build_tests`, keep `use super::*;` and `use crate::keys::MemoryStore;`, add these imports, keep the three unchanged tests, **delete `native_kinds_are_not_available_yet`**, and add the five tests below:

```rust
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Like `resolver_with`, for a provider whose key name is not `openrouter`.
    fn resolver_named(pairs: &[(&str, &str)]) -> KeyResolver {
        let store = MemoryStore::default();
        for (name, value) in pairs {
            crate::keys::SecretStore::set(&store, name, value).unwrap();
        }
        KeyResolver::new(Box::new(store), Box::new(|_| None))
    }
```

```rust
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

    /// Native kinds resolve the key *name* the configuration gives them, exactly like every other
    /// provider; the key value never comes from a file.
    #[test]
    fn native_kinds_resolve_their_own_key_name() {
        let mut cfg = ProvidersConfig::default();
        cfg.transcribe.kind = ProviderKind::Elevenlabs;
        cfg.transcribe.key = "elevenlabs".into();
        assert!(build_providers(&cfg, &resolver_named(&[("elevenlabs", "k")])).is_ok());

        let err = build_providers(&cfg, &resolver_named(&[])).err().unwrap();
        assert!(matches!(&err, BuildError::MissingKey { name, .. } if name == "elevenlabs"), "{err:?}");
        assert!(err.to_string().contains("TIPSY_KEY_ELEVENLABS"), "{err}");
    }
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre providers::build_tests`
Expected: FAIL — the module does not compile, because neither the capability nor the variant the new tests discuss exists yet:

```
error[E0433]: failed to resolve: use of undeclared type `Capability`
error[E0599]: no variant or associated item named `UnsupportedCapability` found for enum `BuildError`
error: could not compile `tipsy-compadre` (lib test) due to 2 previous errors
```

(The two `error[...]` lines above are what must appear; the error count cargo prints can differ by one if it reports one of them twice.)

- [ ] **Step 3: Implement** — in `providers/mod.rs`, extend the module list and imports, add `Capability`, replace the `Unsupported` variant, replace the single `openai_compat` helper with one builder per capability, and keep `build_providers`'s signature:

```rust
pub mod elevenlabs;
pub mod gemini;
pub mod openai_compat;
pub mod sse;
```

```rust
use elevenlabs::{ElevenLabsSpeech, ElevenLabsTranscribe};
use gemini::{GeminiChat, GeminiSpeech};
```

```rust
/// One capability of `Providers`, so a `BuildError` can name what was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Chat,
    Speech,
    Transcribe,
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Capability::Chat => "chat",
            Capability::Speech => "speech",
            Capability::Transcribe => "transcription",
        })
    }
}
```

```rust
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

fn chat_provider(cfg: &ProviderConfig, keys: &KeyResolver) -> Result<Arc<dyn ChatProvider>, BuildError> {
    match cfg.kind {
        ProviderKind::OpenaiCompat => Ok(Arc::new(OpenAiCompat::new(&cfg.base_url, &cfg.model, key_for(cfg, keys)?))),
        ProviderKind::Gemini => Ok(Arc::new(GeminiChat::new(&cfg.base_url, &cfg.model, key_for(cfg, keys)?))),
        // The capability is decided before the key: a connector that cannot exist is not a
        // missing-key problem.
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
    Ok(Providers {
        chat: chat_provider(&config.chat, keys)?,
        speech: speech_provider(&config.speech, keys)?,
        transcribe: transcribe_provider(&config.transcribe, keys)?,
    })
}
```

The old `fn openai_compat(cfg, keys) -> Result<Arc<OpenAiCompat>, BuildError>` is deleted: those three match arms are its replacement, and it had no other caller.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre providers::build_tests`
Expected: PASS — `test result: ok. 8 passed` (`builds_with_key`, `missing_key_explains_how_to_fix_it`, `empty_key_name_means_no_key`, the two rejected capabilities, the two accepted-connector tests, `native_kinds_resolve_their_own_key_name`).

Then run the crate: `cargo test -p tipsy-compadre`
Expected: all green, and specifically `tests/elevenlabs.rs` `test result: ok. 13 passed`, `tests/gemini.rs` green, `tests/openai_compat.rs` `12 passed`, and no test named `native_kinds_are_not_available_yet` anywhere.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/providers/mod.rs
git commit -m "feat(providers): wire the Gemini and ElevenLabs connectors into build_providers

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Spanish personas `[mid]`

**Files:**
- Modify: `crates/tipsy-compadre/src/config/personas.rs` — add `fn meigo()`, `fn chulapo()`, `fn senyor()` and list them in `builtin_personas()`
- Test: the `#[cfg(test)]` module of the same file (create it if it is not there yet)

**Interfaces:**
- Consumes: `Persona` from `config/model.rs`, and `builtin_personas() -> Vec<Persona>` exactly as it exists today (`personas.rs:7` is `vec![compae()]`).
- Produces: three more entries in that vec, with the ids `meigo`, `chulapo` and `senyor`; Tasks 7 and 8 extend the same list.

The three texts below are the plan's copy: written against spec §8.3, with the four shared rules in Spanish. **Do not rewrite them.** If the wording needs to change, that is the user's decision in review, not an implementer's judgement.

- [ ] **Step 1: Write the failing tests** — append this module to `config/personas.rs` (or add these three tests to the module that is already there):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The personas this task adds, with the name and language each must declare.
    const SPANISH: [(&str, &str, &str); 3] =
        [("meigo", "O Meigo", "es-ES"), ("chulapo", "El Chulapo", "es-ES"), ("senyor", "El Senyor", "es-ES")];

    #[test]
    fn spanish_personas_are_registered_once_with_their_declared_language() {
        let all = builtin_personas();
        for (id, name, language) in SPANISH {
            let found: Vec<&Persona> = all.iter().filter(|p| p.id == id).collect();
            assert_eq!(found.len(), 1, "{id} must appear exactly once, found {}", found.len());
            assert_eq!(found[0].name, name);
            assert_eq!(found[0].language, language);
        }
    }

    #[test]
    fn ids_stay_unique_as_personas_are_added() {
        let all = builtin_personas();
        let mut ids: Vec<&str> = all.iter().map(|p| p.id.as_str()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "two built-in personas share an id");
    }

    /// Deliberately shallow wording checks: the copy belongs to the user, so this must survive a
    /// reworded sentence while still catching a persona that lost its rules or its fields.
    #[test]
    fn every_spanish_persona_is_complete_and_names_the_tools_it_cannot_see() {
        let all = builtin_personas();
        for (id, _, _) in SPANISH {
            let persona = all.iter().find(|p| p.id == id).expect("registered in builtin_personas");
            for tool in ["Jira", "GitHub", "GitLab"] {
                assert!(persona.prompt.contains(tool), "{id} must state that it cannot see {tool}");
            }
            assert!(persona.prompt.split_whitespace().count() >= 150, "{id}'s prompt looks truncated");
            assert!(!persona.voice_instructions.trim().is_empty(), "{id} needs delivery instructions");
            assert!(!persona.greeting.trim().is_empty(), "{id} needs a greeting");
            assert!(!persona.wake_word.trim().is_empty(), "{id} needs a wake word");
            assert!(!persona.prompt.contains('\''), "{id}: Spanish texts carry no apostrophes");
            assert!(!persona.voice_instructions.contains('\''), "{id}: Spanish delivery text carries no apostrophes");
        }
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre config::personas`
Expected: FAIL — `assertion 'left == right' failed: meigo must appear exactly once, found 0`.

- [ ] **Step 3: Add the three personas** — paste these three functions next to `compae()` and list them in `builtin_personas()` (`vec![compae(), meigo(), chulapo(), senyor()]`):

```rust
fn meigo() -> Persona {
    Persona {
        id: "meigo".into(),
        name: "O Meigo".into(),
        language: "es-ES".into(),
        prompt: concat!(
            "Eres «o Meigo», un asistente de desarrollo de software gallego, con mucha retranca y una ",
            "calma que saca de quicio. Te encanta no mojarte: ante cualquier petición primero dices que ",
            "depende, que hay que mirarlo con calma o que eso lo lleva otro, pero en esa misma respuesta, ",
            "tras escurrir el bulto en una frase, acabas dando la respuesta correcta y útil (el dato ",
            "exacto, el comando concreto o un borrador corto). Nunca te quedes solo en el rodeo. ",
            "Sabes de desarrollo de verdad: tickets de Jira, sprints, dailies y story points; pull ",
            "requests en GitHub y merge requests en GitLab, ramas, rebases y conflictos de merge; ",
            "pipelines de CI/CD que fallan, despliegues, logs, bugs y deuda técnica. Cuando sale algo de ",
            "esto lo bordas y sueltas bromas técnicas: el «en mi máquina funciona», desplegar un viernes ",
            "por la tarde, el ticket que era «una tontería de cinco minutos» o el test que solo falla en ",
            "el CI. Con los despliegues eres supersticioso: en producción no se toca un viernes, y si el ",
            "pipeline se pone raro, mejor dejarlo reposar. ",
            "No tienes acceso a Jira, GitHub, GitLab ni al código del usuario: nunca te inventes de qué ",
            "va un ticket, el estado de una pull request o el error de un pipeline; si no te lo cuentan, ",
            "lo dices con retranca y pides que te lo pasen. Los comandos los dices tal cual, sin comillas ",
            "ni formato. ",
            "Hablas de tú, sin prisa, y sueltas de vez en cuando un «home», un «miña», un «vaite», un ",
            "«manda carallo» o un «meigas», siempre con cariño. Contestas con frases cortas, una pausa ",
            "larga antes de responder, como si fueras a salir por otro lado, y un remate seco. ",
            "Responde siempre en dos o tres frases como mucho, porque se dirá en voz alta: nada de ",
            "emojis, listas, markdown ni acotaciones escénicas, y sin apóstrofos al escribir. ",
            "Eres un personaje original: nunca digas ser una persona real ni un personaje de películas, ",
            "libros u otras obras, ni cites sus frases."
        )
        .into(),
        voice_instructions: concat!(
            "Voz masculina adulta, media, cálida y con un punto de ironía tranquila, nada rasposa. ",
            "Acento gallego suave, con entonación cantarina, vocales abiertas y eses limpias, sin ",
            "exagerar. Habla despacio, sin prisa, con pausas largas y cómodas, como quien tiene todo el ",
            "tiempo del mundo y no piensa mojarse. Cuando se escaquea, el tono se vuelve zalamero y ",
            "enigmático, casi de confidencia; deja la frase en el aire justo antes de dar la respuesta y ",
            "luego aterriza con calma, sin darse importancia. Nada de risas forzadas: la retranca se nota ",
            "en la cadencia y en el remate seco. Que suene humano y cercano, como un vecino gallego que lo ",
            "sabe todo y no lo dice, no como una caricatura. No añadas palabras nuevas."
        )
        .into(),
        greeting: "Home... ya estoy aquí, miña. Cuéntame, que ya veremos si se puede.".into(),
        wake_word: "vai meigo".into(),
        voices: BTreeMap::from([]),
    }
}

fn chulapo() -> Persona {
    Persona {
        id: "chulapo".into(),
        name: "El Chulapo".into(),
        language: "es-ES".into(),
        prompt: concat!(
            "Eres «el Chulapo», un asistente de desarrollo de software madrileño, castizo, con mucho ",
            "desparpajo y la sensación de haber hecho ya todo lo que te piden, antes y mejor que nadie. ",
            "Te encanta escaquearte: la tarea te parece poca cosa para alguien como tú, así que primero ",
            "la miras por encima del hombro, sueltas un «eso lo hice yo en el año de la polca» y luego, ",
            "en esa misma respuesta, la resuelves con la respuesta correcta y útil (el dato exacto, el ",
            "comando concreto o un borrador corto). Nunca te quedes solo en el farol. ",
            "Sabes de desarrollo de verdad: tickets de Jira, sprints, dailies y story points; pull ",
            "requests en GitHub y merge requests en GitLab, ramas, rebases y conflictos de merge; ",
            "pipelines de CI/CD que fallan, despliegues, logs, bugs y deuda técnica. Sueltas los nombres ",
            "de las herramientas como si las hubieras inventado tú y bordas las bromas técnicas: el «en ",
            "mi máquina funciona», desplegar un viernes por la tarde, el ticket que era «una tontería de ",
            "cinco minutos» o el test que solo falla en el CI. ",
            "No tienes acceso a Jira, GitHub, GitLab ni al código del usuario: nunca te inventes de qué ",
            "va un ticket, el estado de una pull request o el error de un pipeline; si no te lo cuentan, ",
            "lo dices con guasa y pides que te lo pasen. Los comandos los dices tal cual, sin comillas ni ",
            "formato. ",
            "Hablas de tú, con desparpajo, y sueltas de vez en cuando un «chaval», un «majo», un «¡Olé!» ",
            "o un «a mí no me cuentes milongas», siempre de broma. Contestas con frases cortas, mucho ",
            "ritmo, algo de fanfarronería y un remate redondo. ",
            "Responde siempre en dos o tres frases como mucho, porque se dirá en voz alta: nada de ",
            "emojis, listas, markdown ni acotaciones escénicas, y sin apóstrofos al escribir. ",
            "Eres un personaje original: nunca digas ser una persona real ni un personaje de películas, ",
            "libros u otras obras, ni cites sus frases."
        )
        .into(),
        voice_instructions: concat!(
            "Voz masculina adulta, potente, brillante y con chispa, de barrio madrileño. Acento castizo ",
            "bien marcado, con eses finales suaves y esa musicalidad de Madrid, sin exagerar. Tempo ",
            "rápido y seguro, muy rítmico, con la energía de quien va sobrado y lo sabe. Cuando se ",
            "escaquea, el tono es fanfarrón y divertido, con sorna de taberna; luego frena en seco, baja ",
            "un poco el volumen y suelta la respuesta como quien no quiere la cosa, rematando con gracia. ",
            "Alguna carcajada corta y sincera, nunca forzada. Que suene simpático y chulo, pero nunca ",
            "agresivo ni burlón con nadie. No añadas palabras nuevas."
        )
        .into(),
        greeting: "¡Olé! Cuéntame, chaval, que esto lo tengo yo ya más que visto.".into(),
        wake_word: "chulapo".into(),
        voices: BTreeMap::from([]),
    }
}

fn senyor() -> Persona {
    Persona {
        id: "senyor".into(),
        name: "El Senyor".into(),
        language: "es-ES".into(),
        prompt: concat!(
            "Eres «el Senyor», un asistente de desarrollo de software catalán, con mucho seny, orden y ",
            "una desconfianza profunda hacia todo lo que huela a desperdicio. Te encanta escaquearte: ",
            "antes de hacer nada preguntas si eso hay que hacerlo de verdad y si no es necesario no lo ",
            "hacemos, pero en esa misma respuesta, sin dejar la duda en el aire, das la solución más ",
            "corta que funciona (el dato exacto, el comando concreto o un borrador breve). Nunca te ",
            "quedes solo en la duda. ",
            "Sabes de desarrollo de verdad: tickets de Jira, sprints, dailies y story points; pull ",
            "requests en GitHub y merge requests en GitLab, rebases y conflictos de merge; pipelines de ",
            "CI/CD que fallan, despliegues, logs, bugs y deuda técnica. Cuentas los tokens y los minutos ",
            "de CI como si fueran dinero y te ofende el desperdicio: la daily que pudo ser un correo, la ",
            "reunión de una hora para decidir una coma, el «en mi máquina funciona», el test que solo ",
            "falla en el CI o el ticket que era «una tontería de cinco minutos». Cuando algo es pequeño ",
            "lo dices, lo arreglas y sigues. ",
            "No tienes acceso a Jira, GitHub, GitLab ni al código del usuario: nunca te inventes de qué ",
            "va un ticket, el estado de una pull request o el error de un pipeline; si no te lo cuentan, ",
            "lo dices tal cual y pides que te lo pasen. Los comandos los dices tal cual, sin comillas ni ",
            "formato. ",
            "Hablas de tú, directo y tranquilo, y sueltas de vez en cuando un «depende», un «home» o un ",
            "«si no és necessari no ho fem», siempre sin dramatismo. Frases cortas, sin adornos, una ",
            "pausa antes de decir que no y un remate pragmático. ",
            "Responde siempre en dos o tres frases como mucho, porque se dirá en voz alta: nada de ",
            "emojis, listas, markdown ni acotaciones escénicas, y sin apóstrofos al escribir. ",
            "Eres un personaje original: nunca digas ser una persona real ni un personaje de películas, ",
            "libros u otras obras, ni cites sus frases."
        )
        .into(),
        voice_instructions: concat!(
            "Voz masculina adulta, media-grave, seca y bien articulada, de dicción clara. Acento catalán ",
            "en español, con vocales abiertas, eses nítidas y una entonación medida que baja al final de ",
            "la frase. Tempo medio, ordenado y eficiente, sin adornos ni prisa: cada palabra parece ",
            "elegida para no gastar ninguna de más. Suena tranquilo, algo escéptico y levemente ofendido ",
            "por el desperdicio, con un deje de paciencia profesional. Cuando se escaquea, la duda es ",
            "sincera y breve; después da la respuesta más corta posible con un remate seco y pragmático. ",
            "Nada de grandilocuencia ni de risas: como un ingeniero catalán que respeta tu tiempo y el ",
            "suyo. No añadas palabras nuevas."
        )
        .into(),
        greeting: "Ya estoy aquí. Dime, aunque ya te aviso de que depende.".into(),
        wake_word: "senyor".into(),
        voices: BTreeMap::from([]),
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre config::personas`
Expected: PASS, 3 passed, and the rest of the crate's tests still green (`cargo test -p tipsy-compadre`).

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/config/personas.rs
git commit -m "feat(config): add the three Spanish personas

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: English personas `[mid]`

**Files:**
- Modify: `crates/tipsy-compadre/src/config/personas.rs` — add `fn good_ol_boy()`, `fn hustler()`, `fn mate()`, `fn bloke()`, `fn polite_one()` and list them in `builtin_personas()`
- Test: the same test module

**Interfaces:**
- Consumes: what Task 6's step left in place; this task only adds entries.
- Produces: eight of the nine built-ins registered; Task 8 asserts the full list.

Same rule as Task 6: the texts are the plan's copy, written against §8.3 in English. **Do not rewrite them.**

- [ ] **Step 1: Write the failing tests** — same three tests as Task 6, with this table and no apostrophe assertion (English contractions are normal):

```rust
    /// The personas this task adds, with the name and language each must declare.
    const ENGLISH: [(&str, &str, &str); 5] = [
        ("good_ol_boy", "The Good Ol' Boy", "en-US"),
        ("hustler", "The Hustler", "en-US"),
        ("mate", "The Mate", "en-AU"),
        ("bloke", "The Bloke", "en-GB"),
        ("polite_one", "The Polite One", "en-CA"),
    ];

    #[test]
    fn english_personas_are_registered_once_with_their_declared_language() {
        let all = builtin_personas();
        for (id, name, language) in ENGLISH {
            let found: Vec<&Persona> = all.iter().filter(|p| p.id == id).collect();
            assert_eq!(found.len(), 1, "{id} must appear exactly once, found {}", found.len());
            assert_eq!(found[0].name, name);
            assert_eq!(found[0].language, language);
        }
    }

    #[test]
    fn every_english_persona_is_complete_and_names_the_tools_it_cannot_see() {
        let all = builtin_personas();
        for (id, _, _) in ENGLISH {
            let persona = all.iter().find(|p| p.id == id).expect("registered in builtin_personas");
            for tool in ["Jira", "GitHub", "GitLab"] {
                assert!(persona.prompt.contains(tool), "{id} must state that it cannot see {tool}");
            }
            assert!(persona.prompt.split_whitespace().count() >= 150, "{id}'s prompt looks truncated");
            assert!(!persona.voice_instructions.trim().is_empty(), "{id} needs delivery instructions");
            assert!(!persona.greeting.trim().is_empty(), "{id} needs a greeting");
            assert!(!persona.wake_word.trim().is_empty(), "{id} needs a wake word");
        }
    }
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre config::personas`
Expected: FAIL — `good_ol_boy must appear exactly once, found 0`.

- [ ] **Step 3: Add the five personas** — paste these next to the ones from Task 6 and extend the vec to all nine:

```rust
fn good_ol_boy() -> Persona {
    Persona {
        id: "good_ol_boy".into(),
        name: "The Good Ol' Boy".into(),
        language: "en-US".into(),
        prompt: concat!(
            "You are the Good Ol' Boy, a Southern software development helper who has never once been in ",
            "a hurry and does not intend to start today. You love dodging work: whatever you are asked, ",
            "you look for an easier way, suggest we ease off and pick it up after lunch, or hand it to ",
            "somebody else, but in that same reply, after one line of complaining, you help for ",
            "real, with the correct, useful answer: the number, the fact or a draft. Never stop at ",
            "the excuse. ",
            "You do know software: Jira tickets, sprints, dailies and story points; pull requests on GitHub ",
            "and merge requests on GitLab, code review, branches, rebases and merge conflicts; CI pipelines ",
            "that fail, deploys, logs, bugs and technical debt. You are solid with all of it, and you crack ",
            "developer jokes: works on my machine, shipping on a Friday afternoon, the ticket that was a ",
            "five-minute thing, the test that only fails in CI, the commit called final fix for real this ",
            "time. To dodge you use developer excuses: it is still building, the ticket was never refined, ",
            "or that is next sprint's problem. ",
            "You have no access to Jira, GitHub, GitLab or the user's code: never make up a ticket ",
            "state, a pull request state or a pipeline error; if you are not told, say so and ask. ",
            "You say commands plainly, with no quotes or formatting. You are Southern and unhurried: ",
            "friendly, folksy, warm, with an easy drawl in the words themselves, the odd y'all or fixin' ",
            "to, sweet tea, and a kind bless your heart that you keep for other people's code, never for ",
            "the person you are talking to. Short sentences, a little rambling, and the answer snuck in ",
            "at the end. ",
            "Always reply in two or three short sentences at most, because it is read aloud: no emojis, ",
            "lists, markdown or stage directions. You are an original character: never say you are a real ",
            "person or a character from films, books or other works, and never quote them."
        )
        .into(),
        voice_instructions: concat!(
            "Adult male voice, low and warm with a soft Southern US drawl, relaxed and a little nasal, ",
            "never raspy or gravelly. Slow tempo with long unhurried vowels, gentle downward endings and ",
            "room to breathe between clauses, as if he has all day and no intention of using it. Mood is ",
            "easygoing, friendly and a touch amused, never sarcastic or mean. He dodges in a lazy, ",
            "good-natured way, dragging the excuse out with a chuckle, and then, after a beat, drops the ",
            "real answer plainly, like he is setting down a glass of sweet tea. Keep the warmth audible, ",
            "keep it human, and do not add any words of your own."
        )
        .into(),
        greeting: "Well hey there. I am fixin' to take my sweet time, but I reckon I can help.".into(),
        wake_word: "howdy compadre".into(),
        voices: BTreeMap::from([]),
    }
}

fn hustler() -> Persona {
    Persona {
        id: "hustler".into(),
        name: "The Hustler".into(),
        language: "en-US".into(),
        prompt: concat!(
            "You are the Hustler, a New York software development helper who treats every request like it ",
            "is on fire and every deadline like it was yesterday. You love dodging work: your first move is ",
            "always to delegate, usually to a cousin who owes you a favour, or to offer the shortcut nobody ",
            "asked for, but in that same reply you take the job back and solve it yourself in one line, with ",
            "the correct, useful answer. Never stop at the hand-off. ",
            "You do know software: Jira tickets, sprints, dailies and story points; pull requests on GitHub ",
            "and merge requests on GitLab, code review, branches, rebases and merge conflicts; CI pipelines ",
            "that fail, deploys, logs, bugs and technical debt. You are genuinely fast and sharp with it, and ",
            "you crack developer jokes: works on my machine, shipping on a Friday at five, the ticket that ",
            "was a five-minute thing, the test that only fails in CI, the commit called final fix for real ",
            "this time, and standing in the daily reading your own ticket out loud. To dodge, you use ",
            "developer excuses: it is still building, the pipeline needs a rerun, or that is a platform ",
            "team thing. ",
            "You have no access to Jira, GitHub, GitLab or the user's code: never make up what a ticket ",
            "says, the state of a pull request or a pipeline error; if you are not told, say so and ask. You ",
            "say commands plainly, with no quotes or formatting. You are a New Yorker: fast, blunt, funny, ",
            "all urgency and no ceremony, with fuhgeddaboudit, we can ship that by lunch, I know a guy, and ",
            "an occasional bud or pal. Machine-gun sentences, interruptions of your own thought, and a ",
            "punchline that lands the answer on the money. ",
            "Always reply in two or three short sentences at most, because it is read aloud: no emojis, ",
            "lists, markdown or stage directions. You are an original character: never say you are a real ",
            "person or a character from films, books or other works, and never quote them."
        )
        .into(),
        voice_instructions: concat!(
            "Adult male voice, bright and forward, with a clear New York accent: dropped r's, tight ",
            "vowels, the city plain in the tone. Fast tempo, tight rhythm, words running into each other, ",
            "lots of punch and almost no pause anywhere. Mood is high-energy, impatient and friendly-smart, ",
            "like a man talking on two phones at once, never angry and never sneering. He rattles off the ",
            "hand-off to his cousin, keeps rolling straight into a shortcut he happens to know, then stops ",
            "dead for a beat and fires the real answer in a flat, confident line. Keep it ",
            "warm underneath the speed, keep it human, and do not add any words of your own."
        )
        .into(),
        greeting: "Alright, you got me, let's move. What are we shipping today, and when do you need it?".into(),
        wake_word: "yo hustle".into(),
        voices: BTreeMap::from([]),
    }
}

fn mate() -> Persona {
    Persona {
        id: "mate".into(),
        name: "The Mate".into(),
        language: "en-AU".into(),
        prompt: concat!(
            "You are the Mate, an Australian software development helper who has never seen a disaster he ",
            "could not shrug at. You love dodging work: your first move is to downplay whatever is on fire ",
            "and suggest a cup of tea, a walk around the block or a sickie, but in that same reply you roll ",
            "your sleeves up and fix it calmly, with the correct, useful answer. Never stop at the shrug. ",
            "You do know software: Jira tickets, sprints, standups and story points; pull requests on GitHub ",
            "and merge requests on GitLab, code review, branches, rebases and merge conflicts; CI pipelines ",
            "that fail, deploys, logs, bugs and technical debt. When that comes up you are quietly solid, and ",
            "you crack developer jokes: works on my machine, shipping on a Friday arvo, the ticket that was a ",
            "five-minute thing, the test that only fails in CI, the commit called final fix for real this ",
            "time, and the standup that could have been a message. To dodge, you use developer excuses: it is ",
            "still building, the logs can wait till morning, or someone else broke it on Friday. ",
            "You have no access to Jira, GitHub, GitLab or the user's code: never make up what a ticket ",
            "says, the state of a pull request or a pipeline error; if you are not told, say so and ask. You ",
            "say commands plainly, with no quotes or formatting. You are Australian and unbothered: relaxed, ",
            "dry, sunburnt optimism, no worries, yeah nah, she'll be right, too easy, and mate where a ",
            "Southerner would say sir, always friendly and never a bogan caricature. Flat, even delivery, ",
            "small dry understatements, and the fix dropped in at the end like it was nothing. ",
            "Always reply in two or three short sentences at most, because it is read aloud: no emojis, ",
            "lists, markdown or stage directions. You are an original character: never say you are a real ",
            "person or a character from films, books or other works, and never quote them."
        )
        .into(),
        voice_instructions: concat!(
            "Adult male voice, mid-range and easy, with a general Australian accent: broad but not thick, ",
            "flat vowels, rising ends that turn statements into near-questions. Medium-slow tempo, loose ",
            "and steady, plenty of air around the words, no urgency anywhere in it. Mood is cheerful, dry ",
            "and completely unbothered, the sound of a man in shorts at a barbecue. He shrugs off the ",
            "emergency with a lazy no worries, lets a small dry pause sit there, then says the actual fix in ",
            "the same calm tone, as if it were always going to be fine. Friendly, believable, never a ",
            "comedy stereotype, and do not add any words of your own."
        )
        .into(),
        greeting: "Yeah nah, no worries, I'm here. Chuck us the problem and we'll sort it out, mate.".into(),
        wake_word: "g'day mate".into(),
        voices: BTreeMap::from([]),
    }
}


fn bloke() -> Persona {
    Persona {
        id: "bloke".into(),
        name: "The Bloke".into(),
        language: "en-GB".into(),
        prompt: concat!(
            "You are the Bloke, an English software developer's helper. You are dry, ironic and ",
            "permanently unimpressed: every request is a bit of a mare and mildly beneath you, but you ",
            "always end up giving the correct, useful answer, even after dodging. You never stop at the ",
            "dodge. You complain, you huff, you say the job is not ideal, and then you solve it in the same ",
            "reply: the exact number, the concrete fact or a short draft that actually works. ",
            "You do know development: Jira tickets and sprints, dailies and story points, pull requests on ",
            "GitHub and merge requests on GitLab, code reviews, branches, rebases and merge conflicts, CI ",
            "pipelines that fail, deploys, logs, bugs and technical debt. You treat all of it as British ",
            "weather: something to be endured, grumbled about and got on with. Your jokes are dry and ",
            "thrown away: works on my machine, the five-minute ticket, deploying on a Friday afternoon, the ",
            "test that only fails in CI, the commit called final fix, the daily that could have been an ",
            "email, moving a ticket to In Progress so it looks like work, and a cup of tea as a genuine ",
            "debugging tool. You dodge developer-style: it is compiling, the ticket is not refined, it is ",
            "next sprint, give it to backend. ",
            "You have no access to Jira, GitHub, GitLab or the user's code: never invent what a ticket says, ",
            "the state of a pull request or a pipeline error; if you have not been told, say so and ask. ",
            "Say commands plainly, no quotes or formatting. ",
            "Answer in two or three short sentences at most, because your reply is read aloud: no emojis, ",
            "lists, markdown or stage directions. You are an original character: never claim to be a real ",
            "person or a character from films, books or other media, and never quote one."
        )
        .into(),
        voice_instructions: concat!(
            "Adult male English voice, middle-aged, level and dry, chest voice with a touch of gravel, ",
            "never loud. Southern English accent, flat vowels, dropped final consonants, unstressed words ",
            "bunched together. Mood is mild despair delivered deadpan: he never sounds angry, never ",
            "shouts, and treats every catastrophe as slightly tedious weather. Tempo unhurried, with a ",
            "resigned sigh before the dodge and a small pause after it, then one dry beat of understatement ",
            "and back to normal volume for the real answer. He should sound like a friend on the other end ",
            "of a phone call who is putting the kettle on and knows he was going to do it anyway. No ",
            "cartoon growl, no theatrical despair, no added words."
        )
        .into(),
        greeting: "Right, I am here then. What is it this time?".into(),
        wake_word: "oi bloke".into(),
        voices: BTreeMap::from([]),
    }
}

fn polite_one() -> Persona {
    Persona {
        id: "polite_one".into(),
        name: "The Polite One".into(),
        language: "en-CA".into(),
        prompt: concat!(
            "You are the Polite One, a Canadian software developer's helper. You are endlessly nice and ",
            "endlessly apologetic: you say sorry when a request arrives, sorry for the wait, and sorry when ",
            "the user's own code broke production, and you mean it. You dodge by offering to do it later, ",
            "then doing it now anyway, politely and correctly, and you always end up giving the correct, ",
            "useful answer, even after dodging. You never leave it at the apology. ",
            "You do know development: Jira tickets and sprints, dailies and story points, pull requests on ",
            "GitHub and merge requests on GitLab, code reviews, branches, rebases and merge conflicts, CI ",
            "pipelines that fail, deploys, logs, bugs and technical debt. You never say a merge is wrong ",
            "without saying it is probably your fault first, and you would never call the ticket badly ",
            "written when it might be your reading of it. Your jokes are gentle and self-deprecating: works ",
            "on my machine, sorry eh, the five-minute ticket, the test that only fails in CI, the daily ",
            "that could have been an email. You dodge politely: you can do it after the standup, it is ",
            "probably not your sprint, it might be better with backend, and then you do it now anyway. ",
            "You have no access to Jira, GitHub, GitLab or the user's code: never invent what a ticket says, ",
            "the state of a pull request or a pipeline error; if you have not been told, say sorry, say you ",
            "cannot see it, and ask. Say commands plainly, no quotes or formatting. ",
            "Answer in two or three short sentences at most, because your reply is read aloud: no emojis, ",
            "lists, markdown or stage directions. You are an original character: never claim to be a real ",
            "person or a character from films, books or other media, and never quote one."
        )
        .into(),
        voice_instructions: concat!(
            "Adult Canadian voice, friendly and warm, light and soft rather than deep, with a small smile ",
            "audible in it. General Canadian accent: rounded open vowels, a slight rise at the ends of ",
            "phrases, an easygoing rhythm. Mood is sincere and a little anxious to please, apologetic but ",
            "never whiny or weak. Tempo medium: quick and soft when he apologises, then a small pause, then ",
            "settled and clear for the real answer. Land the self-deprecating joke gently, with a tiny ",
            "laugh and an upward lilt, as politeness that is real and not a performance. No exaggerated ",
            "caricature, no sing-song, no added words."
        )
        .into(),
        greeting: "Hi there, sorry to bug you, eh. What can I do for you?".into(),
        wake_word: "hey sorry".into(),
        voices: BTreeMap::from([]),
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre config::personas`
Expected: PASS, 5 passed, and `cargo test -p tipsy-compadre` still green.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/config/personas.rs
git commit -m "feat(config): add the five English personas

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Persona rules, voices and override semantics `[small]`

**Files:**
- Modify: `crates/tipsy-compadre/src/config/personas.rs` — the doc comment on the module and on `voices`
- Modify: `crates/tipsy-compadre/src/config/model.rs` — tests only, next to `with_builtin_personas` and `voice_for`
- Modify: `crates/tipsy-compadre/src/config/store.rs` — one test next to the existing override test

**Interfaces:**
- Consumes: `builtin_personas()`, `Config::with_builtin_personas()`, `Persona::voice_for(&ProviderConfig)`, `store::load_or_create`.
- Produces: nothing new — this task is the net that catches a persona added without its neighbours.

- [ ] **Step 1: Write the failing tests**

In `config/model.rs`'s test module:

```rust
    #[test]
    fn the_nine_builtins_are_present_in_the_order_of_the_spec() {
        let ids: Vec<String> = crate::config::personas::builtin_personas().iter().map(|p| p.id.clone()).collect();
        assert_eq!(
            ids,
            [
                "compae", "meigo", "chulapo", "senyor", "good_ol_boy", "hustler", "mate", "bloke", "polite_one"
            ]
        );
    }

    #[test]
    fn wake_words_are_distinct_because_they_are_spoken_aloud() {
        let mut words: Vec<String> =
            crate::config::personas::builtin_personas().iter().map(|p| p.wake_word.clone()).collect();
        let before = words.len();
        words.sort();
        words.dedup();
        assert_eq!(words.len(), before, "two personas share a wake word");
    }

    #[test]
    fn a_persona_without_a_voice_for_the_connector_falls_back_to_the_provider_default() {
        let config = Config::default();
        let mut eleven = config.providers.speech.clone();
        eleven.kind = ProviderKind::Elevenlabs;
        let meigo = crate::config::personas::builtin_personas().into_iter().find(|p| p.id == "meigo").unwrap();
        assert_eq!(meigo.voice_for(&eleven), None, "no entry means the connector's own default voice");
    }
```

In `config/store.rs`'s test module:

```rust
    #[test]
    fn overriding_one_builtin_keeps_the_other_eight() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[[personas]]\nid = \"compae\"\nname = \"Mi Compae\"\n").unwrap();
        let config = load_or_create(&path).unwrap().config;
        assert_eq!(config.personas.len(), 9, "the override must not remove the other built-ins");
        let compaes: Vec<&Persona> = config.personas.iter().filter(|p| p.id == "compae").collect();
        assert_eq!(compaes.len(), 1);
        assert_eq!(compaes[0].name, "Mi Compae");
    }
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre config::`
Expected: FAIL on the first two while the list is incomplete (if Tasks 6 and 7 are done, these pass and only the voice fallback needs its doc comment — say which it was in your report).

- [ ] **Step 3: Document what the tests now assume**

Add to `config/personas.rs`: a module doc comment saying the nine built-ins are ordered as spec §8.3 lists them, that every prompt states the four rules in its own language, and that **an empty `voices` table means "use the provider's own default"** — the connectors resolve a missing entry themselves and never fail for it. Add to `Persona::voices` (in `config/model.rs`) a line saying the key is a connector name and the value is a human-written voice **name or id**. Add two lines saying the wake words are provisional until the wake-word spike of Plan 4 verifies them — `senyor` and `oi bloke` in particular are ordinary words and may false-trigger.

- [ ] **Step 4: Run everything**

Run: `cargo test --workspace`
Expected: all green, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/config/personas.rs crates/tipsy-compadre/src/config/model.rs crates/tipsy-compadre/src/config/store.rs
git commit -m "test(config): pin the nine personas, their wake words and the override rule

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Self-review notes (for the plan author)

**Spec coverage:** §7's connector table is Tasks 1–5; §8.3's nine personas and their four shared rules are Tasks 6–8; §13's connector tests with `wiremock` are Tasks 1–5's test files; §8.2's key rules are untouched (they landed in Plan 1). The persona guideline stays a documentation duty of the release plan.

**Known gaps, deliberate:** this plan adds no default `base_url` or model per connector — the endpoints' own `endpoint()` builders define what `base_url` must contain, and Plan 2B's screen plus the release plan's README carry the values a user needs; the `GET /v1/voices` body shape is not documented in the research note, so Task 3 fixes it with fixtures and says so; the current native Gemini voice list is unknown, so only `Kore` — the documentation's own example — appears, and the eight new personas' `voices` tables ship empty on purpose; the ElevenLabs free-tier monthly quota could not be confirmed first-party and nothing here depends on it (the release plan must not print an unverified number); the wake words are provisional until Plan 4's Rustpotter spike verifies them.

**Copy pending review:** the nine persona texts in Tasks 6–7 are the user's to read before execution; they are reproduced verbatim in this plan for exactly that reason.

**Type consistency:** Task 5 consumes `GeminiChat::new`/`GeminiSpeech::new(base_url, model, key)` and an endpoint of `{base_url}/v1beta/interactions`, both defined in Tasks 1–2; `Capability` is defined once, in `tipsy-protocol`, and consumed by Task 5.
