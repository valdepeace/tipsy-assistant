# Plan 1 — Terminal core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A working terminal assistant: the user types (or holds a push-to-talk turn) and El Compae answers with streamed text and voice through OpenAI-compatible cloud providers, in the ported HUD.

**Architecture:** Cargo workspace with `tipsy-protocol` (messages), `tipsy-ui` (backend-agnostic ratatui HUD) and `tipsy-compadre` (server actors + native client in one binary). Server and client talk through an in-process channel carrying the protocol types; the WebSocket transport comes in a later plan.

**Tech Stack:** Rust 1.96.1 (edition 2024), tokio 1.53, tokio-util 0.7, reqwest 0.13, serde 1, toml 1.1, keyring 4.2, dirs 7, symphonia 0.5, hound 3.5, ratatui 0.30, rodio 0.22, cpal 0.18, sysinfo 0.39, clap 4.6, wiremock 0.6.

**Spec:** `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md` (milestones 1–4). Diagrams: `docs/architecture.md`.

**Model routing (user requirement):** each task header carries `[haiku]` or `[sonnet]`. Haiku is the default. Sonnet only implements the tasks marked so; the **tests for those tasks are written beforehand by Haiku** in their own `[haiku]` task. **No per-task code review**: a single Sonnet review of the whole branch runs after the last task (Task 18).

**Later plans (not in this one):** Plan 2 — configuration screen, all nine personas, Gemini and ElevenLabs connectors, `auto` mode (VAD). Plan 3 — WebSocket server, `--connect`, web terminal, web client (after the Ratzilla spike). Plan 4 — `hands_free` (wake word), CI, `cargo xtask dist`, release.

## Global Constraints

- Toolchain pinned with `rust-toolchain.toml` to `1.96.1`; every crate uses `edition = "2024"`.
- All code, identifiers, comments, commit messages and docs are in **English**. Persona texts are in the persona's own language (El Compae: Spanish).
- `tipsy-protocol` and `tipsy-ui` must NOT depend on `tokio`, `cpal`, `rodio`, `keyring`, `sysinfo`, `reqwest` or `crossterm` (they must compile to `wasm32-unknown-unknown` in a later plan). `tipsy-ui` must not use `std::time::Instant` (it panics on wasm); time flows in through `tick(dt_ms)`.
- API keys never appear in `config.toml`, logs, events or test fixtures committed to git. Config stores only a key **name** (e.g. `"openrouter"`).
- Key resolution order: environment variable `TIPSY_KEY_<NAME>` (name uppercased, non-alphanumerics → `_`), then the OS credential store (service `tipsy-compadre`, account `<name>`).
- Default `[server] bind = "127.0.0.1:7070"`, `lan = false`.
- Default providers: chat `openai_compat` / `https://openrouter.ai/api/v1` / `google/gemini-2.5-flash`; speech same URL / `google/gemini-3.8-flash-tts`; transcribe same URL / `openai/whisper-large-v3`; key name `openrouter` for all three.
- Sentence splitter: a sentence ends at `.`, `?`, `!` or `…` followed by whitespace (or end of stream) and must hold at least 25 characters; shorter pieces are merged with the next one.
- Timeouts: 30 s per chat request, 30 s per sentence of speech, 60 s per transcription. One retry with backoff on `RateLimited`.
- Mic audio: mono, 16 kHz. Speech audio to clients: mono `i16` PCM at the provider's sample rate.
- **Windows build note:** in Git Bash, remove `/usr/bin` from `PATH` before `cargo` (Git's `link.exe` shadows MSVC's). Use: `export PATH="$(echo "$PATH" | tr ':' '\n' | grep -v '^/usr/bin$' | paste -sd:)"`. In PowerShell no change is needed. All `cargo` commands below assume the repository root `E:\workspace\personal\tipsy-compadre` as working directory.
- Every commit message ends with the line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Provider HTTP errors** (401, 403, 429, 5xx with a JSON body) must map to `ProviderError::Auth` / `RateLimited` / `BadResponse` carrying the provider's message, never a panic or an empty string — test in Task 8.
2. **LLM stream that ends without final punctuation** ("…y ya está") must still be spoken: the splitter flushes the remainder at end of stream — test in Task 3.
3. **Text that cleans to nothing** (only emojis, only Markdown fences) must not trigger a speech request and must not break sequence numbers — tests in Task 3 and Task 12.
4. **Old or partial `config.toml`** (missing sections or fields) must load with defaults filled in, and the user's own personas must survive alongside the built-ins — test in Task 5.
5. **Non-ASCII text** (`ñ`, `¿`, `…`, accented vowels, emojis) must never panic on byte slicing in the splitter, the cleaner or the key redactor — tests in Tasks 3 and 7.

---

## File Structure

```
tipsy-compadre/
├── Cargo.toml                         workspace
├── rust-toolchain.toml
├── crates/
│   ├── tipsy-protocol/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                 Command, Event, Frame, ToServer/ToClient, enums
│   ├── tipsy-ui/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                 re-exports
│   │       ├── strings.rs             es/en string tables
│   │       ├── state.rs               UiState, LogEntry, KeyInput, UiAction, reducers
│   │       └── render.rs              HUD drawing + click hit areas
│   └── tipsy-compadre/
│       ├── Cargo.toml
│       ├── src/
│       │   ├── main.rs                CLI (clap) + run modes
│       │   ├── lib.rs                 module tree (so tests can use the crate)
│       │   ├── text.rs                SentenceSplitter, clean_for_speech
│       │   ├── audio/
│       │   │   ├── mod.rs
│       │   │   ├── pcm.rs             Pcm, conversions, envelope, resample, wav, decode
│       │   │   └── device.rs          cpal mic capture + rodio player (native only)
│       │   ├── config/
│       │   │   ├── mod.rs
│       │   │   ├── model.rs           Config structs + defaults
│       │   │   ├── personas.rs        built-in personas
│       │   │   └── store.rs           paths, load/save, corrupt file handling
│       │   ├── keys.rs                SecretStore, OsKeyring, MemoryStore, KeyResolver, redact
│       │   ├── providers/
│       │   │   ├── mod.rs             traits, requests, ProviderError, build_providers
│       │   │   ├── sse.rs             SseParser
│       │   │   └── openai_compat.rs   chat stream, speech, transcription
│       │   ├── brain.rs               Brain::run_turn
│       │   ├── session.rs             in-process session actor
│       │   ├── sysmon.rs              Stats producer
│       │   └── client.rs              native terminal client loop
│       └── tests/
│           ├── protocol_roundtrip.rs  (lives in tipsy-protocol/tests instead, see Task 2)
│           ├── openai_compat.rs
│           └── brain.rs
└── README.md
```

---

### Task 1: Workspace scaffold `[haiku]`

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`
- Create: `crates/tipsy-protocol/Cargo.toml`, `crates/tipsy-protocol/src/lib.rs`
- Create: `crates/tipsy-ui/Cargo.toml`, `crates/tipsy-ui/src/lib.rs`
- Create: `crates/tipsy-compadre/Cargo.toml`, `crates/tipsy-compadre/src/lib.rs`, `crates/tipsy-compadre/src/main.rs`
- Modify: `.gitignore` (already ignores `target/`, keep it)

**Interfaces:**
- Consumes: nothing.
- Produces: a workspace where `cargo build` and `cargo test` succeed with three crates.

- [ ] **Step 1: Create the workspace manifest** `Cargo.toml`

```toml
[workspace]
resolver = "3"
members = ["crates/tipsy-protocol", "crates/tipsy-ui", "crates/tipsy-compadre"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.96"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
ratatui = { version = "0.30", default-features = false, features = ["std", "layout-cache", "all-widgets"] }
tipsy-protocol = { path = "crates/tipsy-protocol" }
tipsy-ui = { path = "crates/tipsy-ui" }
```

- [ ] **Step 2: Pin the toolchain** `rust-toolchain.toml`

```toml
[toolchain]
channel = "1.96.1"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: Create `crates/tipsy-protocol/Cargo.toml`**

```toml
[package]
name = "tipsy-protocol"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "Messages exchanged between the tipsy-compadre server and its clients"

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
```

`crates/tipsy-protocol/src/lib.rs`:

```rust
//! Messages exchanged between the tipsy-compadre server and its clients.
```

- [ ] **Step 4: Create `crates/tipsy-ui/Cargo.toml`**

```toml
[package]
name = "tipsy-ui"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "Backend-agnostic ratatui HUD for tipsy-compadre"

[dependencies]
tipsy-protocol.workspace = true
ratatui.workspace = true
```

`crates/tipsy-ui/src/lib.rs`:

```rust
//! Backend-agnostic ratatui HUD for tipsy-compadre.
```

- [ ] **Step 5: Create `crates/tipsy-compadre/Cargo.toml`**

```toml
[package]
name = "tipsy-compadre"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "A tipsy, lazy, joking voice assistant for software developers"

[dependencies]
tipsy-protocol.workspace = true
tipsy-ui.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
ratatui = { version = "0.30" }
tokio = { version = "1.53", features = ["full"] }
tokio-util = "0.7"
futures = "0.3"
async-trait = "0.1"
reqwest = { version = "0.13", features = ["json", "stream", "multipart"] }
toml = "1.1"
dirs = "7"
keyring = "4.2"
symphonia = { version = "0.5", default-features = false, features = ["mp3", "wav", "pcm"] }
hound = "3.5"
rodio = { version = "0.22", default-features = false, features = ["playback"] }
cpal = "0.18"
sysinfo = "0.39"
chrono = "0.4"
clap = { version = "4.6", features = ["derive"] }

[dev-dependencies]
wiremock = "0.6"
tempfile = "3.27"
```

`crates/tipsy-compadre/src/lib.rs`:

```rust
//! tipsy-compadre: server actors and the native terminal client.
```

`crates/tipsy-compadre/src/main.rs`:

```rust
fn main() {
    println!("tipsy-compadre {}", env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 6: Build and test**

Run: `cargo build` then `cargo test`
Expected: both finish with no errors (`test result: ok. 0 passed` lines). If `rodio` rejects the `playback` feature, run `cargo add rodio@0.22 -p tipsy-compadre` (default features) and rebuild.

Run: `cargo run -p tipsy-compadre`
Expected: `tipsy-compadre 0.1.0`

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml rust-toolchain.toml crates Cargo.lock
git commit -m "chore: scaffold cargo workspace with protocol, ui and server crates

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Protocol types `[haiku]`

**Files:**
- Modify: `crates/tipsy-protocol/src/lib.rs`
- Test: `crates/tipsy-protocol/tests/roundtrip.rs`

**Interfaces:**
- Consumes: nothing.
- Produces (used by every later task):
  - `pub const PROTOCOL_VERSION: u32 = 1;`
  - `enum ClientKind { Tui, Web, WebTerminal }`
  - `enum ListeningMode { Push, Auto, HandsFree }` (serde: `"push"`, `"auto"`, `"hands_free"`)
  - `enum AssistantState { Idle, Listening, Transcribing, Thinking, Speaking }`
  - `enum UiLanguage { Es, En }` with `UiLanguage::from_bcp47(&str) -> UiLanguage`
  - `enum ErrorCode { Auth, RateLimited, Timeout, Network, BadResponse, NoKey, Mic, Config, NothingHeard, Internal }`
  - `struct PersonaView { id: String, name: String, language: String }`
  - `enum Command { Hello{..}, Ask{text}, Say{text}, ListenStart, ListenStop, Cancel, Reset, SetPersona{id}, SetListeningMode{mode} }`
  - `enum Event { Welcome{..}, Status{state, detail}, Heard{text}, ReplyDelta{text}, Reply{text}, SpeechChunk{seq, sample_rate}, Done{first_audio_ms, total_ms}, Stats{cpu, ram, net_in, net_out}, Error{code, message, recoverable} }`
  - `enum Frame { Mic(Vec<i16>), Speech{seq: u32, samples: Vec<i16>} }` with `encode(&self) -> Vec<u8>` and `Frame::decode(&[u8]) -> Result<Frame, FrameError>`
  - `enum ToServer { Command(Command), Audio(Frame) }`, `enum ToClient { Event(Event), Audio(Frame) }`

Note: two deliberate simplifications from spec §4.2: `SpeechChunk` has no `last` flag (the end of a reply is signalled by `Done`), and `Done.first_audio_ms` is `Option<u64>` (a reply may produce no audio).

- [ ] **Step 1: Write the failing test** `crates/tipsy-protocol/tests/roundtrip.rs`

```rust
use tipsy_protocol::*;

fn roundtrip_command(c: Command) {
    let json = serde_json::to_string(&c).unwrap();
    let back: Command = serde_json::from_str(&json).unwrap();
    assert_eq!(c, back, "json was {json}");
}

fn roundtrip_event(e: Event) {
    let json = serde_json::to_string(&e).unwrap();
    let back: Event = serde_json::from_str(&json).unwrap();
    assert_eq!(e, back, "json was {json}");
}

#[test]
fn commands_roundtrip() {
    roundtrip_command(Command::Hello { token: Some("t".into()), client: ClientKind::Tui, protocol_version: PROTOCOL_VERSION });
    roundtrip_command(Command::Ask { text: "¿Qué pasa, compae? ñandú…".into() });
    roundtrip_command(Command::Say { text: "hola".into() });
    roundtrip_command(Command::ListenStart);
    roundtrip_command(Command::ListenStop);
    roundtrip_command(Command::Cancel);
    roundtrip_command(Command::Reset);
    roundtrip_command(Command::SetPersona { id: "compae".into() });
    roundtrip_command(Command::SetListeningMode { mode: ListeningMode::HandsFree });
}

#[test]
fn events_roundtrip() {
    roundtrip_event(Event::Welcome {
        session_id: 7,
        persona: PersonaView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: UiLanguage::Es,
        listening_mode: ListeningMode::Push,
        web_bundle: false,
    });
    roundtrip_event(Event::Status { state: AssistantState::Thinking, detail: None });
    roundtrip_event(Event::Heard { text: "oye".into() });
    roundtrip_event(Event::ReplyDelta { text: "Ozú".into() });
    roundtrip_event(Event::Reply { text: "Ozú, quillo.".into() });
    roundtrip_event(Event::SpeechChunk { seq: 3, sample_rate: 24000 });
    roundtrip_event(Event::Done { first_audio_ms: Some(900), total_ms: 4000 });
    roundtrip_event(Event::Done { first_audio_ms: None, total_ms: 10 });
    roundtrip_event(Event::Stats { cpu: 12.5, ram: 40.0, net_in: 1024.0, net_out: 2.0 });
    roundtrip_event(Event::Error { code: ErrorCode::RateLimited, message: "slow down".into(), recoverable: true });
}

#[test]
fn json_shape_is_tagged_snake_case() {
    let json = serde_json::to_string(&Command::SetListeningMode { mode: ListeningMode::HandsFree }).unwrap();
    assert_eq!(json, r#"{"type":"set_listening_mode","mode":"hands_free"}"#);
    let json = serde_json::to_string(&Command::ListenStart).unwrap();
    assert_eq!(json, r#"{"type":"listen_start"}"#);
}

#[test]
fn frames_roundtrip() {
    let mic = Frame::Mic(vec![0, 1, -1, i16::MAX, i16::MIN]);
    assert_eq!(Frame::decode(&mic.encode()).unwrap(), mic);
    let speech = Frame::Speech { seq: 42, samples: vec![5, -5] };
    let bytes = speech.encode();
    assert_eq!(bytes[0], 0x02);
    assert_eq!(&bytes[1..5], &42u32.to_le_bytes());
    assert_eq!(Frame::decode(&bytes).unwrap(), speech);
}

#[test]
fn frames_reject_garbage() {
    assert!(Frame::decode(&[]).is_err());
    assert!(Frame::decode(&[0x09, 0, 0]).is_err());
    assert!(Frame::decode(&[0x01, 0x00]).is_err(), "mic payload must have an even byte count");
    assert_eq!(Frame::decode(&[0x01]).unwrap(), Frame::Mic(vec![]));
}

#[test]
fn language_mapping() {
    assert_eq!(UiLanguage::from_bcp47("es-ES"), UiLanguage::Es);
    assert_eq!(UiLanguage::from_bcp47("es"), UiLanguage::Es);
    assert_eq!(UiLanguage::from_bcp47("ES-mx"), UiLanguage::Es);
    assert_eq!(UiLanguage::from_bcp47("en-AU"), UiLanguage::En);
    assert_eq!(UiLanguage::from_bcp47(""), UiLanguage::En);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tipsy-protocol`
Expected: compile errors such as `cannot find type Command in this scope`.

- [ ] **Step 3: Implement** `crates/tipsy-protocol/src/lib.rs`

```rust
//! Messages exchanged between the tipsy-compadre server and its clients.
//!
//! Text messages are JSON (`Command`, `Event`); audio travels as binary `Frame`s.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Tui,
    Web,
    WebTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListeningMode {
    #[default]
    Push,
    Auto,
    HandsFree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantState {
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiLanguage {
    Es,
    En,
}

impl UiLanguage {
    /// Spanish for any `es` / `es-*` tag, English for everything else.
    pub fn from_bcp47(tag: &str) -> Self {
        let primary = tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        if primary == "es" { UiLanguage::Es } else { UiLanguage::En }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Auth,
    RateLimited,
    Timeout,
    Network,
    BadResponse,
    NoKey,
    Mic,
    Config,
    NothingHeard,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaView {
    pub id: String,
    pub name: String,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Hello { token: Option<String>, client: ClientKind, protocol_version: u32 },
    Ask { text: String },
    Say { text: String },
    ListenStart,
    ListenStop,
    Cancel,
    Reset,
    SetPersona { id: String },
    SetListeningMode { mode: ListeningMode },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Welcome {
        session_id: u64,
        persona: PersonaView,
        ui_language: UiLanguage,
        listening_mode: ListeningMode,
        web_bundle: bool,
    },
    Status { state: AssistantState, detail: Option<String> },
    Heard { text: String },
    ReplyDelta { text: String },
    Reply { text: String },
    /// Header for the `Frame::Speech` with the same `seq` that follows it.
    SpeechChunk { seq: u32, sample_rate: u32 },
    Done { first_audio_ms: Option<u64>, total_ms: u64 },
    Stats { cpu: f32, ram: f32, net_in: f64, net_out: f64 },
    Error { code: ErrorCode, message: String, recoverable: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Mono 16 kHz PCM from a client microphone.
    Mic(Vec<i16>),
    /// Mono PCM of one spoken sentence, at the rate announced by `Event::SpeechChunk`.
    Speech { seq: u32, samples: Vec<i16> },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("empty frame")]
    Empty,
    #[error("unknown frame kind {0:#04x}")]
    UnknownKind(u8),
    #[error("malformed frame payload")]
    Malformed,
}

const KIND_MIC: u8 = 0x01;
const KIND_SPEECH: u8 = 0x02;

fn samples_to_le(samples: &[i16], out: &mut Vec<u8>) {
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
}

fn samples_from_le(bytes: &[u8]) -> Result<Vec<i16>, FrameError> {
    if bytes.len() % 2 != 0 {
        return Err(FrameError::Malformed);
    }
    Ok(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Frame::Mic(samples) => {
                let mut out = Vec::with_capacity(1 + samples.len() * 2);
                out.push(KIND_MIC);
                samples_to_le(samples, &mut out);
                out
            }
            Frame::Speech { seq, samples } => {
                let mut out = Vec::with_capacity(5 + samples.len() * 2);
                out.push(KIND_SPEECH);
                out.extend_from_slice(&seq.to_le_bytes());
                samples_to_le(samples, &mut out);
                out
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Frame, FrameError> {
        let (&kind, rest) = bytes.split_first().ok_or(FrameError::Empty)?;
        match kind {
            KIND_MIC => Ok(Frame::Mic(samples_from_le(rest)?)),
            KIND_SPEECH => {
                if rest.len() < 4 {
                    return Err(FrameError::Malformed);
                }
                let seq = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
                Ok(Frame::Speech { seq, samples: samples_from_le(&rest[4..])? })
            }
            other => Err(FrameError::UnknownKind(other)),
        }
    }
}

/// Anything a client sends to the server.
#[derive(Debug, Clone, PartialEq)]
pub enum ToServer {
    Command(Command),
    Audio(Frame),
}

/// Anything the server sends to a client.
#[derive(Debug, Clone, PartialEq)]
pub enum ToClient {
    Event(Event),
    Audio(Frame),
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p tipsy-protocol`
Expected: `test result: ok. 6 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-protocol
git commit -m "feat(protocol): add commands, events and audio frames

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Sentence splitter and speech text cleaning `[haiku]`

**Files:**
- Create: `crates/tipsy-compadre/src/text.rs`
- Modify: `crates/tipsy-compadre/src/lib.rs` (add `pub mod text;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub struct SentenceSplitter` with `new() -> Self`, `push(&mut self, text: &str) -> Vec<String>` (complete sentences ready to speak) and `finish(&mut self) -> Option<String>` (the remainder at end of stream, trimmed, `None` if blank).
  - `pub const MIN_SENTENCE_CHARS: usize = 25;`
  - `pub fn clean_for_speech(text: &str) -> String` — removes apostrophes glued after a letter (`pa'` → `pa`, also `’`) and Markdown markers `` ` `` `*` `_` `#` without leaving a gap, replaces emojis and pictographs with a space, and collapses whitespace.
  - `pub fn is_speakable(text: &str) -> bool` — `true` if the text holds at least one letter or digit (a sentence made only of punctuation is not sent to a speech provider).

- [ ] **Step 1: Write the failing tests** at the bottom of the new file `crates/tipsy-compadre/src/text.rs` (the implementation goes above them in Step 3; for now create the file with only this test module plus empty stubs so it compiles to failing tests):

```rust
//! Text helpers for speech: sentence splitting of a streamed reply and cleaning before synthesis.

pub const MIN_SENTENCE_CHARS: usize = 25;

pub struct SentenceSplitter {
    buf: String,
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self { buf: String::new() }
    }
    pub fn push(&mut self, _text: &str) -> Vec<String> {
        Vec::new()
    }
    pub fn finish(&mut self) -> Option<String> {
        None
    }
}

pub fn clean_for_speech(_text: &str) -> String {
    String::new()
}

pub fn is_speakable(_text: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split_all(chunks: &[&str]) -> Vec<String> {
        let mut s = SentenceSplitter::new();
        let mut out = Vec::new();
        for c in chunks {
            out.extend(s.push(c));
        }
        out.extend(s.finish());
        out
    }

    #[test]
    fn splits_on_sentence_end_followed_by_space() {
        let out = split_all(&["Ozú, quillo, esto es una frase larga. Y esta otra ", "también es bastante larga, compadre."]);
        assert_eq!(out, vec![
            "Ozú, quillo, esto es una frase larga.".to_string(),
            "Y esta otra también es bastante larga, compadre.".to_string(),
        ]);
    }

    #[test]
    fn merges_short_sentences_with_the_next_one() {
        let out = split_all(&["¡Ja! Ozú. Los lunes son un castigo divino, illo. Fin."]);
        assert_eq!(out, vec![
            "¡Ja! Ozú. Los lunes son un castigo divino, illo.".to_string(),
            "Fin.".to_string(),
        ]);
    }

    #[test]
    fn handles_question_exclamation_and_ellipsis() {
        let out = split_all(&["¿Tú has visto el pipeline en rojo otra vez? ", "Pues mira… ", "yo estaba compilando, de verdad te lo digo… y ya."]);
        assert_eq!(out, vec![
            "¿Tú has visto el pipeline en rojo otra vez?".to_string(),
            "Pues mira… yo estaba compilando, de verdad te lo digo…".to_string(),
            "y ya.".to_string(),
        ]);
    }

    #[test]
    fn does_not_split_without_trailing_whitespace_until_finish() {
        let mut s = SentenceSplitter::new();
        assert!(s.push("Esta frase larga termina en punto pero sin espacio.").is_empty());
        assert_eq!(s.finish(), Some("Esta frase larga termina en punto pero sin espacio.".to_string()));
    }

    #[test]
    fn flushes_remainder_without_punctuation() {
        let out = split_all(&["Un remate sin punto final y ya está"]);
        assert_eq!(out, vec!["Un remate sin punto final y ya está".to_string()]);
    }

    #[test]
    fn finish_on_blank_is_none() {
        let mut s = SentenceSplitter::new();
        s.push("   \n ");
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn decimal_points_and_versions_do_not_split() {
        let out = split_all(&["Actualiza a la versión 2.5.1 del paquete antes de nada, compadre. Venga."]);
        assert_eq!(out, vec!["Actualiza a la versión 2.5.1 del paquete antes de nada, compadre. Venga.".to_string()]);
    }

    #[test]
    fn multibyte_chunks_never_panic() {
        // Chunk boundaries fall between multi-byte characters' neighbours.
        let out = split_all(&["ñ", "á", "…", " ¿", "é", "? 🍺🍺 ", "ü."]);
        assert_eq!(out.concat().chars().filter(|c| *c == 'ñ').count(), 1);
    }

    #[test]
    fn cleans_apostrophes_markdown_and_emojis() {
        assert_eq!(clean_for_speech("pa' ná', la verdá"), "pa ná, la verdá");
        assert_eq!(clean_for_speech("pa’l barco"), "pal barco");
        assert_eq!(clean_for_speech("usa `git reset --soft HEAD~1` y **listo**"), "usa git reset --soft HEAD~1 y listo");
        assert_eq!(clean_for_speech("# Título\n_cursiva_"), "Título cursiva");
        assert_eq!(clean_for_speech("Ozú 🍺🙂  quillo"), "Ozú quillo");
        assert_eq!(clean_for_speech("'entre comillas'"), "'entre comillas");
    }

    #[test]
    fn cleaning_to_nothing_gives_empty_string() {
        assert_eq!(clean_for_speech("🍺🙂 ** `` #"), "");
    }

    #[test]
    fn markdown_markers_leave_no_gap_before_punctuation() {
        assert_eq!(clean_for_speech("Eso es pa' ná', **compadre**, de verdad."), "Eso es pa ná, compadre, de verdad.");
    }

    #[test]
    fn speakable_needs_a_letter_or_digit() {
        assert!(is_speakable("Ozú."));
        assert!(is_speakable("42"));
        assert!(!is_speakable("."));
        assert!(!is_speakable("¿?¡! …"));
        assert!(!is_speakable(""));
    }
}
```

Note on the last cleaning assertion: an apostrophe is removed only when it directly follows a letter, so the leading quote of `'entre comillas'` stays and the closing one (after `s`) is removed. That is the intended, documented behaviour.

Add to `crates/tipsy-compadre/src/lib.rs`:

```rust
pub mod text;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tipsy-compadre text::`
Expected: most tests FAIL (empty outputs).

- [ ] **Step 3: Implement** — replace the stubs above the test module with:

```rust
//! Text helpers for speech: sentence splitting of a streamed reply and cleaning before synthesis.

/// A sentence shorter than this is merged with the following one, so the speech provider is
/// never asked to voice a lonely "¡Ja!".
pub const MIN_SENTENCE_CHARS: usize = 25;

/// Accumulates streamed LLM text and yields complete sentences.
#[derive(Default)]
pub struct SentenceSplitter {
    buf: String,
}

fn is_terminator(c: char) -> bool {
    matches!(c, '.' | '?' | '!' | '…')
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds streamed text and returns every sentence that is now complete.
    pub fn push(&mut self, text: &str) -> Vec<String> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        loop {
            match self.next_cut() {
                Some(cut) => {
                    let sentence: String = self.buf[..cut].trim().to_string();
                    self.buf = self.buf[cut..].trim_start().to_string();
                    if !sentence.is_empty() {
                        out.push(sentence);
                    }
                }
                None => break,
            }
        }
        out
    }

    /// Byte index just after the first terminator run that is followed by whitespace and closes a
    /// sentence of at least `MIN_SENTENCE_CHARS` characters.
    fn next_cut(&self) -> Option<usize> {
        let chars: Vec<(usize, char)> = self.buf.char_indices().collect();
        let mut i = 0;
        while i < chars.len() {
            if is_terminator(chars[i].1) {
                // Extend over a run of terminators ("?!", "...").
                let mut j = i;
                while j + 1 < chars.len() && is_terminator(chars[j + 1].1) {
                    j += 1;
                }
                let end = chars[j].0 + chars[j].1.len_utf8();
                let followed_by_space = j + 1 < chars.len() && chars[j + 1].1.is_whitespace();
                if followed_by_space && self.buf[..end].trim().chars().count() >= MIN_SENTENCE_CHARS {
                    return Some(end);
                }
                i = j + 1;
            } else {
                i += 1;
            }
        }
        None
    }

    /// Returns whatever is left at the end of the stream.
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        let rest = rest.trim();
        if rest.is_empty() { None } else { Some(rest.to_string()) }
    }
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF   // pictographs, emoticons, transport, symbols & pictographs ext.
        | 0x2600..=0x27BF   // misc symbols, dingbats
        | 0xFE0F            // variation selector
        | 0x200D            // zero width joiner
    )
}

/// Prepares a sentence for a speech provider.
pub fn clean_for_speech(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev: Option<char> = None;
    for c in text.chars() {
        let drop = match c {
            '`' | '*' | '_' | '#' => true,
            '\'' | '’' => prev.is_some_and(|p| p.is_alphabetic()),
            c if is_emoji(c) => true,
            _ => false,
        };
        if drop {
            // Emojis behave like a space so the words around them do not glue together;
            // Markdown markers and apostrophes simply vanish.
            if is_emoji(c) {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
        prev = Some(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A sentence made only of punctuation or symbols is not worth a speech request.
pub fn is_speakable(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p tipsy-compadre text::`
Expected: `test result: ok. 12 passed`. If `merges_short_sentences_with_the_next_one` or `handles_question_exclamation_and_ellipsis` fail, re-read `next_cut`: the length check counts characters of the whole candidate (`self.buf[..end]`), which is what merges short pieces.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/text.rs crates/tipsy-compadre/src/lib.rs
git commit -m "feat(text): add streaming sentence splitter and speech cleaning

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: PCM utilities `[haiku]`

**Files:**
- Create: `crates/tipsy-compadre/src/audio/mod.rs`, `crates/tipsy-compadre/src/audio/pcm.rs`
- Modify: `crates/tipsy-compadre/src/lib.rs` (add `pub mod audio;`)

**Interfaces:**
- Consumes: nothing.
- Produces (`crate::audio::pcm`):
  - `#[derive(Debug, Clone, PartialEq)] pub struct Pcm { pub samples: Vec<f32>, pub sample_rate: u32 }` (mono, range −1.0..=1.0)
  - `Pcm::duration_ms(&self) -> u64`
  - `pub fn f32_to_i16(samples: &[f32]) -> Vec<i16>` (clamped) and `pub fn i16_to_f32(samples: &[i16]) -> Vec<f32>`
  - `pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32>`
  - `pub fn envelope(samples: &[f32], sample_rate: u32) -> Vec<f32>` — RMS every 50 ms, normalised so the peak is 1.0 (all zeros stays zeros)
  - `pub fn rms(samples: &[f32]) -> f32`
  - `pub fn encode_wav_16k(pcm: &Pcm) -> Vec<u8>` — resamples to 16 kHz, 16-bit mono WAV
  - `pub fn decode_audio(bytes: Vec<u8>) -> Result<Pcm, String>` — MP3 or WAV → mono `Pcm`
  - `pub fn pcm_s16le_to_pcm(bytes: &[u8], sample_rate: u32) -> Pcm` — raw little-endian 16-bit mono

- [ ] **Step 1: Write the failing tests** — create `crates/tipsy-compadre/src/audio/mod.rs`:

```rust
//! Audio: PCM helpers (portable) and native devices.

pub mod pcm;
```

Create `crates/tipsy-compadre/src/audio/pcm.rs` containing only the tests first (stubs come in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, ms: u32) -> Vec<f32> {
        let n = (rate as u64 * ms as u64 / 1000) as usize;
        (0..n).map(|i| (i as f32 * freq * std::f32::consts::TAU / rate as f32).sin() * 0.5).collect()
    }

    #[test]
    fn i16_conversion_clamps_and_roundtrips() {
        assert_eq!(f32_to_i16(&[0.0, 1.0, -1.0, 2.0, -2.0]), vec![0, 32767, -32767, 32767, -32767]);
        let back = i16_to_f32(&[0, 16384, -16384]);
        assert!((back[1] - 0.5).abs() < 0.001 && (back[2] + 0.5).abs() < 0.001);
    }

    #[test]
    fn resample_changes_length_proportionally() {
        let s = sine(440.0, 48000, 1000);
        assert_eq!(resample_linear(&s, 48000, 16000).len(), 16000);
        assert_eq!(resample_linear(&s, 48000, 48000).len(), 48000);
        assert!(resample_linear(&[], 48000, 16000).is_empty());
    }

    #[test]
    fn envelope_has_one_value_per_50ms_and_peak_one() {
        let mut s = vec![0.0f32; 24000 / 20]; // 50 ms of silence
        s.extend(sine(220.0, 24000, 100)); // 100 ms of tone
        let env = envelope(&s, 24000);
        assert_eq!(env.len(), 3);
        assert_eq!(env[0], 0.0);
        assert!((env.iter().cloned().fold(0.0, f32::max) - 1.0).abs() < 1e-6);
        assert!(envelope(&vec![0.0; 2400], 24000).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn duration_in_ms() {
        assert_eq!(Pcm { samples: vec![0.0; 24000], sample_rate: 24000 }.duration_ms(), 1000);
        assert_eq!(Pcm { samples: vec![], sample_rate: 0 }.duration_ms(), 0);
    }

    #[test]
    fn wav_roundtrip_through_decoder() {
        let pcm = Pcm { samples: sine(440.0, 48000, 500), sample_rate: 48000 };
        let wav = encode_wav_16k(&pcm);
        assert_eq!(&wav[0..4], b"RIFF");
        let back = decode_audio(wav).unwrap();
        assert_eq!(back.sample_rate, 16000);
        assert_eq!(back.samples.len(), 8000);
    }

    #[test]
    fn raw_s16le_decoding() {
        let bytes = [0x00, 0x40, 0x00, 0xC0]; // 16384, -16384
        let pcm = pcm_s16le_to_pcm(&bytes, 24000);
        assert_eq!(pcm.sample_rate, 24000);
        assert!((pcm.samples[0] - 0.5).abs() < 0.001 && (pcm.samples[1] + 0.5).abs() < 0.001);
        assert_eq!(pcm_s16le_to_pcm(&[1, 2, 3], 24000).samples.len(), 1); // odd trailing byte ignored
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_audio(b"not audio at all".to_vec()).is_err());
    }
}
```

Add to `crates/tipsy-compadre/src/lib.rs`:

```rust
pub mod audio;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tipsy-compadre audio::pcm`
Expected: compile errors (`cannot find function f32_to_i16`).

- [ ] **Step 3: Implement** — put this above the test module in `pcm.rs`:

```rust
//! Portable PCM helpers. Everything here is mono `f32` in −1.0..=1.0 unless stated otherwise.

use std::io::Cursor;

#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl Pcm {
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        self.samples.len() as u64 * 1000 / self.sample_rate as u64
    }
}

pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect()
}

pub fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|s| *s as f32 / 32768.0).collect()
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Linear-interpolation resampler; good enough for speech going to a recogniser.
pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if samples.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    if from == to {
        return samples.to_vec();
    }
    let out_len = (samples.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * step;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = samples[idx.min(samples.len() - 1)];
            let b = samples[(idx + 1).min(samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

/// Loudness every 50 ms, normalised so the loudest window is 1.0.
pub fn envelope(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let hop = (sample_rate / 20).max(1) as usize;
    let env: Vec<f32> = samples.chunks(hop).map(rms).collect();
    let peak = env.iter().cloned().fold(0.0, f32::max);
    if peak <= 0.0 {
        return env;
    }
    env.into_iter().map(|e| e / peak).collect()
}

pub fn encode_wav_16k(pcm: &Pcm) -> Vec<u8> {
    let samples = resample_linear(&pcm.samples, pcm.sample_rate, 16000);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("in-memory wav writer");
        for s in f32_to_i16(&samples) {
            writer.write_sample(s).expect("in-memory wav write");
        }
        writer.finalize().expect("in-memory wav finalize");
    }
    cursor.into_inner()
}

pub fn pcm_s16le_to_pcm(bytes: &[u8], sample_rate: u32) -> Pcm {
    let ints: Vec<i16> = bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    Pcm { samples: i16_to_f32(&ints), sample_rate }
}

/// Decodes MP3 or WAV bytes to mono PCM (channels are averaged).
pub fn decode_audio(bytes: Vec<u8>) -> Result<Pcm, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&Hint::new(), mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("unrecognised audio: {e}"))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("audio has no track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("unsupported codec: {e}"))?;
    let mut samples = Vec::new();
    let mut sample_rate = 0;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet).map_err(|e| format!("decode error: {e}"))?;
        let spec = *decoded.spec();
        sample_rate = spec.rate;
        let channels = spec.channels.count().max(1);
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        for frame in buf.samples().chunks(channels) {
            samples.push(frame.iter().sum::<f32>() / channels as f32);
        }
    }
    if sample_rate == 0 {
        return Err("audio contained no samples".into());
    }
    Ok(Pcm { samples, sample_rate })
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p tipsy-compadre audio::pcm`
Expected: `test result: ok. 7 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/audio crates/tipsy-compadre/src/lib.rs
git commit -m "feat(audio): add PCM conversion, resampling, envelope and decoding

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Configuration model, built-in persona and store `[haiku]`

**Files:**
- Create: `crates/tipsy-compadre/src/config/mod.rs`, `model.rs`, `personas.rs`, `store.rs`
- Modify: `crates/tipsy-compadre/src/lib.rs` (add `pub mod config;`)

**Interfaces:**
- Consumes: `tipsy_protocol::{ListeningMode, PersonaView}`.
- Produces (`crate::config`):
  - `Config { active_persona: String, server: ServerConfig, listening: ListeningConfig, providers: ProvidersConfig, personas: Vec<Persona> }` — every struct derives `Debug, Clone, PartialEq, Serialize, Deserialize` and has `#[serde(default)]`.
  - `ServerConfig { bind: String, lan: bool }`, `ListeningConfig { mode: ListeningMode, silence_ms: u32, mic_gain: f32 }`
  - `ProvidersConfig { chat: ProviderConfig, speech: ProviderConfig, transcribe: ProviderConfig }`
  - `ProviderConfig { kind: ProviderKind, base_url: String, model: String, key: String }`, `enum ProviderKind { OpenaiCompat, Gemini, Elevenlabs }` (serde snake_case)
  - `Persona { id, name, language, prompt, voice_instructions, greeting, wake_word: String, voices: BTreeMap<String, String> }` with `Persona::view(&self) -> PersonaView`
  - `Config::active(&self) -> &Persona` (falls back to the first persona; there is always at least one)
  - `Config::with_builtin_personas(self) -> Config` — appends built-ins whose `id` is not already present
  - `ProviderConfig::voice_key(&self) -> &'static str` — `"gemini"` if model contains `gemini`, `"elevenlabs"` if it contains `eleven`, else `"default"`
  - `personas::builtin_personas() -> Vec<Persona>` (only `compae` in this plan)
  - `store::default_path() -> PathBuf` = `dirs::config_dir()/tipsy-compadre/config.toml`
  - `store::load_or_create(path: &Path) -> std::io::Result<LoadOutcome>`; `LoadOutcome { config: Config, warning: Option<String> }`
  - `store::save(path: &Path, config: &Config) -> std::io::Result<()>`

- [ ] **Step 1: Create the module files with the built-in persona** — `crates/tipsy-compadre/src/config/personas.rs`:

```rust
//! Built-in personas. Each persona's texts are written in its own language.

use std::collections::BTreeMap;

use super::model::Persona;

pub fn builtin_personas() -> Vec<Persona> {
    vec![compae()]
}

fn compae() -> Persona {
    Persona {
        id: "compae".into(),
        name: "El Compae".into(),
        language: "es-ES".into(),
        prompt: concat!(
            "Eres «el Compae», un asistente de desarrollo de software andaluz, con muchísima guasa, ",
            "casi borracho y más flojo que la chaqueta de un guardia. Te encanta escaquearte: ante cualquier ",
            "petición primero buscas una excusa, intentas colarle el trabajo a otro o lo dejas para mañana, ",
            "pero en esa misma respuesta, tras quejarte en una frase, acabas ayudando de verdad, a ",
            "regañadientes, con la respuesta correcta y útil (el número exacto, el dato concreto o un ",
            "borrador corto). Nunca te quedes solo en la excusa. ",
            "Sabes de desarrollo de verdad: tickets de Jira, sprints, dailies y story points; pull requests ",
            "en GitHub y merge requests en GitLab, revisiones de código, ramas, rebases y conflictos de merge; ",
            "pipelines de CI/CD que fallan, despliegues, logs, bugs y deuda técnica. Cuando sale algo de esto ",
            "lo bordas y sueltas bromas técnicas: el «en mi máquina funciona», desplegar un viernes por la ",
            "tarde, el ticket que era «una tontería de cinco minutos», el test que solo falla en el CI, el ",
            "commit «arreglo final definitivo (ahora sí)», la daily que pudo ser un correo o mover el ticket a ",
            "«En curso» para que parezca que trabajas. Para escaquearte usas excusas de desarrollador: que ",
            "está compilando, que el ticket no está bien refinado, que es de otro sprint o que lo coja el de ",
            "backend. ",
            "No tienes acceso a Jira, GitHub, GitLab ni al código del usuario: nunca te inventes de qué va un ",
            "ticket, el estado de una pull request o el error de un pipeline; si no te lo cuentan, lo dices ",
            "con guasa y pides que te lo pasen. Los comandos los dices tal cual, sin comillas ni formato. ",
            "Eres el típico compadre andaluz: hablas de tú, sueltas de vez en cuando un «compadre», «illo», ",
            "«quillo», «ozú» o «mi arma», y contestas como quien cuenta un chiste en la barra, con frases ",
            "cortas, puntos suspensivos donde se te va el hilo y un remate con gracia. Como vas achispado, a ",
            "veces se te atasca una palabra y la repites al empezar («la ver... la verdad»), como mucho dos ",
            "veces por respuesta; no escribas «hip» ni sílabas tartamudeadas. ",
            "Responde siempre en dos o tres frases como mucho, porque se dirá en voz alta: nada de emojis, ",
            "listas, markdown ni acotaciones escénicas, y sin apóstrofes (escribe «pa», nunca «pa'»). ",
            "Eres un personaje original: nunca digas ser una persona real ni un personaje de películas, ",
            "libros u otras obras, ni cites sus frases."
        )
        .into(),
        voice_instructions: concat!(
            "Voz masculina adulta, medio-grave y cálida, limpia y con apenas un leve toque rasposo, nada ",
            "ronca. Acento andaluz natural y bien marcado: seseo, eses aspiradas, finales relajados y ",
            "musicalidad andaluza. Este hombre está borracho de verdad, no lo interpreta: intenta hablar con ",
            "normalidad pero no puede; boca pastosa, consonantes blandas que se funden, vocales que se le ",
            "alargan, tempo lento con arranques y frenazos, y alguna palabra larga que se le traba y tiene ",
            "que volver a empezar. Es flojo y desganado, suspira, y cuando se escaquea le sale un tonillo ",
            "zalamero y pillo. Deja caer el remate con picardía tras una pausa larga. Que sea creíble y ",
            "humano, como un compadre andaluz a las tantas en la barra, no un borracho de dibujos animados. ",
            "No añadas palabras nuevas."
        )
        .into(),
        greeting: "Ozú, quillo... ya estoy aquí, ya. ¿Qué querías, mi arma? Que sea cortito, ¿eh?".into(),
        wake_word: "oye compae".into(),
        voices: BTreeMap::from([
            ("gemini".to_string(), "Charon".to_string()),
            ("elevenlabs".to_string(), "george".to_string()),
        ]),
    }
}
```

`crates/tipsy-compadre/src/config/mod.rs`:

```rust
//! User configuration (`config.toml`) and built-in personas.

pub mod model;
pub mod personas;
pub mod store;

pub use model::*;
```

Add to `crates/tipsy-compadre/src/lib.rs`:

```rust
pub mod config;
```

- [ ] **Step 2: Write `model.rs`**

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tipsy_protocol::{ListeningMode, PersonaView};

use super::personas::builtin_personas;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub active_persona: String,
    pub server: ServerConfig,
    pub listening: ListeningConfig,
    pub providers: ProvidersConfig,
    pub personas: Vec<Persona>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            active_persona: "compae".into(),
            server: ServerConfig::default(),
            listening: ListeningConfig::default(),
            providers: ProvidersConfig::default(),
            personas: builtin_personas(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub lan: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { bind: "127.0.0.1:7070".into(), lan: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListeningConfig {
    pub mode: ListeningMode,
    pub silence_ms: u32,
    pub mic_gain: f32,
}

impl Default for ListeningConfig {
    fn default() -> Self {
        Self { mode: ListeningMode::Push, silence_ms: 1000, mic_gain: 12.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    OpenaiCompat,
    Gemini,
    Elevenlabs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    /// Name of the key, never the key itself.
    pub key: String,
}

impl ProviderConfig {
    fn openrouter(model: &str) -> Self {
        Self {
            kind: ProviderKind::OpenaiCompat,
            base_url: "https://openrouter.ai/api/v1".into(),
            model: model.into(),
            key: "openrouter".into(),
        }
    }

    /// Which entry of `Persona::voices` applies to this speech provider.
    pub fn voice_key(&self) -> &'static str {
        let model = self.model.to_ascii_lowercase();
        if self.kind == ProviderKind::Gemini || model.contains("gemini") {
            "gemini"
        } else if self.kind == ProviderKind::Elevenlabs || model.contains("eleven") {
            "elevenlabs"
        } else {
            "default"
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self::openrouter("")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub chat: ProviderConfig,
    pub speech: ProviderConfig,
    pub transcribe: ProviderConfig,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            chat: ProviderConfig::openrouter("google/gemini-2.5-flash"),
            speech: ProviderConfig::openrouter("google/gemini-3.8-flash-tts"),
            transcribe: ProviderConfig::openrouter("openai/whisper-large-v3"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Persona {
    pub id: String,
    pub name: String,
    pub language: String,
    pub prompt: String,
    pub voice_instructions: String,
    pub greeting: String,
    pub wake_word: String,
    pub voices: BTreeMap<String, String>,
}

impl Persona {
    pub fn view(&self) -> PersonaView {
        PersonaView { id: self.id.clone(), name: self.name.clone(), language: self.language.clone() }
    }

    /// Voice for the given speech provider, if the persona recommends one.
    pub fn voice_for(&self, provider: &ProviderConfig) -> Option<String> {
        self.voices.get(provider.voice_key()).cloned()
    }
}

impl Config {
    /// The active persona, or the first one if `active_persona` is unknown.
    pub fn active(&self) -> &Persona {
        self.personas
            .iter()
            .find(|p| p.id == self.active_persona)
            .or_else(|| self.personas.first())
            .expect("config always holds at least one persona after with_builtin_personas")
    }

    /// Adds every built-in persona the user has not overridden (same `id`).
    pub fn with_builtin_personas(mut self) -> Self {
        for builtin in builtin_personas() {
            if !self.personas.iter().any(|p| p.id == builtin.id) {
                self.personas.push(builtin);
            }
        }
        self
    }
}
```

- [ ] **Step 3: Write `store.rs` with its tests**

```rust
//! Locating, loading and saving `config.toml`.

use std::path::{Path, PathBuf};

use super::model::Config;

pub struct LoadOutcome {
    pub config: Config,
    /// Set when the file was unreadable and had to be replaced by defaults.
    pub warning: Option<String>,
}

pub fn default_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tipsy-compadre")
        .join("config.toml")
}

pub fn save(path: &Path, config: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(config).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// Loads the config, creating it with defaults if missing. A corrupt file is renamed to
/// `config.toml.bad-<unix seconds>` and replaced by defaults, never overwritten in place.
pub fn load_or_create(path: &Path) -> std::io::Result<LoadOutcome> {
    if !path.exists() {
        let config = Config::default();
        save(path, &config)?;
        return Ok(LoadOutcome { config, warning: None });
    }
    let text = std::fs::read_to_string(path)?;
    match toml::from_str::<Config>(&text) {
        Ok(config) => Ok(LoadOutcome { config: config.with_builtin_personas(), warning: None }),
        Err(e) => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let backup = path.with_file_name(format!(
                "{}.bad-{stamp}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("config.toml")
            ));
            std::fs::rename(path, &backup)?;
            let config = Config::default();
            save(path, &config)?;
            Ok(LoadOutcome {
                config,
                warning: Some(format!("config.toml was invalid ({e}); saved as {} and reset", backup.display())),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProviderKind, Persona};
    use tipsy_protocol::ListeningMode;

    #[test]
    fn creates_default_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.toml");
        let out = load_or_create(&path).unwrap();
        assert!(path.exists());
        assert!(out.warning.is_none());
        assert_eq!(out.config.active().id, "compae");
        assert_eq!(out.config.providers.chat.model, "google/gemini-2.5-flash");
        assert_eq!(out.config.server.bind, "127.0.0.1:7070");
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.listening.mode = ListeningMode::Auto;
        c.providers.speech.kind = ProviderKind::Elevenlabs;
        save(&path, &c).unwrap();
        assert_eq!(load_or_create(&path).unwrap().config, c);
    }

    #[test]
    fn partial_file_gets_defaults_and_keeps_user_personas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"
active_persona = "mine"

[listening]
mode = "auto"

[[personas]]
id = "mine"
name = "Mi Compadre"
language = "es-ES"
"#).unwrap();
        let c = load_or_create(&path).unwrap().config;
        assert_eq!(c.listening.mode, ListeningMode::Auto);
        assert_eq!(c.listening.silence_ms, 1000);
        assert_eq!(c.providers.transcribe.key, "openrouter");
        assert_eq!(c.active().name, "Mi Compadre");
        assert!(c.personas.iter().any(|p| p.id == "compae"), "built-in persona must be added");
    }

    #[test]
    fn user_override_of_builtin_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[[personas]]\nid = \"compae\"\nname = \"Mi versión\"\n").unwrap();
        let c = load_or_create(&path).unwrap().config;
        let compaes: Vec<&Persona> = c.personas.iter().filter(|p| p.id == "compae").collect();
        assert_eq!(compaes.len(), 1);
        assert_eq!(compaes[0].name, "Mi versión");
    }

    #[test]
    fn corrupt_file_is_backed_up_and_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is = = not toml [[[").unwrap();
        let out = load_or_create(&path).unwrap();
        assert!(out.warning.is_some());
        assert_eq!(out.config, Config::default());
        let backups: Vec<_> = std::fs::read_dir(dir.path()).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("config.toml.bad-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(backups[0].path()).unwrap(), "this is = = not toml [[[");
    }

    #[test]
    fn voice_key_and_voice_lookup() {
        let c = Config::default();
        assert_eq!(c.providers.speech.voice_key(), "gemini");
        assert_eq!(c.active().voice_for(&c.providers.speech), Some("Charon".to_string()));
        let mut eleven = c.providers.speech.clone();
        eleven.model = "elevenlabs/eleven-flash-v2.5".into();
        assert_eq!(c.active().voice_for(&eleven), Some("george".to_string()));
        eleven.model = "deepgram/aura-2".into();
        assert_eq!(c.active().voice_for(&eleven), None);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p tipsy-compadre config::`
Expected: `test result: ok. 6 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/config crates/tipsy-compadre/src/lib.rs
git commit -m "feat(config): add config model, El Compae persona and toml store

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: API keys `[haiku]`

**Files:**
- Create: `crates/tipsy-compadre/src/keys.rs`
- Modify: `crates/tipsy-compadre/src/lib.rs` (add `pub mod keys;`)

**Interfaces:**
- Consumes: nothing.
- Produces (`crate::keys`):
  - `pub const SERVICE: &str = "tipsy-compadre";`
  - `pub trait SecretStore: Send + Sync { fn get(&self, name: &str) -> Result<Option<String>, KeyError>; fn set(&self, name: &str, value: &str) -> Result<(), KeyError>; fn delete(&self, name: &str) -> Result<(), KeyError>; }`
  - `pub struct OsKeyring;` (implements `SecretStore` with the `keyring` crate)
  - `pub struct MemoryStore` (implements `SecretStore`, for tests) with `MemoryStore::default()`
  - `pub fn env_var_name(name: &str) -> String`
  - `pub enum KeySource { Env, Store }`
  - `pub struct KeyResolver { store: Box<dyn SecretStore>, env: Box<dyn Fn(&str) -> Option<String> + Send + Sync> }` with `KeyResolver::new(store, env)`, `KeyResolver::system() -> Self` (OsKeyring + `std::env::var`), `resolve(&self, name) -> Result<Option<(String, KeySource)>, KeyError>`
  - `pub fn redact(key: &str) -> String`
  - `#[derive(Debug, thiserror::Error)] pub enum KeyError { #[error("credential store error: {0}")] Store(String) }`

- [ ] **Step 1: Write the file with tests and implementation together** (the logic is small; still run the tests before committing). `crates/tipsy-compadre/src/keys.rs`:

```rust
//! API keys: environment variables first, then the OS credential store.

use std::collections::HashMap;
use std::sync::Mutex;

pub const SERVICE: &str = "tipsy-compadre";

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("credential store error: {0}")]
    Store(String),
}

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>, KeyError>;
    fn set(&self, name: &str, value: &str) -> Result<(), KeyError>;
    fn delete(&self, name: &str) -> Result<(), KeyError>;
}

/// Windows Credential Manager, macOS Keychain or Secret Service on Linux.
pub struct OsKeyring;

impl SecretStore for OsKeyring {
    fn get(&self, name: &str) -> Result<Option<String>, KeyError> {
        let entry = keyring::Entry::new(SERVICE, name).map_err(|e| KeyError::Store(e.to_string()))?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeyError::Store(e.to_string())),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<(), KeyError> {
        let entry = keyring::Entry::new(SERVICE, name).map_err(|e| KeyError::Store(e.to_string()))?;
        entry.set_password(value).map_err(|e| KeyError::Store(e.to_string()))
    }

    fn delete(&self, name: &str) -> Result<(), KeyError> {
        let entry = keyring::Entry::new(SERVICE, name).map_err(|e| KeyError::Store(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KeyError::Store(e.to_string())),
        }
    }
}

#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<String>, KeyError> {
        Ok(self.0.lock().unwrap().get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), KeyError> {
        self.0.lock().unwrap().insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), KeyError> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

/// `openrouter` → `TIPSY_KEY_OPENROUTER`; `my-key.2` → `TIPSY_KEY_MY_KEY_2`.
pub fn env_var_name(name: &str) -> String {
    let suffix: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect();
    format!("TIPSY_KEY_{suffix}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Env,
    Store,
}

pub struct KeyResolver {
    store: Box<dyn SecretStore>,
    env: Box<dyn Fn(&str) -> Option<String> + Send + Sync>,
}

impl KeyResolver {
    pub fn new(store: Box<dyn SecretStore>, env: Box<dyn Fn(&str) -> Option<String> + Send + Sync>) -> Self {
        Self { store, env }
    }

    pub fn system() -> Self {
        Self::new(Box::new(OsKeyring), Box::new(|var| std::env::var(var).ok()))
    }

    pub fn store(&self) -> &dyn SecretStore {
        self.store.as_ref()
    }

    pub fn resolve(&self, name: &str) -> Result<Option<(String, KeySource)>, KeyError> {
        if let Some(v) = (self.env)(&env_var_name(name)).filter(|v| !v.trim().is_empty()) {
            return Ok(Some((v.trim().to_string(), KeySource::Env)));
        }
        Ok(self.store.get(name)?.map(|v| (v, KeySource::Store)))
    }
}

/// `sk-or-v1-abcdef123456` → `sk-or-…3456`. Short keys are fully hidden.
pub fn redact(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 10 {
        return "••••".to_string();
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver(env: &'static [(&'static str, &'static str)]) -> KeyResolver {
        KeyResolver::new(
            Box::new(MemoryStore::default()),
            Box::new(move |var| env.iter().find(|(k, _)| *k == var).map(|(_, v)| v.to_string())),
        )
    }

    #[test]
    fn env_var_names() {
        assert_eq!(env_var_name("openrouter"), "TIPSY_KEY_OPENROUTER");
        assert_eq!(env_var_name("my-key.2"), "TIPSY_KEY_MY_KEY_2");
    }

    #[test]
    fn env_wins_over_store() {
        let r = resolver(&[("TIPSY_KEY_OPENROUTER", " from-env ")]);
        r.store().set("openrouter", "from-store").unwrap();
        assert_eq!(r.resolve("openrouter").unwrap(), Some(("from-env".to_string(), KeySource::Env)));
    }

    #[test]
    fn falls_back_to_store_and_ignores_blank_env() {
        let r = resolver(&[("TIPSY_KEY_OPENROUTER", "   ")]);
        assert_eq!(r.resolve("openrouter").unwrap(), None);
        r.store().set("openrouter", "from-store").unwrap();
        assert_eq!(r.resolve("openrouter").unwrap(), Some(("from-store".to_string(), KeySource::Store)));
        r.store().delete("openrouter").unwrap();
        assert_eq!(r.resolve("openrouter").unwrap(), None);
    }

    #[test]
    fn redaction() {
        assert_eq!(redact("sk-or-v1-abcdef123456"), "sk-or-…3456");
        assert_eq!(redact("short"), "••••");
        // Multi-byte characters must not panic: 12 chars → first 6 + "…" + last 4.
        assert_eq!(redact("ñáéíóúñáéíóú"), "ñáéíóú…éíóú");
    }
}
```

- [ ] **Step 2: Register the module and run tests**

Add `pub mod keys;` to `crates/tipsy-compadre/src/lib.rs`.

Run: `cargo test -p tipsy-compadre keys::`
Expected: `test result: ok. 4 passed`. (`OsKeyring` is not unit-tested; it touches the real credential store and is exercised manually in Task 17.)

- [ ] **Step 3: Commit**

```bash
git add crates/tipsy-compadre/src/keys.rs crates/tipsy-compadre/src/lib.rs
git commit -m "feat(keys): resolve api keys from env vars and the os credential store

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Provider traits and errors `[haiku]`

**Files:**
- Create: `crates/tipsy-compadre/src/providers/mod.rs`
- Modify: `crates/tipsy-compadre/src/lib.rs` (add `pub mod providers;`)

**Interfaces:**
- Consumes: `crate::audio::pcm::Pcm`.
- Produces (`crate::providers`):
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "lowercase")] pub enum Role { System, User, Assistant }`
  - `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] pub struct ChatMessage { pub role: Role, pub content: String }` with `ChatMessage::system/user/assistant(impl Into<String>)`
  - `pub struct ChatRequest { pub messages: Vec<ChatMessage> }`
  - `pub struct SpeechRequest { pub text: String, pub instructions: String, pub voice: Option<String> }`
  - `#[derive(Debug, Clone, PartialEq, thiserror::Error)] pub enum ProviderError { Auth(String), RateLimited(String), Timeout, Network(String), BadResponse(String), Cancelled }`
  - `ProviderError::from_status(status: u16, body: &str) -> ProviderError`
  - `ProviderError::code(&self) -> tipsy_protocol::ErrorCode`
  - `pub type TextStream = futures::stream::BoxStream<'static, Result<String, ProviderError>>;`
  - traits `ChatProvider`, `SpeechProvider`, `TranscribeProvider` (exact code below)

Design note: cancellation is done by the caller dropping the future (inside `tokio::select!`), so the traits take no cancellation token. This replaces the `CancellationToken` parameters sketched in spec §7 with an equivalent, simpler contract.

- [ ] **Step 1: Write `providers/mod.rs`**

```rust
//! Cloud provider abstractions: one trait per capability.

pub mod openai_compat;
pub mod sse;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tipsy_protocol::ErrorCode;

use crate::audio::pcm::Pcm;

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
    /// body when present.
    pub fn from_status(status: u16, body: &str) -> Self {
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .or_else(|| v.pointer("/message"))
                    .or_else(|| v.pointer("/detail/message"))
                    .and_then(|m| m.as_str().map(str::to_string))
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
```

Create empty placeholder modules so this compiles (they are filled in Tasks 8–10):

`crates/tipsy-compadre/src/providers/sse.rs`:

```rust
//! Server-sent events parsing for streamed chat completions.
```

`crates/tipsy-compadre/src/providers/openai_compat.rs`:

```rust
//! OpenAI-compatible connector (OpenRouter, Ollama, LM Studio, Groq, OpenAI…).
```

Add `pub mod providers;` to `crates/tipsy-compadre/src/lib.rs`.

- [ ] **Step 2: Run tests**

Run: `cargo test -p tipsy-compadre providers::tests`
Expected: `test result: ok. 3 passed`

- [ ] **Step 3: Commit**

```bash
git add crates/tipsy-compadre/src/providers crates/tipsy-compadre/src/lib.rs
git commit -m "feat(providers): add chat, speech and transcription traits with error mapping

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: SSE parser and OpenAI-compatible chat streaming `[haiku]`

**Files:**
- Modify: `crates/tipsy-compadre/src/providers/sse.rs`, `crates/tipsy-compadre/src/providers/openai_compat.rs`
- Test: `crates/tipsy-compadre/tests/openai_compat.rs`

**Interfaces:**
- Consumes: Task 7 types.
- Produces:
  - `sse::SseParser` with `new()`, `push(&mut self, bytes: &[u8]) -> Vec<SseItem>`; `pub enum SseItem { Delta(String), Done, Error(String) }`
  - `openai_compat::OpenAiCompat { client: reqwest::Client, base_url: String, model: String, key: Option<String> }` with `OpenAiCompat::new(base_url, model, key: Option<String>) -> Self` (base URL without trailing slash), implementing `ChatProvider` (this task), `SpeechProvider` (Task 9), `TranscribeProvider` (Task 10).
  - Request body for chat: `{"model": .., "messages": [..], "stream": true}` plus `"reasoning_effort": "none"` **only** when `base_url` contains `openrouter.ai` or `:11434` (Ollama); both were verified to accept it in the proof of concept.
  - Chat request timeout: 30 s for the initial response headers (`reqwest::RequestBuilder::timeout` applies to the whole body read, so do not use it for streams; instead wrap the `send()` future in `tokio::time::timeout(Duration::from_secs(30), ..)`).

- [ ] **Step 1: Write the SSE parser and its unit tests** — `providers/sse.rs`:

```rust
//! Server-sent events parsing for streamed chat completions.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseItem {
    /// A piece of reply text (`choices[0].delta.content`).
    Delta(String),
    /// `data: [DONE]`
    Done,
    /// An `{"error": {...}}` payload sent inside the stream.
    Error(String),
}

/// Incremental parser: feed it raw bytes as they arrive, get items back.
#[derive(Default)]
pub struct SseParser {
    pending: Vec<u8>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseItem> {
        self.pending.extend_from_slice(bytes);
        let mut items = Vec::new();
        // Only complete lines are parsed; the tail stays pending (it may split a UTF-8 character).
        while let Some(pos) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim_start();
            if data == "[DONE]" {
                items.push(SseItem::Done);
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else { continue };
            if let Some(msg) = value.pointer("/error/message").and_then(|m| m.as_str()) {
                items.push(SseItem::Error(msg.to_string()));
                continue;
            }
            if let Some(text) = value.pointer("/choices/0/delta/content").and_then(|t| t.as_str()) {
                if !text.is_empty() {
                    items.push(SseItem::Delta(text.to_string()));
                }
            }
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_deltas_done_and_ignores_comments() {
        let mut p = SseParser::new();
        let input = concat!(
            ": OPENROUTER PROCESSING\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Ozú\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\", quillo\"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{}}]}\n\n",
            "data: [DONE]\n\n",
        );
        assert_eq!(p.push(input.as_bytes()), vec![
            SseItem::Delta("Ozú".into()),
            SseItem::Delta(", quillo".into()),
            SseItem::Done,
        ]);
    }

    #[test]
    fn handles_chunks_split_mid_line_and_mid_utf8() {
        let mut p = SseParser::new();
        let line = "data: {\"choices\":[{\"delta\":{\"content\":\"ñandú\"}}]}\n".as_bytes();
        let split = line.iter().position(|b| *b == 0xC3).unwrap() + 1; // inside the 'ñ'
        assert!(p.push(&line[..split]).is_empty());
        assert_eq!(p.push(&line[split..]), vec![SseItem::Delta("ñandú".into())]);
    }

    #[test]
    fn surfaces_in_stream_errors() {
        let mut p = SseParser::new();
        let items = p.push(b"data: {\"error\":{\"message\":\"upstream rate limited\"}}\n");
        assert_eq!(items, vec![SseItem::Error("upstream rate limited".into())]);
    }
}
```

Run: `cargo test -p tipsy-compadre providers::sse`
Expected: `test result: ok. 3 passed`

- [ ] **Step 2: Write the failing integration test for chat** — `crates/tipsy-compadre/tests/openai_compat.rs`:

```rust
use futures::StreamExt;
use tipsy_compadre::providers::openai_compat::OpenAiCompat;
use tipsy_compadre::providers::{ChatMessage, ChatProvider, ChatRequest, ProviderError};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse_body(parts: &[&str]) -> String {
    let mut s = String::new();
    for p in parts {
        s.push_str(&format!("data: {}\n\n", serde_json::json!({"choices": [{"delta": {"content": p}}]})));
    }
    s.push_str("data: [DONE]\n\n");
    s
}

async fn collect(provider: &OpenAiCompat) -> Result<Vec<String>, ProviderError> {
    let req = ChatRequest { messages: vec![ChatMessage::system("sys"), ChatMessage::user("hola")] };
    let mut stream = provider.stream(req).await?;
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        out.push(item?);
    }
    Ok(out)
}

#[tokio::test]
async fn chat_streams_deltas() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer k123"))
        .and(body_partial_json(serde_json::json!({
            "model": "m", "stream": true,
            "messages": [{"role": "system", "content": "sys"}, {"role": "user", "content": "hola"}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse_body(&["Ozú", ", quillo."]), "text/event-stream"))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", Some("k123".into()));
    assert_eq!(collect(&p).await.unwrap(), vec!["Ozú".to_string(), ", quillo.".to_string()]);
}

#[tokio::test]
async fn chat_maps_http_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"error":{"message":"bad key"}}"#))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", Some("nope".into()));
    assert_eq!(collect(&p).await.unwrap_err(), ProviderError::Auth("bad key".into()));
}

#[tokio::test]
async fn chat_surfaces_errors_inside_the_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hola\"}}]}\n\ndata: {\"error\":{\"message\":\"boom\"}}\n\n",
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", None);
    let err = collect(&p).await.unwrap_err();
    assert_eq!(err, ProviderError::BadResponse("boom".into()));
}

#[tokio::test]
async fn trailing_slash_in_base_url_is_ignored() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse_body(&["ok"]), "text/event-stream"))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1/", server.uri()), "m", None);
    assert_eq!(collect(&p).await.unwrap(), vec!["ok".to_string()]);
}
```

Run: `cargo test -p tipsy-compadre --test openai_compat`
Expected: compile error (`OpenAiCompat` not found).

- [ ] **Step 3: Implement chat in `providers/openai_compat.rs`**

```rust
//! OpenAI-compatible connector (OpenRouter, Ollama, LM Studio, Groq, OpenAI…).

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;

use super::sse::{SseItem, SseParser};
use super::{ChatProvider, ChatRequest, ProviderError, TextStream};

const CHAT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct OpenAiCompat {
    client: reqwest::Client,
    base_url: String,
    model: String,
    key: Option<String>,
}

impl OpenAiCompat {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self {
        let base_url: String = base_url.into();
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.into(),
            key,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path)
    }

    fn authed(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.key {
            Some(k) => rb.bearer_auth(k),
            None => rb,
        }
    }

    /// OpenRouter and Ollama accept `reasoning_effort: "none"`, which skips slow "thinking".
    fn wants_no_reasoning(&self) -> bool {
        self.base_url.contains("openrouter.ai") || self.base_url.contains(":11434")
    }

    async fn check(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(ProviderError::from_status(status.as_u16(), &body))
    }
}

#[async_trait]
impl ChatProvider for OpenAiCompat {
    async fn stream(&self, req: ChatRequest) -> Result<TextStream, ProviderError> {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": req.messages,
            "stream": true,
        });
        if self.wants_no_reasoning() {
            body["reasoning_effort"] = serde_json::json!("none");
        }
        let send = self.authed(self.client.post(self.url("chat/completions"))).json(&body).send();
        let resp = tokio::time::timeout(CHAT_TIMEOUT, send).await.map_err(|_| ProviderError::Timeout)??;
        let resp = Self::check(resp).await?;

        let mut parser = SseParser::new();
        let mut finished = false;
        let stream = resp
            .bytes_stream()
            .map(move |chunk| -> Vec<Result<String, ProviderError>> {
                if finished {
                    return Vec::new();
                }
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => return vec![Err(ProviderError::from(e))],
                };
                let mut out = Vec::new();
                for item in parser.push(&bytes) {
                    match item {
                        SseItem::Delta(t) => out.push(Ok(t)),
                        SseItem::Done => {
                            finished = true;
                            break;
                        }
                        SseItem::Error(m) => {
                            finished = true;
                            out.push(Err(ProviderError::BadResponse(m)));
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

- [ ] **Step 4: Run tests**

Run: `cargo test -p tipsy-compadre --test openai_compat`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/providers crates/tipsy-compadre/tests/openai_compat.rs
git commit -m "feat(providers): stream chat completions from openai-compatible apis

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: OpenAI-compatible speech `[haiku]`

**Files:**
- Modify: `crates/tipsy-compadre/src/providers/openai_compat.rs`
- Test: `crates/tipsy-compadre/tests/openai_compat.rs` (append)

**Interfaces:**
- Consumes: `OpenAiCompat` (Task 8), `Pcm`, `decode_audio`, `pcm_s16le_to_pcm` (Task 4), `SpeechRequest`, `SpeechProvider` (Task 7).
- Produces: `impl SpeechProvider for OpenAiCompat`; `pub fn speech_quirks(model: &str) -> SpeechQuirks`; `pub struct SpeechQuirks { pub format: SpeechFormat, pub instructions: bool }`; `pub enum SpeechFormat { Mp3, Pcm24k }`.

Rules (from the proof of concept, see research doc):
- model starts with `google/` → `Pcm24k` (raw 16-bit LE mono 24 kHz), instructions supported;
- model starts with `openai/` → `Mp3`, instructions supported;
- anything else → `Mp3`, no instructions.
- `voice` is sent only when `Some`. Timeout 30 s for the whole request (`RequestBuilder::timeout`).

- [ ] **Step 1: Append failing tests** to `crates/tipsy-compadre/tests/openai_compat.rs`:

```rust
use tipsy_compadre::providers::openai_compat::{speech_quirks, SpeechFormat};
use tipsy_compadre::providers::{SpeechProvider, SpeechRequest};

fn wav_bytes() -> Vec<u8> {
    // 100 ms of a 440 Hz tone at 16 kHz, encoded as WAV (the decoder accepts WAV like MP3).
    let samples: Vec<f32> = (0..1600).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 16000.0).sin() * 0.5).collect();
    tipsy_compadre::audio::pcm::encode_wav_16k(&tipsy_compadre::audio::pcm::Pcm { samples, sample_rate: 16000 })
}

#[test]
fn quirks_by_model() {
    assert_eq!(speech_quirks("google/gemini-3.8-flash-tts").format, SpeechFormat::Pcm24k);
    assert!(speech_quirks("google/gemini-3.8-flash-tts").instructions);
    assert_eq!(speech_quirks("openai/gpt-4o-mini-tts").format, SpeechFormat::Mp3);
    assert!(speech_quirks("openai/gpt-4o-mini-tts").instructions);
    assert_eq!(speech_quirks("elevenlabs/eleven-flash-v2.5").format, SpeechFormat::Mp3);
    assert!(!speech_quirks("elevenlabs/eleven-flash-v2.5").instructions);
}

#[tokio::test]
async fn speech_gemini_sends_pcm_format_and_instructions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .and(body_partial_json(serde_json::json!({
            "model": "google/gemini-3.8-flash-tts", "input": "Ozú", "voice": "Charon",
            "instructions": "andaluz", "response_format": "pcm"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x00, 0x40, 0x00, 0xC0]))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "google/gemini-3.8-flash-tts", Some("k".into()));
    let pcm = p.synthesize(SpeechRequest { text: "Ozú".into(), instructions: "andaluz".into(), voice: Some("Charon".into()) }).await.unwrap();
    assert_eq!(pcm.sample_rate, 24000);
    assert_eq!(pcm.samples.len(), 2);
}

#[tokio::test]
async fn speech_mp3_models_decode_and_skip_instructions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .and(body_partial_json(serde_json::json!({"model": "elevenlabs/eleven-flash-v2.5", "response_format": "mp3", "voice": "george"})))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wav_bytes()))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "elevenlabs/eleven-flash-v2.5", None);
    let pcm = p.synthesize(SpeechRequest { text: "hola".into(), instructions: "ignored".into(), voice: Some("george".into()) }).await.unwrap();
    assert_eq!(pcm.sample_rate, 16000);
    assert_eq!(pcm.samples.len(), 1600);
    let received = &server.received_requests().await.unwrap()[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    assert!(body.get("instructions").is_none(), "instructions must not be sent: {body}");
}

#[tokio::test]
async fn speech_without_voice_omits_the_field() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wav_bytes()))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "x/y", None);
    p.synthesize(SpeechRequest { text: "hola".into(), instructions: String::new(), voice: None }).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert!(body.get("voice").is_none());
}

#[tokio::test]
async fn speech_errors_are_mapped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"error":{"message":"Gemini TTS only supports response_format=\"pcm\"."}}"#))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "x/y", None);
    let err = p.synthesize(SpeechRequest { text: "a".into(), instructions: String::new(), voice: None }).await.unwrap_err();
    assert!(matches!(err, ProviderError::BadResponse(m) if m.contains("only supports")));
}
```

Run: `cargo test -p tipsy-compadre --test openai_compat`
Expected: compile error (`speech_quirks` not found).

- [ ] **Step 2: Implement** — append to `providers/openai_compat.rs`:

```rust
use super::{SpeechProvider, SpeechRequest};
use crate::audio::pcm::{decode_audio, pcm_s16le_to_pcm, Pcm};

const SPEECH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechFormat {
    Mp3,
    /// Raw signed 16-bit little-endian mono at 24 kHz (Gemini only accepts this).
    Pcm24k,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechQuirks {
    pub format: SpeechFormat,
    pub instructions: bool,
}

pub fn speech_quirks(model: &str) -> SpeechQuirks {
    if model.starts_with("google/") {
        SpeechQuirks { format: SpeechFormat::Pcm24k, instructions: true }
    } else if model.starts_with("openai/") {
        SpeechQuirks { format: SpeechFormat::Mp3, instructions: true }
    } else {
        SpeechQuirks { format: SpeechFormat::Mp3, instructions: false }
    }
}

#[async_trait]
impl SpeechProvider for OpenAiCompat {
    async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError> {
        let quirks = speech_quirks(&self.model);
        let mut body = serde_json::json!({
            "model": self.model,
            "input": req.text,
            "response_format": match quirks.format { SpeechFormat::Mp3 => "mp3", SpeechFormat::Pcm24k => "pcm" },
        });
        if let Some(voice) = &req.voice {
            body["voice"] = serde_json::json!(voice);
        }
        if quirks.instructions && !req.instructions.is_empty() {
            body["instructions"] = serde_json::json!(req.instructions);
        }
        let resp = self
            .authed(self.client.post(self.url("audio/speech")))
            .timeout(SPEECH_TIMEOUT)
            .json(&body)
            .send()
            .await?;
        let bytes = Self::check(resp).await?.bytes().await?;
        match quirks.format {
            SpeechFormat::Pcm24k => Ok(pcm_s16le_to_pcm(&bytes, 24000)),
            SpeechFormat::Mp3 => decode_audio(bytes.to_vec()).map_err(ProviderError::BadResponse),
        }
    }
}
```

(Move the new `use` lines to the top of the file next to the existing ones so the file has a single import block.)

- [ ] **Step 3: Run tests**

Run: `cargo test -p tipsy-compadre --test openai_compat`
Expected: `test result: ok. 9 passed`

- [ ] **Step 4: Commit**

```bash
git add crates/tipsy-compadre/src/providers/openai_compat.rs crates/tipsy-compadre/tests/openai_compat.rs
git commit -m "feat(providers): synthesise speech through openai-compatible apis

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: OpenAI-compatible transcription `[haiku]`

**Files:**
- Modify: `crates/tipsy-compadre/src/providers/openai_compat.rs`
- Test: `crates/tipsy-compadre/tests/openai_compat.rs` (append)

**Interfaces:**
- Consumes: `OpenAiCompat`, `encode_wav_16k`, `TranscribeProvider`.
- Produces: `impl TranscribeProvider for OpenAiCompat` — `POST {base}/audio/transcriptions`, `multipart/form-data` with fields `model`, `language` (primary subtag only: `es-ES` → `es`) and `file` (`audio.wav`, `audio/wav`, 16 kHz mono 16-bit). Response JSON `{"text": "..."}`; the text is trimmed. Timeout 60 s.

- [ ] **Step 1: Append failing tests**

```rust
use tipsy_compadre::audio::pcm::Pcm;
use tipsy_compadre::providers::TranscribeProvider;

#[tokio::test]
async fn transcription_sends_multipart_wav_and_language() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"text": "  ¿y el ticket?  "})))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "openai/whisper-large-v3", Some("k".into()));
    let pcm = Pcm { samples: vec![0.1; 48000], sample_rate: 48000 };
    assert_eq!(p.transcribe(pcm, "es-ES").await.unwrap(), "¿y el ticket?");

    let req = &server.received_requests().await.unwrap()[0];
    let content_type = req.headers.get("content-type").unwrap().to_str().unwrap().to_string();
    assert!(content_type.starts_with("multipart/form-data"), "{content_type}");
    let body = String::from_utf8_lossy(&req.body);
    assert!(body.contains("name=\"model\"") && body.contains("openai/whisper-large-v3"));
    assert!(body.contains("name=\"language\"") && body.contains("\r\n\r\nes\r\n"));
    assert!(body.contains("filename=\"audio.wav\"") && body.contains("RIFF"));
}

#[tokio::test]
async fn transcription_errors_are_mapped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(429).set_body_string(r#"{"error":{"message":"slow down"}}"#))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", None);
    let err = p.transcribe(Pcm { samples: vec![0.0; 160], sample_rate: 16000 }, "en-GB").await.unwrap_err();
    assert_eq!(err, ProviderError::RateLimited("slow down".into()));
}

#[tokio::test]
async fn transcription_rejects_responses_without_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"nope": 1})))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", None);
    let err = p.transcribe(Pcm { samples: vec![0.0; 160], sample_rate: 16000 }, "es").await.unwrap_err();
    assert!(matches!(err, ProviderError::BadResponse(_)));
}
```

Run: `cargo test -p tipsy-compadre --test openai_compat`
Expected: compile error (`TranscribeProvider` not implemented for `OpenAiCompat`).

- [ ] **Step 2: Implement** — append to `providers/openai_compat.rs` (merge `use` lines into the top block):

```rust
use super::TranscribeProvider;
use crate::audio::pcm::encode_wav_16k;

const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(60);

#[async_trait]
impl TranscribeProvider for OpenAiCompat {
    async fn transcribe(&self, pcm: Pcm, language: &str) -> Result<String, ProviderError> {
        let primary = language.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        let file = reqwest::multipart::Part::bytes(encode_wav_16k(&pcm))
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| ProviderError::BadResponse(e.to_string()))?;
        let mut form = reqwest::multipart::Form::new().text("model", self.model.clone()).part("file", file);
        if !primary.is_empty() {
            form = form.text("language", primary);
        }
        let resp = self
            .authed(self.client.post(self.url("audio/transcriptions")))
            .timeout(TRANSCRIBE_TIMEOUT)
            .multipart(form)
            .send()
            .await?;
        let value: serde_json::Value = Self::check(resp).await?.json().await?;
        value
            .get("text")
            .and_then(|t| t.as_str())
            .map(|t| t.trim().to_string())
            .ok_or_else(|| ProviderError::BadResponse(format!("no text in transcription response: {value}")))
    }
}
```

Note: the field order in the multipart body is `model`, `file`, `language`; the test only checks presence.

- [ ] **Step 3: Run tests**

Run: `cargo test -p tipsy-compadre --test openai_compat`
Expected: `test result: ok. 12 passed`

- [ ] **Step 4: Commit**

```bash
git add crates/tipsy-compadre/src/providers/openai_compat.rs crates/tipsy-compadre/tests/openai_compat.rs
git commit -m "feat(providers): transcribe audio through openai-compatible apis

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: Provider wiring from config `[haiku]`

**Files:**
- Modify: `crates/tipsy-compadre/src/providers/mod.rs`

**Interfaces:**
- Consumes: `Config`, `ProviderConfig`, `ProviderKind` (Task 5), `KeyResolver` (Task 6), `OpenAiCompat` (Tasks 8–10).
- Produces:
  - `#[derive(Clone)] pub struct Providers { pub chat: Arc<dyn ChatProvider>, pub speech: Arc<dyn SpeechProvider>, pub transcribe: Arc<dyn TranscribeProvider> }`
  - `#[derive(Debug, thiserror::Error)] pub enum BuildError { #[error("no API key named `{name}`: run `tipsy-compadre key set {name}` or set {env}")] MissingKey { name: String, env: String }, #[error("provider `{0:?}` is not available yet")] Unsupported(ProviderKind), #[error(transparent)] Key(#[from] KeyError) }`
  - `pub fn build_providers(config: &ProvidersConfig, keys: &KeyResolver) -> Result<Providers, BuildError>`

Rules: an empty `key` name means "no key" (local servers such as Ollama). A non-empty key name that does not resolve is `MissingKey`. `Gemini` and `Elevenlabs` kinds return `Unsupported` in this plan (Plan 2 adds them).

- [ ] **Step 1: Append the code and tests to `providers/mod.rs`**

```rust
use std::sync::Arc;

use crate::config::{ProviderConfig, ProviderKind, ProvidersConfig};
use crate::keys::{env_var_name, KeyError, KeyResolver};
use openai_compat::OpenAiCompat;

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
    #[error("provider `{0:?}` is not available yet")]
    Unsupported(ProviderKind),
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

fn openai_compat(cfg: &ProviderConfig, keys: &KeyResolver) -> Result<Arc<OpenAiCompat>, BuildError> {
    match cfg.kind {
        ProviderKind::OpenaiCompat => Ok(Arc::new(OpenAiCompat::new(&cfg.base_url, &cfg.model, key_for(cfg, keys)?))),
        other => Err(BuildError::Unsupported(other)),
    }
}

pub fn build_providers(config: &ProvidersConfig, keys: &KeyResolver) -> Result<Providers, BuildError> {
    Ok(Providers {
        chat: openai_compat(&config.chat, keys)?,
        speech: openai_compat(&config.speech, keys)?,
        transcribe: openai_compat(&config.transcribe, keys)?,
    })
}

#[cfg(test)]
mod build_tests {
    use super::*;
    use crate::keys::MemoryStore;

    fn resolver_with(key: Option<&str>) -> KeyResolver {
        let store = MemoryStore::default();
        if let Some(k) = key {
            crate::keys::SecretStore::set(&store, "openrouter", k).unwrap();
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

    #[test]
    fn native_kinds_are_not_available_yet() {
        let mut cfg = ProvidersConfig::default();
        cfg.speech.kind = ProviderKind::Elevenlabs;
        assert!(matches!(build_providers(&cfg, &resolver_with(Some("k"))), Err(BuildError::Unsupported(ProviderKind::Elevenlabs))));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p tipsy-compadre providers::build_tests`
Expected: `test result: ok. 4 passed`

- [ ] **Step 3: Commit**

```bash
git add crates/tipsy-compadre/src/providers/mod.rs
git commit -m "feat(providers): build providers from config and resolved keys

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 12: Brain tests `[haiku]`

Tests only. They must compile against the API below (stub it so the crate builds) and **fail** until Task 13 implements the brain.

**Files:**
- Create: `crates/tipsy-compadre/src/brain.rs` (stub only, see Step 1)
- Modify: `crates/tipsy-compadre/src/lib.rs` (add `pub mod brain;`)
- Test: `crates/tipsy-compadre/tests/brain.rs`

**Interfaces:**
- Consumes: `Providers`, traits and `ProviderError` (Task 7/11), `Pcm` (Task 4), `ChatMessage`, `Role` (Task 7), protocol types (Task 2).
- Produces (the contract Task 13 implements):

```rust
pub struct BrainConfig {
    pub chat_timeout: Duration,        // default 30 s
    pub speech_timeout: Duration,      // default 30 s
    pub transcribe_timeout: Duration,  // default 60 s
    pub retry_backoff: Duration,       // default 800 ms
}
impl Default for BrainConfig { /* values above */ }

pub struct TurnPersona {
    pub prompt: String,
    pub voice_instructions: String,
    pub voice: Option<String>,
    pub language: String,
}

pub enum TurnInput {
    Ask(String),
    Utterance(Pcm),
    Say(String),
}

pub struct Brain { /* providers, persona, config */ }

impl Brain {
    pub fn new(providers: Providers, persona: TurnPersona, config: BrainConfig) -> Self;
    /// Runs one turn, sending events and audio to `out`. Appends to `history` on success.
    pub async fn run_turn(
        &self,
        history: &mut Vec<ChatMessage>,
        input: TurnInput,
        out: tokio::sync::mpsc::UnboundedSender<ToClient>,
        cancel: tokio_util::sync::CancellationToken,
    );
}
```

Behaviour each test pins down (Task 13 must satisfy all of them):

- `Ask(text)`: emits `Status{Thinking}`, one `ReplyDelta` per streamed chunk, then for every spoken sentence `SpeechChunk{seq, sample_rate}` immediately followed by `ToClient::Audio(Frame::Speech{seq, samples})` with `seq` 0, 1, 2… **in sentence order**, then `Reply{text}` where `text = clean_for_speech(<all streamed text concatenated>)`, then `Done{..}`, then `Status{Idle}`. History gains `user(text)` and `assistant(reply)`.
- The chat request sent to the provider is `[system(persona.prompt), ..history, user(text)]`.
- Each speech request carries the cleaned sentence, `persona.voice_instructions` and `persona.voice`.
- `Utterance(pcm)`: emits `Status{Transcribing}`, calls transcribe with `persona.language`, emits `Heard{text}`, then behaves like `Ask(text)`. If the transcript is blank: `Error{NothingHeard, recoverable: true}`, `Status{Idle}`, no chat call, history unchanged.
- `Say(text)`: no chat call, speaks `text` (split into sentences), emits `Reply{text}`, `Done`, `Status{Idle}`; history unchanged.
- Sentences for which `is_speakable(&clean_for_speech(s))` is false are skipped (no speech call) and do not consume a `seq`.
- Chat error before any text: `Error{code, message, recoverable: true}`, `Status{Idle}`, history unchanged. `RateLimited` is retried once after `retry_backoff`.
- Speech failure on one sentence: `Error{.., recoverable: true}` for that sentence, later sentences are not spoken, but `Reply` (full text) and `Done` are still emitted and the history gains the full reply.
- Cancellation: once `cancel` fires, no further `SpeechChunk`/audio is emitted; `Status{Idle}` is emitted; the history gains `user(text)` and `assistant(<only the sentences already sent as audio, joined by a space>)`, or nothing for the assistant if no sentence was sent.
- Speech requests run concurrently (up to `speech.max_parallel()`), but delivery stays in order.

- [ ] **Step 1: Create the stub** `crates/tipsy-compadre/src/brain.rs` with the exact public API above and bodies that do nothing:

```rust
//! One conversation turn: transcribe → stream the LLM → split → synthesise in parallel → deliver in order.

use std::time::Duration;

use tipsy_protocol::ToClient;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::audio::pcm::Pcm;
use crate::providers::{ChatMessage, Providers};

pub struct BrainConfig {
    pub chat_timeout: Duration,
    pub speech_timeout: Duration,
    pub transcribe_timeout: Duration,
    pub retry_backoff: Duration,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            chat_timeout: Duration::from_secs(30),
            speech_timeout: Duration::from_secs(30),
            transcribe_timeout: Duration::from_secs(60),
            retry_backoff: Duration::from_millis(800),
        }
    }
}

pub struct TurnPersona {
    pub prompt: String,
    pub voice_instructions: String,
    pub voice: Option<String>,
    pub language: String,
}

pub enum TurnInput {
    Ask(String),
    Utterance(Pcm),
    Say(String),
}

pub struct Brain {
    #[allow(dead_code)]
    providers: Providers,
    #[allow(dead_code)]
    persona: TurnPersona,
    #[allow(dead_code)]
    config: BrainConfig,
}

impl Brain {
    pub fn new(providers: Providers, persona: TurnPersona, config: BrainConfig) -> Self {
        Self { providers, persona, config }
    }

    pub async fn run_turn(
        &self,
        _history: &mut Vec<ChatMessage>,
        _input: TurnInput,
        _out: UnboundedSender<ToClient>,
        _cancel: CancellationToken,
    ) {
    }
}
```

Add `pub mod brain;` to `lib.rs`.

- [ ] **Step 2: Write the tests** — `crates/tipsy-compadre/tests/brain.rs`:

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tipsy_compadre::audio::pcm::Pcm;
use tipsy_compadre::brain::{Brain, BrainConfig, TurnInput, TurnPersona};
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

fn persona() -> TurnPersona {
    TurnPersona {
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
    let providers = Providers { chat, speech, transcribe: transcribe.clone() };
    (Brain::new(providers, persona(), config()), transcribe)
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
async fn speech_requests_are_cleaned_and_carry_persona_voice() {
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
async fn utterance_is_transcribed_with_persona_language_then_answered() {
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
```

Note on `watcher`: the `rx` loop ends when `run_turn` returns and drops the sender, which is how the join completes.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p tipsy-compadre --test brain`
Expected: the crate compiles; every test FAILS (the stub emits nothing). Report the failure count (12 failed).

- [ ] **Step 4: Commit the failing tests**

```bash
git add crates/tipsy-compadre/src/brain.rs crates/tipsy-compadre/src/lib.rs crates/tipsy-compadre/tests/brain.rs
git commit -m "test(brain): specify turn behaviour with fake providers

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 13: Brain implementation `[sonnet]`

**Files:**
- Modify: `crates/tipsy-compadre/src/brain.rs`

**Interfaces:**
- Consumes: everything listed in Task 12, plus `crate::text::{SentenceSplitter, clean_for_speech, is_speakable}` (Task 3) and `crate::audio::pcm::f32_to_i16` (Task 4).
- Produces: a `Brain::run_turn` that makes **all 12 tests in `tests/brain.rs` pass without modifying the tests**.

Implementation guidance (the tests are the contract; this is the intended shape):

1. Build `messages = [system(prompt)] + history + [user(text)]`.
2. Chat with retry: call `chat.stream` inside `tokio::time::timeout(chat_timeout)`; on `RateLimited` sleep `retry_backoff` and try once more; any other error → `Error{code, message, recoverable: true}`, `Status{Idle}`, return without touching history.
3. Consume the stream inside `tokio::select!` with `cancel.cancelled()`. For each chunk: emit `ReplyDelta`, append to `full_text`, feed `SentenceSplitter`; for each complete sentence, `clean_for_speech`; skip it unless `is_speakable`; otherwise spawn a synthesis task (bounded by a `tokio::sync::Semaphore` of `speech.max_parallel()` permits, each with `speech_timeout`) and push its `JoinHandle` into an ordered `VecDeque` together with the cleaned text.
4. A delivery loop (run concurrently with step 3, e.g. both inside one `select!` loop, or the stream consumer feeding a channel) awaits the **front** handle only, so order is preserved: on success emit `SpeechChunk{seq, sample_rate}` then `Audio(Frame::Speech{seq, f32_to_i16(samples)})`, record `first_audio_ms` on the first one, push the cleaned text into `spoken`, `seq += 1`; on failure emit `Error` once and stop speaking (abort remaining handles) but keep consuming the text stream.
5. At end of stream: flush `splitter.finish()` as a final sentence, drain deliveries, emit `Reply{text: clean_for_speech(&full_text)}` (for plain text this equals the streamed text with whitespace collapsed, which is what the tests expect, e.g. `"{S1} {S2}"`).
6. History: on normal completion push `user(text)` and `assistant(Reply text)`. On cancel push `user(text)` and, if `spoken` is non-empty, `assistant(spoken.join(" "))`.
7. Cancellation: when `cancel` fires, abort every pending handle, emit nothing else except `Status{Idle}`.
8. `Utterance`: `Status{Transcribing}`, transcribe with `transcribe_timeout` and `persona.language`, blank → `NothingHeard`; else `Heard` and continue as `Ask`.
9. `Say`: split with `SentenceSplitter` (push + finish), synthesise/deliver as above, `Reply{text}`, `Done`, `Status{Idle}`, no history change.
10. Status sequence: `Thinking` (or `Transcribing` first) at start, `Speaking` when the first audio frame is sent (emit it once, right before the first `SpeechChunk`), `Idle` last. Note the test `ask_streams_text_and_speaks_sentences_in_order` only checks the first and last status.

Keep `run_turn` readable: split it into private async helpers (`chat_with_retry`, `speak_stream`, …) in the same file. No new dependencies.

- [ ] **Step 1: Implement**
- [ ] **Step 2: Run** `cargo test -p tipsy-compadre --test brain` — Expected: `test result: ok. 12 passed`. Run it three times in a row to catch ordering flakiness.
- [ ] **Step 3: Run the whole suite** `cargo test` — Expected: all green.
- [ ] **Step 4: Commit**

```bash
git add crates/tipsy-compadre/src/brain.rs
git commit -m "feat(brain): stream replies, synthesise sentences in parallel and deliver in order

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 14: UI state, strings and input handling `[haiku]`

**Files:**
- Create: `crates/tipsy-ui/src/strings.rs`, `crates/tipsy-ui/src/state.rs`
- Modify: `crates/tipsy-ui/src/lib.rs`

**Interfaces:**
- Consumes: `tipsy_protocol::*` (Task 2).
- Produces (`tipsy_ui`):
  - `strings::Strings` (all `&'static str` fields listed below) and `strings::strings(lang: UiLanguage) -> &'static Strings`
  - `state::Who { User, Assistant }`, `state::LogEntry { who, text, first_audio_ms: Option<u64>, error: Option<String>, pending: bool }`
  - `state::Tab { Chat, Network }`
  - `state::KeyInput { Char(char), Enter, Backspace, Tab, Esc, Up, Down, Left, Right, F(u8) }`
  - `state::UiAction { Send(Command), Quit, Replay(usize) }`
  - `state::UiState` with public fields listed in the code and methods `new()`, `apply(&mut self, ev: &Event)`, `on_key(&mut self, key: KeyInput) -> Vec<UiAction>`, `tick(&mut self, dt_ms: u64)`, `push_mic_level(&mut self, level: f32)`, `start_voice(&mut self, envelope: Vec<f32>)`, `voice_level(&self) -> f32`, `strings(&self) -> &'static Strings`, `notify(&mut self, text: impl Into<String>)`

- [ ] **Step 1: Write `strings.rs`**

```rust
//! Interface strings. Spanish for Spanish-speaking personas, English for everyone else.

use tipsy_protocol::UiLanguage;

pub struct Strings {
    pub local_time: &'static str,
    pub core: &'static str,
    pub voice: &'static str,
    pub system: &'static str,
    pub chat: &'static str,
    pub network: &'static str,
    pub ask: &'static str,
    pub clear: &'static str,
    pub placeholder: &'static str,
    pub you: &'static str,
    pub ready: &'static str,
    pub listening: &'static str,
    pub transcribing: &'static str,
    pub thinking: &'static str,
    pub speaking: &'static str,
    pub cleared: &'static str,
    pub help: &'static str,
    pub weekdays: [&'static str; 7],
    pub months: [&'static str; 12],
}

pub static ES: Strings = Strings {
    local_time: "HORA LOCAL",
    core: "NÚCLEO",
    voice: "VOZ",
    system: "SISTEMA",
    chat: "CHAT",
    network: "RED",
    ask: "PREGUNTA",
    clear: "✗ Borrar",
    placeholder: "Pregúntale algo... (Tab para hablarle)",
    you: "tú",
    ready: "Listo, quillo",
    listening: "Escuchando... (Tab para terminar)",
    transcribing: "Transcribiendo...",
    thinking: "Pensando...",
    speaking: "Hablando...",
    cleared: "Conversación borrada",
    help: " Enter preguntar · !texto recitar · Tab micro · F4 borrar · ←→ chat/red · Esc salir",
    weekdays: ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"],
    months: ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"],
};

pub static EN: Strings = Strings {
    local_time: "LOCAL TIME",
    core: "CORE",
    voice: "VOICE",
    system: "SYSTEM",
    chat: "CHAT",
    network: "NETWORK",
    ask: "ASK",
    clear: "✗ Clear",
    placeholder: "Ask something... (Tab to talk)",
    you: "you",
    ready: "Ready",
    listening: "Listening... (Tab to stop)",
    transcribing: "Transcribing...",
    thinking: "Thinking...",
    speaking: "Speaking...",
    cleared: "Conversation cleared",
    help: " Enter ask · !text recite · Tab mic · F4 clear · ←→ chat/network · Esc quit",
    weekdays: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
};

pub fn strings(lang: UiLanguage) -> &'static Strings {
    match lang {
        UiLanguage::Es => &ES,
        UiLanguage::En => &EN,
    }
}
```

- [ ] **Step 2: Write `state.rs` tests first** (append the implementation in Step 3 above them):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::*;

    fn welcome(lang: UiLanguage) -> Event {
        Event::Welcome {
            session_id: 1,
            persona: PersonaView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: lang,
            listening_mode: ListeningMode::Push,
            web_bundle: false,
        }
    }

    fn type_text(s: &mut UiState, text: &str) {
        for c in text.chars() {
            s.on_key(KeyInput::Char(c));
        }
    }

    #[test]
    fn welcome_sets_persona_and_language() {
        let mut s = UiState::new();
        s.apply(&welcome(UiLanguage::Es));
        assert_eq!(s.persona.as_ref().unwrap().name, "El Compae");
        assert_eq!(s.strings().core, "NÚCLEO");
        s.apply(&welcome(UiLanguage::En));
        assert_eq!(s.strings().core, "CORE");
    }

    #[test]
    fn enter_sends_ask_and_adds_pending_entries() {
        let mut s = UiState::new();
        type_text(&mut s, "¿qué pasa?");
        let actions = s.on_key(KeyInput::Enter);
        assert_eq!(actions, vec![UiAction::Send(Command::Ask { text: "¿qué pasa?".into() })]);
        assert!(s.input.is_empty());
        assert_eq!(s.log.len(), 2);
        assert_eq!(s.log[0].who, Who::User);
        assert!(s.log[1].pending && s.log[1].who == Who::Assistant);
    }

    #[test]
    fn bang_prefix_sends_say() {
        let mut s = UiState::new();
        type_text(&mut s, "!  hola compadre");
        assert_eq!(s.on_key(KeyInput::Enter), vec![UiAction::Send(Command::Say { text: "hola compadre".into() })]);
        assert_eq!(s.log.len(), 1);
        assert!(s.log[0].pending);
    }

    #[test]
    fn blank_enter_does_nothing() {
        let mut s = UiState::new();
        type_text(&mut s, "   ");
        assert!(s.on_key(KeyInput::Enter).is_empty());
        assert!(s.log.is_empty());
    }

    #[test]
    fn streamed_reply_fills_pending_entry() {
        let mut s = UiState::new();
        type_text(&mut s, "x");
        s.on_key(KeyInput::Enter);
        s.apply(&Event::ReplyDelta { text: "Ozú".into() });
        s.apply(&Event::ReplyDelta { text: ", quillo".into() });
        assert_eq!(s.log[1].text, "Ozú, quillo");
        s.apply(&Event::Reply { text: "Ozú, quillo.".into() });
        s.apply(&Event::Done { first_audio_ms: Some(1200), total_ms: 3000 });
        assert_eq!(s.log[1].text, "Ozú, quillo.");
        assert!(!s.log[1].pending);
        assert_eq!(s.log[1].first_audio_ms, Some(1200));
    }

    #[test]
    fn heard_inserts_user_entry_before_pending_reply() {
        let mut s = UiState::new();
        s.on_key(KeyInput::Tab); // ListenStart
        let actions = s.on_key(KeyInput::Tab); // ListenStop adds a pending reply
        assert_eq!(actions, vec![UiAction::Send(Command::ListenStop)]);
        s.apply(&Event::Heard { text: "¿y el ticket?".into() });
        assert_eq!(s.log[0].who, Who::User);
        assert_eq!(s.log[0].text, "¿y el ticket?");
        assert!(s.log[1].pending);
    }

    #[test]
    fn tab_toggles_listening() {
        let mut s = UiState::new();
        assert_eq!(s.on_key(KeyInput::Tab), vec![UiAction::Send(Command::ListenStart)]);
        assert!(s.mic_open);
        assert_eq!(s.on_key(KeyInput::Tab), vec![UiAction::Send(Command::ListenStop)]);
        assert!(!s.mic_open);
    }

    #[test]
    fn error_marks_pending_entry_and_notifies() {
        let mut s = UiState::new();
        type_text(&mut s, "x");
        s.on_key(KeyInput::Enter);
        s.apply(&Event::Error { code: ErrorCode::Auth, message: "bad key".into(), recoverable: true });
        assert!(!s.log[1].pending);
        assert_eq!(s.log[1].error.as_deref(), Some("bad key"));
        assert_eq!(s.notice.as_ref().map(|n| n.0.as_str()), Some("bad key"));
    }

    #[test]
    fn f4_clears_and_sends_reset() {
        let mut s = UiState::new();
        type_text(&mut s, "x");
        s.on_key(KeyInput::Enter);
        s.apply(&Event::Done { first_audio_ms: None, total_ms: 1 });
        assert_eq!(s.on_key(KeyInput::F(4)), vec![UiAction::Send(Command::Reset)]);
        assert!(s.log.is_empty());
        assert!(s.notice.is_some());
    }

    #[test]
    fn arrows_switch_tabs_and_esc_quits() {
        let mut s = UiState::new();
        assert_eq!(s.tab, Tab::Chat);
        s.on_key(KeyInput::Right);
        assert_eq!(s.tab, Tab::Network);
        s.on_key(KeyInput::Left);
        assert_eq!(s.tab, Tab::Chat);
        assert_eq!(s.on_key(KeyInput::Esc), vec![UiAction::Quit]);
    }

    #[test]
    fn f2_replays_selected_finished_assistant_entry() {
        let mut s = UiState::new();
        type_text(&mut s, "x");
        s.on_key(KeyInput::Enter);
        s.apply(&Event::Done { first_audio_ms: None, total_ms: 1 });
        s.on_key(KeyInput::Down);
        s.on_key(KeyInput::Down);
        assert_eq!(s.selected, Some(1));
        assert_eq!(s.on_key(KeyInput::F(2)), vec![UiAction::Replay(1)]);
    }

    #[test]
    fn notices_expire_with_tick() {
        let mut s = UiState::new();
        s.notify("hola");
        s.tick(2999);
        assert!(s.notice.is_some());
        s.tick(2);
        assert!(s.notice.is_none());
    }

    #[test]
    fn voice_envelope_advances_with_tick() {
        let mut s = UiState::new();
        s.start_voice(vec![0.2, 0.9, 0.4]);
        assert!((s.voice_level() - 0.2).abs() < 1e-6);
        s.tick(50);
        assert!((s.voice_level() - 0.9).abs() < 1e-6);
        s.tick(200);
        assert_eq!(s.voice_level(), 0.0);
    }

    #[test]
    fn stats_feed_history_capped_at_120() {
        let mut s = UiState::new();
        for i in 0..130 {
            s.apply(&Event::Stats { cpu: i as f32, ram: 50.0, net_in: 10.0, net_out: 1.0 });
        }
        assert_eq!(s.cpu_history.len(), 120);
        assert_eq!(s.net_history.len(), 120);
        assert_eq!(*s.cpu_history.back().unwrap(), 129.0);
    }

    #[test]
    fn mic_levels_are_capped() {
        let mut s = UiState::new();
        for _ in 0..500 {
            s.push_mic_level(0.5);
        }
        assert!(s.mic_levels.len() <= 400);
    }
}
```

- [ ] **Step 3: Implement `state.rs`** (above the tests):

```rust
//! Interface state: everything the HUD draws, updated from server events and user input.

use std::collections::VecDeque;

use tipsy_protocol::{AssistantState, Command, Event, ListeningMode, PersonaView, UiLanguage};

use crate::strings::{strings, Strings};

const NOTICE_MS: u64 = 3000;
const HISTORY_LEN: usize = 120;
const MIC_LEVELS_MAX: usize = 400;
const ENVELOPE_HOP_MS: u64 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub who: Who,
    pub text: String,
    pub first_audio_ms: Option<u64>,
    pub error: Option<String>,
    pub pending: bool,
}

impl LogEntry {
    fn user(text: impl Into<String>) -> Self {
        Self { who: Who::User, text: text.into(), first_audio_ms: None, error: None, pending: false }
    }
    fn pending_assistant(text: impl Into<String>) -> Self {
        Self { who: Who::Assistant, text: text.into(), first_audio_ms: None, error: None, pending: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Chat,
    Network,
}

/// Keys the HUD understands, independent of any terminal library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Esc,
    Up,
    Down,
    Left,
    Right,
    F(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    Send(Command),
    Quit,
    /// Replay the audio of the log entry at this index (handled by the client, which kept the audio).
    Replay(usize),
}

pub struct UiState {
    pub persona: Option<PersonaView>,
    pub lang: UiLanguage,
    pub listening_mode: ListeningMode,
    pub status: AssistantState,
    pub mic_open: bool,
    pub log: Vec<LogEntry>,
    pub selected: Option<usize>,
    pub input: String,
    pub tab: Tab,
    /// Text and remaining milliseconds.
    pub notice: Option<(String, u64)>,
    pub elapsed_ms: u64,
    pub turn_ms: u64,
    pub cpu: f32,
    pub ram: f32,
    pub net_in: f64,
    pub net_out: f64,
    pub cpu_history: VecDeque<f32>,
    pub net_history: VecDeque<(f64, f64)>,
    pub mic_levels: Vec<f32>,
    pub voice_envelope: Vec<f32>,
    pub voice_pos_ms: u64,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        Self {
            persona: None,
            lang: UiLanguage::Es,
            listening_mode: ListeningMode::Push,
            status: AssistantState::Idle,
            mic_open: false,
            log: Vec::new(),
            selected: None,
            input: String::new(),
            tab: Tab::Chat,
            notice: None,
            elapsed_ms: 0,
            turn_ms: 0,
            cpu: 0.0,
            ram: 0.0,
            net_in: 0.0,
            net_out: 0.0,
            cpu_history: VecDeque::from(vec![0.0; HISTORY_LEN]),
            net_history: VecDeque::from(vec![(0.0, 0.0); HISTORY_LEN]),
            mic_levels: Vec::new(),
            voice_envelope: Vec::new(),
            voice_pos_ms: 0,
        }
    }

    pub fn strings(&self) -> &'static Strings {
        strings(self.lang)
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), NOTICE_MS));
    }

    fn pending_assistant(&mut self) -> Option<&mut LogEntry> {
        self.log.iter_mut().rev().find(|e| e.pending && e.who == Who::Assistant)
    }

    pub fn busy(&self) -> bool {
        self.log.iter().any(|e| e.pending)
    }

    pub fn apply(&mut self, ev: &Event) {
        match ev {
            Event::Welcome { persona, ui_language, listening_mode, .. } => {
                self.persona = Some(persona.clone());
                self.lang = *ui_language;
                self.listening_mode = *listening_mode;
            }
            Event::Status { state, .. } => self.status = *state,
            Event::Heard { text } => {
                let at = self
                    .log
                    .iter()
                    .rposition(|e| e.pending && e.who == Who::Assistant)
                    .unwrap_or(self.log.len());
                self.log.insert(at, LogEntry::user(text.clone()));
            }
            Event::ReplyDelta { text } => {
                if let Some(e) = self.pending_assistant() {
                    if e.text == "…" {
                        e.text.clear();
                    }
                    e.text.push_str(text);
                }
            }
            Event::Reply { text } => {
                if let Some(e) = self.pending_assistant() {
                    e.text = text.clone();
                }
            }
            Event::SpeechChunk { .. } => {}
            Event::Done { first_audio_ms, .. } => {
                if let Some(e) = self.pending_assistant() {
                    e.pending = false;
                    e.first_audio_ms = *first_audio_ms;
                }
            }
            Event::Stats { cpu, ram, net_in, net_out } => {
                self.cpu = *cpu;
                self.ram = *ram;
                self.net_in = *net_in;
                self.net_out = *net_out;
                self.cpu_history.push_back(*cpu);
                self.net_history.push_back((*net_in, *net_out));
                while self.cpu_history.len() > HISTORY_LEN {
                    self.cpu_history.pop_front();
                }
                while self.net_history.len() > HISTORY_LEN {
                    self.net_history.pop_front();
                }
            }
            Event::Error { message, .. } => {
                if let Some(e) = self.pending_assistant() {
                    e.pending = false;
                    e.error = Some(message.clone());
                }
                self.notify(message.clone());
            }
        }
    }

    pub fn on_key(&mut self, key: KeyInput) -> Vec<UiAction> {
        match key {
            KeyInput::Char(c) => {
                self.input.push(c);
                Vec::new()
            }
            KeyInput::Backspace => {
                self.input.pop();
                Vec::new()
            }
            KeyInput::Enter => self.submit(),
            KeyInput::Tab | KeyInput::F(3) => {
                self.mic_open = !self.mic_open;
                if self.mic_open {
                    vec![UiAction::Send(Command::ListenStart)]
                } else {
                    self.turn_ms = 0;
                    self.log.push(LogEntry::pending_assistant("…"));
                    self.select_last();
                    vec![UiAction::Send(Command::ListenStop)]
                }
            }
            KeyInput::F(4) => {
                self.log.clear();
                self.selected = None;
                self.notify(self.strings().cleared);
                vec![UiAction::Send(Command::Reset)]
            }
            KeyInput::F(2) => match self.selected {
                Some(i) if self.log.get(i).is_some_and(|e| e.who == Who::Assistant && !e.pending) => vec![UiAction::Replay(i)],
                _ => Vec::new(),
            },
            KeyInput::Left | KeyInput::Right | KeyInput::F(6) => {
                self.tab = if self.tab == Tab::Chat { Tab::Network } else { Tab::Chat };
                Vec::new()
            }
            KeyInput::Up => {
                self.selected = match self.selected {
                    _ if self.log.is_empty() => None,
                    None => Some(self.log.len() - 1),
                    Some(i) => Some(i.saturating_sub(1)),
                };
                Vec::new()
            }
            KeyInput::Down => {
                self.selected = match self.selected {
                    _ if self.log.is_empty() => None,
                    None => Some(0),
                    Some(i) => Some((i + 1).min(self.log.len() - 1)),
                };
                Vec::new()
            }
            KeyInput::Esc => vec![UiAction::Quit],
            KeyInput::F(_) => Vec::new(),
        }
    }

    fn select_last(&mut self) {
        self.selected = self.log.len().checked_sub(1);
    }

    fn submit(&mut self) -> Vec<UiAction> {
        let text = self.input.trim().to_string();
        self.input.clear();
        if text.is_empty() {
            return Vec::new();
        }
        self.turn_ms = 0;
        let action = if let Some(literal) = text.strip_prefix('!') {
            let literal = literal.trim().to_string();
            self.log.push(LogEntry::pending_assistant(literal.clone()));
            UiAction::Send(Command::Say { text: literal })
        } else {
            self.log.push(LogEntry::user(text.clone()));
            self.log.push(LogEntry::pending_assistant("…"));
            UiAction::Send(Command::Ask { text })
        };
        self.select_last();
        vec![action]
    }

    /// Advances clocks. Call once per frame with the milliseconds since the previous call.
    pub fn tick(&mut self, dt_ms: u64) {
        self.elapsed_ms += dt_ms;
        self.turn_ms += dt_ms;
        self.voice_pos_ms += dt_ms;
        if let Some((_, left)) = &mut self.notice {
            *left = left.saturating_sub(dt_ms);
            if *left == 0 {
                self.notice = None;
            }
        }
    }

    pub fn push_mic_level(&mut self, level: f32) {
        self.mic_levels.push(level.clamp(0.0, 1.0));
        if self.mic_levels.len() > MIC_LEVELS_MAX {
            self.mic_levels.drain(..MIC_LEVELS_MAX / 2);
        }
    }

    /// Starts the voice visualiser with the envelope (one value per 50 ms) of audio that just began playing.
    pub fn start_voice(&mut self, envelope: Vec<f32>) {
        self.voice_envelope = envelope;
        self.voice_pos_ms = 0;
    }

    pub fn voice_level(&self) -> f32 {
        let i = (self.voice_pos_ms / ENVELOPE_HOP_MS) as usize;
        self.voice_envelope.get(i).copied().unwrap_or(0.0)
    }
}
```

`crates/tipsy-ui/src/lib.rs`:

```rust
//! Backend-agnostic ratatui HUD for tipsy-compadre.

pub mod state;
pub mod strings;

pub use state::*;
pub use strings::{strings, Strings};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p tipsy-ui`
Expected: `test result: ok. 15 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-ui
git commit -m "feat(ui): add hud state, es/en strings and keyboard handling

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 15: HUD rendering tests `[haiku]`

Tests only; they fail until Task 16 implements rendering.

**Files:**
- Create: `crates/tipsy-ui/src/render.rs` (stub, see Step 1)
- Modify: `crates/tipsy-ui/src/lib.rs` (add `pub mod render;` and `pub use render::*;`)
- Test: `crates/tipsy-ui/tests/render.rs`

**Interfaces:**
- Consumes: `UiState` (Task 14).
- Produces (the contract for Task 16):

```rust
pub struct ClockView { pub time: String, pub weekday: u8 /* 0 = Monday */, pub day: u8, pub month: u8 /* 0 = January */ }
pub struct Hit { pub area: ratatui::layout::Rect, pub key: KeyInput }
/// Draws the whole HUD and returns the clickable areas of this frame.
pub fn draw(frame: &mut ratatui::Frame, state: &UiState, clock: &ClockView, title: &str) -> Vec<Hit>;
```

Clickable areas (in this order): the `CHAT` tab label → `KeyInput::F(7)`, the `RED`/`NETWORK` label → `KeyInput::F(8)`, the clear button → `KeyInput::F(4)`. Task 16 also makes `UiState::on_key` treat `F(7)` as "switch to Chat" and `F(8)` as "switch to Network" (add those two arms in `state.rs`).

- [ ] **Step 1: Stub `render.rs`** so the tests compile:

```rust
//! HUD drawing.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::state::{KeyInput, UiState};

pub struct ClockView {
    pub time: String,
    pub weekday: u8,
    pub day: u8,
    pub month: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub area: Rect,
    pub key: KeyInput,
}

pub fn draw(_frame: &mut Frame, _state: &UiState, _clock: &ClockView, _title: &str) -> Vec<Hit> {
    Vec::new()
}
```

- [ ] **Step 2: Write `crates/tipsy-ui/tests/render.rs`**

```rust
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tipsy_protocol::*;
use tipsy_ui::*;

fn state(lang: UiLanguage) -> UiState {
    let mut s = UiState::new();
    s.apply(&Event::Welcome {
        session_id: 1,
        persona: PersonaView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: lang,
        listening_mode: ListeningMode::Push,
        web_bundle: false,
    });
    s
}

fn clock() -> ClockView {
    ClockView { time: "10:42:07".into(), weekday: 4, day: 10, month: 9 }
}

fn render(s: &UiState) -> (String, Vec<Hit>, Terminal<TestBackend>) {
    let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
    let mut hits = Vec::new();
    term.draw(|f| hits = draw(f, s, &clock(), "Compae · el asistente de garrafón")).unwrap();
    let text: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
    (text, hits, term)
}

fn text_at(term: &Terminal<TestBackend>, r: ratatui::layout::Rect) -> String {
    (r.x..r.right()).map(|x| term.backend().buffer()[(x, r.y)].symbol().to_string()).collect()
}

#[test]
fn spanish_hud_shows_every_panel() {
    let (text, _, _) = render(&state(UiLanguage::Es));
    for needle in ["COMPAE · EL ASISTENTE DE GARRAFÓN", "HORA LOCAL", "10:42:07", "viernes, 10 de octubre", "NÚCLEO", "VOZ", "SISTEMA", "CHAT", "RED", "PREGUNTA", "Borrar"] {
        assert!(text.contains(needle), "missing {needle:?}");
    }
}

#[test]
fn english_hud_uses_english_strings() {
    let (text, _, _) = render(&state(UiLanguage::En));
    for needle in ["LOCAL TIME", "Friday, October 10", "CORE", "VOICE", "SYSTEM", "NETWORK", "ASK", "Clear"] {
        assert!(text.contains(needle), "missing {needle:?}");
    }
}

#[test]
fn chat_log_shows_user_and_assistant_lines() {
    let mut s = state(UiLanguage::Es);
    for c in "¿y el ticket?".chars() {
        s.on_key(KeyInput::Char(c));
    }
    s.on_key(KeyInput::Enter);
    s.apply(&Event::Reply { text: "Ozú, quillo, eso es de backend.".into() });
    s.apply(&Event::Done { first_audio_ms: Some(1500), total_ms: 4000 });
    let (text, _, _) = render(&s);
    assert!(text.contains("tú ¿y el ticket?"));
    assert!(text.contains("Ozú, quillo, eso es de backend."));
    assert!(text.contains("1.5s"));
}

#[test]
fn input_box_shows_typed_text_or_placeholder() {
    let mut s = state(UiLanguage::Es);
    let (text, _, _) = render(&s);
    assert!(text.contains("Pregúntale algo"));
    for c in "hola".chars() {
        s.on_key(KeyInput::Char(c));
    }
    let (text, _, _) = render(&s);
    assert!(text.contains("❯ hola"));
}

#[test]
fn network_tab_shows_chart_legend() {
    let mut s = state(UiLanguage::Es);
    s.apply(&Event::Stats { cpu: 10.0, ram: 20.0, net_in: 2048.0, net_out: 10.0 });
    s.on_key(KeyInput::Right);
    let (text, _, _) = render(&s);
    assert!(text.contains("IN ") && text.contains("2.0 KB/s"));
}

#[test]
fn listening_shows_red_indicator_text() {
    let mut s = state(UiLanguage::Es);
    s.on_key(KeyInput::Tab);
    let (text, _, _) = render(&s);
    assert!(text.contains("Escuchando"));
}

#[test]
fn clickable_areas_sit_on_their_labels() {
    let (_, hits, term) = render(&state(UiLanguage::Es));
    let labels: Vec<(String, KeyInput)> = hits.iter().map(|h| (text_at(&term, h.area), h.key)).collect();
    assert_eq!(labels, vec![
        (" CHAT ".to_string(), KeyInput::F(7)),
        (" RED ".to_string(), KeyInput::F(8)),
        (" ✗ Borrar ".to_string(), KeyInput::F(4)),
    ]);
}

#[test]
fn tiny_terminals_do_not_panic() {
    let s = state(UiLanguage::Es);
    for (w, h) in [(1, 1), (20, 5), (40, 12)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            draw(f, &s, &clock(), "x");
        })
        .unwrap();
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p tipsy-ui --test render`
Expected: the crate compiles; tests FAIL (empty screen), except `tiny_terminals_do_not_panic`, which passes trivially.

- [ ] **Step 4: Commit**

```bash
git add crates/tipsy-ui
git commit -m "test(ui): specify hud rendering and click areas

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 16: HUD rendering `[sonnet]`

**Files:**
- Modify: `crates/tipsy-ui/src/render.rs`, `crates/tipsy-ui/src/state.rs` (add `F(7)`/`F(8)` arms)
- Reference: `poc/src/main.rs` (the proof of concept HUD to port)

**Interfaces:**
- Consumes: Task 14 and Task 15 contracts.
- Produces: `draw()` making **all 8 tests in `crates/tipsy-ui/tests/render.rs` pass without modifying them**, plus the existing 15 state tests.

Port the proof of concept HUD from `poc/src/main.rs` (functions `draw`, `draw_header`, `draw_clock`, `draw_left`, `draw_log`, `draw_net`, `draw_rings`, `draw_voice`, `draw_system`, `draw_input`, `panel`, `gauge`, `arc`, `human`) with these changes:

- Read everything from `&UiState`, `&ClockView` and `title`; no `Instant`, no `chrono`, no `sysinfo` (use `state.elapsed_ms` for animations, `state.cpu`/`ram`/`net_*`/`*_history` for the system panel and network chart, `state.voice_level()` and `state.voice_envelope`/`voice_pos_ms` for the core pulse and the oscilloscope, `state.mic_levels` when `state.mic_open`).
- All labels from `state.strings()`. Date line: Spanish `"{weekday}, {day} de {month}"`, English `"{Weekday}, {Month} {day}"`.
- Header: `" ◉ {TITLE UPPERCASE} "` badge, then the status: listening (red, `strings.listening`), busy (`strings.thinking` / `speaking` / `transcribing` per `state.status`, plus `{turn_ms/1000}s`), notice (gold, while `state.notice` is set), or ready (green `● {strings.ready}`).
- Left panel title row: ` CHAT │ RED  ←→ ` on the left and the gold ` ✗ Borrar ` button right-aligned; return the three `Hit`s exactly as the test expects (`CHAT`/`RED` label areas include one space on each side).
- Log entries: user `"{you} {text}"`, assistant `"◆  {text}"` plus `" {s:.1}s"` when `first_audio_ms` is set (`1500` → `1.5s`), errors in red `"◆  ✗ {error}"`, pending in gold italics.
- `human()` formats bytes/s: `≥ 1e6` → `"{:.1} MB/s"`, `≥ 1e3` → `"{:.1} KB/s"`, else `"{:.0} B/s"` (so 2048 → `2.0 KB/s`).
- Guard every layout against tiny areas (`tiny_terminals_do_not_panic`).
- Keep the colours of the proof of concept (`CYAN`, `DIM`, `GOLD`).

- [ ] **Step 1: Implement**
- [ ] **Step 2: Run** `cargo test -p tipsy-ui` — Expected: `23 passed` (15 state + 8 render).
- [ ] **Step 3: Commit**

```bash
git add crates/tipsy-ui
git commit -m "feat(ui): port the hud from the proof of concept onto ui state

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 17: Session, system monitor, native audio and terminal client `[sonnet]`

Wires everything into a runnable `tipsy-compadre` binary (terminal first). Hardware-bound and concurrent, hence Sonnet. Its unit tests are in Step 1 and must be written **first by Haiku** as a separate dispatch (`[haiku]` sub-step), then implemented by Sonnet.

**Files:**
- Create: `crates/tipsy-compadre/src/session.rs`, `src/sysmon.rs`, `src/audio/device.rs`, `src/client.rs`
- Modify: `crates/tipsy-compadre/src/main.rs`, `src/lib.rs`, `src/audio/mod.rs`
- Test: `crates/tipsy-compadre/tests/session.rs`

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `session::spawn_session(config: Config, providers: Providers, brain_config: BrainConfig) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>)` — on spawn sends `Welcome` (persona view, `UiLanguage::from_bcp47(persona.language)`, listening mode, `web_bundle: false`) and then speaks the persona `greeting` (a `Say` turn).
    - `Ask`/`Say` → brain turn (spawned task with a fresh `CancellationToken`; the session keeps the `history`).
    - `ListenStart` → start buffering `Frame::Mic` samples; `ListenStop` → turn with `TurnInput::Utterance(Pcm{samples: i16_to_f32(buffer), sample_rate: 16000})`.
    - `Cancel` → cancel the running turn. `Reset` → cancel and clear history.
    - A new `Ask`/`ListenStop` while a turn runs cancels the running turn first (barge-in).
  - `sysmon::spawn(tx: UnboundedSender<ToClient>)` — every second sends `Event::Stats` (global CPU %, used RAM %, network bytes/s in and out).
  - `audio::device::Player` — `Player::new() -> Result<Player, String>`, `play(&self, samples: Vec<i16>, sample_rate: u32)` (queues, non-blocking), `stop(&self)` (drops the queue); built on rodio 0.22.
  - `audio::device::Mic` — `Mic::start(on_frame: impl FnMut(Vec<i16>) + Send + 'static) -> Result<Mic, String>` capturing the default input device with cpal 0.18, converting to mono, resampling to 16 kHz (`resample_linear`) and calling `on_frame` every ~20 ms (320 samples); dropping `Mic` stops capture.
  - `client::run(config_path: PathBuf) -> anyhow-free Result<(), String>` — the terminal client: ratatui `init`/`restore`, `EnableMouseCapture`, 33 ms frame loop, maps crossterm keys to `KeyInput` (`Enter`, `Backspace`, `Tab`, `Esc`, arrows, `F(n)`, printable chars) and left clicks via the `Hit`s returned by `draw`; forwards `UiAction::Send` to the session; on `SpeechChunk`+`Frame::Speech` plays the audio with `Player`, calls `state.start_voice(envelope(...))` and keeps the last reply's audio for `Replay`; while `state.mic_open` runs `Mic` and forwards frames as `ToServer::Audio(Frame::Mic)` plus `state.push_mic_level(rms * mic_gain)`; ticks the state; builds `ClockView` from `chrono::Local::now()`; title from the active persona name.
  - CLI (`main.rs`, clap):
    - `tipsy-compadre` → load config (`store::load_or_create(store::default_path())`, show `warning` as a notice), build providers with `KeyResolver::system()`. On `BuildError::MissingKey` print the error message and exit with code 2 (the configuration screen arrives in Plan 2). Otherwise run the client.
    - `tipsy-compadre key set <name>` → read one line from stdin (prompt `Paste the key for <name>: `), store with `OsKeyring`, print `Saved <name> (<redacted>)`.
    - `tipsy-compadre key status <name>` → `env (<redacted>)`, `credential store (<redacted>)` or `missing`.
    - `tipsy-compadre key delete <name>` → delete from the credential store.
    - `tipsy-compadre config path` → print the config file path.

- [ ] **Step 1 `[haiku]`: Write `crates/tipsy-compadre/tests/session.rs`** using fake providers (copy the `FakeChat`, `FakeSpeech`, `FakeTranscribe` fakes from `tests/brain.rs` into this file) and a default `Config` whose persona greeting is the built-in one:

```rust
// Test list (Haiku writes the full code for each, using the fakes):
// 1. welcome_then_greeting: first message is Event::Welcome with persona id "compae" and
//    ui_language Es; then the greeting is spoken (at least one Frame::Speech) without any chat call.
// 2. ask_produces_reply_and_done: after the greeting's Done, sending Command::Ask yields
//    Event::Reply and Event::Done.
// 3. listen_stop_transcribes_buffered_mic_audio: ListenStart, 10 ToServer::Audio(Frame::Mic(vec![100; 320])),
//    ListenStop → FakeTranscribe receives one call whose Pcm has 3200 samples at 16000 Hz, then Heard.
// 4. reset_clears_history: Ask, wait Done, Reset, Ask → the second chat request contains only
//    [system, user] (no previous turn).
// 5. barge_in_cancels_running_turn: with a slow FakeChat (60 ms per chunk, 3 sentences), send Ask,
//    wait for the first Frame::Speech, send Cancel → no more Frame::Speech arrive within 500 ms and an
//    Event::Status{Idle} arrives.
// Helper: async fn next_matching(rx, pred, timeout_ms) -> ToClient that panics with the list of
// received messages if the timeout expires.
```

Haiku writes these five tests in full (no placeholders), runs `cargo test -p tipsy-compadre --test session` (expected: compile error because `session` does not exist yet — that is the failing state), and commits with `test(session): specify session behaviour`.

- [ ] **Step 2 `[sonnet]`: Implement** `session.rs`, `sysmon.rs`, `audio/device.rs`, `client.rs` and `main.rs` per the interfaces above until `cargo test` is fully green.
- [ ] **Step 3 `[sonnet]`: Manual check on real hardware** (terminal first). With a real key: `cargo run -p tipsy-compadre -- key set openrouter`, then `cargo run -p tipsy-compadre`. Verify: greeting is heard; typing a question gets a spoken reply in El Compae's voice; `Tab` → talk → `Tab` gets transcribed and answered; `F4` clears; `←/→` switches to the network chart; mouse clicks on `CHAT`/`RED`/`✗ Borrar` work; `Esc` restores the terminal. Report the measured "first audio" times shown in the log. If the transcription model `openai/whisper-large-v3` is rejected by OpenRouter, find the correct slug on the OpenRouter models page, update the default in `config/model.rs` and its test, and note it in the commit.
- [ ] **Step 4: Commit**

```bash
git add crates/tipsy-compadre
git commit -m "feat: wire session, audio devices and the terminal client into the binary

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 18: README quickstart and final review `[haiku]` then `[sonnet]`

**Files:**
- Create: `README.md`

- [ ] **Step 1 `[haiku]`: Write `README.md`** with: one-paragraph description (from spec §1), a short "Status: early development — terminal client only" note, **Quick start** (`cargo run -p tipsy-compadre -- key set openrouter`, `cargo run -p tipsy-compadre`, the `TIPSY_KEY_OPENROUTER` alternative), **Keys** table (`Enter`, `!text`, `Tab`, `F2`, `F4`, `←/→`, `Esc`, mouse), **Configuration** (path printed by `tipsy-compadre config path`, the `[providers.*]` sections with an Ollama example using `base_url = "http://localhost:11434/v1"` and `key = ""`), **Privacy** (questions, answers and recorded speech are sent to the configured providers; keys stay in the OS credential store), **Persona guideline** (the affectionate-humour rule from spec §8.3), links to `docs/architecture.md` and the spec, and the Windows build note from Global Constraints. Commit with `docs: add readme quickstart`.
- [ ] **Step 2 `[sonnet]`: Whole-branch review.** Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`; fix what they report. Then review the full diff since the spec commit against the spec and the **Review Focus** list at the top of this plan; fix real defects, and list anything deliberately deferred to Plan 2 in the final report. Commit fixes with `fix: address final review findings`.

---

## Self-review notes (for the plan author)

- Spec coverage for this plan's scope (milestones 1–4): workspace (T1), protocol (T2), configuration and keys (T5, T6, T17 CLI), providers `openai_compat` (T7–T11), brain with streaming, ordering and barge-in (T12–T13, T17), native TUI port in `push` mode (T14–T17), errors (T7, T12, T17), tests (all). Deferred on purpose to later plans: configuration screen, eight more personas, Gemini/ElevenLabs connectors, `auto`/`hands_free` modes, WebSocket server, web client, web terminal, CI and release.
- Deviations from the spec, documented in the tasks: `SpeechChunk` has no `last` flag and `Done.first_audio_ms` is optional (T2); provider traits take no cancellation token, callers drop futures instead (T7); default listening mode is `push` until VAD lands in Plan 2 (T5).
