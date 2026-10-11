# Plan 2B — Configuration screen and auto mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Amended during execution — read the ledger beside it.** This document's task bodies were
> written before any of the work began, and the implementation changed some of the interfaces they
> pin. `VOICE_LABEL_ROW` is now the computed `ConfigForm::voice_label_row()`; a profile contributes
> **seven** field rows, not eleven; `ConfigForm::tab_title` is a method rather than a free function;
> and the mode strings that are serde spellings are deliberately identical in both string tables
> rather than translated. Task 3 recorded twenty amendments to its own brief, Task 8 fixed four
> defects in its brief and Task 9 two, and this plan's `mic_device` field is stored, rendered and
> **read by nobody** (the README says so). **Where this document and the code disagree, the code and
> `.superpowers/sdd/2026-10-10-plan-2b-config-screen-and-auto-mode/progress.md` win** — the ledger
> carries every ruling, every amendment and every carried finding.
**Goal:** Make the assistant configurable from inside the terminal — a full-screen screen on `F9` (or the header's ⚙) with Providers, Listening, Profiles and Server tabs, applied on save without restarting — and make `auto` mode real, so one press of `Tab` starts listening and the assistant stops by itself after the configured silence.

**Architecture:** The screen's editing state lives in `tipsy-ui` as a **form view model** of plain values (`ConfigForm`), never in the native configuration types: `tipsy-ui` must keep compiling to `wasm32-unknown-unknown` and cannot depend on `tipsy-compadre`. The native client owns the two conversions (`Config` → `ConfigForm` to open the screen, `ConfigForm` → `Config` on save), writes the file, rebuilds the providers and applies the new configuration to the running session without a restart. Voice activity detection is a small state machine in `tipsy-compadre` over `earshot`, driven by the microphone frames the client already streams to the session, and it is paused while the assistant speaks.

**Tech Stack:** the workspace as Plan 2A leaves it, plus **`earshot` 1.2** (pure Rust, `MIT OR Apache-2.0`, no C toolchain, no model file; the VAD research note is the source of that choice) for the detector, and no other new dependency — the token is generated with a crate the workspace already builds.

**Spec:** `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md` — §6 (listening modes and the VAD), §8.1–8.2 (the config file and keys), §9 (the configuration screen, the header and the keyboard/mouse rule), §13 (interface and listening tests).

## Research inputs

- `.superpowers/sdd/plan-2-research/rust-vad.md` — the `earshot` recommendation, the exact version, the 256-sample/16 kHz frame it wants, the proposed `is_voice` signature and the **API uncertainty the first task must settle** (the published 1.2.2 may expose only the deprecated `predict_i16`).

## Decisions this plan makes, and why

- **The screen edits a view model, not the configuration types.** `crates/tipsy-ui/src/config_form.rs` holds `ConfigForm` — providers, listening, profiles and server as plain strings, numbers, booleans, enums and small tables — plus the reducer that handles keys and mouse. `tipsy-compadre` converts `Config` ⇄ `ConfigForm` and performs every effect (write the file, rebuild providers, retest, regenerate the token). This is what keeps the HUD reusable by the future web client and the dependency arrow pointing one way; the conversion gets a round-trip test so the two shapes cannot drift.
- **A save writes the whole file and only on an explicit save.** `F9`/⚙ opens the screen on a copy; `Ctrl+S` (and the save button) writes it through `store::save`; `Esc` closes discarding nothing but the edits. A failed write leaves the file and the running configuration exactly as they were and shows the error in the screen's notice line.
- **Applying a saved configuration rebuilds the providers in place** and tells the running session about the changes (listening mode, profile, timeouts) through the existing in-process channel, so a mode change takes effect on the next turn rather than at the next launch.
- **`TestProvider` becomes a protocol pair.** The screen's "Test" button needs a request and a result that the client can render, so `tipsy-protocol` gains one command and one event (`Command::TestProvider { capability }` and `Event::TestResult { capability, ok, latency_ms, message }`), serde round-tripped and wasm-safe like the rest of the crate, and the client measures the latency of a real minimal call.
- **The server token lives in the configuration now.** §9's Server tab displays and regenerates it; the WebSocket server that *requires* it arrives in Plan 3. The field defaults in for existing files, and regeneration is explicit (the screen's button) rather than automatic.
- **`mic_device` is optional and native-only.** The Listening tab shows the device name for the native client and the field defaults in for old files; the web client has no such concept and the screen hides the row when the value is absent rather than inventing one.
- **The detector is `earshot`, behind our own state machine.** The crate answers "voice or not" per 16 ms frame; `tipsy-compadre` owns the onset counter, the hangover that implements `silence_ms`, the pause/resume around the assistant's speech plus 300 ms, and the decision to end the utterance. Task 8 starts by *verifying the published API* (the research note flags that 1.2.2's docs list only the deprecated `predict_i16`) and falls back to the deprecated entry point if that is what the published version has — recorded in the task, with a test either way.
- **A hand-written energy/zero-crossing VAD is the documented fallback**, not the plan: the research note describes what it would have to get right (noise-floor asymmetry, hysteresis, pre-roll so the first syllable is never clipped), and Task 8 keeps that note in its brief in case `earshot` disappoints on the real microphone.

## Global Constraints

- Toolchain pinned by `rust-toolchain.toml` to `1.96.1`; every crate uses `edition = "2024"`.
- All code, identifiers, comments, docs and commit messages are in **English**. Interface strings and profile texts are the exception: the two static string tables in `tipsy-ui` hold es/en copy, and profile texts stay in their own language.
- `tipsy-protocol` and `tipsy-ui` must NOT depend on `tokio`, `cpal`, `rodio`, `keyring`, `sysinfo`, `reqwest` or `crossterm`, and `tipsy-ui` must not use `std::time::Instant` — the screen is ticked like the rest of the HUD. The new protocol pair keeps both crates wasm-clean.
- API keys never appear in `config.toml`, logs, events or test fixtures; the screen shows a key **name** and its resolution source, never a key.
- Timeouts: 30 s per chat request, 30 s per sentence of speech, 60 s per transcription. Exactly one retry with backoff on `RateLimited`, in the brain.
- Mic audio is mono 16 kHz; the VAD's frames are the client's own `Frame::Mic` frames and nothing about them changes on the wire.
- The keyboard/mouse rule of §9 holds for every screen action, and no default shortcut may use a browser-reserved key (`F5`, `F11`, `Ctrl+W`, …), because the same interface is rendered in a browser in a later plan.
- Every commit message ends with the trailer line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A failed save must never damage the user's configuration.** The write goes to the real file only after the serialisation succeeded, a failure leaves both file and running configuration untouched, and the screen says what happened. Task 6 owns the test (a read-only config path is the cheapest way to force it).
2. **`auto` mode must not send audio it did not hear.** Silence before the first syllable, the 300 ms guard after the assistant stops speaking, and the hangover that ends an utterance are all gate conditions: no frames from outside the utterance may reach transcription, and the assistant's own voice must never re-open the microphone. Task 8 and Task 9 own those tests.
3. **An old or hand-edited `config.toml` must still load.** The two new fields (`server.token`, `listening.mic_device`) default in, a file written by Plan 1 keeps working, and a file edited while the app runs is re-read on open (not cached forever). Task 1 and Task 6 own those tests.
4. **The screen must be usable on a small terminal.** Every tab renders without panicking or garbling when the terminal is near the layout's minimum, and a value that does not fit is truncated rather than overlapping its neighbour. Task 3 and Task 5 own those tests.
5. **The `Test` button must tell the truth.** A test result distinguishes "the provider answered" from "the provider refused" from "no key", reports the measured latency in milliseconds, and never claims success on an error body — Task 7 owns it.

## File Structure

```
crates/tipsy-protocol/src/lib.rs        MODIFY: Command::TestProvider + Event::TestResult, serde tests
crates/tipsy-ui/src/
├── strings.rs                          MODIFY: the screen's es/en labels
├── state.rs                            MODIFY: open/close the screen, hold the form, the new actions
├── config_form.rs                      NEW: ConfigForm, its fields, its reducer (keyboard and mouse)
├── render.rs                           MODIFY: ⚙ button and mode indicator in the header; dispatch to the screen
└── render_config.rs                    NEW: the four tabs' layout and their clickable areas
crates/tipsy-compadre/src/
├── config/model.rs                     MODIFY: server.token, listening.mic_device
├── config/store.rs                     MODIFY: token generation, defaults for old files
├── vad.rs                              NEW: the earshot-backed detector and our hangover/guard state machine
├── session.rs                          MODIFY: auto mode's utterance boundaries; apply a changed configuration
├── client.rs                           MODIFY: the screen's effects, the Test call, mic frames into the VAD
├── providers/mod.rs                    MODIFY: the minimal probe the Test command runs
└── main.rs                             MODIFY: nothing user-visible; the config path the screen writes
```

## Interfaces this plan fixes

These three shapes are the contract between the tasks; every task body quotes them instead of re-inventing them.

### The form view model (`crates/tipsy-ui/src/config_form.rs`)

Plain values only — no `Config`, no native type, nothing that breaks the wasm boundary:

```rust
pub enum ConfigTab { Providers, Listening, Profiles, Server }

pub struct ProviderForm { pub capability: Capability, pub kind: String, pub base_url: String, pub model: String, pub key_name: String }
pub struct ListeningForm { pub mode: ListeningMode, pub silence_ms: String, pub mic_gain: String, pub mic_device: Option<String> }
pub struct ServerForm { pub bind: String, pub lan: bool, pub token: String }
pub struct ProfileForm {
    pub id: String, pub name: String, pub language: String, pub prompt: String,
    pub voice_instructions: String, pub greeting: String, pub wake_word: String,
    pub voices: Vec<(String, String)>,     // connector name -> voice name or id
}

pub struct ConfigForm {
    pub tab: ConfigTab,
    pub field: usize,                      // focused row inside the current tab
    pub editing: Option<String>,           // the buffer while a text row is being edited
    pub active_profile: String,
    pub providers: Vec<ProviderForm>,      // always three, in Capability order
    pub listening: ListeningForm,
    pub profiles: Vec<ProfileForm>,
    pub server: ServerForm,
    pub notice: Option<(NoticeKind, String)>,   // Ok | Warning | Error, shown under the tab row
    pub dirty: bool,
}

pub enum ConfigAction {
    Open, Close, NextTab, PrevTab, NextField, PrevField, BeginEdit, Push(char), Backspace, CommitEdit,
    Toggle, SelectProfile(usize), Save, Test, RegenerateToken,
}

impl ConfigForm {
    pub fn rows(&self) -> Vec<FormRow>;                       // what the active tab shows, for rendering and clicks
    pub fn apply(&mut self, action: ConfigAction) -> Vec<ConfigEffect>;
}

pub enum ConfigEffect { None, Save, Test(Capability), RegenerateToken }
```

`ConfigForm::rows` is what makes the rendering and the click areas agree: the renderer draws the rows it returns, and the hit areas are the row rectangles of that same list.

### The protocol pair (`crates/tipsy-protocol/src/lib.rs`)

Serde and naming follow the crate's existing enums exactly:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability { Chat, Speech, Transcribe }

// in Command:
TestProvider { capability: Capability },

// in Event:
TestResult { capability: Capability, ok: bool, latency_ms: u64, message: String },
```

`message` carries the provider's own words on failure, or a short human summary on success ("1.2 s, 412 bytes" style); it is never empty, and `ok` is `true` only when a real minimal call answered.

### The detector (`crates/tipsy-compadre/src/vad.rs`)

Our own boundary, so that the crate's API churn — the research note flags that the published `earshot` 1.2.2 may expose only the deprecated `predict_i16` — is contained in one function:

```rust
pub enum VadEvent { None, SpeechStarted, SpeechEnded }

pub struct VadState { /* detector, counters, hangover, in_speech, paused */ }

impl VadState {
    pub fn new(silence_ms: u32) -> Self;
    pub fn pause(&mut self);                       // the assistant is speaking, or the 300 ms guard
    pub fn resume(&mut self);
    pub fn push(&mut self, frame: &[i16]) -> VadEvent;
}
```

`push` takes one `Frame::Mic`'s worth of samples as the client produces them — 16 kHz mono, **320 samples / 20 ms** (`audio/device.rs`'s `MIC_FRAME_SAMPLES`) — and re-chunks them internally into the detector's own 256-sample windows through a carry buffer, so our boundary does not inherit the crate's window size. It returns `SpeechEnded` once when `silence_ms` has elapsed since the last voiced frame, and `SpeechStarted` once on the first voiced frame after a silent run. While paused it consumes frames and reports `None`.

## Tasks

| # | Task `[tier]` | Deliverable | Files | Depends on |
|---|---|---|---|---|
| 1 | Config model additions `[small]` | `server.token`, `listening.mic_device`, token generation and defaults for old files, with tests | `config/model.rs`, `config/store.rs` | — |
| 2 | The protocol's test pair `[small]` | `Command::TestProvider` + `Event::TestResult`, serde round trip, wasm-clean | `tipsy-protocol/src/lib.rs` | — |
| 3 | Screen strings and form state `[mid]` | The es/en labels and `ConfigForm` with its reducer (tabs, fields, editing, toggles, profile list) | `tipsy-ui/src/strings.rs`, `config_form.rs` | — |
| 4 | The four tabs' rendering `[mid]` | `render_config.rs`: layout, values, cursor, and the click areas that match their labels | `tipsy-ui/src/render_config.rs` | 3 |
| 5 | Opening the screen and the header controls `[small]` | `F9`/⚙ opens it, `Esc` closes, the header shows the mode indicator, small terminals survive | `tipsy-ui/src/render.rs`, `state.rs` | 3, 4 |
| 6 | Save and apply without restarting `[mid]` | `Config` ⇄ `ConfigForm`, a save that cannot damage the file, providers rebuilt in place | `tipsy-compadre/src/client.rs`, `config/store.rs` | 1, 3 |
| 7 | The Test button `[mid]` | The minimal probe per capability, real latency, honest results, rendered in the screen | `providers/mod.rs`, `client.rs`, `tipsy-ui/src/config_form.rs` | 2, 6 |
| 8 | The detector `[mid]` | `vad.rs`: `earshot` behind our onset/hangover/guard state machine, verified against the published API | `tipsy-compadre/src/vad.rs` | — |
| 9 | `auto` mode end to end `[mid]` | The session ends the utterance by itself after `silence_ms`, the mic never re-opens while speaking, `Tab` still stops | `session.rs`, `client.rs` | 8 |
| 10 | README and the plan's own review `[small]` | The quickstart documents the screen and the three modes; the whole-branch review before merging | `README.md` | 1–9 |

---

### Task 1: Config model additions `[small]`

**Files:**
- Modify: `crates/tipsy-compadre/Cargo.toml:8-29` — one new line under `[dependencies]`: `getrandom = "0.4"`, the crate that supplies the randomness (0.4.3 is already in `Cargo.lock:1322-1334`; see Step 3)
- Modify: `crates/tipsy-compadre/src/config/model.rs:38-63` — `ServerConfig.token` and `ServerConfig::has_token()`, `ListeningConfig.mic_device`
- Modify: `crates/tipsy-compadre/src/config/store.rs:20-30` — insert `pub fn new_token()` between `save` and `load_or_create`
- Modify: `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md:247-254` — §8.1's example file: the two new keys, one line each, and nothing else in the spec (Step 6)
- Test: `crates/tipsy-compadre/src/config/store.rs:59-211` — the existing `#[cfg(test)] mod tests`
- Lock: `Cargo.lock` — commit it with the change: the new edge rewrites the `tipsy-compadre` entry. No package and no version is added to the lock (Step 3 proves it).

**Interfaces:**
- Consumes (read from the committed code, not recalled):
  - `pub struct Config { pub active_profile: String, pub server: ServerConfig, pub listening: ListeningConfig, pub providers: ProvidersConfig, pub profiles: Vec<Profile> }` with `#[serde(default)]` — `config/model.rs:8-24`
  - `pub struct ServerConfig { pub bind: String, pub lan: bool }`, `Default { bind: "127.0.0.1:7070".into(), lan: false }` — `config/model.rs:38-49`
  - `pub struct ListeningConfig { pub mode: ListeningMode, pub silence_ms: u32, pub mic_gain: f32 }`, `Default { mode: ListeningMode::Push, silence_ms: 1000, mic_gain: 12.0 }` — `config/model.rs:51-63`
  - `pub fn save(path: &Path, config: &Config) -> std::io::Result<()>` — `config/store.rs:20-26`
  - `pub fn load_or_create(path: &Path) -> std::io::Result<LoadOutcome>` with `pub struct LoadOutcome { pub config: Config, pub warning: Option<String> }` — `config/store.rs:7-11`, `:30-57`. It rewrites nothing on the happy path: only a missing or corrupt file is written.
  - `getrandom::fill(dest: &mut [u8]) -> Result<(), getrandom::Error>` — `getrandom` 0.4.3, `src/lib.rs:87`, `Error` is a `NonZeroI32`/`NonZeroUsize` wrapper implementing `core::error::Error` (`src/error.rs:45`, `:210`), so `std::io::Error::other` accepts it.
- Produces:
  - `pub struct ServerConfig { pub bind: String, pub lan: bool, pub token: String }`, `Default { bind: "127.0.0.1:7070".into(), lan: false, token: String::new() }`
  - `pub fn ServerConfig::has_token(&self) -> bool`
  - `pub struct ListeningConfig { pub mode: ListeningMode, pub silence_ms: u32, pub mic_gain: f32, pub mic_device: Option<String> }`, `Default { mode: ListeningMode::Push, silence_ms: 1000, mic_gain: 12.0, mic_device: None }`
  - `pub fn new_token() -> std::io::Result<String>` — 32 characters, alphabet `0-9a-f`
  - Tasks 4, 6 and 7 rely on these: `ServerForm::token` and `ListeningForm::mic_device` (the plan's fixed shapes are `token: String` and `mic_device: Option<String>`, and these two fields are exactly what the `Config` ⇄ `ConfigForm` conversion reads and writes), and Task 6 calls `store::new_token()` for `ConfigEffect::RegenerateToken`.

**What this task decides, and why**

- **The token lives in `config.toml`, and it is not an API key.** Spec §9 makes the Server tab *display* the token, so it cannot live only in the credential store; it is a bearer token we mint for our own WebSocket server, not a provider key, so the Global Constraint "API keys never appear in `config.toml`" does not apply to it. It must still never be echoed into an event, a notice or a log line — only into the file and onto the screen.
- **The randomness comes from `getrandom` 0.4.3, already in `Cargo.lock`** (`Cargo.lock:1322-1334`, pulled in by `rand` 0.10.3 at `Cargo.lock:2735-2744`). It is the operating system's cryptographic source, which is what an authentication token deserves. `fastrand` 2.5.0 is in the lock too (`Cargo.lock:1124-1128`, via `tempfile`) and is smaller, but it is a userspace PRNG, not a CSPRNG, so it is not used here. **No dependency outside the lock is added.**
- **The token is 16 bytes of OS entropy as 32 lowercase hex characters** (`0-9a-f`, 128 bits). Hex keeps the mapping bias-free (no `% 62`), keeps the value copy-pasteable into a browser URL or a `Hello { token }` message, and keeps the screen's one-line rendering honest. Its shape is pinned by a test rather than described in prose alone.
- **It is generated on demand and only on demand.** `ServerConfig::default()` — and therefore every file created from defaults, and every old file that lacks the key — carries `token = ""`. Nothing calls `new_token()` on load: the screen's Regenerate button does (`ConfigEffect::RegenerateToken`), and a user can paste their own value over it in the text row.
- **An empty token is a state, not an error.** `has_token()` is `false` for an empty *or whitespace-only* value; the file is not corrupt, no warning is raised, no backup is made, and no token is invented. The screen shows the unset state (Task 4's copy) and still lets the row be edited. Plan 3's server is the layer that must refuse a non-loopback bind while `has_token()` is `false` — this plan only guarantees that the state is *knowable* and that regenerating yields a fresh one.
- **`mic_device` is `Option<String>`, not `String`.** `None` means "the system default device" and is the only value the web client can have, so the Listening tab hides the row when it is absent (the plan's decision). It also has to survive `toml::to_string_pretty`: toml 1.1.8 serialises a `None` struct field as `SerializationStrategy::Skip` and omits the key (`toml-1.1.8`, `src/ser/document/strategy.rs:113-115`, `src/ser/document/map.rs:139-141`), which is why the default file simply has no `mic_device` line and why an empty string — which would round-trip as a device with an empty name — is the wrong encoding. Step 1 pins exactly that, and Step 6 writes both keys into spec §8.1's example so the design doc keeps describing the file this task makes real.

- [ ] **Step 1: Write the failing tests** — in `crates/tipsy-compadre/src/config/store.rs`, extend the test module's import line and append these six tests inside the existing `mod tests`:

```rust
    use crate::config::{ProviderKind, Profile, ServerConfig};
```

```rust
    /// Review Focus 3, first half: a `config.toml` written by Plan 1 (no `token`, no
    /// `mic_device`) is a valid file, not a corrupt one: it loads, nothing else moves, and the two
    /// new fields come up at their defaults.
    #[test]
    fn plan_1_file_without_token_or_mic_device_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"
active_profile = "compae"

[server]
bind = "127.0.0.1:7070"
lan = false

[listening]
mode = "auto"
silence_ms = 1500
mic_gain = 9.5
"#).unwrap();

        let out = load_or_create(&path).unwrap();
        assert!(out.warning.is_none(), "an old file is not a corrupt file: {:?}", out.warning);
        assert!(out.config.server.token.is_empty(), "the token defaults in empty, never generated");
        assert!(!out.config.server.has_token(), "an empty token means no token yet");
        assert!(out.config.listening.mic_device.is_none(), "no device is configured by default");
        // Nothing else moved.
        assert_eq!(out.config.server.bind, "127.0.0.1:7070");
        assert!(!out.config.server.lan);
        assert_eq!(out.config.listening.mode, ListeningMode::Auto);
        assert_eq!(out.config.listening.silence_ms, 1500);
        assert_eq!(out.config.listening.mic_gain, 9.5);
        // Loading is not a migration: the user's file is left exactly as it was.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("token"), "loading must not rewrite the file, got:\n{text}");
    }

    /// Review Focus 3, second half: nothing is cached between loads, so a file edited while the app
    /// runs is seen the next time the screen opens one — including a token pasted in by hand.
    #[test]
    fn a_file_edited_after_the_first_load_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[server]\nbind = \"127.0.0.1:7070\"\n").unwrap();
        assert!(load_or_create(&path).unwrap().config.server.token.is_empty());

        std::fs::write(&path, "[server]\nbind = \"127.0.0.1:7070\"\ntoken = \"pasted-by-hand\"\n").unwrap();
        let again = load_or_create(&path).unwrap().config;
        assert_eq!(again.server.token, "pasted-by-hand");
        assert!(again.server.has_token());
    }

    /// The value the WebSocket server will require in Plan 3 is the one that comes back from the
    /// file, spelled the same way.
    #[test]
    fn token_survives_a_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.server.token = "0123456789abcdef0123456789abcdef".into();
        c.server.lan = true;
        c.listening.mic_device = Some("Rode NT-USB".into());
        save(&path, &c).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("token = \"0123456789abcdef0123456789abcdef\""),
            "the token must be written under [server], got:\n{text}"
        );
        assert!(
            text.contains("mic_device = \"Rode NT-USB\""),
            "the device must be written under [listening], got:\n{text}"
        );
        assert_eq!(load_or_create(&path).unwrap().config, c);
    }

    /// `mic_device` is an `Option` because toml skips a `None` field instead of refusing to write
    /// the file, and because "no device chosen" must not be spelled as an empty device name.
    #[test]
    fn absent_mic_device_is_none_and_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let c = Config::default();
        save(&path, &c).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("mic_device"), "a None device is skipped, got:\n{text}");
        assert!(text.contains("mic_gain = 12.0"), "the rest of [listening] is still written:\n{text}");

        let back = load_or_create(&path).unwrap().config;
        assert!(back.listening.mic_device.is_none());
        assert_eq!(back, c);
    }

    /// The documented shape of a generated token: 32 lowercase hex characters, and two calls do not
    /// collide. The shape is the contract the screen's row and Plan 3's handshake rely on.
    #[test]
    fn new_token_has_the_documented_shape_and_differs_between_calls() {
        let first = new_token().unwrap();
        let second = new_token().unwrap();
        assert_eq!(first.len(), 32, "16 bytes as lowercase hex, got {first:?}");
        assert!(
            first.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "the alphabet is 0-9a-f, got {first:?}"
        );
        assert_ne!(first, second, "two generations must not return the same token");
        let server = ServerConfig { token: first.clone(), ..ServerConfig::default() };
        assert!(server.has_token(), "a generated token counts as configured");
    }

    /// An empty token is a state, not an error: it is what a fresh install and every file written
    /// before this plan carry, and loading never fills it in — only the screen's Regenerate button
    /// (or the user's own paste) does.
    #[test]
    fn empty_token_is_a_state_and_is_never_filled_in_on_load() {
        assert!(!ServerConfig::default().has_token());
        assert!(!Config::default().server.has_token());
        let blank = ServerConfig { token: "   ".into(), ..ServerConfig::default() };
        assert!(!blank.has_token(), "a hand-edited blank token is not a token");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[server]\nbind = \"0.0.0.0:7070\"\nlan = true\n").unwrap();
        let loaded = load_or_create(&path).unwrap();
        assert!(loaded.warning.is_none(), "a missing token is not a corrupt file");
        assert!(loaded.config.server.token.is_empty(), "nothing is generated on load");
        assert!(!loaded.config.server.has_token());
    }
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --lib config::store`

Expected: FAIL — the run stops at the compiler, one error per missing item, e.g.

```
error[E0609]: no field `token` on type `ServerConfig`
error[E0609]: no field `mic_device` on type `ListeningConfig`
error[E0599]: no method named `has_token` found for struct `ServerConfig` in the current scope
error[E0425]: cannot find function `new_token` in this scope
```

(Rustc reports these one occurrence at a time and gives up after a handful; that is the expected failure. No test in this module reaches the harness yet.)

- [ ] **Step 3: Name the randomness — add the dependency line**

In `crates/tipsy-compadre/Cargo.toml`, under `[dependencies]` (next to the other pinned versions, e.g. after `dirs = "7"`):

```toml
getrandom = "0.4"
```

Run: `cargo check --offline -p tipsy-compadre --lib`

Expected: `Finished` — resolution succeeds **offline**, because `getrandom` 0.4.3 is already in `Cargo.lock` (line 1322-1334, pulled in by `rand` 0.10.3 at 2735-2744) and in the local registry cache. `git diff Cargo.lock` must show only `getrandom` added to the `tipsy-compadre` entry: no new `[[package]]`, no second `0.4.x`. If cargo asks for the network or reports a resolution error, the version is wrong or the crate is not in the lock: stop and report that instead of adding a crate the workspace does not already build.

- [ ] **Step 4: Implement the two fields, the generator and the check**

`crates/tipsy-compadre/src/config/model.rs` — replace `ServerConfig` and its `Default` (lines 30-41) with:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub lan: bool,
    /// The token the WebSocket server will require (spec §9; Plan 3 puts it to use). Empty means
    /// "none generated yet": the screen shows the unset state and its Regenerate button fills it
    /// in. It is *our* bearer token, not a provider key, so the file is its home — and it still
    /// never belongs in an event, a notice or a log line.
    pub token: String,
}

impl ServerConfig {
    /// False while no token has been generated: a fresh install, or a file written before this
    /// plan. A whitespace-only value (a hand-edited file) counts as no token.
    pub fn has_token(&self) -> bool {
        !self.token.trim().is_empty()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { bind: "127.0.0.1:7070".into(), lan: false, token: String::new() }
    }
}
```

and replace `ListeningConfig` and its `Default` (lines 43-55) with:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListeningConfig {
    pub mode: ListeningMode,
    pub silence_ms: u32,
    pub mic_gain: f32,
    /// Input device for the native client (spec §9). `None` — the default, and the only value the
    /// web client can have — means the system default device; the serializer leaves the key out
    /// rather than writing an empty name.
    pub mic_device: Option<String>,
}

impl Default for ListeningConfig {
    fn default() -> Self {
        Self { mode: ListeningMode::Push, silence_ms: 1000, mic_gain: 12.0, mic_device: None }
    }
}
```

`crates/tipsy-compadre/src/config/store.rs` — insert between `save` (ends line 26) and `load_or_create` (line 30):

```rust
/// Bytes of OS entropy behind one token: 128 bits, i.e. 32 lowercase hex characters.
const TOKEN_BYTES: usize = 16;
const HEX: &[u8; 16] = b"0123456789abcdef";

/// A fresh `server.token`: [`TOKEN_BYTES`] bytes from the operating system's cryptographic
/// generator (`getrandom` 0.4.3, already in `Cargo.lock` through `rand` 0.10.3, so this adds no
/// crate to the workspace) rendered as 32 lowercase hex characters, alphabet `0-9a-f`.
///
/// Called only when someone asks: the configuration screen's **Regenerate** button
/// (`ConfigEffect::RegenerateToken`, Task 6) or a user pasting a value of their own. Nothing calls
/// it while loading, so an empty `token` stays empty until then (`ServerConfig::has_token`).
pub fn new_token() -> std::io::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    let mut token = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(token)
}
```

The `io::Result` is deliberate: a failing OS generator becomes a line in the screen's notice (Task 6 renders it), never a panic and never a silently empty token.

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --lib config::store`

Expected: PASS, 12 tests:

```
running 12 tests
test config::store::tests::a_file_edited_after_the_first_load_is_read_again ... ok
test config::store::tests::absent_mic_device_is_none_and_is_not_written ... ok
test config::store::tests::corrupt_file_is_backed_up_and_reset ... ok
test config::store::tests::creates_default_file_when_missing ... ok
test config::store::tests::empty_token_is_a_state_and_is_never_filled_in_on_load ... ok
test config::store::tests::new_token_has_the_documented_shape_and_differs_between_calls ... ok
test config::store::tests::partial_file_gets_defaults_and_keeps_user_profiles ... ok
test config::store::tests::plan_1_file_without_token_or_mic_device_still_loads ... ok
test config::store::tests::save_then_load_roundtrips ... ok
test config::store::tests::token_survives_a_save_load_roundtrip ... ok
test config::store::tests::user_override_of_builtin_wins ... ok
test config::store::tests::voice_key_and_voice_lookup ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

The six that were already there must still pass unchanged — `partial_file_gets_defaults_and_keeps_user_profiles` and `corrupt_file_is_backed_up_and_reset` in particular are the old-partial-file and the corrupt-file contracts, and neither is touched. Finish with `cargo test -p tipsy-compadre` (bin plus lib, all green) and `cargo clippy -p tipsy-compadre --all-targets` (no new warnings).

- [ ] **Step 6: Document the two keys in the spec's example file**

In `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md`, §8.1's example file (lines 247-254), add one line to each table, in the doc's own trailing-comment style, and change nothing else in the spec:

```toml
[server]
bind = "127.0.0.1:7070"
lan = false
token = ""               # server token; empty until the Server tab generates one

[listening]
mode = "auto"            # push | auto | hands_free
silence_ms = 1000
mic_gain = 12.0
# mic_device = "..."     # input device, native client only; omitted means the system default
```

`token = ""` is written uncommented because it is a real, valid value: the empty token is exactly the state every file written before this plan carries, and the design doc is the authority for the file's shape. `mic_device` is shown **commented out on purpose** — the code's "no device chosen" is the *absent key* (`None`, Step 4 and the test in Step 1), while an uncommented `mic_device = ""` would mean a device whose name is the empty string, which is a different and useless value. Only these two lines change.

- [ ] **Step 7: Commit**

```bash
git add crates/tipsy-compadre/Cargo.toml Cargo.lock crates/tipsy-compadre/src/config/model.rs crates/tipsy-compadre/src/config/store.rs docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md
git commit -m "feat(config): add server.token and listening.mic_device

Both default in for a config.toml written before this plan: the token comes up
empty (generated on demand by the screen's Regenerate button, never on load) and
mic_device comes up absent. Randomness comes from getrandom, already in the
lock, and spec 8.1's example now lists both keys.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The protocol's test pair `[small]`

**Files:**
- Modify: `crates/tipsy-protocol/src/lib.rs:51-64` — add `Capability` after `ErrorCode`, before `ProfileView` (line 66)
- Modify: `crates/tipsy-protocol/src/lib.rs:73-85` — add `Command::TestProvider` as the last variant of `Command`
- Modify: `crates/tipsy-protocol/src/lib.rs:87-106` — add `Event::TestResult` as the last variant of `Event`
- Test: `crates/tipsy-protocol/tests/roundtrip.rs` — append two tests; the existing `roundtrip_command` (`:3-7`) and `roundtrip_event` (`:9-13`) helpers are reused unchanged
- Modify, exhaustiveness only, one arm each and no behaviour: `crates/tipsy-compadre/src/session.rs:215-231`, `crates/tipsy-compadre/src/client.rs:285-297`, `crates/tipsy-ui/src/state.rs:165-236`

**Interfaces:**
- Consumes (the crate's existing style, quoted from the committed code — do not invent a tag or a rename rule):
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]` on every small enum (`ClientKind` `lib.rs:9-15`, `ListeningMode` `:17-24`, `AssistantState` `:26-34`, `ErrorCode` `:51-64`)
  - `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(tag = "type", rename_all = "snake_case")] pub enum Command { Hello { token: Option<String>, client: ClientKind, protocol_version: u32 }, Ask { text: String }, Say { text: String }, ListenStart, ListenStop, Cancel, Reset, SetProfile { id: String }, SetListeningMode { mode: ListeningMode } }` — `lib.rs:73-85`. The enum-level `rename_all` renames **variants only**; field names are already snake_case, and `tests/roundtrip.rs:49-53` pins the wire form: `Command::SetListeningMode { mode: ListeningMode::HandsFree }` is exactly `{"type":"set_listening_mode","mode":"hands_free"}` and `Command::ListenStart` is `{"type":"listen_start"}`.
  - `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(tag = "type", rename_all = "snake_case")] pub enum Event { Welcome { session_id: u64, profile: ProfileView, ui_language: UiLanguage, listening_mode: ListeningMode, web_bundle: bool }, Status { state: AssistantState, detail: Option<String> }, Heard { text: String }, ReplyDelta { text: String }, Reply { text: String }, SpeechChunk { seq: u32, sample_rate: u32 }, Done { first_audio_ms: Option<u64>, total_ms: u64 }, Stats { cpu: f32, ram: f32, net_in: f64, net_out: f64 }, Error { code: ErrorCode, message: String, recoverable: bool } }` — `lib.rs:87-106`, same attributes, so the tag comes first and the payload fields follow in declaration order.
  - `fn roundtrip_command(c: Command)` and `fn roundtrip_event(e: Event)` — `tests/roundtrip.rs:3-13`, both `serde_json::to_string` → `from_str` → `assert_eq!` against the original, with the JSON in the panic message.
  - The three matches a new variant breaks (this is why the task has four files): `UiState::apply` matches `&Event` exhaustively at `tipsy-ui/src/state.rs:166-235`; `Session::command` matches `Command` exhaustively at `tipsy-compadre/src/session.rs:216-230`; `starts_turn` matches `&Command` exhaustively at `tipsy-compadre/src/client.rs:286-296`.
- Produces (exactly the shapes "Interfaces this plan fixes" gives, nothing more):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability { Chat, Speech, Transcribe }

// in Command:
TestProvider { capability: Capability },

// in Event:
TestResult { capability: Capability, ok: bool, latency_ms: u64, message: String },
```

  - Tasks 3 and 4 build `ProviderForm { capability, kind, base_url, model, key_name }` and `ConfigEffect::Test(Capability)` on it; Task 7 runs the probe and raises the result; Task 5 renders nothing of it yet. `Capability` needs no `Default`: the form always builds the three capabilities explicitly, in order.
  - `PROTOCOL_VERSION` stays `1` (`lib.rs:7`). The two variants are additive and every existing variant keeps its shape byte for byte; Plan 3's server is the first thing to answer them.
- **This task touches four files, not one, and that is deliberate:** the protocol crate plus the three exhaustive matches listed above. Tasks 3, 4, 5 and 6 all run `cargo test` on `tipsy-ui` and `tipsy-compadre`, so a variant added here may not leave either of them uncompilable; Steps 5 and 6 add one behaviour-free arm to each match. Task 7 may replace the client's and the session's arms with the real probe, whichever side ends up running it — nothing else in this task anticipates that choice.

**Why this task touches three files beyond the protocol crate**

`Command` and `Event` are matched exhaustively — no wildcard arms — in exactly three places, and the compiler names them the moment the variants exist. Tasks 3, 4, 5, 6 and 7 all run `cargo test -p tipsy-ui` or `cargo test -p tipsy-compadre`, so this task cannot leave the workspace uncompilable and hand the arms to a later task. Each arm is one line with no behaviour; Step 5 shows the exact failure and Step 6 shows the exact arms. Nothing about them is a shortcut for Task 7: if Task 7 decides the session — rather than the native client — owns the probe, it replaces the session's arm with the real handler, and the other two stay as they are.

- [ ] **Step 1: Write the failing tests** — append these two tests to `crates/tipsy-protocol/tests/roundtrip.rs` (`use tipsy_protocol::*;` at line 1 already brings `Capability` in):

```rust
#[test]
fn test_provider_and_test_result_roundtrip() {
    for capability in [Capability::Chat, Capability::Speech, Capability::Transcribe] {
        roundtrip_command(Command::TestProvider { capability });
    }
    // A success with a measured latency and the short summary of what answered.
    roundtrip_event(Event::TestResult {
        capability: Capability::Chat,
        ok: true,
        latency_ms: 1234,
        message: "1.2 s, 412 bytes".into(),
    });
    // A refusal with no latency at all: `ok = false` and the provider's own words, which may not
    // be ASCII.
    roundtrip_event(Event::TestResult {
        capability: Capability::Transcribe,
        ok: false,
        latency_ms: 0,
        message: "no key named openrouter — set TIPSY_KEY_OPENROUTER".into(),
    });
}

#[test]
fn test_pair_json_shape_is_tagged_snake_case() {
    for (capability, json) in [
        (Capability::Chat, "chat"),
        (Capability::Speech, "speech"),
        (Capability::Transcribe, "transcribe"),
    ] {
        assert_eq!(serde_json::to_string(&capability).unwrap(), format!("\"{json}\""));
    }

    let json = serde_json::to_string(&Command::TestProvider { capability: Capability::Speech }).unwrap();
    assert_eq!(json, r#"{"type":"test_provider","capability":"speech"}"#);

    let json = serde_json::to_string(&Event::TestResult {
        capability: Capability::Transcribe,
        ok: false,
        latency_ms: 0,
        message: "no key named openrouter".into(),
    })
    .unwrap();
    assert_eq!(
        json,
        r#"{"type":"test_result","capability":"transcribe","ok":false,"latency_ms":0,"message":"no key named openrouter"}"#
    );
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-protocol --test roundtrip`

Expected: FAIL — the test target does not compile:

```
error[E0433]: failed to resolve: use of undeclared type `Capability`
error[E0599]: no variant or associated item named `TestProvider` found for enum `Command` in the current scope
error[E0599]: no variant or associated item named `TestResult` found for enum `Event` in the current scope
```

- [ ] **Step 3: Add `Capability` and the two variants** (English doc comments, the crate's own attribute style, nothing reordered)

In `crates/tipsy-protocol/src/lib.rs`, after `ErrorCode` (line 64) and before `ProfileView`:

```rust
/// Which connector the configuration screen's "Test" button is asking about (spec §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Chat,
    Speech,
    Transcribe,
}
```

As the last variant of `Command` (after `SetListeningMode`, line 84):

```rust
    /// The configuration screen's "Test" button: one minimal call against the connector for this
    /// capability. Plan 2B Task 7 runs the probe and answers with `Event::TestResult`.
    TestProvider { capability: Capability },
```

As the last variant of `Event` (after `Error`, line 105):

```rust
    /// The answer to `Command::TestProvider`. `message` is never empty: the provider's own words
    /// on failure, a short human summary on success. `ok` is `true` only when a real minimal call
    /// answered — never for an error body.
    TestResult { capability: Capability, ok: bool, latency_ms: u64, message: String },
```

- [ ] **Step 4: Run the pair's tests and watch them pass**

Run: `cargo test -p tipsy-protocol --test roundtrip`

Expected: PASS, 8 tests:

```
running 8 tests
test commands_roundtrip ... ok
test events_roundtrip ... ok
test frames_reject_garbage ... ok
test frames_roundtrip ... ok
test json_shape_is_tagged_snake_case ... ok
test language_mapping ... ok
test test_pair_json_shape_is_tagged_snake_case ... ok
test test_provider_and_test_result_roundtrip ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

- [ ] **Step 5: Watch the dependents break, and note exactly where**

Run: `cargo check --workspace --all-targets`

Expected: FAIL with exactly three errors, one per exhaustive match — one `E0004` per file and no more:

```
error[E0004]: non-exhaustive patterns: `Command::TestProvider { .. }` not covered
   --> crates\tipsy-compadre\src\client.rs:286:11
error[E0004]: non-exhaustive patterns: `Command::TestProvider { .. }` not covered
   --> crates\tipsy-compadre\src\session.rs:216:9
error[E0004]: non-exhaustive patterns: `Event::TestResult { .. }` not covered
   --> crates\tipsy-ui\src\state.rs:166:9
```

(`UiState::apply` matches a reference, so rustc may print the reference form `&Event::TestResult { .. }`; the three `E0004`s are what matters. Any *other* error means the task is wrong, not the dependents.)

- [ ] **Step 6: Keep the workspace compiling — one arm per match**

`crates/tipsy-compadre/src/session.rs`, in `fn command` (lines 216-230), immediately before `Command::Hello { .. } => {}`:

```rust
            // The Test probe asks a provider a question, not the session: Plan 2B Task 7 runs it in
            // the native client and Plan 3's server answers it over the wire. Accepted and ignored,
            // so the match stays exhaustive.
            Command::TestProvider { .. } => {}
```

`crates/tipsy-compadre/src/client.rs`, in `starts_turn` (lines 286-296), extending the `false` arm:

```rust
        Command::Hello { .. }
        | Command::TestProvider { .. }
        | Command::ListenStart
        | Command::Cancel
        | Command::SetProfile { .. }
        | Command::SetListeningMode { .. } => false,
```

`crates/tipsy-ui/src/state.rs`, in `UiState::apply` (lines 166-235), after `Event::SpeechChunk { .. } => {}` (line 203):

```rust
            // A Test result belongs to the configuration screen's notice line (`ConfigForm::notice`,
            // Plan 2B Task 7), not to the HUD's log or its status rings: nothing to record here.
            Event::TestResult { .. } => {}
```

No imports change anywhere: each arm names its variant through the path it already uses.

- [ ] **Step 7: Prove the workspace is whole again**

Run: `cargo check --workspace --all-targets`

Expected: `Finished` for all three crates, no warnings.

Run: `cargo test --workspace`

Expected: all green — the protocol's 8, the UI's render/state tests and the compadre's session/brain/client tests included. Those suites are the regression net for the three arms: they cover the same matches, so a wrong arm (a Test silencing a reply in progress, a Test result landing in the log) shows up there. The wasm boundary is untouched by construction — this task adds no dependency, no feature and no `std` item, only two `Serialize`/`Deserialize` variants and a plain `Copy` enum. The `wasm32-unknown-unknown` target is not installed on this machine (`rustup target list --installed` lists only `x86_64-pc-windows-msvc`), so there is no `--target` build in this step; if the target is added later, `cargo check -p tipsy-protocol --target wasm32-unknown-unknown` is the check, and nothing in this task can affect its result.

- [ ] **Step 8: Commit**

```bash
git add crates/tipsy-protocol/src/lib.rs crates/tipsy-protocol/tests/roundtrip.rs crates/tipsy-compadre/src/session.rs crates/tipsy-compadre/src/client.rs crates/tipsy-ui/src/state.rs
git commit -m "feat(protocol): add TestProvider and TestResult

The screen's Test button gets a request and a result: Capability, the command
and the event, serde round-tripped and wasm-clean. The three exhaustive matches
in tipsy-ui and tipsy-compadre get a no-op arm so the workspace keeps building;
Task 7 fills in the probe.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Screen strings and form state `[mid]`

**Files:**
- Modify: `crates/tipsy-ui/src/lib.rs:3-5` — `pub mod config_form;`, between `pub mod render;` and the re-export block
- Modify: `crates/tipsy-ui/src/strings.rs:5-76` — the screen's es/en labels: one new field group in `Strings`, one value in `ES` and one in `EN`
- Create: `crates/tipsy-ui/src/config_form.rs` — `ConfigForm`, its row list and its reducer, plus the same module's `#[cfg(test)] mod tests`
- Test: `crates/tipsy-ui/src/config_form.rs` — the reducer tests below

**Interfaces:**
- Consumes (read from the committed code, not recalled):
  - `pub static ES: Strings` and `pub static EN: Strings` with `pub fn strings(lang: UiLanguage) -> &'static Strings` — `strings.rs:27-76`; every field is `&'static str`. A language is chosen from the active profile's tag by `UiLanguage::from_bcp47` (`tipsy-protocol/src/lib.rs:43-49`), which is rendered into `UiState::lang` by `Event::Welcome` (`state.rs:167-171`); `UiState::strings()` (`state.rs:132-134`) is the only accessor.
  - `pub enum KeyInput { Char(char), Enter, Backspace, Tab, Esc, Up, Down, Left, Right, F(u8) }` — `state.rs:52-64`. `Char` carries no modifier: `KeyInput::Char('\u{13}')` is the one spelling `Ctrl+S` has (`client.rs:339` maps every `KeyCode::Char` to it, whatever the modifiers, and Task 5 adds the one line that tells a control character apart).
  - `pub enum ListeningMode { Push, Auto, HandsFree }` — `tipsy-protocol/src/lib.rs:17-24` (plain `Copy + PartialEq + Eq`, **no `Display`**, so this task renders it through the string table).
  - `pub enum Capability { Chat, Speech, Transcribe }` — `tipsy-protocol/src/lib.rs:69-90`, already landed by Plan 2A Task 5 (commit `cb4d469`), with `Display` beside it: `chat`, `speech`, `transcription`. **Do not declare a second one.**
  - `pub enum ProviderKind { OpenaiCompat, Gemini, Elevenlabs }` and `Profile`'s shape — `tipsy-compadre/src/config/model.rs:65-145`. `tipsy-ui` cannot import them, so this task renders those two vocabularies through the string table's `kind_*` fields; the native client does the mapping in Task 6.
- Produces (the plan's "Interfaces this plan fixes" section verbatim, plus the small additions the four tabs need):

```rust
// strings.rs — appended to `Strings` and given one value in `ES` and one in `EN`
pub cfg_title: &'static str, pub cfg_providers: &'static str, pub cfg_listening: &'static str,
pub cfg_profiles: &'static str, pub cfg_server: &'static str,
pub cfg_connector: &'static str, pub cfg_base_url: &'static str, pub cfg_model: &'static str,
pub cfg_key_name: &'static str, pub cfg_test: &'static str,
pub cfg_mode: &'static str, pub cfg_silence_ms: &'static str, pub cfg_mic_gain: &'static str,
pub cfg_mic_device: &'static str,
pub cfg_profile: &'static str, pub cfg_name: &'static str, pub cfg_language: &'static str,
pub cfg_prompt: &'static str, pub cfg_voice_instructions: &'static str, pub cfg_greeting: &'static str,
pub cfg_wake_word: &'static str, pub cfg_voices: &'static str,
pub cfg_bind: &'static str, pub cfg_lan: &'static str, pub cfg_token: &'static str,
pub cfg_regenerate: &'static str,
pub cfg_save: &'static str, pub cfg_dirty: &'static str,
pub cfg_unset: &'static str, pub cfg_help: &'static str, pub cfg_help_editing: &'static str,
pub cfg_cancel: &'static str, pub cfg_value_not_whole: &'static str,
pub kind_openai_compat: &'static str, pub kind_gemini: &'static str, pub kind_elevenlabs: &'static str,
pub mode_push: &'static str, pub mode_auto: &'static str, pub mode_hands_free: &'static str,

// config_form.rs
pub enum ConfigTab { Providers, Listening, Profiles, Server }
pub struct ProviderForm { pub capability: Capability, pub kind: String, pub base_url: String, pub model: String, pub key_name: String }
pub struct ListeningForm { pub mode: ListeningMode, pub silence_ms: String, pub mic_gain: String, pub mic_device: Option<String> }
pub struct ServerForm { pub bind: String, pub lan: bool, pub token: String }
pub struct ProfileForm { pub id: String, pub name: String, pub language: String, pub prompt: String,
    pub voice_instructions: String, pub greeting: String, pub wake_word: String,
    pub voices: Vec<(String, String)> }
pub struct ConfigForm { pub tab: ConfigTab, pub field: usize, pub editing: Option<String>,
    pub active_profile: String, pub providers: Vec<ProviderForm>, pub listening: ListeningForm,
    pub profiles: Vec<ProfileForm>, pub server: ServerForm,
    pub notice: Option<(NoticeKind, String)>, pub dirty: bool }
pub enum ConfigAction { Open, Close, NextTab, PrevTab, NextField, PrevField, BeginEdit, Push(char),
    Backspace, CommitEdit, Toggle, SelectProfile(usize), Save, Test, RegenerateToken }
pub enum ConfigEffect { None, Save, Test(Capability), RegenerateToken }
pub enum NoticeKind { Ok, Warning, Error }
pub struct FormRow { pub label: &'static str, pub value: String, pub editable: bool, pub is_active: bool }

impl ConfigForm {
    pub fn rows(&self) -> Vec<FormRow>;
    pub fn apply(&mut self, action: ConfigAction) -> Vec<ConfigEffect>;
    pub fn key(&mut self, key: KeyInput) -> KeyOutcome;   // Task 5 feeds keys through this
    pub fn row_labels(&self) -> Vec<&'static str>;
    pub fn string_keys(&self) -> Vec<&'static str>;
    pub fn unset(&self) -> bool;                          // true for the empty server token
    pub fn tab_title(tab: ConfigTab, strings: &Strings) -> &'static str;
    pub fn profile_row_count(profiles: &[ProfileForm]) -> usize;
}
pub struct KeyOutcome { pub action: Option<ConfigAction>, pub effects: Vec<ConfigEffect> }
```

  - `rows()` is the tab's visible rows in order, with `field` indexing them: providers = **5 rows × 3 capabilities** (capability-major, in `Capability` order: connector, base URL, model, key name, Test), Listening = 4 rows (`mic_device` present) or 3 (absent), Profiles = `profile_row_count` (1 profile-list row + 11 field rows + one row per voice entry), Server = 3 rows (`bind`, `lan`, and the **Save** row).
  - `ConfigForm::default()` builds the three providers from... nothing — `tipsy-ui` has no configuration types, so a fresh form is `listening: ListeningForm { mode: ListeningMode::Push, silence_ms: "1000".into(), mic_gain: "12.0".into(), mic_device: None }`, empty providers, **one** profile (`ProfileForm { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into(), ..Default::default() }`), `server: ServerForm { bind: "127.0.0.1:7070".into(), lan: false, token: String::new() }`. **Task 6 replaces every field of it from the real `Config`; this default exists only so `ConfigForm::default()` is not a panic.** Every field is `to_owned` here, so `tipsy-ui` still depends on nothing but `tipsy-protocol`.
  - The voice table is a **sorted `Vec`, never a map**: a map's order is not a screen order, and the row index `field` is what the clicks name back (`Tipsy-compadre`'s `Profile::voices` is a `BTreeMap`, so Task 6's two conversions produce and consume the sorted order for free).
  - **The edit buffer is empty when an edit begins, and the buffer *replaces* the value on commit.** The cursor is always at the end of the buffer, which is the only cursor `String` state can express without a second index; a row is corrected by typing it again. `rows()` therefore keeps returning the field's *committed* value while a row is being edited, and Task 4 renders the buffer dimmed next to it. This is what makes both tests below exact.
  - `Tipsy-ui` keeps its dependency and time-source boundary: the task adds no dependency, opens no feature, and imports no `std::time`. Step 5 proves it from the manifest and the sources.
  - **The reducer returns `ConfigEffect` values that are all `None` except the four below.** Only `ConfigAction::Save` produces `ConfigEffect::Save`, `ConfigAction::Test` on a Test row produces `ConfigEffect::Test(capability)`, `ConfigAction::RegenerateToken` on the token row produces `ConfigEffect::RegenerateToken`, and `ConfigAction::Toggle` on a Test row produces `ConfigEffect::Test(capability)`: a keyboard or mouse activation and the Tab/Enter route into the same row, so they may not disagree. Every other action produces an empty `Vec` (the plan's `ConfigEffect::None` is what the signature *could* have been; `Vec` is what it fixes, so the excluded variant is never constructed).

**What this task decides, and why**

- **The labels live in the table, and only there.** `strings.rs` already holds every word the HUD shows; the screen's copy joins it rather than being spelled inside `render_config.rs` or `config_form.rs`. `row.label` is therefore a `&'static str` borrowed from `Strings`, not a `String`: a label cannot be built at runtime, so a label cannot escape the table. The two tests that compare `strings(UiLanguage::Es).cfg_x` with the literal in `ES` and `EN` are what keeps the two tables from drifting.
- **`row_labels()` and `string_keys()` exist for the tests, and they are honest.** `row_labels()` is `rows().into_iter().map(|r| r.label).collect()` and `string_keys()` walks `Strings` and returns every screen field's value — the title, the four tab titles, the row labels, the three kinds, the three modes, the two help lines, and the four notice-word fields. The test does not re-derive the list; it asks the form which strings it uses and checks each one against both tables, which is what makes "every key the screen uses" a mechanical test rather than a checklist.
- **Text that only renders when a condition holds is still in `string_keys()`.** `cfg_mic_device` and `cfg_help_editing` are returned whether or not the row or the help line is currently on screen: the missing row is a *state* (`mic_device: None`), not a reason for the table to be incomplete, and a table that lacks the key the next keystroke needs is exactly the bug this test exists to catch.
- **A tab switch resets `field` to 0.** Rows are per tab, so an index that was in range in one tab is meaningless in the next; the alternative is a focus that silently points at nothing.
- **`SelectProfile(index)` sets `active_profile`, and does not move `field`.** The profiles tab has two kinds of focus — which profile is active (the id string, what the session is told) and which row is selected (an index, what the keyboard moves) — and collapsing them would make `NextField` change the assistant's profile as a side effect of walking the form. `(index)` is clamped against `profiles.len()`: an out-of-range index from a stale click is a no-op, not a panic.
- **Editing a profile's `id` keeps `active_profile` on the profile that was focused.** If the old `id` equals `active_profile`, the commit writes the new one into `active_profile` too; otherwise a user renaming the active profile would silently switch the assistant to another one.
- **A commit that cannot be parsed is rejected, not clamped.** `silence_ms` must be a `u32` and `mic_gain` an `f32`: anything else leaves the row exactly as it was and shows `cfg_value_not_whole` in the notice line (`NoticeKind::Warning`), which is the same promise Task 6 makes for a failed save. A *parsed* value is merely `clamp`ed into its field's sensible range (`100..=30_000` ms, `0.0..=30.0` gain), because a clamped number is still a number the user typed.
- **`Ctrl+S` is spelled `KeyInput::Char('\u{13}')`.** `KeyInput` is the HUD's own vocabulary and is deliberately free of modifiers (`state.rs:51-64`); `KeyCode::Char('s')` with `CONTROL` arrives at `client.rs:339` as a plain `Char`. A control character is an unambiguous `char` that no text field wants, so no variant is added and no existing match changes. `client.rs`'s one-line special case for `KeyModifiers::CONTROL` is Task 5's, not this task's.
- **`Toggle` has one meaning per tab, and it is the only cycling key.** On Listening it advances `mode` `Push → Auto → HandsFree → Push`; on Server it flips `lan`; on every other row it does nothing. The Providers tab's kind selector therefore does **not** claim `Toggle`: `ProviderForm::kind` carries whatever string the configuration had, and the string table's `kind_*` fields are what render it (Task 4) and what Task 6 parses back. The Providers tab's **Test** row does raise an effect, and it is a row of the row list like any other: the tab's clickable areas come from that list, so the Test button is pinned to the row it acts on instead of to a second rectangle that could drift.

- [ ] **Step 1: The failing tests — declare the module and write the reducer tests**

  First the module line, so the test target can see the file at all. In `crates/tipsy-ui/src/lib.rs`, replace lines 3-5:

```rust
pub mod config_form;
pub mod render;
pub mod state;
```

  Then create `crates/tipsy-ui/src/config_form.rs` with the full test module below and **no implementation** — the tests are the specification of Step 3, and they may not be where the code goes:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::strings::{strings, EN, ES};

    /// A form open on Providers: what `ConfigAction::Open` must produce, spelled out.
    fn providers() -> ConfigForm {
        let mut f = ConfigForm::default();
        f.providers = vec![
            ProviderForm {
                capability: Capability::Chat,
                kind: "openai_compat".into(),
                base_url: "https://openrouter.ai/api/v1".into(),
                model: "google/gemini-2.5-flash".into(),
                key_name: "openrouter".into(),
            },
            ProviderForm {
                capability: Capability::Speech,
                kind: "openai_compat".into(),
                base_url: "https://openrouter.ai/api/v1".into(),
                model: "google/gemini-3.8-flash-tts".into(),
                key_name: "openrouter".into(),
            },
            ProviderForm {
                capability: Capability::Transcribe,
                kind: "openai_compat".into(),
                base_url: "https://openrouter.ai/api/v1".into(),
                model: "openai/whisper-large-v3".into(),
                key_name: "openrouter".into(),
            },
        ];
        f
    }

    /// A form open on Listening, with the mode cycling test's start state explicit.
    fn listening() -> ConfigForm {
        let mut f = providers();
        f.tab = ConfigTab::Listening;
        f
    }

    /// Two profiles, the second one active, with one voice each.
    fn profiles() -> ConfigForm {
        let mut f = providers();
        f.tab = ConfigTab::Profiles;
        f.active_profile = "meigo".into();
        f.profiles = vec![
            ProfileForm {
                id: "compae".into(),
                name: "El Compae".into(),
                language: "es-ES".into(),
                prompt: "p".into(),
                voice_instructions: "v".into(),
                greeting: "g".into(),
                wake_word: "oye compae".into(),
                voices: vec![("gemini".into(), "Charon".into())],
            },
            ProfileForm {
                id: "meigo".into(),
                name: "O Meigo".into(),
                language: "es-ES".into(),
                prompt: "p".into(),
                voice_instructions: "v".into(),
                greeting: "g".into(),
                wake_word: "oye meigo".into(),
                voices: vec![("gemini".into(), "Puck".into())],
            },
        ];
        f
    }

    /// The effects the screen asked the client for, with the no-ops dropped: the twelve tests below
    /// read a short list instead of eleven empty `Vec`s.
    fn effects(effects: Vec<ConfigEffect>) -> Vec<ConfigEffect> {
        effects.into_iter().filter(|e| *e != ConfigEffect::None).collect()
    }

    /// Types `text` one character at a time, the way a terminal delivers it.
    fn type_text(f: &mut ConfigForm, text: &str) {
        for c in text.chars() {
            f.apply(ConfigAction::Push(c));
        }
    }

    /// Moves focus to the row with this label, inside the current tab, and stops there: a label is
    /// what a reader knows, `field` is what the code moves.
    fn focus(f: &mut ConfigForm, label: &str) {
        let at = f.row_labels().iter().position(|l| *l == label).unwrap_or_else(|| panic!("no {label:?} row"));
        f.field = at;
    }

    #[test]
    fn the_tabs_wrap_around_in_both_directions() {
        let mut f = ConfigForm::default();
        assert_eq!(f.tab, ConfigTab::Providers);
        assert!(effects(f.apply(ConfigAction::PrevTab)).is_empty());
        assert_eq!(f.tab, ConfigTab::Server, "backwards from the first tab wraps");
        assert_eq!(f.tab_title(f.tab, strings(UiLanguage::Es)), ES.cfg_server);

        f.tab = ConfigTab::Server;
        f.field = 2;
        f.apply(ConfigAction::NextTab);
        assert_eq!(f.tab, ConfigTab::Providers, "forwards from the last tab wraps");
        assert_eq!(f.tab_title(f.tab, strings(UiLanguage::En)), EN.cfg_providers);
        assert_eq!(f.field, 0, "the rows belong to the tab, so the focus starts over");
    }

    #[test]
    fn the_field_stops_at_the_first_and_last_row_of_its_tab() {
        let mut f = providers();
        f.field = 0;
        f.apply(ConfigAction::PrevField);
        assert_eq!(f.field, 0, "there is no row before the first one");

        let last = f.rows().len() - 1;
        f.field = last;
        assert_eq!(f.rows()[last].label, ES.cfg_test, "the last provider row is the third group's Test row");
        f.apply(ConfigAction::NextField);
        assert_eq!(f.field, last, "and there is none after the last one");

        f.field = 0;
        f.apply(ConfigAction::NextField);
        assert_eq!(f.field, 1, "inside the list it still moves");

        // An empty row list cannot be walked into a panic: this is what a form with no profile
        // would look like, and `field` has nowhere to go.
        let mut empty = ConfigForm::default();
        empty.profiles.clear();
        empty.tab = ConfigTab::Profiles;
        empty.field = 0;
        empty.apply(ConfigAction::NextField);
        assert_eq!(empty.field, 0);
        assert!(empty.rows().is_empty());
    }

    #[test]
    fn the_providers_tab_shows_one_group_of_rows_for_each_capability() {
        let f = providers();
        let rows = f.rows();
        assert_eq!(rows.len(), 15, "five rows per capability, three capabilities");
        assert_eq!(rows.len(), f.row_labels().len(), "rows and labels are the same rows");
        assert_eq!(rows[0].label, ES.cfg_connector);
        assert_eq!(rows[0].value, "openai_compat");
        assert!(!rows[0].editable, "the connector is a list, not a text row");
        assert_eq!(rows[1].label, ES.cfg_base_url);
        assert!(rows[1].editable);
        assert_eq!(rows[1].value, "https://openrouter.ai/api/v1");
        assert_eq!(rows[4].label, ES.cfg_test, "every group ends with its own Test row");
        assert!(!rows[4].editable);
        assert_eq!(rows[5].label, ES.cfg_connector, "the second capability starts over");
        assert_eq!(rows[5].value, "openai_compat");
        assert_eq!(rows[9].label, ES.cfg_test, "and so does its group");
        assert_eq!(rows[10].label, ES.cfg_connector, "and the third one");
        assert_eq!(rows[14].label, ES.cfg_test);
        assert_eq!(rows[3].label, ES.cfg_key_name);
        assert_eq!(rows[3].value, "openrouter", "the key name is what the row shows, never a key");
    }

    #[test]
    fn the_listening_tab_hides_the_device_row_when_no_device_is_configured() {
        let mut f = listening();
        assert_eq!(f.rows().len(), 3, "the absent device is an absent row, not an empty one");
        let rows = f.rows();
        assert_eq!(rows[0].label, ES.cfg_mode);
        assert!(!rows[0].editable, "the mode is cycled with the toggle key");
        assert_eq!(rows[0].value, ES.mode_push);
        assert_eq!(rows[1].label, ES.cfg_silence_ms);
        assert!(rows[1].editable);
        assert_eq!(rows[1].value, "1000");
        assert_eq!(rows[2].label, ES.cfg_mic_gain);
        assert_eq!(rows[2].value, "12.0");

        f.listening.mic_device = Some("Rode NT-USB".into());
        let rows = f.rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[3].label, ES.cfg_mic_device);
        assert_eq!(rows[3].value, "Rode NT-USB");
        assert!(rows[3].editable);
        f.listening.mic_device = Some(String::new());
        assert_eq!(f.rows().len(), 4, "an empty name is a named device: the row stays on screen");
    }

    #[test]
    fn the_profiles_tab_shows_the_active_marker_before_the_fields_and_the_voices_last() {
        let mut f = profiles();
        assert_eq!(ConfigForm::profile_row_count(&f.profiles), 14, "1 + 11 + 2 voices");
        assert_eq!(f.rows().len(), 14);
        let rows = f.rows();
        assert_eq!(rows[0].label, ES.cfg_profile);
        assert_eq!(rows[0].value, "El Compae", "the list shows the names, one row per profile");
        assert!(!rows[0].is_active);
        assert!(!rows[1].is_active);
        assert!(rows[1].value.contains("O Meigo"));
        assert!(rows[1].value.contains("◉"), "the active profile is marked in the row itself");
        assert!(rows[1].is_active);
        assert_eq!(rows[2].label, ES.cfg_name, "the fields are the same for every profile");
        assert_eq!(rows[2].value, "O Meigo");
        assert_eq!(rows[12].label, ES.cfg_voices, "the voice rows are last");
        assert_eq!(rows[12].value, "gemini");
        assert_eq!(rows[13].value, "Puck", "the voices shown are the active profile's");

        f.active_profile = "compae".into();
        let rows = f.rows();
        assert!(rows[0].is_active);
        assert!(!rows[1].is_active);
        assert_eq!(rows[13].value, "Charon");
    }

    #[test]
    fn the_server_tab_ends_with_the_save_row_and_shows_the_unset_token() {
        let mut f = providers();
        f.tab = ConfigTab::Server;
        assert!(f.unset(), "an empty token is the unset state");
        let rows = f.rows();
        assert_eq!(rows.len(), 3, "bind, lan and Save: the token row is where Regenerate acts");
        assert_eq!(rows[0].label, ES.cfg_bind);
        assert_eq!(rows[0].value, "127.0.0.1:7070");
        assert_eq!(rows[1].label, ES.cfg_lan);
        assert!(!rows[1].editable);
        assert_eq!(rows[1].value, ES.cfg_unset, "a false toggle is shown as the unset word");
        assert_eq!(rows[2].label, ES.cfg_save, "the Save row is the tab's last row");
        assert!(!rows[2].is_active, "nothing is toggled on: Save is an action, not a switch");
        assert!(!rows[2].editable, "and it is not a text row either");

        f.server.lan = true;
        assert_eq!(f.rows()[1].value, "on");
        f.server.token = "0123456789abcdef0123456789abcdef".into();
        assert!(!f.unset());
        assert_eq!(f.rows()[0].value, "127.0.0.1:7070", "the token has no row of its own");
    }

    #[test]
    fn an_edit_starts_empty_and_replaces_the_value_on_commit() {
        let mut f = providers();
        focus(&mut f, ES.cfg_model);
        assert!(effects(f.apply(ConfigAction::BeginEdit)).is_empty());
        assert_eq!(f.editing.as_deref(), Some(""), "the buffer replaces the value, it does not extend it");
        assert_eq!(f.rows()[f.field].value, "google/gemini-2.5-flash", "the field still reads what it had");
        assert!(!f.dirty);

        type_text(&mut f, "ollama/llama3");
        assert_eq!(f.editing.as_deref(), Some("ollama/llama3"));
        assert!(effects(f.apply(ConfigAction::CommitEdit)).is_empty());
        assert_eq!(f.editing, None);
        assert_eq!(f.providers[0].model, "ollama/llama3");
        assert!(f.dirty, "a committed edit is an unsaved change");
        assert_eq!(f.rows()[f.field].value, "ollama/llama3");
    }

    #[test]
    fn backspace_removes_the_last_character_of_the_buffer_and_nothing_else() {
        let mut f = providers();
        focus(&mut f, ES.cfg_key_name);
        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "openroutr");
        f.apply(ConfigAction::Backspace);
        assert_eq!(f.editing.as_deref(), Some("openrout"));
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.providers[0].key_name, "openrout");

        // An empty buffer is a state, not a panic: the value is replaced by nothing.
        f.apply(ConfigAction::BeginEdit);
        f.apply(ConfigAction::Backspace);
        assert_eq!(f.editing.as_deref(), Some(""));
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.providers[0].key_name, "");
        assert!(f.dirty);
    }

    #[test]
    fn a_number_that_is_no_number_is_refused_and_says_so() {
        let mut f = listening();
        focus(&mut f, ES.cfg_silence_ms);
        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "mil");
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.silence_ms, "1000", "the row keeps the value it had");
        assert!(!f.dirty, "a refused edit is not a change to save");
        assert_eq!(
            f.notice.as_ref().map(|(_, text)| text.as_str()),
            Some(ES.cfg_value_not_whole),
            "the screen says what happened"
        );

        // A number that parses is clamped into its range rather than refused.
        focus(&mut f, ES.cfg_silence_ms);
        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "4");
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.silence_ms, "100");

        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "999999");
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.silence_ms, "30000");

        focus(&mut f, ES.cfg_mic_gain);
        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "7.5");
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.mic_gain, "7.5");

        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "99");
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.mic_gain, "30");
        assert!(f.dirty);
    }

    #[test]
    fn clearing_the_device_row_means_the_system_default_device() {
        let mut f = listening();
        f.listening.mic_device = Some("Rode NT-USB".into());
        focus(&mut f, ES.cfg_mic_device);
        f.apply(ConfigAction::BeginEdit);
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.mic_device, None, "no device chosen is the absent row");
        assert_eq!(f.rows().len(), 3);
        assert!(f.dirty);
    }

    #[test]
    fn the_toggle_cycles_the_listening_mode_and_the_server_toggle() {
        let mut f = listening();
        f.field = 0;
        assert_eq!(f.listening.mode, ListeningMode::Push);
        for (expected, wire) in [
            (ListeningMode::Auto, ES.mode_auto),
            (ListeningMode::HandsFree, ES.mode_hands_free),
            (ListeningMode::Push, ES.mode_push),
        ] {
            assert!(effects(f.apply(ConfigAction::Toggle)).is_empty());
            assert_eq!(f.listening.mode, expected);
            assert_eq!(f.rows()[0].value, wire);
            assert!(f.dirty);
        }
        // The mode has exactly three states, so the fourth toggle is where it started.
        f.apply(ConfigAction::Toggle);
        assert_eq!(f.listening.mode, ListeningMode::Auto);

        f.tab = ConfigTab::Server;
        f.field = 1;
        assert!(!f.server.lan);
        assert!(effects(f.apply(ConfigAction::Toggle)).is_empty());
        assert!(f.server.lan);
        f.apply(ConfigAction::Toggle);
        assert!(!f.server.lan);

        // A row that is not a toggle is left exactly as it was.
        f.tab = ConfigTab::Listening;
        f.field = 1;
        let before = f.listening.silence_ms.clone();
        f.apply(ConfigAction::Toggle);
        assert_eq!(f.listening.silence_ms, before);
    }

    #[test]
    fn choosing_a_profile_sets_the_active_id_and_leaves_the_row_focus_alone() {
        let mut f = profiles();
        f.field = 3;
        assert!(effects(f.apply(ConfigAction::SelectProfile(0))).is_empty());
        assert_eq!(f.active_profile, "compae");
        assert_eq!(f.field, 3, "clicking a profile chooses it; it does not move the keyboard focus");
        assert!(f.dirty);

        // A stale click cannot index past the list, and cannot panic either.
        f.apply(ConfigAction::SelectProfile(9));
        assert_eq!(f.active_profile, "compae", "out of range is a no-op");
    }

    #[test]
    fn save_and_regenerate_are_the_only_effects_the_reducer_raises() {
        let mut f = providers();
        assert_eq!(effects(f.apply(ConfigAction::Save)), vec![ConfigEffect::Save]);
        assert!(f.dirty);

        // Every capability has a Test row, and the row names the connector to probe.
        for (field, capability) in
            [(1, Capability::Chat), (6, Capability::Speech), (11, Capability::Transcribe)]
        {
            f.tab = ConfigTab::Providers;
            f.field = field;
            assert_eq!(
                effects(f.apply(ConfigAction::Test)),
                vec![ConfigEffect::Test(capability)],
                "the group the focused row belongs to decides which connector is probed"
            );
        }
        // And the Test row of a group is the same button: activating it raises the same effect.
        f.field = 14;
        assert_eq!(effects(f.apply(ConfigAction::Toggle)), vec![ConfigEffect::Test(Capability::Transcribe)]);

        // On the Server tab the token is regenerated and the form is left untouched: the client
        // mints the value (Task 6) and writes it into this row.
        f.tab = ConfigTab::Server;
        f.field = 2;
        assert_eq!(effects(f.apply(ConfigAction::RegenerateToken)), vec![ConfigEffect::RegenerateToken]);
        assert!(f.dirty);
        // And the row is an action rather than a switch: activating it twice does not flip anything.
        assert!(!f.server.lan);
    }

    #[test]
    fn ctrl_r_regenerates_the_token_without_editing_any_row() {
        let mut f = providers();
        f.tab = ConfigTab::Server;
        // The token row is the first row of the tab, and the shortcut works from anywhere in it.
        f.field = 0;
        let outcome = f.key(KeyInput::Char('\u{12}'));
        assert_eq!(outcome.action, Some(ConfigAction::RegenerateToken));
        assert_eq!(outcome.effects, vec![ConfigEffect::RegenerateToken]);
        assert!(f.editing.is_none(), "a shortcut is not an edit");
        assert_eq!(f.server.bind, "127.0.0.1:7070", "and it writes no row itself: the client mints the token");
    }

    #[test]
    fn closing_discards_the_edits_and_leaves_the_form_as_it_was() {
        let mut f = providers();
        focus(&mut f, ES.cfg_base_url);
        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "http://localhost:11434/v1");
        assert!(f.editing.is_some());
        f.apply(ConfigAction::Close);
        assert_eq!(f.editing, None, "the half-typed value is the edit, and it goes with the screen");
        assert_eq!(f.providers[0].base_url, "https://openrouter.ai/api/v1", "the field never moved");
        assert!(!f.dirty, "nothing was written, so there is nothing unsaved either");
    }

    #[test]
    fn the_navigation_keys_mean_the_same_thing_as_the_actions() {
        let mut f = providers();
        assert_eq!(f.key(KeyInput::F(9)).action, None, "the screen is already open");
        assert_eq!(f.key(KeyInput::Down).action, Some(ConfigAction::NextField));
        assert_eq!(f.key(KeyInput::Up).action, Some(ConfigAction::PrevField));
        assert_eq!(f.key(KeyInput::Tab).action, Some(ConfigAction::NextTab));
        assert_eq!(f.key(KeyInput::Right).action, Some(ConfigAction::NextTab));
        assert_eq!(f.key(KeyInput::Left).action, Some(ConfigAction::PrevTab));
        assert_eq!(f.key(KeyInput::Enter).action, Some(ConfigAction::BeginEdit));
        assert_eq!(f.key(KeyInput::Esc).action, Some(ConfigAction::Close));
        assert_eq!(f.key(KeyInput::Char('\u{13}')).action, Some(ConfigAction::Save));
        assert_eq!(f.key(KeyInput::Char('\u{12}')).action, Some(ConfigAction::RegenerateToken));
        assert_eq!(f.key(KeyInput::Char('x')).action, Some(ConfigAction::Push('x')));
        assert_eq!(f.key(KeyInput::Char('\u{13}')).effects, vec![ConfigEffect::Save]);
        assert_eq!(f.key(KeyInput::Char('\u{12}')).effects, vec![ConfigEffect::RegenerateToken]);
        assert_eq!(f.key(KeyInput::Char('x')).effects, Vec::new(), "an open screen types nothing into the void");

        f.field = 0;
        assert_eq!(f.key(KeyInput::Enter).action, Some(ConfigAction::Toggle), "a toggle row has no text to edit");
    }

    #[test]
    fn while_a_row_is_being_edited_the_keys_edit_and_do_not_navigate() {
        let mut f = providers();
        focus(&mut f, ES.cfg_model);
        f.apply(ConfigAction::BeginEdit);
        assert_eq!(f.key(KeyInput::Char('a')).action, Some(ConfigAction::Push('a')));
        assert_eq!(f.key(KeyInput::Backspace).action, Some(ConfigAction::Backspace));
        assert_eq!(f.key(KeyInput::Enter).action, Some(ConfigAction::CommitEdit));
        assert_eq!(f.key(KeyInput::Esc).action, Some(ConfigAction::Close), "Esc is the screen's, not the buffer's");
        assert_eq!(
            f.key(KeyInput::Down).action,
            None,
            "Down is neither text nor a way out: it must not move the focus mid-edit"
        );
        assert_eq!(f.key(KeyInput::Tab).action, None, "nor may Tab jump to the next tab");
    }

    /// The screen's copy is in the tables and nowhere else, and both tables carry every key the
    /// screen can show — including the ones the current state happens to keep off screen.
    #[test]
    fn both_string_tables_carry_every_key_the_screen_uses() {
        // The whole form, which is the widest set of keys any state can ask for: two profiles with a
        // voice each, a device chosen, and a token generated.
        let mut f = profiles();
        f.listening.mic_device = Some("Rode NT-USB".into());
        f.server.token = "0123456789abcdef0123456789abcdef".into();
        f.notice = Some((NoticeKind::Warning, ES.cfg_value_not_whole.into()));

        let mut keys = f.string_keys();
        for tab in [ConfigTab::Providers, ConfigTab::Listening, ConfigTab::Profiles, ConfigTab::Server] {
            for lang in [UiLanguage::Es, UiLanguage::En] {
                keys.push(ConfigForm::tab_title(tab, strings(lang)));
            }
        }
        keys.push(ES.cfg_value_not_whole);
        keys.push(EN.cfg_value_not_whole);

        for key in keys {
            assert!(!key.trim().is_empty(), "a label may not be blank");
            assert!(
                ES.cfg_help.contains("F9") && EN.cfg_help.contains("F9"),
                "the help line says how the screen is opened: {}",
                ES.cfg_help
            );
            assert!(!ES.cfg_help_editing.trim().is_empty());
            assert!(!EN.cfg_help_editing.trim().is_empty());
        }

        // The two tables are different tables, not one table read twice.
        assert_ne!(ES.cfg_providers, EN.cfg_providers);
        assert_ne!(ES.cfg_save, EN.cfg_save);
        assert_ne!(ES.mode_hands_free, EN.mode_hands_free);

        // And the labels the rows hand out are exactly the table's.
        for label in f.row_labels() {
            assert!(
                [ES.cfg_connector, ES.cfg_base_url, ES.cfg_model, ES.cfg_key_name, ES.cfg_test]
                    .contains(&label),
                "an unknown row label: {label:?}"
            );
        }
    }
}
```

  The tests are in the same module as the code they exercise (`crates/tipsy-ui/src/config_form.rs`), which is where the plan puts them; `crate::strings::{strings, EN, ES}` is the table they check.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p tipsy-ui --lib config_form`

Expected: FAIL — the file has no implementation, so every test is a compile error (rustc reports the first few and gives up):

```
error[E0433]: failed to resolve: use of undeclared crate or module `config_form`
error[E0433]: failed to resolve: could not find `ProviderForm` in the crate root
error[E0412]: cannot find type `ConfigForm` in this scope
error[E0599]: no variant or associated item named `Providers` found for enum `ConfigTab` in the current scope
```

- [ ] **Step 3: Implement the forms, the row list and the reducer (English comments, nothing invented)**

  **3a. `crates/tipsy-ui/src/strings.rs`.** Add this field group to `Strings` after `pub network: &'static str,` (line 13), keeping the declaration order below:

```rust
    pub cfg_title: &'static str,
    pub cfg_providers: &'static str,
    pub cfg_listening: &'static str,
    pub cfg_profiles: &'static str,
    pub cfg_server: &'static str,
    pub cfg_connector: &'static str,
    pub cfg_base_url: &'static str,
    pub cfg_model: &'static str,
    pub cfg_key_name: &'static str,
    pub cfg_test: &'static str,
    pub cfg_mode: &'static str,
    pub cfg_silence_ms: &'static str,
    pub cfg_mic_gain: &'static str,
    pub cfg_mic_device: &'static str,
    pub cfg_profile: &'static str,
    pub cfg_name: &'static str,
    pub cfg_language: &'static str,
    pub cfg_prompt: &'static str,
    pub cfg_voice_instructions: &'static str,
    pub cfg_greeting: &'static str,
    pub cfg_wake_word: &'static str,
    pub cfg_voices: &'static str,
    pub cfg_bind: &'static str,
    pub cfg_lan: &'static str,
    pub cfg_token: &'static str,
    pub cfg_regenerate: &'static str,
    pub cfg_save: &'static str,
    pub cfg_dirty: &'static str,
    pub cfg_unset: &'static str,
    pub cfg_help: &'static str,
    pub cfg_help_editing: &'static str,
    pub cfg_cancel: &'static str,
    pub cfg_value_not_whole: &'static str,
    pub kind_openai_compat: &'static str,
    pub kind_gemini: &'static str,
    pub kind_elevenlabs: &'static str,
    pub mode_push: &'static str,
    pub mode_auto: &'static str,
    pub mode_hands_free: &'static str,
```

  In `ES`, after `network: "RED",`:

```rust
    cfg_title: "CONFIGURACIÓN",
    cfg_providers: "Proveedores",
    cfg_listening: "Escucha",
    cfg_profiles: "Perfiles",
    cfg_server: "Servidor",
    cfg_connector: "Conector",
    cfg_base_url: "URL base",
    cfg_model: "Modelo",
    cfg_key_name: "Nombre de la clave",
    cfg_test: "Probar",
    cfg_mode: "Modo",
    cfg_silence_ms: "Silencio (ms)",
    cfg_mic_gain: "Ganancia del micro",
    cfg_mic_device: "Dispositivo de entrada",
    cfg_profile: "Perfil",
    cfg_name: "Nombre",
    cfg_language: "Idioma",
    cfg_prompt: "Prompt",
    cfg_voice_instructions: "Instrucciones de voz",
    cfg_greeting: "Saludo",
    cfg_wake_word: "Palabra clave",
    cfg_voices: "Voces",
    cfg_bind: "Dirección",
    cfg_lan: "Red local",
    cfg_token: "Token",
    cfg_regenerate: "Regenerar token",
    cfg_save: "Guardar",
    cfg_dirty: "sin guardar",
    cfg_unset: "sin definir",
    cfg_help: " ↑↓ fila · ←→ pestaña · Enter editar · Enter/espacio alternar · Ctrl+S guardar · Esc cerrar",
    cfg_help_editing: " escribiendo… · Enter aplicar · Esc descartar",
    cfg_cancel: "descartar",
    cfg_value_not_whole: "número no válido",
    kind_openai_compat: "openai_compat",
    kind_gemini: "gemini",
    kind_elevenlabs: "elevenlabs",
    mode_push: "push",
    mode_auto: "auto",
    mode_hands_free: "hands free",
```

  In `EN`, after `network: "NETWORK",`:

```rust
    cfg_title: "CONFIGURATION",
    cfg_providers: "Providers",
    cfg_listening: "Listening",
    cfg_profiles: "Profiles",
    cfg_server: "Server",
    cfg_connector: "Connector",
    cfg_base_url: "Base URL",
    cfg_model: "Model",
    cfg_key_name: "Key name",
    cfg_test: "Test",
    cfg_mode: "Mode",
    cfg_silence_ms: "Silence (ms)",
    cfg_mic_gain: "Mic gain",
    cfg_mic_device: "Input device",
    cfg_profile: "Profile",
    cfg_name: "Name",
    cfg_language: "Language",
    cfg_prompt: "Prompt",
    cfg_voice_instructions: "Voice instructions",
    cfg_greeting: "Greeting",
    cfg_wake_word: "Wake word",
    cfg_voices: "Voices",
    cfg_bind: "Bind",
    cfg_lan: "LAN",
    cfg_token: "Token",
    cfg_regenerate: "Regenerate token",
    cfg_save: "Save",
    cfg_dirty: "unsaved",
    cfg_unset: "unset",
    cfg_help: " ↑↓ row · ←→ tab · Enter edit · Enter/space toggle · Ctrl+S save · Esc close",
    cfg_help_editing: " typing… · Enter apply · Esc discard",
    cfg_cancel: "discard",
    cfg_value_not_whole: "not a number",
    kind_openai_compat: "openai_compat",
    kind_gemini: "gemini",
    kind_elevenlabs: "elevenlabs",
    mode_push: "push",
    mode_auto: "auto",
    mode_hands_free: "hands free",
```

  The three `kind_*` values and the three `mode_*` values are the **wire spellings** of `ProviderKind` and `ListeningMode`, and they are identical in both tables on purpose: they are what Task 6 parses back into the native enums, so they may not be translated. The four notice words (`cfg_value_not_whole`, `cfg_cancel`, `cfg_dirty`, `cfg_unset`) are the only ones Task 6 and Task 4 may put in a sentence.

  **3b. `crates/tipsy-ui/src/config_form.rs`** — below the `#[cfg(test)] mod tests` of Step 1:

```rust
//! The configuration screen's form view model.
//!
//! Plain values only: no `Config` (that type lives in `tipsy-compadre`, which this crate may not
//! depend on), no clock, no terminal. The native client converts `Config` ⇄ `ConfigForm` and is the
//! only thing that reads or writes the file; everything on this side is what the screen shows and
//! what a key or a click does to it.

use tipsy_protocol::{Capability, ListeningMode, UiLanguage};

use crate::state::KeyInput;
use crate::strings::{strings, Strings};

/// The keyboard's `Ctrl+S`: `KeyInput::Char` carries no modifiers, and a control character is the
/// one `char` a text row would never want, so the two chords the screen has need no new variant.
/// `Ctrl+S` saves and `Ctrl+R` regenerates the server token, and `Ctrl+R` is not a browser-reserved
/// key (spec §9 rules out `F5`, `F11` and `Ctrl+W`).
pub const SAVE_KEY: char = '\u{13}';
pub const REGENERATE_KEY: char = '\u{12}';

/// The silence threshold a form accepts, in milliseconds: `silence_ms` is read by the VAD, which
/// cannot hear a pause shorter than this or wait longer than that.
pub const SILENCE_MS_MIN: u32 = 100;
pub const SILENCE_MS_MAX: u32 = 30_000;
/// The gain the form accepts. Above this a quiet room is already clipping.
pub const MIC_GAIN_MIN: f32 = 0.0;
pub const MIC_GAIN_MAX: f32 = 30.0;

const PROFILE_FIELD_COUNT: usize = 11;
/// The Profiles tab's row layout, and the indices a later task may name without re-deriving them:
/// `SERVER_ROW_SAVE` is the Server tab's last row (`bind`, `lan`, Save), and `VOICE_LABEL_ROW` is
/// where the voice rows start inside the Profiles tab — the label `cfg_voices` at that index in the
/// row list marks the table. Task 4's tests use both, so they are `pub`.
pub const SERVER_ROW_SAVE: usize = 2;
pub const VOICE_LABEL_ROW: usize = ROW_VOICES;
const SERVER_ROW_COUNT: usize = 3;
const ROW_VOICES: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigTab {
    Providers,
    Listening,
    Profiles,
    Server,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeKind {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigEffect {
    None,
    Save,
    Test(Capability),
    RegenerateToken,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormRow {
    /// From the string table: a label is a `&'static str`, so it cannot be built at runtime and
    /// cannot escape `Strings`.
    pub label: &'static str,
    pub value: String,
    pub editable: bool,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderForm {
    pub capability: Capability,
    pub kind: String,
    pub base_url: String,
    pub model: String,
    pub key_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListeningForm {
    pub mode: ListeningMode,
    pub silence_ms: String,
    pub mic_gain: String,
    /// `None` is the system default device and hides the row; `Some("")` is a named device whose
    /// name is empty, which is a different (and useless) value, so it keeps its row. `Config` holds
    /// the same distinction.
    pub mic_device: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerForm {
    pub bind: String,
    pub lan: bool,
    pub token: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileForm {
    pub id: String,
    pub name: String,
    pub language: String,
    pub prompt: String,
    pub voice_instructions: String,
    pub greeting: String,
    pub wake_word: String,
    /// Sorted by connector name: a map's order is not a screen order, and the row index `field` is
    /// what the clicks name back. `Config`'s `BTreeMap` arrives here in exactly this order.
    pub voices: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigForm {
    pub tab: ConfigTab,
    /// The focused row inside the current tab, an index into [`ConfigForm::rows`].
    pub field: usize,
    /// The text being typed right now. It starts **empty** and replaces the row's value when it is
    /// committed, because the cursor is at the end of the buffer and there is no second cursor.
    pub editing: Option<String>,
    pub active_profile: String,
    pub providers: Vec<ProviderForm>,
    pub listening: ListeningForm,
    pub profiles: Vec<ProfileForm>,
    pub server: ServerForm,
    pub notice: Option<(NoticeKind, String)>,
    pub dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigAction {
    Open,
    Close,
    NextTab,
    PrevTab,
    NextField,
    PrevField,
    BeginEdit,
    Push(char),
    Backspace,
    CommitEdit,
    Toggle,
    SelectProfile(usize),
    Save,
    Test,
    RegenerateToken,
}

/// What one key did: the action it was translated into, if any, and the effects that action had.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyOutcome {
    pub action: Option<ConfigAction>,
    pub effects: Vec<ConfigEffect>,
}

impl Default for ConfigForm {
    fn default() -> Self {
        Self {
            tab: ConfigTab::Providers,
            field: 0,
            editing: None,
            active_profile: "compae".into(),
            providers: Vec::new(),
            listening: ListeningForm {
                mode: ListeningMode::Push,
                silence_ms: "1000".into(),
                mic_gain: "12.0".into(),
                mic_device: None,
            },
            // One placeholder profile, so the rows below have something to index. The native client
            // replaces every field of this form from the real configuration (Task 6).
            profiles: vec![ProfileForm {
                id: "compae".into(),
                name: "El Compae".into(),
                language: "es-ES".into(),
                ..ProfileForm::default()
            }],
            server: ServerForm { bind: "127.0.0.1:7070".into(), lan: false, token: String::new() },
            notice: None,
            dirty: false,
        }
    }
}

impl ConfigForm {
    ... (the implementation the tests above pin: `rows`, `apply`, `key`, `row_labels`,
    `string_keys`, `unset`, `tab_title`, `profile_row_count`, and the private helpers
    `field_rows`, `commit_edit`, `write_field`, `set_number`, `clear_field`, `next_field`,
    `prev_field`, `edit_action`, `activate`, and the `rows` builders `provider_rows`,
    `listening_rows`, `profile_rows`, `server_rows`)
}
```

  The implementation is mechanical, and the tests above are its specification. The only decisions left to it, and how to take them:

- `rows()` matches `self.tab` and returns the four builders' output; the tests reach the row list through `rows()` and `row_labels()` only, so neither helper needs to exist beyond those two.
- `row_labels()` is `self.rows().into_iter().map(|row| row.label).collect()`.
- The builders are `provider_rows` (the focused index is `self.field`; every row is `editable` except the connector, and no row is a cycling toggle), `listening_rows` (three rows, plus the device row when `mic_device` is `Some`; row values are `strings().mode_push`/`mode_auto`/`mode_hands_free` for the mode and the stored strings for the rest), `profile_rows` (the list row, whose value is `format!("● {name}")` for the active profile and `format!("  {name}")` for the others; then the eleven field rows of the **active** profile; then one row per voice entry, whose label is `strings().cfg_voices` and whose value is the connector), and `server_rows` (bind, LAN — `editable: false`, `value` the LAN toggle's own word — and the Save row).
- `profile_row_count(profiles)` is `1 + PROFILE_FIELD_COUNT + profiles.iter().map(|p| p.voices.len()).sum::<usize>()`, and a form with no profiles at all has an empty rows list.
- `next_field`/`prev_field` saturate at the ends and do nothing on an empty list; `next_tab`/`prev_tab` set `field = 0` and, if a row was being edited, leave the buffer to `Close` — a tab switch while typing is refused instead, see below.
- `apply` returns `Vec::new()` for everything but the four effects the tests pin, and the `ConfigEffect::None` variant is never constructed (it is in the plan's signature, so it is declared; `effects()` in the tests filters it out so either spelling passes).
- A **new profile has no row**: `ConfigAction` has no `NewProfile`, and the plan's Profiles tab is "active profile, multi-line editors for its texts, voices table". Adding one is a new action, not a new row.
- `is_active` is `true` on the profile list's active row and `false` everywhere else, matching the `◉` marker the row's value carries.
- `set_number(label, current, min, max)`: `trim().parse::<u32>()` (or `f32`); on failure leave the row alone, set `notice = Some((NoticeKind::Warning, strings().cfg_value_not_whole.into()))`, return `false` (so `dirty` is not set); on success write the clamped value with `to_string()` and return `true`.
- `clear_field()` empties an editable string column (and, on the device row, sets `mic_device = None`); it is what an empty committed buffer means.
- Commit on the device row: an empty buffer is `None`, anything else is `Some(that)`; commit on a voice row replaces only the voice's value, not its connector.
- `activate(row)`: the action a **non-editable** row takes when it is actuated — by `Enter` on it or by a click, which is the same call. On Providers, a row whose row list label is the group's Test row (the fifth row of each group) raises `ConfigEffect::Test(capability)`; on Server, the LAN row (index 1) flips `lan`; the Save row (`SERVER_ROW_SAVE`) raises `ConfigEffect::Save`; the token row does nothing by itself (Regenerate is its own action, raised by the button and by `Ctrl+S`'s neighbour key); on every other row it does nothing. `enter_action()` returns `Toggle` on exactly these rows, so the keyboard and the mouse reach `activate` by one path.
- `key()`:

```rust
    pub fn key(&mut self, key: KeyInput) -> KeyOutcome {
        let action = match key {
            // A control character is the one `char` a text row wants none of: the two chords the
            // screen has arrive as their own control characters, so no variant is needed for either.
            KeyInput::Char(SAVE_KEY) => Some(ConfigAction::Save),
            KeyInput::Char(REGENERATE_KEY) => Some(ConfigAction::RegenerateToken),
            KeyInput::Char(c) => {
                if self.editing.is_some() {
                    Some(ConfigAction::Push(c))
                } else {
                    // Typing outside an edit is not a text entry: a printable key is not a shortcut,
                    // and every shortcut the form has is a control character or a named key.
                    None
                }
            }
            KeyInput::Backspace => self.editing.is_some().then_some(ConfigAction::Backspace),
            KeyInput::Enter => Some(if self.editing.is_some() { ConfigAction::CommitEdit } else { self.enter_action() }),
            KeyInput::Tab | KeyInput::Right => {
                if self.editing.is_some() { None } else { Some(ConfigAction::NextTab) }
            }
            KeyInput::Left => {
                if self.editing.is_some() { None } else { Some(ConfigAction::PrevTab) }
            }
            KeyInput::Up => self.editing.is_none().then_some(ConfigAction::PrevField),
            KeyInput::Down => self.editing.is_none().then_some(ConfigAction::NextField),
            KeyInput::Esc => Some(ConfigAction::Close),
            KeyInput::F(_) => None,
        };
        match action {
            Some(action) => KeyOutcome { action: Some(action), effects: self.apply(action) },
            None => KeyOutcome { action: None, effects: Vec::new() },
        }
    }
```

    The cases above are the ones the tests in Step 1 pin: the two control characters, a printable character inside and outside an edit, `Backspace`, `Enter`, the four navigation keys, `Esc` and the unused `F` keys. `enter_action()` returns `BeginEdit` on an editable row and `Toggle` on every non-editable one, and `apply(ConfigAction::Toggle)` is what decides whether that row raises anything; `key(KeyInput::Char(c))` outside an edit is `None` — a stray printable key while the screen is open edits nothing.

- `tab_title(tab, strings)` is a `match` over the four tabs returning `cfg_providers`, `cfg_listening`, `cfg_profiles` and `cfg_server`.
- `string_keys()` returns every screen field of `Strings`, in table order: the four tab titles and `cfg_title` through `cfg_value_not_whole`, then `kind_*`, then `mode_*`, plus `strings(UiLanguage::Es).cfg_help`, `strings(UiLanguage::Es).cfg_help_editing`, and their `En` twins. Task 4 uses the same list for its help and notice lines.
- `unset()` is `self.server.token.trim().is_empty()`.

- [ ] **Step 4: Run them and watch them pass**

Run: `cargo test -p tipsy-ui --lib config_form`

Expected: PASS, 18 tests:

```
running 18 tests
test config_form::tests::a_number_that_is_no_number_is_refused_and_says_so ... ok
test config_form::tests::an_edit_starts_empty_and_replaces_the_value_on_commit ... ok
test config_form::tests::backspace_removes_the_last_character_of_the_buffer_and_nothing_else ... ok
test config_form::tests::both_string_tables_carry_every_key_the_screen_uses ... ok
test config_form::tests::choosing_a_profile_sets_the_active_id_and_leaves_the_row_focus_alone ... ok
test config_form::tests::clearing_the_device_row_means_the_system_default_device ... ok
test config_form::tests::closing_discards_the_edits_and_leaves_the_form_as_it_was ... ok
test config_form::tests::ctrl_r_regenerates_the_token_without_editing_any_row ... ok
test config_form::tests::save_and_regenerate_are_the_only_effects_the_reducer_raises ... ok
test config_form::tests::the_field_stops_at_the_first_and_last_row_of_its_tab ... ok
test config_form::tests::the_listening_tab_hides_the_device_row_when_no_device_is_configured ... ok
test config_form::tests::the_navigation_keys_mean_the_same_thing_as_the_actions ... ok
test config_form::tests::the_profiles_tab_shows_the_active_marker_before_the_fields_and_the_voices_last ... ok
test config_form::tests::the_providers_tab_shows_one_group_of_rows_for_each_capability ... ok
test config_form::tests::the_server_tab_ends_with_the_save_row_and_shows_the_unset_token ... ok
test config_form::tests::the_tabs_wrap_around_in_both_directions ... ok
test config_form::tests::the_toggle_cycles_the_listening_mode_and_the_server_toggle ... ok
test config_form::tests::while_a_row_is_being_edited_the_keys_edit_and_do_not_navigate ... ok

test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

  Then the crate: `cargo test -p tipsy-ui` — the 18 above plus the existing `state` and `render` tests, all green and unchanged.

- [ ] **Step 5: Prove the boundary the constraints name**

Run:

```powershell
Select-String -Path crates/tipsy-ui/src/config_form.rs, crates/tipsy-ui/src/strings.rs -Pattern 'tokio|cpal|rodio|keyring|sysinfo|reqwest|crossterm|Instant|std::time'
```

Expected: **no output** (`[exit code: 1]` is the empty result, not a failure). `crates/tipsy-ui/Cargo.toml` still lists only `tipsy-protocol` and `ratatui`, so `cargo tree -p tipsy-ui -e normal` shows no new edge and the crate keeps compiling for `wasm32-unknown-unknown`: it added no dependency, no feature, no clock and no thread. `git diff --stat crates/tipsy-ui/Cargo.toml` is empty.

- [ ] **Step 6: Commit**

```bash
git add crates/tipsy-ui/src/lib.rs crates/tipsy-ui/src/strings.rs crates/tipsy-ui/src/config_form.rs
git commit -m "feat(ui): add the screen's strings and ConfigForm

The configuration screen's es/en labels join the two static tables, and
ConfigForm holds the four tabs as plain values beside the reducer that walks
them: tabs, rows, text edits, toggles, the profile list and the voice table.
tipsy-ui keeps its boundary: no dependency, no clock, no native type.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The four tabs' rendering `[mid]`

**Files:**
- Modify: `crates/tipsy-ui/src/lib.rs:3-6` — `pub mod render_config;` after `pub mod render;`, once Task 3 has put `pub mod config_form;` there
- Create: `crates/tipsy-ui/src/render_config.rs` — `draw_form` plus its `#[cfg(test)] mod tests`
- Test: `crates/tipsy-ui/src/render_config.rs` — the `TestBackend` tests below

**Interfaces:**
- Consumes (from Task 3, quoted from `config_form.rs` as it lands — do not re-derive them):
  - `pub struct FormRow { pub label: &'static str, pub value: String, pub editable: bool, pub is_active: bool }`
  - `pub fn ConfigForm::rows(&self) -> Vec<FormRow>`, `pub fn ConfigForm::row_labels(&self) -> Vec<&'static str>`, `pub fn ConfigForm::unset(&self) -> bool`, `pub fn ConfigForm::tab_title(tab: ConfigTab, strings: &Strings) -> &'static str`, `pub fn ConfigForm::profile_row_count(profiles: &[ProfileForm]) -> usize`
  - `pub enum ConfigTab { Providers, Listening, Profiles, Server }`, `pub enum ConfigAction { .. }`, `pub enum ConfigEffect { .. }`, `pub enum NoticeKind { Ok, Warning, Error }`
  - The buffer convention: `ConfigForm::editing` is the text being typed and it **replaces** the focused row's value on commit, so `editing == Some("")` is "the row is being edited and nothing has been typed yet".
- Consumes (from the committed HUD, which this file mirrors rather than imports):
  - `pub struct Hit { pub area: Rect, pub key: KeyInput }` and `fn panel(title: &str) -> Block<'static>` — `render.rs:48-52`, `:483-491`. Both are private to `render.rs`; `Hit` is public, `panel` is not, so this file draws its own borders with the same style constants (`Color::Rgb(0, 210, 255)` cyan, `Color::Rgb(0, 95, 125)` dim, `Color::Rgb(255, 190, 60)` gold) declared locally.
  - `pub struct UiState` with `pub fn strings(&self) -> &'static Strings` — `state.rs:74-134`; the form itself is `state.config_form: Option<ConfigForm>`, which Task 5 adds. **This task renders a form handed to it, so it needs no `UiState` field**: `draw_form(frame, form, strings, lang, elapsed_ms) -> Vec<Hit>`.
  - `KeyInput` — `state.rs:52-64`.
  - The test style is `crates/tipsy-ui/tests/render.rs`: `Terminal::new(TestBackend::new(w, h))`, `term.draw(|f| ...)`, then reading `term.backend().buffer()`; `text_at` (`:32-34`) reads one `Rect`'s symbols off the buffer.
- Produces:

```rust
/// Draws the configuration screen over the whole frame and reports every clickable area, header
/// controls first.
pub fn draw_form(frame: &mut Frame, form: &ConfigForm, strings: &Strings, elapsed_ms: u64) -> Vec<Hit>;
```

  - The returned `Vec<Hit>` is `[mode, ⚙, four tab labels, the visible rows' value rects, Test, Save]` plus the row rectangles the keyboard sends to: `KeyInput::F(7)`–`KeyInput::F(10)` select the tab (the HUD's `F7`/`F8` are chat/network, and the screen's tabs take the next two), `KeyInput::F(4)` is Save, `KeyInput::F(2)` is Test (both from the HUD's own vocabulary: `F4` is the HUD's clear, `F2` its replay, and the screen reuses neither while it is open), and every row reports `KeyInput::Enter`.
  - **Every clickable rect comes from `form.rows()`**: the renderer draws the rows that list returns and the hit areas are that same list's rectangles, so what is drawn and what is clickable cannot disagree. A value rect is clipped to its column (`Rect { x, y, width: width.min(area.right() - x), height: 1 }`), so a click on the column's right edge belongs to the row whose column it is.
  - Task 5 calls this from `render.rs::draw` and returns its hits after the header's; nothing else in `tipsy-ui` may compute a screen hit rectangle.

**What this task decides, and why**

- **The screen is drawn as a whole frame, not inside the HUD's panels.** §9 calls it "full screen", and the row list is one widget of variable length; nesting it in the HUD's 30%-wide left column would put two rows in a row of screen and make the columns (a label, a value, a Test button) disagree with the panel's width. The layout is therefore the frame's: a title row, a tab row, a notice row, the body, and a help line (plus the editing help line where there is room for it).
- **The body lays out label and value in two columns.** Label width is a third of the body, at least 12 columns and at most 26: enough for "Instrucciones de voz" at a reasonable terminal, and it cannot squeeze the value out on a narrow one. The value column is everything that is left, clipped from the left when the row is being edited (the cursor is at the end of the buffer) and from the right otherwise.
- **A value too long for its column is cut at the column's edge, never spilled.** `Paragraph` does not wrap without `.wrap(..)`, and this task does not use `.wrap`: a long base URL ends at the column boundary, and the test below proves the neighbouring column's first cell is untouched. Cutting counts **characters**, not columns, and takes at least one character per call, so a symbol wider than the column cannot loop for ever — the same rule `render.rs:281-293` already uses.
- **The focused row is drawn as a bar, the active profile has a marker, and neither moves the other.** Focus is `form.field`, which is also what `next_field`/`prev_field` move; the active profile is `form.active_profile`, which `rows()` reports as `is_active`. The two cannot drift because the test below reads the same `rows()` the renderer drew.
- **The edit cursor is a `│` drawn between the committed value and the buffer.** The buffer is not empty when the row is edited (it starts empty by design, and the value stays on screen), so the cursor position is `cursor_x + text_width(value)`, clamped into the column. When the text does not fit, the value is cut from the left and the cursor sits at the column's last cell.
- **The notice line is always there, and it is one row.** A save failure (Task 6), a refused number (Task 3) and a successful test (Task 7) all land in the same line, coloured by `NoticeKind` (`Ok` green, `Warning` gold, `Error` red). An empty line is drawn when there is no notice: the body never moves up and down under the user as notices come and go.
- **`dirty` is shown next to the title, not as a notice.** `strings().cfg_dirty` marks the title while there is something unsaved, so a notice never has to say it.
- **A terminal too small is not a panic and not a garbled screen.** Every rectangle is computed with saturating arithmetic and skipped when it is empty (`Rect::is_empty`), the row loop stops when the body runs out of rows, and the layout uses `Constraint::Min`: at 1×1 the screen draws whatever fits and returns no hits, at 80×24 it is fully usable. Review Focus 4's promise is the test below, not prose.

- [ ] **Step 1: The failing tests**

  In `crates/tipsy-ui/src/lib.rs`, replace lines 3-4:

```rust
pub mod config_form;
pub mod render;
pub mod render_config;
```

  Then create `crates/tipsy-ui/src/render_config.rs` with the test module below and no `draw_form` yet:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_form::{ConfigTab, ListeningForm, NoticeKind, ProfileForm, ProviderForm, ServerForm};
    use crate::strings::strings;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use tipsy_protocol::{Capability, ListeningMode, UiLanguage};

    /// A frame's worth of symbols, exactly as `crates/tipsy-ui/tests/render.rs` reads them.
    fn rendered(form: &ConfigForm, lang: UiLanguage, w: u16, h: u16) -> (String, Vec<Hit>, Terminal<TestBackend>) {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Vec::new();
        term.draw(|f| hits = draw_form(f, form, strings(lang), 0)).unwrap();
        let text: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
        (text, hits, term)
    }

    /// The symbols of one rectangle, as `tests/render.rs`'s `text_at` reads them.
    fn text_at(term: &Terminal<TestBackend>, r: Rect) -> String {
        (r.x..r.right()).map(|x| term.backend().buffer()[(x, r.y)].symbol().to_string()).collect()
    }

    fn providers() -> ConfigForm {
        let mut f = ConfigForm::default();
        f.providers = vec![
            ProviderForm {
                capability: Capability::Chat,
                kind: "openai_compat".into(),
                base_url: "https://openrouter.ai/api/v1".into(),
                model: "google/gemini-2.5-flash".into(),
                key_name: "openrouter".into(),
            },
            ProviderForm {
                capability: Capability::Speech,
                kind: "openai_compat".into(),
                base_url: "https://openrouter.ai/api/v1".into(),
                model: "google/gemini-3.8-flash-tts".into(),
                key_name: "openrouter".into(),
            },
            ProviderForm {
                capability: Capability::Transcribe,
                kind: "openai_compat".into(),
                base_url: "https://openrouter.ai/api/v1".into(),
                model: "openai/whisper-large-v3".into(),
                key_name: "openrouter".into(),
            },
        ];
        f
    }

    /// The four tabs, each with a value that must not be mistaken for another tab's.
    fn tabs() -> Vec<(ConfigForm, &'static str)> {
        let es = strings(UiLanguage::Es);

        let mut listening = providers();
        listening.tab = ConfigTab::Listening;
        listening.listening = ListeningForm {
            mode: ListeningMode::Auto,
            silence_ms: "1500".into(),
            mic_gain: "9.5".into(),
            mic_device: Some("Rode NT-USB".into()),
        };

        let mut profiles = providers();
        profiles.tab = ConfigTab::Profiles;
        profiles.active_profile = "compae".into();
        profiles.profiles = vec![
            ProfileForm {
                id: "compae".into(),
                name: "El Compae".into(),
                language: "es-ES".into(),
                prompt: "p".into(),
                voice_instructions: "v".into(),
                greeting: "g".into(),
                wake_word: "oye compae".into(),
                voices: vec![("gemini".into(), "Charon".into())],
            },
            ProfileForm {
                id: "meigo".into(),
                name: "O Meigo".into(),
                language: "es-ES".into(),
                prompt: "p".into(),
                voice_instructions: "v".into(),
                greeting: "g".into(),
                wake_word: "oye meigo".into(),
                voices: vec![("gemini".into(), "Puck".into())],
            },
        ];

        let mut server = providers();
        server.tab = ConfigTab::Server;
        server.server = ServerForm { bind: "0.0.0.0:7070".into(), lan: true, token: "-".repeat(70) };

        vec![
            (providers(), es.cfg_connector),
            (listening, es.cfg_mode),
            (profiles, es.cfg_wake_word),
            (server, es.cfg_bind),
        ]
    }

    /// The rect of the row whose label is `label`, off the drawn buffer: a label is drawn at the
    /// left of its row, on the label column, so its `y` is the row's `y`.
    fn row_at(term: &Terminal<TestBackend>, label: &str) -> Rect {
        let buffer = term.backend().buffer();
        let area = buffer.area;
        for y in area.top()..area.bottom() {
            let text: String =
                (area.left()..area.right()).map(|x| buffer[(x, y)].symbol().to_string()).collect();
            if let Some(at) = text.find(label) {
                return Rect::new(area.left() + at as u16, y, label.chars().count() as u16, 1);
            }
        }
        panic!("no row labelled {label:?} in the drawn buffer");
    }

    #[test]
    fn every_tab_draws_its_title_its_rows_and_the_help_line() {
        // Every row of every tab, at a width that holds all of them, in both languages.
        let mut all: Vec<&'static str> = Vec::new();
        for lang in [UiLanguage::Es, UiLanguage::En] {
            let s = strings(lang);
            assert!(!s.cfg_title.is_empty());
            for (f, _) in tabs() {
                let (text, _, _) = rendered(&f, lang, 120, 40);
                assert!(text.contains(s.cfg_title), "the screen's name is missing in {lang:?}:\n{text}");
                assert!(text.contains(ConfigForm::tab_title(f.tab, s)), "no tab title for {:?}", f.tab);
                assert!(text.contains(s.cfg_test), "the Test button is missing:\n{text}");
                assert!(text.contains(s.cfg_save), "the Save button is missing:\n{text}");
                assert!(text.contains(s.cfg_help), "the help line is missing:\n{text}");
                for row in f.rows() {
                    assert!(text.contains(row.label), "the label {:?} is not drawn:\n{text}", row.label);
                    assert!(text.contains(&row.value), "the value {:?} is not drawn:\n{text}", row.value);
                    if !all.contains(&row.label) {
                        all.push(row.label);
                    }
                }
            }
        }
        // The four tabs together are what makes every label in the table reachable, so a label that
        // no tab can draw is a label nothing checks.
        let es = strings(UiLanguage::Es);
        for label in [
            es.cfg_connector,
            es.cfg_base_url,
            es.cfg_model,
            es.cfg_key_name,
            es.cfg_mode,
            es.cfg_silence_ms,
            es.cfg_mic_gain,
            es.cfg_mic_device,
            es.cfg_name,
            es.cfg_language,
            es.cfg_prompt,
            es.cfg_voice_instructions,
            es.cfg_greeting,
            es.cfg_wake_word,
            es.cfg_voices,
            es.cfg_bind,
            es.cfg_lan,
            es.cfg_save,
        ] {
            assert!(all.contains(&label), "no tab draws the {label:?} row");
        }
    }

    #[test]
    fn the_focused_row_is_marked_and_only_it_is() {
        let mut f = providers();
        f.field = 1;
        let (text, _, term) = rendered(&f, UiLanguage::Es, 120, 40);
        let row = row_at(&term, strings(UiLanguage::Es).cfg_base_url);
        let focused = term.backend().buffer()[(row.x, row.y)].style().bg;
        assert!(focused.is_some(), "the focused row is drawn on a bar:\n{text}");
        let other = row_at(&term, strings(UiLanguage::Es).cfg_model);
        assert_ne!(term.backend().buffer()[(other.x, other.y)].style().bg, focused);
    }

    #[test]
    fn the_active_profile_is_marked_in_the_list() {
        let mut f = providers();
        f.tab = ConfigTab::Profiles;
        f.active_profile = "meigo".into();
        f.profiles = vec![
            ProfileForm { id: "compae".into(), name: "El Compae".into(), ..ProfileForm::default() },
            ProfileForm { id: "meigo".into(), name: "O Meigo".into(), ..ProfileForm::default() },
        ];
        let (text, _, _) = rendered(&f, UiLanguage::Es, 120, 40);
        assert!(text.contains("◉ O Meigo") || text.contains("● O Meigo"), "the active profile is marked:\n{text}");
        assert!(!text.contains("◉ El Compae") && !text.contains("● El Compae"), "and only it is:\n{text}");
    }

    #[test]
    fn the_edited_row_shows_its_value_and_the_cursor_line() {
        let mut f = providers();
        f.field = 1;
        f.editing = Some(String::new());
        let (text, _, _) = rendered(&f, UiLanguage::Es, 120, 40);
        assert!(text.contains("https://openrouter.ai/api/v1")
            || text.contains("https://openrouter.ai/api/v"), "the value stays on screen while it is edited:\n{text}");
        assert!(text.contains('│'), "the cursor is drawn:\n{text}");
        assert!(text.contains(strings(UiLanguage::Es).cfg_help_editing), "and the help line says what Enter does:\n{text}");

        f.editing = Some("con".into());
        let (text, _, _) = rendered(&f, UiLanguage::Es, 120, 40);
        assert!(text.contains("con"), "what has been typed is on screen:\n{text}");
    }

    #[test]
    fn the_notice_line_is_drawn_and_carries_the_notice_kind() {
        for (kind, notice) in [
            (NoticeKind::Warning, "número no válido"),
            (NoticeKind::Error, "no se pudo escribir el archivo"),
            (NoticeKind::Ok, "guardado"),
        ] {
            let mut f = providers();
            f.notice = Some((kind, notice.into()));
            let (text, _, _) = rendered(&f, UiLanguage::Es, 120, 40);
            assert!(text.contains(notice), "the notice is missing:\n{text}");
        }
        // With nothing to say, the notice row is still a row: the fixed layout is what keeps the
        // body from moving, and the rows below it are the same rows either way.
        let bare = providers();
        let with_notice = {
            let mut f = providers();
            f.notice = Some((NoticeKind::Warning, "x".into()));
            f
        };
        let a = rendered(&bare, UiLanguage::Es, 120, 40).0;
        let b = rendered(&with_notice, UiLanguage::Es, 120, 40).0;
        assert!(a.contains(strings(UiLanguage::Es).cfg_help) && b.contains(strings(UiLanguage::Es).cfg_help));
        assert!(a.contains(strings(UiLanguage::Es).cfg_connector) && b.contains(strings(UiLanguage::Es).cfg_connector));
        // The two buffers are the same size, so an unchanged row is at an unchanged offset: the
        // notice took a row of its own rather than pushing the body down.
        let at = |text: &str, needle: &str| text.find(needle).unwrap();
        assert_eq!(
            at(&a, strings(UiLanguage::Es).cfg_connector) / 120,
            at(&b, strings(UiLanguage::Es).cfg_connector) / 120,
            "the first row is on the same line with and without a notice"
        );
    }

    #[test]
    fn every_clickable_area_sits_on_its_label() {
        let f = providers();
        let (_, hits, term) = rendered(&f, UiLanguage::Es, 120, 40);
        let labels: Vec<String> = hits.iter().map(|h| text_at(&term, h.area)).collect();
        // The screen's own controls, in the order `draw_form` reports them. The four tab labels are
        // the ones `ConfigForm::tab_title` names; the two buttons are the two last hits.
        let es = strings(UiLanguage::Es);
        assert_eq!(labels[0], es.cfg_providers);
        assert_eq!(labels[1], es.cfg_listening);
        assert_eq!(labels[2], es.cfg_profiles);
        assert_eq!(labels[3], es.cfg_server);
        assert_eq!(labels[hits.len() - 2], es.cfg_test);
        assert_eq!(labels[hits.len() - 1], es.cfg_save);
        // And the row hits are one per drawn row, on the row they name.
        let rows = f.rows();
        assert_eq!(hits.len() - 6, rows.len(), "every row has exactly one hit: {} hits, {} rows", hits.len(), rows.len());
    }

    #[test]
    fn a_value_too_long_for_its_column_is_cut_and_never_spills() {
        let mut f = providers();
        // "Instrucciones de voz" is the longest label there is, and a 70-character base URL cannot
        // fit beside it on a 60-column terminal.
        f.providers[0].base_url = "x".repeat(70);
        f.tab = ConfigTab::Listening;
        f.field = 0;
        let (_, hits, term) = rendered(&f, UiLanguage::Es, 60, 20);
        let row = hits.iter().find(|h| h.key == KeyInput::Enter && h.area.y > 3).expect("a row hit");
        let drawn = text_at(&term, row.area);
        assert!(!drawn.contains('x'), "the URL belongs to the Providers tab, not this row: {drawn:?}");

        f.tab = ConfigTab::Providers;
        f.field = 1;
        let (_, hits, term) = rendered(&f, UiLanguage::Es, 60, 20);
        let row = hits.iter().find(|h| h.key == KeyInput::Enter && h.area.y > 3).expect("a row hit");
        let drawn = text_at(&term, row.area);
        assert!(drawn.chars().all(|c| c == 'x' || c == ' '), "the value fills its column and stops: {drawn:?}");
        assert!(
            row.area.right() <= 60,
            "the value's column ends at the frame: {:?}",
            row.area
        );
    }

    #[test]
    fn every_tab_renders_without_panicking_near_the_layout_minimum() {
        for (f, _) in tabs() {
            for (w, h) in [(1, 1), (20, 5), (40, 12), (60, 20), (80, 24)] {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                term.draw(|frame| draw_form(frame, &f, strings(UiLanguage::Es), 0))
                    .unwrap_or_else(|e| panic!("{:?} failed to draw on {w}x{h}: {e}", f.tab));
            }
        }
        // The screen has nothing to draw on a frame this small, and says so by returning no hits
        // rather than by reporting a rectangle nobody can see.
        let mut term = Terminal::new(TestBackend::new(1, 1)).unwrap();
        let mut hits = vec![Hit { area: Rect::ZERO, key: KeyInput::Enter }];
        term.draw(|frame| hits = draw_form(frame, &providers(), strings(UiLanguage::Es), 0)).unwrap();
        assert_eq!(hits, vec![Hit { area: Rect::ZERO, key: KeyInput::Enter }], "a 1x1 frame has no clickable area");
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p tipsy-ui --lib render_config`

Expected: FAIL — `draw_form` does not exist yet:

```
error[E0433]: failed to resolve: use of undeclared crate or module `render_config`
error[E0425]: cannot find function `draw_form` in this scope
error[E0599]: no method named `rows` found for struct `ConfigForm` in the current scope
```

  (The last one disappears with Task 3's implementation in place; if Task 3 is already committed, the first two are the whole failure.)

- [ ] **Step 3: Implement `draw_form`**

  Write `crates/tipsy-ui/src/render_config.rs` so that everything above is true, and nothing else. The shape, and the reasons for the three decisions that are not mechanical:

```rust
//! The configuration screen: the four tabs of `ConfigForm`, drawn from `ConfigForm::rows`.
//!
//! One row list feeds both the drawing and the click areas, so what is on screen and what a click
//! can reach cannot disagree. The screen is drawn as a whole frame (spec §9: full screen), and every
//! rectangle is clipped before it is used, so a terminal near the layout's minimum stays readable
//! instead of overlapping itself.

use ratatui::Frame;
use ratatui::layout::Constraint::{Length, Min, Percentage};
use ratatui::layout::{Alignment, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::config_form::{ConfigForm, ConfigTab, FormRow, NoticeKind, SERVER_ROW_SAVE, VOICE_LABEL_ROW};
use crate::state::KeyInput;
use crate::strings::Strings;

const CYAN: Color = Color::Rgb(0, 210, 255);
const DIM: Color = Color::Rgb(0, 95, 125);
const GOLD: Color = Color::Rgb(255, 190, 60);
const BAR: Color = Color::Rgb(0, 30, 40);
/// The `F` keys the screen's tabs answer to: the HUD already spends `F7` and `F8` on chat/network,
/// so the screen's four tabs take the four after them.
const TAB_KEYS: [u8; 4] = [7, 8, 9, 10];
const SAVE_KEY_F: u8 = 4;
const TEST_KEY_F: u8 = 2;
/// The label column: a third of the body, wide enough for "Instrucciones de voz" and narrow enough
/// that the value column still has room on a small terminal.
const LABEL_MIN: u16 = 12;
const LABEL_MAX: u16 = 26;

pub fn draw_form(frame: &mut Frame, form: &ConfigForm, strings: &Strings, elapsed_ms: u64) -> Vec<Hit> {
    let [title, tabs, notice, body, help] =
        Layout::vertical([Length(1), Length(1), Length(1), Min(3), Length(1)]).areas(frame.area());
    // The title row (the screen's name, `cfg_dirty` while `form.dirty`), the tab row (the four
    // clickable labels), the notice row (`form.notice`, one row whatever it says), the body (the
    // rows of `form.rows()`) and the help line.
}
```

  The implementation is mechanical and the tests above pin all of it. The three decisions worth writing down in the code:

- **Column geometry**: `label_w = (body.width / 3).clamp(LABEL_MIN, LABEL_MAX)`, but never more than `body.width.saturating_sub(4)` (a value column narrower than four cells shows nothing worth showing); the value column is `body.x + label_w` to `body.right()`. A row's `hit` is a one-cell-high `Rect` inside that column, clipped with `width.min(area.right().saturating_sub(x))`, and a row whose `y` is at or past `body.bottom()` is not drawn and not reported. The help line is `help.x + 1 .. help.right()`, so it has `area.width - 1` columns — at 120 columns that is exactly the 119 `cfg_help` takes, which is why the Step 1 fixtures use 120.
- **The layout, row by row** (this is the geometry Task 4's tests read: a row's `y` is `3 + its index in the row list`, so the first row is on line 3 and a hit whose `y > 3` is not that first row): line 0 is the title (` cfg_title ` bold cyan, then `cfg_dirty` in gold when the form is dirty), line 1 is the tab row (the four labels, one space between them, the active tab on a cyan bar, the row starting one column in), line 2 is the notice (`""` when there is none, otherwise the notice's text coloured by its `NoticeKind`: `Ok` green, `Warning` gold, `Error` red), lines 3.. carry the rows (the label in the left column, dim; the value in the right, white, or on a cyan bar when the row is focused), and the last line is the help (`cfg_help`, or `cfg_help` and `cfg_help_editing` together while a row is being edited). A row list longer than the body is cut at the last row the body holds: the rows that do not fit are not drawn and report no hit, so a click cannot reach a row nobody can see.
- **Cutting**: `fn cut(text: &str, width: usize) -> String` walks `char_indices`, sums `Span::from(c).width()` **from the left** for a plain value and **from the right** for the value of the row being edited (the cursor is at the end of the buffer), always keeping at least one character, and stops at `width`. `render.rs:281-293` is the same rule and is the place to copy it from.
- **Hits**: the four tab labels are drawn as one `Line` of `Span`s and measured with `Span::width()` exactly as `render.rs:143-149` measures the chat/network tabs; the row hits come from the same loop that draws the rows; Test is drawn on the `cfg_test` label's own rect and Save on the `cfg_save` row's rect, so their areas are the row list's rectangles rather than a second computation. The returned order is `[tabs…, rows…, test, save]` (**no mode and no ⚙ here**: those two belong to the HUD's header, Task 5, which owns their hits), and the test above reads it in that order.
- **Nothing is drawn outside the frame, and no line may run past its own end.** Every `Span` this file builds is cut to the rectangle it is drawn in — including the tab row, the title row and the help line, which are cut to `area.width` — because ratatui's `Paragraph` drops the overflow rather than wrapping it, and a line wider than the frame would spill into the cells a neighbouring widget is about to write. The two assertions in `every_tab_draws_its_title_its_rows_and_the_help_line` hold at 120 columns for that reason, and the row test in `a_value_too_long_for_its_column_is_cut_and_never_spills` is what pins the cutting itself.

  Two constants the test module imports must be exported by `config_form.rs` in this step, and Task 3's Step 3 mentions them: `pub const SERVER_ROW_SAVE: usize = 2;` and `pub const VOICE_LABEL_ROW: usize = ROW_VOICES;` — Task 4 must not re-derive the row indices the tests name.

- [ ] **Step 4: Run them and watch them pass**

Run: `cargo test -p tipsy-ui --lib render_config`

Expected: PASS, 8 tests:

```
running 8 tests
test render_config::tests::a_value_too_long_for_its_column_is_cut_and_never_spills ... ok
test render_config::tests::every_clickable_area_sits_on_its_label ... ok
test render_config::tests::every_tab_draws_its_title_its_rows_and_the_help_line ... ok
test render_config::tests::every_tab_renders_without_panicking_near_the_layout_minimum ... ok
test render_config::tests::the_active_profile_is_marked_in_the_list ... ok
test render_config::tests::the_edited_row_shows_its_value_and_the_cursor_line ... ok
test render_config::tests::the_focused_row_is_marked_and_only_it_is ... ok
test render_config::tests::the_notice_line_is_drawn_and_carries_the_notice_kind ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

- [ ] **Step 5: Run the whole crate, because this file may not move the HUD**

Run: `cargo test -p tipsy-ui`

Expected: all green — the 17 reducer tests, these 8, the existing `state` tests, and `crates/tipsy-ui/tests/render.rs`'s eight tests **unchanged and passing**: this task touches no line of `render.rs`, so the frozen HUD assertions are not in play yet. Then `cargo clippy -p tipsy-ui --all-targets` with no new warning.

- [ ] **Step 6: Commit**

```bash
git add crates/tipsy-ui/src/lib.rs crates/tipsy-ui/src/render_config.rs crates/tipsy-ui/src/config_form.rs
git commit -m "feat(ui): render the four configuration tabs

Providers, Listening, Profiles and Server are drawn from ConfigForm::rows, and
their click areas are that same list's rectangles, so what is on screen and what
a click reaches cannot disagree. The focused row, the edit cursor, the notice
line and the help line come with them, and a value too long for its column is
cut instead of spilling into its neighbour.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Opening the screen and the header controls `[small]`

**Files:**
- Modify: `crates/tipsy-ui/src/state.rs:66-301` — `UiAction::OpenConfig`, the `config_form` field, and the screen's key routing in `on_key`
- Modify: `crates/tipsy-ui/src/render.rs:54-97` — the header's mode indicator and ⚙ button with their `Hit`s, and the dispatch to `render_config::draw_form`
- Modify: `crates/tipsy-compadre/src/client.rs:328-342` — `key_input`: one arm for `KeyModifiers::CONTROL`, mapping a control chord to `KeyInput::Char(c & 0x1f)`
- Modify: `crates/tipsy-compadre/src/client.rs:149-154` — `on_action`: `UiAction::OpenConfig` loads the configuration and opens the screen
- Test: `crates/tipsy-ui/tests/render.rs` — five new tests appended; **the eight existing tests in the file do not change by one character**
- Test: `crates/tipsy-ui/src/state.rs` — three new tests inside the existing `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes (read from the committed code, not recalled):
  - `pub fn draw(frame: &mut Frame, state: &UiState, clock: &ClockView, title: &str) -> Vec<Hit>` — `render.rs:54-75`. The layout is `[header(1), body(Min 10), input(3), help(1)]`; `draw_header` is `render.rs:78-97` and takes `(frame, state, area, title)`.
  - `pub struct Hit { pub area: Rect, pub key: KeyInput }` — `render.rs:48-52`; `render.rs::draw_left` returns exactly three, in the order chat, network, clear (`:146-150`).
  - `pub fn UiState::on_key(&mut self, key: KeyInput) -> Vec<UiAction>` — `state.rs:238-301`; `KeyInput` is `state.rs:52-64`; `KeyInput::F(9)` is unbound today and falls through `KeyInput::F(_) => Vec::new()` (`:299`).
  - `pub enum UiAction { Send(Command), Quit, Replay(usize) }` — `state.rs:66-72`. `client.rs:151` consumes it: `for action in self.state.on_key(key) { quit |= self.on_action(action, &to_server); }`.
  - `pub fn draw_frame(terminal: &mut ratatui::DefaultTerminal, state: &UiState) -> Result<Vec<Hit>, String>` — `client.rs:308-318`, which is where the new hits reach `hit_at` (`:321-325`) and become keys on the next frame.
  - `fn key_input(key: &KeyEvent) -> Option<KeyInput>` — `client.rs:328-342`; every `KeyCode::Char(c)` becomes `KeyInput::Char(c)` whatever its modifiers, so `Ctrl+S` arrives as `Char('\u{13}')` and nothing in `tipsy-ui` needs a modifier. `use crossterm::event::KeyModifiers` is already in scope through `client.rs`'s `event` imports.
- Consumes (from Tasks 3 and 4):
  - `pub fn ConfigForm::key(&mut self, key: KeyInput) -> KeyOutcome` with `pub struct KeyOutcome { pub action: Option<ConfigAction>, pub effects: Vec<ConfigEffect> }`, `pub fn ConfigForm::apply(&mut self, ConfigAction) -> Vec<ConfigEffect>`, `pub fn ConfigForm::default()`, `pub enum ConfigEffect { None, Save, Test(Capability), RegenerateToken }` — `config_form.rs`.
  - `pub fn draw_form(frame: &mut Frame, form: &ConfigForm, strings: &Strings, elapsed_ms: u64) -> Vec<Hit>` — `render_config.rs`, returning `[tabs…, rows…, test, save]`.
- Produces:
  - `UiAction::OpenConfig` — one new variant, and the only one this task adds. `Save`, `Test` and `RegenerateToken` are **not** actions: Task 6 owns the client side of them and Task 7 owns the probe, so this task drops them (`ConfigEffect::Save` from a `Ctrl+S` the client cannot yet honour must not be silently swallowed *and* must not be invented here; Step 4 shows the one line that records them in the notice instead).
  - `pub struct UiState { …, pub config_form: Option<ConfigForm> }` — `Some` while the configuration screen is open. **`UiState` derives nothing** (`state.rs:74`), so no other derive changes.
  - The header contract: `draw` returns, in order, `[mode, ⚙, chat, network, clear]`, where `mode`'s area is the listening-mode indicator's label and `⚙`'s area is the gear button's, both on the header row, and both are `KeyInput::F(9)`; the screen's hits follow when the screen is open, so `render_config`'s own tests stay valid.
  - The routing contract: while `config_form.is_some()`, `on_key` sends **every** key to `ConfigForm::key` except `F(9)`, which closes the screen by the same route `Esc` uses, and `Esc` closes the screen instead of quitting. `Tab` stops opening the microphone, `Char` stops filling the input box, and `Enter` stops sending a question while the screen is up.
  - The small-terminal contract: the header's spans are clipped to the header row and no rectangle is ever built outside the frame, so `tiny_terminals_do_not_panic` keeps passing unchanged and the screen degrades the same way `render_config` already does.
  - `pub fn UiState::show_config_form(&mut self, form: ConfigForm)` — what Task 6 calls once `Config` ⇄ `ConfigForm` exists; the `UiAction::OpenConfig` the screen raises is its request.

**What this task decides, and why**

- **Two header controls, one key each.** `F9` opens the screen, and the ⚙ button *is* `F9` — the same key, reported as one `Hit` on the button's own rectangle, so the keyboard and the mouse go through one code path (spec §9's keyboard/mouse rule). The mode indicator reports `F9` too: it is the same "eye on the configuration" gesture in the header, and giving it a second key would invent a binding §9 does not name.
- **The mode indicator is the listening mode's own word plus the mic, not a status.** §9 says the header shows the listening mode (`push`, `auto`, `hands free`) "red while the mic is open"; `draw_header` already colours a red `strings.listening` for an open mic, and that text is the *status*, not the mode. The indicator therefore draws `format!(" {} ", mode_word)` from `mode_push`/`mode_auto`/`mode_hands_free`, in `Color::Red` when `state.mic_open` and `DIM` otherwise: the word says which mode is configured, the colour says whether the microphone is listening. In `hands_free` the mode stays active after a turn, so the mode word keeps saying so while the red goes out.
- **The title comes first and the status stays.** The frozen tests want `COMPAE · EL ASISTENTE DE GARRAFÓN`, `Escuchando` and `⚠` all on the header row, so the row is `[badge, mode, status, ⚙]` with the status taking whatever is left, and the gear is drawn last, at the right edge, with its rectangle measured from the right the same way `render.rs:149` measures the clear button. Nothing moves to a second row: the header is `Length(1)` and always will be, because `render.rs:55`'s layout is part of the frozen HUD.
- **A frame too small for the header draws no header hit.** At 1×1 the header row is clipped, `Span::width()` is not consulted for a rectangle that does not exist, and `draw` returns only the hits that fit: `[mode, ⚙]` are reported only when their rectangles are non-empty, exactly like `draw_left`'s rows are clipped today.
- **The screen replaces the HUD, and the HUD's hits are not reported while it is up.** `draw` dispatches to `draw_form` and returns the header's two hits followed by the screen's; the chat/network/clear hits belong to a screen that is not on the frame any more, and reporting them would make a click on the screen's tab row clear the conversation. `UiState` keeps its log, input and selection untouched while the screen is open, so `Esc` returns to exactly the HUD the user left.
- **`F9` closes as well as opens.** §9 names one key for the screen; a modal with a one-way door on that key would be a trap, and `Esc` is the documented way out — so `F9` toggles, `Esc` closes, and both are tested.
- **`Ctrl+S` and `Ctrl+R` are recognised in `client.rs`, and the effects are honest about what they cannot do yet.** `key_input` gains one arm (`KeyModifiers::CONTROL` + `KeyCode::Char(c)` → the control character `c & 0x1f`), because the mapping from a terminal key to the HUD's vocabulary is `client.rs`'s job and nowhere else: `Ctrl+S` is `'\u{13}'` (`ConfigForm::SAVE_KEY`) and `Ctrl+R` is `'\u{12}'` (`REGENERATE_KEY`), which Task 6 turns into the save and the regenerated token. The `ConfigEffect` a key raises is handled in `UiState::config_key`, which is what Task 6 extends; this task renders the effect as a notice line reading the screen's own word for it (`cfg_save`, `cfg_token`), so a key that is wired but not yet implemented says so instead of doing nothing.

- [ ] **Step 1: The failing tests**

  **1a. `crates/tipsy-ui/tests/render.rs`** — append these four tests. The eight tests already in the file are the frozen HUD specification and are **not** touched:

```rust
#[test]
fn f9_opens_the_configuration_screen_and_esc_closes_it() {
    let mut s = state(UiLanguage::Es);
    s.on_key(KeyInput::F(9));
    let (text, _, _) = render(&s);
    assert!(text.contains("CONFIGURACIÓN"), "F9 opens the screen:\n{text}");
    for needle in ["Proveedores", "Escucha", "Perfiles", "Servidor", "Conector", "Guardar"] {
        assert!(text.contains(needle), "the screen is missing {needle:?}:\n{text}");
    }
    // Esc closes the screen instead of quitting the HUD.
    let actions = s.on_key(KeyInput::Esc);
    assert_eq!(actions, Vec::new(), "Esc belongs to the screen while it is open");
    let (text, _, _) = render(&s);
    assert!(text.contains("NÚCLEO"), "the HUD is back:\n{text}");
    assert!(text.contains("PREGUNTA"));
    assert!(!text.contains("Proveedores"));

    // And F9 closes it too, so the one key §9 names is not a one-way door.
    s.on_key(KeyInput::F(9));
    assert!(render(&s).0.contains("CONFIGURACIÓN"));
    s.on_key(KeyInput::F(9));
    assert!(!render(&s).0.contains("CONFIGURACIÓN"));
}

#[test]
fn the_screen_swallows_the_hud_keys_while_it_is_open() {
    let mut s = state(UiLanguage::Es);
    s.on_key(KeyInput::F(9));
    assert_eq!(s.on_key(KeyInput::Tab), Vec::new(), "Tab must not open the microphone behind the screen");
    assert!(!s.mic_open);
    for c in "hola".chars() {
        s.on_key(KeyInput::Char(c));
    }
    assert!(s.input.is_empty(), "typing in the screen does not fill the question box");
    assert_eq!(s.on_key(KeyInput::Enter), Vec::new(), "and Enter does not send a question");
    assert!(s.log.is_empty());
    // The microphone itself never moved, so closing the screen leaves the HUD as it was.
    s.on_key(KeyInput::Esc);
    assert!(!s.mic_open);
}

#[test]
fn the_header_shows_the_gear_and_the_listening_mode() {
    let mut s = state(UiLanguage::Es);
    let (text, hits, term) = render(&s);
    assert!(text.contains("⚙"), "the gear button is in the header:\n{text}");
    assert_eq!(text_at(&term, hits[0].area), " push ", "the mode is named in its own area");

    s.apply(&Event::Welcome {
        session_id: 1,
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: UiLanguage::Es,
        listening_mode: ListeningMode::Auto,
        web_bundle: false,
    });
    let (_, hits, term) = render(&s);
    assert_eq!(text_at(&term, hits[0].area), " auto ", "the configured mode, not a fixed word");

    s.apply(&Event::Welcome {
        session_id: 1,
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: UiLanguage::En,
        listening_mode: ListeningMode::HandsFree,
        web_bundle: false,
    });
    let (_, hits, term) = render(&s);
    assert_eq!(text_at(&term, hits[0].area), " hands free ", "the English table's spelling");

    // … and the microphone is the colour, not the word: the mode stays on screen while it is open,
    // so an open mic cannot overwrite it.
    s.on_key(KeyInput::Tab);
    let (_, hits, term) = render(&s);
    assert_eq!(text_at(&term, hits[0].area), " hands free ", "the mode survives an open microphone");
    let area = hits[0].area;
    assert_eq!(term.backend().buffer()[(area.x + 1, area.y)].style().fg, Some(ratatui::style::Color::Red));
}

#[test]
fn the_gears_area_sits_on_its_label_and_opens_the_screen() {
    // The screen closed: the two header controls are the first two hits, before the HUD's three.
    let mut s = state(UiLanguage::Es);
    let (_, hits, term) = render(&s);
    assert_eq!(hits[1].key, KeyInput::F(9), "the gear answers F9");
    assert_eq!(text_at(&term, hits[1].area), " ⚙ ");
    assert_eq!(hits[0].key, KeyInput::F(9), "and so does the mode indicator");
    assert_eq!(text_at(&term, hits[0].area), " push ", "the indicator's area is its own word");
    assert_eq!(hits[2].key, KeyInput::F(7), "the HUD's own three hits follow, unchanged");

    // Clicking it is the same gesture as pressing it: the key the hit carries opens the screen.
    let (_, hits, _) = render(&s);
    s.on_key(hits[1].key);
    assert!(render(&s).0.contains("CONFIGURACIÓN"), "a click on the gear opens the screen");
    // And the two header controls are still where they were, so the screen can be closed the same
    // way it was opened.
    let (_, hits, term) = render(&s);
    assert_eq!(text_at(&term, hits[1].area), " ⚙ ");
    s.on_key(hits[1].key);
    assert!(!render(&s).0.contains("CONFIGURACIÓN"));
}

#[test]
fn the_screen_survives_a_terminal_too_small_for_it() {
    let mut s = state(UiLanguage::Es);
    s.on_key(KeyInput::F(9));
    for (w, h) in [(1, 1), (20, 5), (40, 12), (60, 20)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            draw(f, &s, &clock(), "x");
        })
        .unwrap();
    }
}
```

  **1b. `crates/tipsy-ui/src/state.rs`** — inside the existing `#[cfg(test)] mod tests`, after `an_idle_status_keeps_a_failure_visible_and_stops_the_spinner`:

```rust
    #[test]
    fn f9_opens_the_screen_on_a_default_form_and_closes_it_again() {
        let mut s = UiState::new();
        assert!(s.config_form.is_none());
        assert!(s.on_key(KeyInput::F(9)).is_empty(), "opening the screen sends nothing to the session");
        let form = s.config_form.as_ref().expect("the screen is open");
        assert_eq!(form.tab, crate::config_form::ConfigTab::Providers);
        assert_eq!(form.field, 0);
        assert!(form.editing.is_none());
        assert!(!form.dirty);

        // While it is open, nothing reaches the log or the microphone.
        s.on_key(KeyInput::Tab);
        s.on_key(KeyInput::Char('x'));
        s.on_key(KeyInput::Enter);
        assert!(!s.mic_open && s.input.is_empty() && s.log.is_empty(), "the screen has the keyboard");

        // Esc closes it instead of quitting, and F9 does the same.
        assert!(s.on_key(KeyInput::Esc).is_empty());
        assert!(s.config_form.is_none());
        assert_eq!(s.on_key(KeyInput::Esc), vec![UiAction::Quit], "and then Esc is the HUD's again");
        s.on_key(KeyInput::F(9));
        s.on_key(KeyInput::F(9));
        assert!(s.config_form.is_none(), "F9 is not a one-way door");
    }

    #[test]
    fn opening_the_screen_asks_the_client_for_the_configuration() {
        let mut s = UiState::new();
        // The client owns the file, so the only thing the HUD can do is ask for the conversion. The
        // action is the whole request; the form above is what is shown until it answers.
        assert_eq!(s.on_key(KeyInput::F(9)), vec![UiAction::OpenConfig]);
    }

    #[test]
    fn a_screen_effect_the_client_cannot_honour_yet_says_so() {
        let mut s = UiState::new();
        s.apply(&welcome(UiLanguage::En));
        s.on_key(KeyInput::F(9));
        // Ctrl+S reaches the form as the control character no text row wants.
        s.on_key(KeyInput::Char('\u{13}'));
        assert_eq!(
            s.notice.as_ref().map(|(text, _)| text.as_str()),
            Some("Save"),
            "a key that is wired but not implemented reports the effect instead of doing nothing"
        );
        assert!(s.config_form.as_ref().is_some_and(|f| f.dirty), "and the form knows there is something to save");
    }
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p tipsy-ui`

Expected: FAIL — the test targets do not compile:

```
error[E0609]: no field `config_form` on type `UiState`
error[E0599]: no variant or associated item named `OpenConfig` found for enum `UiAction`
error[E0599]: no method named `key` found for struct `ConfigForm` in the current scope
```

  and once the field and the variant exist, the four render tests fail on their assertions (`the gear button is in the header`) rather than at the compiler.

- [ ] **Step 3: `state.rs` — the form, the action and the routing**

  In `UiAction` (`state.rs:66-72`):

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    Send(Command),
    Quit,
    /// Replay the audio of the last completed reply (handled by the client, which kept it).
    Replay(usize),
    /// Open the configuration screen: the client loads `config.toml`, converts it to a
    /// [`ConfigForm`] and calls [`UiState::show_config_form`]. The HUD holds a default form until
    /// then, so the screen is usable the moment the key is pressed.
    OpenConfig,
}
```

  In `UiState`, after `pub voice_pos_ms: u64,` (`:96`):

```rust
    /// The configuration screen's form while it is open: `None` is the closed HUD. Kept as plain
    /// values (Task 3) so this crate still knows nothing about `config.toml`.
    pub config_form: Option<ConfigForm>,
```

  `UiState::new()` gains `config_form: None,` and a new import at the top of the file:

```rust
use crate::config_form::{ConfigEffect, ConfigForm};
```

  Two new methods, after `pub fn notify`:

```rust
    /// Opens the configuration screen, or closes it if it is already open.
    fn toggle_config_form(&mut self) -> Vec<UiAction> {
        match self.config_form.take() {
            // Closing leaves the HUD exactly as it was: the screen never touched the log, the input
            // or the microphone, and the half-typed edit goes with the form.
            Some(_) => Vec::new(),
            None => {
                self.config_form = Some(ConfigForm::default());
                vec![UiAction::OpenConfig]
            }
        }
    }

    /// Puts the loaded configuration on screen: the client's answer to [`UiAction::OpenConfig`].
    pub fn show_config_form(&mut self, form: ConfigForm) {
        self.config_form = Some(form);
    }

    /// The screen's own keys: the form translates and reduces them, and the effects it raises are
    /// what the client has to do about it.
    fn config_key(&mut self, key: KeyInput) -> Vec<UiAction> {
        let Some(form) = self.config_form.as_mut() else {
            return Vec::new();
        };
        let outcome = form.key(key);
        let actions = Vec::new();
        for effect in outcome.effects {
            match effect {
                ConfigEffect::None => {}
                // The client side of these three lands in Task 6 (save and regenerate) and Task 7
                // (the probe). Named with the screen's own words so the key is honest today, and set
                // as a notice rather than swallowed. `Capability` already speaks for itself.
                ConfigEffect::Save => self.notify(self.strings().cfg_save),
                ConfigEffect::Test(capability) => self.notify(capability.to_string()),
                ConfigEffect::RegenerateToken => self.notify(self.strings().cfg_token),
            }
        }
        actions
    }
```

  and `on_key` opens with the routing, before its existing `match key`:

```rust
    pub fn on_key(&mut self, key: KeyInput) -> Vec<UiAction> {
        if self.config_form.is_some() {
            // F9 and Esc are the screen's door: everything else — Tab, Enter, characters, F4 —
            // belongs to the form while it is on screen.
            return if matches!(key, KeyInput::Esc | KeyInput::F(9)) {
                self.toggle_config_form()
            } else {
                self.config_key(key)
            };
        }
        if key == KeyInput::F(9) {
            return self.toggle_config_form();
        }
        match key {
            // … every existing arm of `on_key`, unchanged …
        }
    }
```

  The existing arms do not change: `KeyInput::F(9)` was reaching `KeyInput::F(_) => Vec::new()` and is now handled above the match, and no other arm's meaning moves.

  **One detail the test in Step 1b pins**: `ConfigEffect::Test(capability)` notifies `capability.to_string()`, which is `chat`/`speech`/`transcription` through the `Display` impl `tipsy-protocol` already carries (`lib.rs:82-90`); `Save` notifies `cfg_save` and `RegenerateToken` notifies `cfg_token`, both from the screen's own table. "Save" and "Token" are the row labels of the things the client cannot yet do, which is the point of the notice: the words come from the table like every other screen word, and Task 6 and Task 7 replace these three notices with the real outcome (the written file, the regenerated token, the probe's result) without adding a string.

- [ ] **Step 4: `render.rs` — the header controls and the dispatch**

  `draw`'s first three lines become a dispatch, and the header returns its hits:

```rust
pub fn draw(frame: &mut Frame, state: &UiState, clock: &ClockView, title: &str) -> Vec<Hit> {
    let [header, body, input, help] = Layout::vertical([Length(1), Min(10), Length(3), Length(1)]).areas(frame.area());
    // The screen is full screen (spec §9): while it is up the HUD is not drawn at all, and its
    // clickable rows are not reported either — a click on the screen may not clear the conversation.
    let Some(form) = &state.config_form else {
        let [left, center, right] = Layout::horizontal([Percentage(LEFT_PERCENT), Min(30), Percentage(26)]).areas(body);
        let mut hits = draw_header(frame, state, header, title);
        let [clock_area, log] = Layout::vertical([Length(5), Min(3)]).areas(left);
        draw_clock(frame, state, clock, clock_area);
        hits.extend(draw_left(frame, state, log));
        let [rings, voice] = Layout::vertical([Min(8), Length(6)]).areas(center);
        draw_rings(frame, state, rings);
        draw_voice(frame, state, voice);
        draw_system(frame, state, right);
        draw_input(frame, state, input);
        if !help.is_empty() {
            frame.render_widget(Line::from(state.strings().help).fg(DIM), help);
        }
        return hits;
    };
    let mut hits = draw_header(frame, state, header, title);
    let _ = clock;
    hits.extend(crate::render_config::draw_form(frame, form, state.strings(), state.elapsed_ms));
    hits
}
```

  `draw_header` becomes the three-part row and returns its two hits:

```rust
/// Badge with the profile's title, the listening-mode indicator, whatever the assistant is doing,
/// and the ⚙ button — whose click is reported as a [`Hit`], like the HUD's other titles.
fn draw_header(frame: &mut Frame, state: &UiState, area: Rect, title: &str) -> Vec<Hit> {
    let strings = state.strings();
    let spin = SPINNER[(state.elapsed_ms / 150) as usize % SPINNER.len()];
    let mode = match state.listening_mode {
        ListeningMode::Push => strings.mode_push,
        ListeningMode::Auto => strings.mode_auto,
        ListeningMode::HandsFree => strings.mode_hands_free,
    };
    let status: Span = if state.mic_open {
        Span::from(strings.listening).fg(Color::Red).bold()
    } else if state.busy() {
        let what = match state.status {
            AssistantState::Transcribing => strings.transcribing,
            AssistantState::Speaking => strings.speaking,
            _ => strings.thinking,
        };
        Span::from(format!("{spin} {what} {}s", state.turn_ms / 1000)).fg(CYAN)
    } else if let Some((text, _)) = &state.notice {
        Span::from(format!("⚠ {text}")).fg(GOLD).bold()
    } else {
        Span::from(format!("● {}", strings.ready)).fg(Color::Green)
    };
    let badge = Span::from(format!(" ◉ {} ", title.to_uppercase())).fg(Color::Black).bg(CYAN).bold();
    // The mode is a word (§9: `push`, `auto`, `hands free`) and the microphone is the colour: red
    // while it is open, whatever the mode.
    let mode_span = Span::from(format!(" {mode} ")).fg(if state.mic_open { Color::Red } else { DIM });
    let gear = Span::from(" ⚙ ").fg(GOLD).bold();
    let gear_w = gear.width() as u16;
    // Measured from the right, the way `draw_left` measures its titles: ratatui does not say where
    // it laid a span out. The mode sits just before the gear, so neither can be drawn over the
    // other whatever the status says, and a frame too narrow for them reports no hit on the
    // truncated side.
    let gear_x = area.right().saturating_sub(gear_w);
    let mode_x = gear_x.saturating_sub(mode_span.width() as u16);
    // The status keeps whatever is left between the badge and the mode, and never more: it is the
    // one span of the four whose length the user's own data decides (`⚠ {notice}`).
    let badge_w = badge.width() as u16;
    let status_w = mode_x.saturating_sub(area.x.saturating_add(badge_w)).max(1) as usize;
    let status = trim(status.content.to_string(), status.style, status_w);
    let mode_hit = Rect::new(mode_x, area.y, (gear_x - mode_x).min(area.right().saturating_sub(mode_x)), 1);
    let gear_hit = Rect::new(gear_x, area.y, gear_w.min(area.right().saturating_sub(gear_x)), 1);
    let hits = [mode_hit, gear_hit]
        .into_iter()
        .filter(|r| !r.is_empty())
        .map(|area| Hit { area, key: KeyInput::F(9) })
        .collect();

    frame.render_widget(Line::from(vec![badge, Span::from("  "), status, mode_span, gear]), area);
    hits
}

/// `text` cut to `width` columns, keeping `style`. The status is the one header span whose length
/// the user's own data decides, and a status longer than its room would push the mode indicator and
/// the gear off the right edge of the row.
fn trim(text: String, style: Style, width: usize) -> Span<'static> {
    let mut cut = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = Span::from(c).width();
        if used + w > width {
            break;
        }
        used += w;
        cut.push(c);
    }
    Span::styled(cut, style)
}
```

  `use tipsy_protocol::{AssistantState, ListeningMode, UiLanguage};` — the `ListeningMode` import is new, and `Hit`/`Rect`/`KeyInput` are already in scope in this file.

  The third status span and the gear cannot overlap on any frame wide enough to draw both (`status` is drawn between them and its own rectangle is not reported), and on a frame that is too narrow the two hits are clipped rather than computed outside the buffer.

- [ ] **Step 5: `client.rs` — the key and the answer to the action**

  In `key_input` (`client.rs:328-342`), before the plain `Char` arm:

```rust
        // A modifier is not part of the HUD's vocabulary (`KeyInput` has no modifier at all), so a
        // control chord arrives as the control character it stands for: `Ctrl+S` and `Ctrl+R` are
        // `'\u{13}'` and `'\u{12}'`, which is the one class of `char` no text row wants
        // (`ConfigForm::SAVE_KEY`, `ConfigForm::REGENERATE_KEY`). Every other modified key is
        // dropped rather than typed into a row as a bare letter.
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let lower = c.to_ascii_lowercase();
            lower.is_ascii_lowercase().then(|| KeyInput::Char((lower as u8 & 0x1f) as char))
        }
        KeyCode::Char(c) => Some(KeyInput::Char(c)),
```

  (The `& 0x1f` is the ASCII control-character identity: `'s' & 0x1f` is `'\u{13}'`, `'r' & 0x1f` is `'\u{12}'`. Spelling them as `'\u{13}'` and `'\u{12}'` on the `config_form` side is what makes the two sides readable.)

  In `on_action` (`client.rs:149-154`'s consumer), extend the existing match with:

```rust
            // The screen asks for the configuration it shows: read it fresh (a hand-edited file is
            // seen the next time the screen opens) and hand it over. Task 6 fills the conversion in;
            // until then the default form the HUD already holds is what the screen shows.
            UiAction::OpenConfig => {}
```

  Task 6 replaces that arm's body with `self.state.show_config_form(config_to_form(&config))`; this task's job is that the action exists, is emitted once per opening, and that the screen is usable immediately from the default form the HUD holds.

- [ ] **Step 6: Run the tests and watch them pass**

Run: `cargo test -p tipsy-ui`

Expected: PASS — 14 tests in `tests/render.rs` (the eight frozen ones plus the six new), 18 in `config_form`, 8 in `render_config`, and 19 in `state` (the sixteen that were there plus three):

```
running 14 tests
test chat_log_shows_user_and_assistant_lines ... ok
test clickable_areas_sit_on_their_labels ... ok
test english_hud_uses_english_strings ... ok
test f9_opens_the_configuration_screen_and_esc_closes_it ... ok
test input_box_shows_typed_text_or_placeholder ... ok
test listening_shows_red_indicator_text ... ok
test network_tab_shows_chart_legend ... ok
test rings_fit_inside_their_panel_when_it_is_not_wide_enough_for_them ... ok
test spanish_hud_shows_every_panel ... ok
test the_gears_area_sits_on_its_label_and_opens_the_screen ... ok
test the_header_shows_the_gear_and_the_listening_mode ... ok
test the_screen_swallows_the_hud_keys_while_it_is_open ... ok
test the_screen_survives_a_terminal_too_small_for_it ... ok
test tiny_terminals_do_not_panic ... ok

test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
```

  The frozen eight are in that list **unmodified** — `git diff crates/tipsy-ui/tests/render.rs` shows only the appended tests, no changed line — and `clickable_areas_sit_on_their_labels` still reads `[(" CHAT ", F(7)), (" RED ", F(8)), (" ✗ Borrar ", F(4))]` because it filters to the HUD's own hits and the two new header hits come first.

  Then the workspace, because `client.rs` changed: `cargo test --workspace` (all green, including `tipsy-compadre`'s `key_input`-adjacent client tests) and `cargo clippy --workspace --all-targets` (no new warning).

- [ ] **Step 7: Commit**

```bash
git add crates/tipsy-ui/src/state.rs crates/tipsy-ui/src/render.rs crates/tipsy-ui/src/config_form.rs crates/tipsy-ui/tests/render.rs crates/tipsy-compadre/src/client.rs
git commit -m "feat(ui): open the configuration screen on F9 and from the gear

The header gains the listening-mode indicator and the gear button, both
answering F9, and the screen takes the whole frame and the keyboard while it is
open: Esc closes it instead of quitting, and Tab, Enter and characters no longer
reach the HUD behind it. The eight existing render tests pass unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Save and apply without restarting `[mid]`

**Files:**
- Create: `crates/tipsy-compadre/src/config/form.rs` — `form_of`, `config_from`, `provider_config`, `kind_name`/`kind_from`, the two number parsers, and their tests
- Modify: `crates/tipsy-compadre/src/config/mod.rs:3-7` — `pub mod form;` beside `model`, `profiles` and `store`
- Modify: `crates/tipsy-compadre/src/providers/mod.rs:152-157` — `ProviderSlot` directly above `Providers` (it is the handle to one)
- Modify: `crates/tipsy-compadre/src/brain.rs:20-25` — `#[derive(Clone)]` on `BrainConfig`; `:51-60` — `Brain.providers` becomes a `ProviderSlot`; `:62-70` — `Brain::new` takes one; `:69-70`, `:100`, `:118`, `:125-142`, `:145-159`, `:163-190`, `:267-316` — the stack is read once at the top of `run_turn` and passed down (the exact lines are listed in Step 7)
- Modify: `crates/tipsy-compadre/src/session.rs:29-36`, `:49-89` — `spawn_session`/`spawn_with_events` take a `ProviderSlot`, and `Session` is built around the configuration, the brain config and that slot; `:101-115` — the new fields; `:215-231` — `SetProfile` and `SetListeningMode` become real handlers
- Modify: `crates/tipsy-compadre/src/client.rs:13-25` (imports), `:35-54` (`run`), `:56-88` (`Client` and `new`), `:99-163` (`frame_loop`'s head and the `on_action` call), `:165-179` (`on_message` gets its Test arm in Task 7), `:240-267` (`on_action` plus the new `open_screen`, `save`, `regenerate_token`, `config_notice`, `on_config_effect`)
- Test: `crates/tipsy-compadre/src/config/form.rs` (its `#[cfg(test)] mod tests`, written in Step 1), `crates/tipsy-compadre/src/client.rs:357-421` (the existing `mod tests`), `crates/tipsy-compadre/tests/session.rs:99-135`, `:189-332`, `crates/tipsy-compadre/tests/brain.rs:111-115`
- **Not** modified: `crates/tipsy-compadre/src/config/store.rs`. Task 1 already gives this task `save`, `load_or_create` and `new_token`, and the review-focus test works precisely because `save` serialises before it writes and never renames (Step 9 relies on a read-only path failing `std::fs::write`).

**Interfaces:**
- Consumes (read from the committed code, not recalled):
  - `pub struct Config { pub active_profile: String, pub server: ServerConfig, pub listening: ListeningConfig, pub providers: ProvidersConfig, pub profiles: Vec<Profile> }`, `#[serde(default)]` — `config/model.rs:8-24`; `pub fn active(&self) -> &Profile` — `:158-166`; `pub fn with_builtin_profiles(self) -> Self` — `:169-176`
  - `pub struct ProviderConfig { pub kind: ProviderKind, pub base_url: String, pub model: String, pub key: String }` — `config/model.rs:74-82`; `pub enum ProviderKind { OpenaiCompat, Gemini, Elevenlabs }` with `#[serde(rename_all = "snake_case")]` — `:65-72`; `pub struct ListeningConfig { pub mode: ListeningMode, pub silence_ms: u32, pub mic_gain: f32, pub mic_device: Option<String> }` and `pub struct ServerConfig { pub bind: String, pub lan: bool, pub token: String }` — as Task 1 leaves them
  - `pub fn save(path: &Path, config: &Config) -> std::io::Result<()>` — `config/store.rs:20-26` (create the directory, serialise, then `std::fs::write`); `pub fn load_or_create(path: &Path) -> std::io::Result<LoadOutcome>`, `pub struct LoadOutcome { pub config: Config, pub warning: Option<String> }` — `:7-11`, `:30-57`; `pub fn new_token() -> std::io::Result<String>` — Task 1
  - `#[derive(Clone)] pub struct Providers { pub chat: Arc<dyn ChatProvider>, pub speech: Arc<dyn SpeechProvider>, pub transcribe: Arc<dyn TranscribeProvider> }` — `providers/mod.rs:152-157`
  - `pub fn build_providers(config: &ProvidersConfig, keys: &KeyResolver) -> Result<Providers, BuildError>` — `providers/mod.rs:223-241`; `pub enum BuildError { MissingKey { name, env }, UnsupportedCapability { capability, kind }, Key(KeyError) }` — `:159-170`
  - `pub struct Brain { providers: Providers, profile: TurnProfile, config: BrainConfig }`; `pub fn new(providers: Providers, profile: TurnProfile, config: BrainConfig) -> Self`; `pub async fn run_turn(&self, history: &mut Vec<ChatMessage>, input: TurnInput, out: UnboundedSender<ToClient>, cancel: CancellationToken)` — `brain.rs:20-70`; `pub struct BrainConfig { pub chat_timeout: Duration, pub speech_timeout: Duration, pub transcribe_timeout: Duration, pub retry_backoff: Duration }` — `:20-25` (no `Clone` today; Step 7 adds it)
  - `pub struct SessionEnds { pub to_server: UnboundedSender<ToServer>, pub from_server: UnboundedReceiver<ToClient>, pub events: UnboundedSender<ToClient> }` — `session.rs:38-45`; `pub fn spawn_session(config: Config, providers: Providers, brain_config: BrainConfig) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>)` — `:29-36`; `pub fn spawn_with_events(config: Config, providers: Providers, brain_config: BrainConfig) -> SessionEnds` — `:49-89`
  - `pub struct KeyResolver` with `pub fn system() -> Self` and `pub fn resolve(&self, name: &str) -> Result<Option<(String, KeySource)>, KeyError>` — `keys.rs:79-103`; `pub trait SecretStore { fn get(&self, …); fn set(&self, …); fn delete(&self, …) }` and `pub struct MemoryStore` — `:14-18`, `:47-62`
  - `pub enum UiAction { Send(Command), Quit, Replay(usize) }` — `tipsy-ui/src/state.rs:66-72`; `pub fn notify(&mut self, text: impl Into<String>)` — `:136-138`; `pub fn strings(&self) -> &'static Strings` — `:132-134`; `pub fn apply(&mut self, ev: &Event)` — `:165-236`
  - The `wiremock` style the connector tests already use: `Mock::given(method("POST")).and(path("…")).respond_with(ResponseTemplate::new(200)…).mount(&server).await` and `server.received_requests().await` — `tests/openai_compat.rs:29-56`; `ResponseTemplate::set_delay(Duration)` is unconditional in wiremock 0.6.5 (`src/response_template.rs:306`), so no feature flag is needed
- **Assumed from Tasks 3 and 5** (names and shapes this task cannot read yet; reconcile with their bodies before implementing — each one is used in exactly the places named here):
  - Task 3's `tipsy-ui/src/config_form.rs` re-exported at the crate root the way `state` and `render` are (`tipsy-ui/src/lib.rs:3-9`): `ConfigForm`, `ConfigAction`, `ConfigEffect`, `ConfigTab`, `NoticeKind`, `ProviderForm`, `ListeningForm`, `ServerForm`, `ProfileForm` — the field names and types exactly as "Interfaces this plan fixes" gives them, every field `pub`, and `NoticeKind::{Ok, Warning, Error}` for `notice: Option<(NoticeKind, String)>`
  - Task 3's `ProviderForm::kind` holds the serde spelling (`"openai_compat" | "gemini" | "elevenlabs"`); `ListeningForm::silence_ms` and `mic_gain` hold what the user typed, as `String`
  - Task 3's reducer: `ConfigForm::apply(ConfigAction::Save)` yields `ConfigEffect::Save` (the tests below set `dirty = true` first, so a reducer that refuses to save a clean form still passes), and `ConfigAction::Test`, `ConfigAction::RegenerateToken` yield `ConfigEffect::Test(Capability::Chat)` on the **first** Providers row and `ConfigEffect::RegenerateToken` respectively. Task 7 takes over the `Test` effect; Step 11 of this task leaves it as a match arm that does nothing yet.
  - Task 5's `tipsy-ui/src/state.rs`: `pub config_form: Option<ConfigForm>` on `UiState`, `pub fn open_config(&mut self, form: ConfigForm)`, and `UiAction::OpenConfig` / `UiAction::Config(ConfigAction)`. If Task 5 named any of these differently, this task's three call sites are the `open_screen` body, the `UiAction::Config` arm and the `UiAction::OpenConfig` arm — and no test below touches them except through `open_screen`.
- Produces:
  - `pub fn form_of(config: &Config) -> ConfigForm` — `config/form.rs`
  - `pub fn config_from(form: &ConfigForm) -> Result<Config, String>` — `config/form.rs`; the `String` is exactly what the screen's notice line shows when a save is refused, and it always names the row it came from
  - `pub fn provider_config(row: &ProviderForm) -> Result<ProviderConfig, String>` — `config/form.rs`; Task 7 converts the row under test with it, so a Test and a save can never read a row differently
  - `#[derive(Clone)] pub struct ProviderSlot(Arc<RwLock<Providers>>)` with `pub fn new(providers: Providers) -> Self`, `pub fn replace(&self, providers: Providers)`, `pub fn current(&self) -> Providers` — `providers/mod.rs`
  - `pub fn spawn_session(config: Config, providers: ProviderSlot, brain_config: BrainConfig) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>)` and `pub fn spawn_with_events(config: Config, providers: ProviderSlot, brain_config: BrainConfig) -> SessionEnds` — `session.rs`. **These two signatures are what make a rebuilt stack live**, and Task 9 and the future web server pass a slot the same way.
  - `pub fn Brain::new(providers: ProviderSlot, profile: TurnProfile, config: BrainConfig) -> Self` — `brain.rs`
  - Private to `session.rs`, and the behaviour Task 9 leans on: `fn set_profile(&mut self, id: String)`, `fn set_listening_mode(&mut self, mode: ListeningMode)` — both re-announce the session in a fresh `Event::Welcome`, which is the only event that carries the profile, the interface language and the mode
  - Private to `client.rs`: `fn new(config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot) -> Self`, `fn open_screen(&mut self)`, `fn save(&mut self, to_server: &UnboundedSender<ToServer>)`, `fn regenerate_token(&mut self)`, `fn config_notice(&mut self, kind: NoticeKind, text: impl Into<String>)`, `fn on_config_effect(&mut self, effect: ConfigEffect, to_server: &UnboundedSender<ToServer>, events: &UnboundedSender<ToClient>)`, `fn on_action(&mut self, action: UiAction, to_server: &UnboundedSender<ToServer>, events: &UnboundedSender<ToClient>) -> bool`

**What this task decides, and why**

- **The conversions live beside the model, not inside the client.** `config/form.rs` is where `Config` meets `ConfigForm`, and the round-trip test there is the thing that keeps the two shapes from drifting. The client only calls them.
- **The order of a save is build, write, apply — and that order is the review focus.** The connectors are built first, because `build_providers` refuses a key that is not there and a `config.toml` the app cannot start from is damage, not a saved edit (a fresh launch fails with exit code 2 — `main.rs:74-87`). The file is written second, through `store::save` and nothing else, which serialises before it writes: a refused write leaves the file byte for byte as it was. Only then does anything running change, so a failure at either earlier step leaves the file, the providers and the session untouched.
- **The providers are a slot, not a rebuilt session.** `ProviderSlot` is a handle to the stack, shared by the client and the session, read once at the top of every turn: a save swaps the connectors in place and the next turn answers with them. Rebuilding the session instead would drop the conversation; swapping the whole `Arc<Brain>` instead would have needed a trigger the fixed protocol does not carry.
- **The channel carries what it can carry.** `Command` is fixed by this plan to gain nothing but the Test pair, so a save tells the running session `SetListeningMode` and `SetProfile`. `SetListeningMode` re-announces the mode (that is the only way the header's indicator can change) and `SetProfile` re-derives the brain's `TurnProfile` from the configuration the session holds, so a profile switch really does change the prompt of the next turn. The *edited texts* of a profile and `mic_device` cannot travel that way: they are written and shown now and take effect at the next launch. That is the honest limit of "changes apply on save" with the protocol this plan fixes, and Step 5's profile test pins the half that does apply.
- **Every refusal has one shape.** `config_notice` puts the low-level truth in `form.notice`: the row that would not parse (`silence_ms: \`mil\` is not a whole number`), the missing key `BuildError`'s own text with `TIPSY_KEY_…` in it, or the config path and its OS error. The form carries no language of its own (`ConfigForm` has no `lang` field), so the client composes no copy it cannot translate — and the failure itself is what the user needs to read.

- [ ] **Step 1: Write the failing tests for the two conversions** — create `crates/tipsy-compadre/src/config/form.rs` with the module doc, the `use` lines and the test module below (no implementation yet), and declare the module in `crates/tipsy-compadre/src/config/mod.rs` (line 6, after `pub mod profiles;`):

```rust
pub mod form;
```

```rust
//! The two conversions between the configuration and the screen's form.
//!
//! `tipsy-ui` may not depend on this crate, so the screen edits plain values (`ConfigForm`) and the
//! native client owns both directions. The round trip is tested here, which is what keeps the two
//! shapes from drifting.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::{ListeningConfig, ProviderConfig, ProvidersConfig, ServerConfig};
    use tipsy_protocol::ListeningMode;

    /// A configuration with every kind of value the screen can change, so the round trip has
    /// something to lose: three connectors of three kinds, both `Option` shapes, a token, a user
    /// profile that overrides nothing.
    fn config() -> Config {
        let mut config = Config::default();
        config.active_profile = "mine".into();
        config.server = ServerConfig {
            bind: "0.0.0.0:7070".into(),
            lan: true,
            token: "0123456789abcdef0123456789abcdef".into(),
        };
        config.listening = ListeningConfig {
            mode: ListeningMode::HandsFree,
            silence_ms: 1500,
            mic_gain: 9.5,
            mic_device: Some("Rode NT-USB".into()),
        };
        config.providers = ProvidersConfig {
            chat: ProviderConfig {
                kind: ProviderKind::Gemini,
                base_url: "https://generativelanguage.test/v1beta".into(),
                model: "gemini-2.5-flash".into(),
                key: "google".into(),
            },
            speech: ProviderConfig {
                kind: ProviderKind::Elevenlabs,
                base_url: "https://api.elevenlabs.test".into(),
                model: "eleven_flash_v2_5".into(),
                key: "elevenlabs".into(),
            },
            transcribe: ProviderConfig {
                kind: ProviderKind::OpenaiCompat,
                base_url: "http://localhost:11434/v1".into(),
                model: "openai/whisper-large-v3".into(),
                key: String::new(),
            },
        };
        config.profiles.insert(
            0,
            Profile {
                id: "mine".into(),
                name: "Mi Compadre".into(),
                language: "es-ES".into(),
                prompt: "Habla como un compae.".into(),
                voice_instructions: "andaluz, tranquilo".into(),
                greeting: "¿Qué pasa, quillo?".into(),
                wake_word: "oye compae".into(),
                voices: BTreeMap::from([
                    ("gemini".to_string(), "Charon".to_string()),
                    ("elevenlabs".to_string(), "george".to_string()),
                ]),
            },
        );
        config
    }

    /// The contract that keeps the two shapes from drifting: what the screen opens on, saved
    /// unchanged, is the configuration that produced it — value for value, order for order.
    #[test]
    fn the_form_and_the_configuration_round_trip() {
        for config in [Config::default(), config()] {
            let form = form_of(&config);
            assert_eq!(config_from(&form).unwrap(), config, "a save must write the same file back");
        }
    }

    /// The three capabilities are always there, in the protocol's own order, spelled the way the
    /// file spells them: the rows the screen draws and the clicks that select them both come from
    /// this list.
    #[test]
    fn the_form_holds_the_three_capabilities_in_order() {
        let form = form_of(&Config::default());
        let capabilities: Vec<Capability> = form.providers.iter().map(|p| p.capability).collect();
        assert_eq!(capabilities, [Capability::Chat, Capability::Speech, Capability::Transcribe]);

        assert_eq!(form.providers[0].kind, "openai_compat");
        assert_eq!(form.providers[0].base_url, "https://openrouter.ai/api/v1");
        assert_eq!(form.providers[0].model, "google/gemini-2.5-flash");
        assert_eq!(form.providers[0].key_name, "openrouter");
        assert_eq!(form.listening.silence_ms, "1000");
        assert_eq!(form.listening.mic_gain, "12");
        assert_eq!(form.server.bind, "127.0.0.1:7070");
        assert!(form.server.token.is_empty(), "no token until the Server tab generates one");
        assert!(!form.dirty, "a form nobody has touched holds nothing unsaved");
        assert!(form.notice.is_none());
        assert_eq!(form.profiles.len(), Config::default().profiles.len());
    }

    /// `mic_device` is the form's one `Option`: absent means the system default and an empty string
    /// means a device with no name, which is a different and useless value.
    #[test]
    fn an_absent_mic_device_stays_absent_through_the_round_trip() {
        let mut config = config();
        config.listening.mic_device = None;
        let form = form_of(&config);
        assert_eq!(form.listening.mic_device, None);
        assert_eq!(config_from(&form).unwrap().listening.mic_device, None);

        config.listening.mic_device = Some(String::new());
        let form = form_of(&config);
        assert_eq!(form.listening.mic_device.as_deref(), Some(""));
        assert_eq!(config_from(&form).unwrap().listening.mic_device, Some(String::new()));
    }

    /// A save that cannot be built is refused *by the name of its row*: "invalid digit found in
    /// string" tells nobody what to fix, and this message is what the notice line shows.
    #[test]
    fn a_value_that_does_not_parse_is_refused_by_the_name_of_its_row() {
        let mut form = form_of(&Config::default());

        form.listening.silence_ms = "mil".into();
        let error = config_from(&form).unwrap_err();
        assert!(error.contains("silence_ms") && error.contains("mil"), "{error}");

        form.listening.silence_ms = "1000".into();
        form.listening.mic_gain = "doce".into();
        let error = config_from(&form).unwrap_err();
        assert!(error.contains("mic_gain") && error.contains("doce"), "{error}");

        form.listening.mic_gain = "12".into();
        form.providers[1].kind = "openia_compat".into();
        let error = config_from(&form).unwrap_err();
        assert!(error.contains("openia_compat"), "{error}");
    }

    /// The connector names the form holds are serde's own: the file, the form and the wire must not
    /// grow a second vocabulary.
    #[test]
    fn connector_names_are_the_serialised_ones() {
        for kind in [ProviderKind::OpenaiCompat, ProviderKind::Gemini, ProviderKind::Elevenlabs] {
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(kind_name(kind)));
            assert_eq!(kind_from(kind_name(kind)).unwrap(), kind);
        }
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --lib config::form`

Expected: FAIL — the module compiles to the point of the missing items and stops, e.g.

```
error[E0425]: cannot find function `form_of` in this scope
error[E0425]: cannot find function `config_from` in this scope
error[E0425]: cannot find function `kind_name` in this scope
error[E0425]: cannot find function `kind_from` in this scope
error[E0433]: failed to resolve: use of undeclared type `Capability`
```

(`Capability` is not yet imported at module level; it arrives with the implementation. No test reaches the harness yet.)

- [ ] **Step 3: Implement the two conversions** — append this to `crates/tipsy-compadre/src/config/form.rs`, above the test module:

```rust
use std::collections::BTreeMap;

use tipsy_protocol::{Capability, ListeningMode};
use tipsy_ui::{ConfigForm, ConfigTab, ListeningForm, ProfileForm, ProviderForm, ServerForm};

use super::model::{Config, ListeningConfig, Profile, ProviderConfig, ProviderKind, ProvidersConfig, ServerConfig};

/// The form the screen opens on: a plain-value copy of `config`, and nothing else.
pub fn form_of(config: &Config) -> ConfigForm {
    ConfigForm {
        tab: ConfigTab::Providers,
        field: 0,
        editing: None,
        active_profile: config.active_profile.clone(),
        // Always three, in `Capability` order: the rows the screen draws and the click areas that
        // select them both come from this list.
        providers: vec![
            provider_form(Capability::Chat, &config.providers.chat),
            provider_form(Capability::Speech, &config.providers.speech),
            provider_form(Capability::Transcribe, &config.providers.transcribe),
        ],
        listening: ListeningForm {
            mode: config.listening.mode,
            silence_ms: config.listening.silence_ms.to_string(),
            mic_gain: config.listening.mic_gain.to_string(),
            mic_device: config.listening.mic_device.clone(),
        },
        profiles: config.profiles.iter().map(profile_form).collect(),
        server: ServerForm {
            bind: config.server.bind.clone(),
            lan: config.server.lan,
            token: config.server.token.clone(),
        },
        notice: None,
        dirty: false,
    }
}

/// The configuration the form describes, or the reason it cannot be one.
///
/// Every message names the row it came from: this is what the screen's notice line shows when a
/// save is refused, and nobody can fix "invalid digit found in string" without knowing the row.
pub fn config_from(form: &ConfigForm) -> Result<Config, String> {
    let mut providers = ProvidersConfig::default();
    for row in &form.providers {
        let provider = provider_config(row)?;
        match row.capability {
            Capability::Chat => providers.chat = provider,
            Capability::Speech => providers.speech = provider,
            Capability::Transcribe => providers.transcribe = provider,
        }
    }
    let profiles = form.profiles.iter().map(profile_from).collect();
    Ok(Config {
        active_profile: form.active_profile.clone(),
        server: ServerConfig {
            bind: form.server.bind.clone(),
            lan: form.server.lan,
            token: form.server.token.clone(),
        },
        listening: ListeningConfig {
            mode: form.listening.mode,
            silence_ms: number("silence_ms", &form.listening.silence_ms)?,
            mic_gain: gain("mic_gain", &form.listening.mic_gain)?,
            mic_device: form.listening.mic_device.clone(),
        },
        providers,
        profiles,
    })
}

/// One row of the Providers tab as a connector configuration — the same conversion a save makes, so
/// a Test (Task 7) can never read a row differently from the save that follows it.
pub fn provider_config(row: &ProviderForm) -> Result<ProviderConfig, String> {
    Ok(ProviderConfig {
        kind: kind_from(&row.kind)?,
        base_url: row.base_url.clone(),
        model: row.model.clone(),
        key: row.key_name.clone(),
    })
}

fn provider_form(capability: Capability, config: &ProviderConfig) -> ProviderForm {
    ProviderForm {
        capability,
        kind: kind_name(config.kind).to_string(),
        base_url: config.base_url.clone(),
        model: config.model.clone(),
        key_name: config.key.clone(),
    }
}

fn profile_form(profile: &Profile) -> ProfileForm {
    ProfileForm {
        id: profile.id.clone(),
        name: profile.name.clone(),
        language: profile.language.clone(),
        prompt: profile.prompt.clone(),
        voice_instructions: profile.voice_instructions.clone(),
        greeting: profile.greeting.clone(),
        wake_word: profile.wake_word.clone(),
        // A `BTreeMap` iterates in key order, so the rows of the voices table come out sorted and go
        // back into a `BTreeMap` unchanged.
        voices: profile.voices.iter().map(|(connector, voice)| (connector.clone(), voice.clone())).collect(),
    }
}

fn profile_from(form: &ProfileForm) -> Profile {
    Profile {
        id: form.id.clone(),
        name: form.name.clone(),
        language: form.language.clone(),
        prompt: form.prompt.clone(),
        voice_instructions: form.voice_instructions.clone(),
        greeting: form.greeting.clone(),
        wake_word: form.wake_word.clone(),
        voices: form.voices.iter().cloned().collect::<BTreeMap<String, String>>(),
    }
}

/// The connector's name as the file and the form spell it (serde's own `snake_case`).
fn kind_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenaiCompat => "openai_compat",
        ProviderKind::Gemini => "gemini",
        ProviderKind::Elevenlabs => "elevenlabs",
    }
}

fn kind_from(name: &str) -> Result<ProviderKind, String> {
    match name {
        "openai_compat" => Ok(ProviderKind::OpenaiCompat),
        "gemini" => Ok(ProviderKind::Gemini),
        "elevenlabs" => Ok(ProviderKind::Elevenlabs),
        other => Err(format!("unknown connector `{other}`: openai_compat, gemini or elevenlabs")),
    }
}

fn number(row: &str, text: &str) -> Result<u32, String> {
    text.trim().parse().map_err(|_| format!("{row}: `{text}` is not a whole number"))
}

fn gain(row: &str, text: &str) -> Result<f32, String> {
    text.trim().parse().map_err(|_| format!("{row}: `{text}` is not a number"))
}
```

`mic_gain` goes through `f32::to_string` and back: Rust's `f32` `Display` is the shortest text that
parses back to the same value (`12.0` → `"12"`), which is why the round trip is exact.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --lib config::form`

Expected: PASS, 5 tests:

```
running 5 tests
test config::form::tests::a_value_that_does_not_parse_is_refused_by_the_name_of_its_row ... ok
test config::form::tests::an_absent_mic_device_stays_absent_through_the_round_trip ... ok
test config::form::tests::connector_names_are_the_serialised_ones ... ok
test config::form::tests::the_form_and_the_configuration_round_trip ... ok
test config::form::tests::the_form_holds_the_three_capabilities_in_order ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

- [ ] **Step 5: Write the failing tests for the live application** — three tests in `crates/tipsy-compadre/tests/session.rs`. Add `ListeningMode` to the protocol import (line 13), change the session helper's spawn (lines 122-123) to hand it a slot, change `tests/brain.rs`'s one `Brain::new` call (lines 113-114) the same way, and append the three tests at the end of the file. Each of them builds its own stack and its own slot — the slot is the handle a save swaps, so the test has to hold it:

```rust
use tipsy_protocol::{AssistantState, Command, Event, Frame, ListeningMode, ToClient, ToServer, UiLanguage};
```

```rust
    /// Review Focus 1's live half: the session reads the provider stack when a turn starts, so a
    /// save that rebuilds the connectors answers the *next* turn with them — no restart, and the
    /// conversation the session was holding is not dropped.
    #[tokio::test]
    async fn a_rebuilt_provider_stack_answers_the_next_turn() {
        let first = FakeChat::new(vec![Ok(vec!["primera respuesta."])]);
        let second = FakeChat::new(vec![Ok(vec!["segunda respuesta."])]);
        let speech = FakeSpeech::new();
        let transcribe = Arc::new(FakeTranscribe {
            text: "",
            languages: Mutex::new(Vec::new()),
            pcms: Mutex::new(Vec::new()),
        });
        let slot = ProviderSlot::new(Providers {
            chat: first.clone(),
            speech: speech.clone(),
            transcribe: transcribe.clone(),
        });
        let (tx, mut rx) = spawn_session(config(), slot.clone(), BrainConfig::default());
        next_matching(&mut rx, is_done, 500).await; // the greeting

        send(&tx, Command::Ask { text: "primera pregunta".into() });
        let reply = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Reply { .. })), 500).await;
        assert_eq!(reply, ToClient::Event(Event::Reply { text: "primera respuesta.".into() }));
        assert_eq!(first.calls(), 1);

        // The save: the connectors are rebuilt and swapped in place, exactly as the client does it.
        slot.replace(Providers { chat: second.clone(), speech, transcribe });

        send(&tx, Command::Ask { text: "segunda pregunta".into() });
        let reply = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Reply { .. })), 500).await;
        assert_eq!(reply, ToClient::Event(Event::Reply { text: "segunda respuesta.".into() }));
        assert_eq!(second.calls(), 1, "the next turn must use the stack the save installed");
        assert_eq!(first.calls(), 1, "the replaced stack must never be called again");
    }

    /// The mode the screen saves is the mode the HUD shows: `Welcome` is the only event that carries
    /// it, so the session announces the change.
    #[tokio::test]
    async fn a_new_listening_mode_is_announced_to_the_client() {
        let slot = ProviderSlot::new(Providers {
            chat: FakeChat::new(vec![]),
            speech: FakeSpeech::new(),
            transcribe: Arc::new(FakeTranscribe {
                text: "",
                languages: Mutex::new(Vec::new()),
                pcms: Mutex::new(Vec::new()),
            }),
        });
        let (tx, mut rx) = spawn_session(config(), slot, BrainConfig::default());
        let first = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Welcome { .. })), 500).await;
        assert!(
            matches!(first, ToClient::Event(Event::Welcome { listening_mode: ListeningMode::Push, .. })),
            "{first:?}"
        );

        send(&tx, Command::SetListeningMode { mode: ListeningMode::Auto });

        let again = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Welcome { .. })), 500).await;
        assert!(
            matches!(again, ToClient::Event(Event::Welcome { listening_mode: ListeningMode::Auto, .. })),
            "the header needs a fresh Welcome to show the mode that was just saved: {again:?}"
        );
    }

    /// The profile half of a save: the switch reaches the running session, so the next turn speaks
    /// the new profile's own prompt and the header and the language follow it.
    #[tokio::test]
    async fn switching_profile_changes_the_prompt_of_the_next_turn() {
        let chat = FakeChat::new(vec![Ok(vec!["Una respuesta suficientemente larga."])]);
        let slot = ProviderSlot::new(Providers {
            chat: chat.clone(),
            speech: FakeSpeech::new(),
            transcribe: Arc::new(FakeTranscribe {
                text: "",
                languages: Mutex::new(Vec::new()),
                pcms: Mutex::new(Vec::new()),
            }),
        });
        let (tx, mut rx) = spawn_session(config(), slot, BrainConfig::default());
        next_matching(&mut rx, is_done, 500).await; // the greeting

        send(&tx, Command::SetProfile { id: "senyor".into() });
        let welcome = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Welcome { .. })), 500).await;
        match welcome {
            ToClient::Event(Event::Welcome { profile, ui_language, .. }) => {
                assert_eq!(profile.id, "senyor");
                assert_eq!(ui_language, UiLanguage::Es);
            }
            other => panic!("the profile switch must be announced in a fresh Welcome, got {other:?}"),
        }

        send(&tx, Command::Ask { text: "¿cuánto cuesta?".into() });
        next_matching(&mut rx, is_done, 500).await;

        let sent = chat.requests.lock().unwrap()[0].messages.clone();
        assert_eq!(sent[0].role, Role::System);
        let senyor = config().profiles.into_iter().find(|p| p.id == "senyor").unwrap().prompt;
        assert_eq!(sent[0].content, senyor, "the next turn must speak as the profile just chosen: {sent:#?}");
    }
```

Also change the existing helper so the file compiles once the spawn signature changes (line 122-123):

```rust
    let providers = ProviderSlot::new(Providers { chat, speech, transcribe: transcribe.clone() });
    let (tx, rx) = spawn_session(config(), providers, BrainConfig::default());
```

and in `crates/tipsy-compadre/tests/brain.rs:113-114`:

```rust
    let providers = ProviderSlot::new(Providers { chat, speech, transcribe: transcribe.clone() });
    (Brain::new(providers, profile(), config()), transcribe)
```

- [ ] **Step 6: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --test session`

Expected: FAIL — the target does not compile:

```
error[E0433]: failed to resolve: use of undeclared type `ProviderSlot`
   --> crates\tipsy-compadre\tests\session.rs:122:22
error[E0308]: mismatched types: expected `Providers`, found `ProviderSlot`
   --> crates\tipsy-compadre\src\session.rs:49:63
```

(`cargo check --workspace --all-targets` reports the same two, plus `tests/brain.rs:114` for
`Brain::new`.)

- [ ] **Step 7: Implement the live stack**

**`crates/tipsy-compadre/src/providers/mod.rs`**, directly above `pub struct Providers` (line 152):

```rust
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
```

**`crates/tipsy-compadre/src/brain.rs`** — the stack is read once per turn and threaded through;
these are all the places, and each is a one-line change:

- line 20: `pub struct BrainConfig {` → `#[derive(Clone)]\npub struct BrainConfig {`
- lines 51-55: `providers: Providers,` → `providers: ProviderSlot,`
- line 58: `pub fn new(providers: Providers, profile: TurnProfile, config: BrainConfig) -> Self` →
  `pub fn new(providers: ProviderSlot, profile: TurnProfile, config: BrainConfig) -> Self`
- add `use crate::providers::ProviderSlot;` to the `crate::providers::{…}` import at lines 15-17
- after `let started = Instant::now();` (line 69), insert:

```rust
        // The stack this turn uses, read once: a save that rebuilds the connectors takes effect on
        // the *next* turn, and never halfway through this one.
        let providers = self.providers.current();
```

- line 100: `let turn = self.answer(&ask, history, &out, &cancel, started).await;` →
  `let turn = self.answer(&providers, &ask, history, &out, &cancel, started).await;`
- line 118: `(None, Some(text)) => self.say(&text, &out, &cancel, started).await,` →
  `(None, Some(text)) => self.say(&providers, &text, &out, &cancel, started).await,`
- lines 125-132: `async fn answer(` gains `providers: &Providers,` as its first parameter after
  `&self`, and line 134 becomes
  `let stream = match self.chat_with_retry(providers, ChatRequest { messages }, out, cancel).await {`
- line 141: `self.speak(stream, out, cancel, started).await` →
  `self.speak(providers, stream, out, cancel, started).await`
- line 145: `async fn say(&self, text: &str, …)` gains `providers: &Providers,` first, and line 148
  becomes `let turn = self.speak(providers, stream, out, cancel, started).await;`
- line 163: `async fn speak(&self, stream: TextStream, …)` gains `providers: &Providers,` first
- line 175: `provider: Arc::clone(&self.providers.speech),` → `provider: Arc::clone(&providers.speech),`
- line 176: `permits: Arc::new(Semaphore::new(self.providers.speech.max_parallel())),` →
  `permits: Arc::new(Semaphore::new(providers.speech.max_parallel())),`
- lines 267-272: `async fn chat_with_retry(&self, request: ChatRequest, out: …, cancel: …)` gains
  `providers: &Providers,` first, and both `self.chat(request.clone(), cancel)` (line 273) and
  `self.chat(request, cancel)` (line 281) become `self.chat(providers, request.clone(), cancel)` and
  `self.chat(providers, request, cancel)`
- line 288: `async fn chat(&self, request: ChatRequest, cancel: &CancellationToken)` gains
  `providers: &Providers,` first, and line 292 becomes
  `result = tokio::time::timeout(self.config.chat_timeout, providers.chat.stream(request)) => {`
- line 302: `async fn transcribe(&self, pcm: Pcm, cancel: &CancellationToken) -> Transcribed` gains
  `providers: &Providers,` first, and line 304 becomes
  `let call = providers.transcribe.transcribe(pcm, language);`
- line 77: `self.transcribe(pcm, &cancel).await` →
  `self.transcribe(&providers, pcm, &cancel).await`

**`crates/tipsy-compadre/src/session.rs`** — the import at line 18 becomes
`use crate::providers::{ChatMessage, ProviderSlot, Role};` (`Providers` is no longer named in this
file), and the session keeps the configuration it was built from, the brain configuration and the
slot, re-deriving its brain when the profile changes:

```rust
pub fn spawn_session(
    config: Config,
    providers: ProviderSlot,
    brain_config: BrainConfig,
) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>) {
```

```rust
pub fn spawn_with_events(config: Config, providers: ProviderSlot, brain_config: BrainConfig) -> SessionEnds {
    let (to_server_tx, to_server_rx) = mpsc::unbounded_channel();
    let (to_client_tx, to_client_rx) = mpsc::unbounded_channel();
    // Held so the client end only closes once the session task itself has ended.
    let keep_alive = to_client_tx.clone();

    let profile = config.active().clone();
    let listening_mode = config.listening.mode;
    let brain = Arc::new(Brain::new(providers.clone(), turn_profile(&config), brain_config.clone()));

    let mut session = Session {
        config,
        brain,
        brain_config,
        providers,
        out: to_client_tx.clone(),
        history: Vec::new(),
        current: None,
        generation: 0,
        listening: false,
        mic: Vec::new(),
        profile: profile.view(),
        ui_language: UiLanguage::from_bcp47(&profile.language),
        listening_mode,
        greeting: profile.greeting.clone(),
    };
```

```rust
struct Session {
    /// The configuration the session was built from. A `SetProfile` re-derives the brain from it, so
    /// a switch takes effect on the next turn.
    config: Config,
    brain: Arc<Brain>,
    brain_config: BrainConfig,
    /// The live provider stack (Task 6): read once per turn, swapped in place by a save.
    providers: ProviderSlot,
    out: UnboundedSender<ToClient>,
    history: Vec<ChatMessage>,
    current: Option<Turn>,
    generation: u64,
    /// True between `ListenStart` and `ListenStop`.
    listening: bool,
    /// Mono 16 kHz samples gathered while listening.
    mic: Vec<i16>,
    profile: ProfileView,
    ui_language: UiLanguage,
    listening_mode: ListeningMode,
    greeting: String,
}
```

```rust
/// What the brain needs from the configuration: the active profile's texts and the voice the speech
/// connector recommends for it. Built when the session starts and again on every `SetProfile`.
fn turn_profile(config: &Config) -> TurnProfile {
    let profile = config.active();
    TurnProfile {
        prompt: profile.prompt.clone(),
        voice_instructions: profile.voice_instructions.clone(),
        voice: profile.voice_for(&config.providers.speech),
        language: profile.language.clone(),
    }
}
```

and the two handlers in `fn command` (replacing `:223-228`):

```rust
            Command::SetProfile { id } => self.set_profile(id),
            Command::SetListeningMode { mode } => self.set_listening_mode(mode),
```

```rust
    /// Applies the profile the configuration screen saved and tells the client about it, so the
    /// header, the interface language and the next turn all follow the switch.
    ///
    /// The edited *texts* of a profile cannot travel the channel (`Command` carries an id), so this
    /// re-derives the brain from the configuration the session holds: an id that is not in it is a
    /// no-op rather than a brain built around nothing.
    fn set_profile(&mut self, id: String) {
        if !self.config.profiles.iter().any(|profile| profile.id == id) {
            return;
        }
        self.config.active_profile = id;
        let profile = self.config.active();
        self.profile = profile.view();
        self.ui_language = UiLanguage::from_bcp47(&profile.language);
        self.brain = Arc::new(Brain::new(
            self.providers.current(),
            turn_profile(&self.config),
            self.brain_config.clone(),
        ));
        self.welcome();
    }

    /// Applies the listening mode a save changed. The mode lives in `Welcome` and nowhere else, so
    /// the session announces it again — that is what moves the header's indicator.
    fn set_listening_mode(&mut self, mode: ListeningMode) {
        self.listening_mode = mode;
        self.welcome();
    }
```

- [ ] **Step 8: Run the session tests and watch them pass**

Run: `cargo test -p tipsy-compadre --test session`

Expected: PASS, 9 tests:

```
running 9 tests
test a_long_conversation_keeps_the_prompt_bounded ... ok
test a_new_listening_mode_is_announced_to_the_client ... ok
test a_rebuilt_provider_stack_answers_the_next_turn ... ok
test ask_produces_reply_and_done ... ok
test barge_in_cancels_running_turn ... ok
test listen_stop_transcribes_buffered_mic_audio ... ok
test reset_clears_history ... ok
test switching_profile_changes_the_prompt_of_the_next_turn ... ok
test welcome_then_greeting ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```

Then `cargo test -p tipsy-compadre --test brain` — every test it already had, unchanged, on a slot.

- [ ] **Step 9: Write the failing tests for the save path** — five tests appended to the existing `#[cfg(test)] mod tests` in `crates/tipsy-compadre/src/client.rs`, plus the helpers above them and the one line in the existing test that constructs a client (line 404):

```rust
    use crate::config::form::form_of;
    use crate::keys::{KeyResolver, MemoryStore, SecretStore};
    use crate::providers::ProviderSlot;
    use tipsy_ui::{ConfigAction, NoticeKind};

    /// A resolver that knows one key — `openrouter`, the name the default configuration uses — and
    /// reads no environment.
    fn resolver() -> KeyResolver {
        let store = MemoryStore::default();
        SecretStore::set(&store, "openrouter", "k-test").unwrap();
        KeyResolver::new(Box::new(store), Box::new(|_| None))
    }

    /// A configuration whose three connectors point at `base`. Nothing in these tests calls them
    /// unless it says so, and it keeps `build_providers` off the real network.
    fn mock_config(base: &str) -> Config {
        let mut config = Config::default();
        for provider in [&mut config.providers.chat, &mut config.providers.speech, &mut config.providers.transcribe] {
            provider.base_url = format!("{base}/v1");
            provider.key = "openrouter".into();
        }
        config
    }

    /// The screen with the edits a user would have made in it: the form holds them, the file does
    /// not, and `dirty` says so.
    fn open_with_edits(client: &mut Client, edited: &Config) {
        client.state.open_config(form_of(edited));
        client.state.config_form.as_mut().unwrap().dirty = true;
    }

    /// Review Focus 3, second half: the screen opens on the file as it is *now*. A `config.toml`
    /// hand-edited while the app runs is what the second open shows, never the copy the first one
    /// read, and opening changes nothing that is running.
    #[test]
    fn opening_the_screen_again_reads_the_file_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config.clone(), path.clone(), keys, slot);

        client.open_screen();
        let first = client.state.config_form.as_ref().unwrap();
        assert_eq!(first.listening.silence_ms, "1000");
        assert!(first.server.token.is_empty());

        // The user edits the file in another window while the app keeps running.
        let mut edited = config.clone();
        edited.listening.silence_ms = 4321;
        edited.server.token = "pasted-by-hand".into();
        store::save(&path, &edited).unwrap();

        client.open_screen();
        let second = client.state.config_form.as_ref().unwrap();
        assert_eq!(second.listening.silence_ms, "4321", "the screen must read the file again");
        assert_eq!(second.server.token, "pasted-by-hand");
        assert_eq!(client.config, config, "opening the screen changes nothing that is running");
    }

    /// Review Focus 1: a write that fails changes nothing at all. The file is byte for byte what it
    /// was, the running configuration and its provider stack are the ones the app started with, and
    /// the screen says why — while the edits stay in the form, unsaved.
    #[test]
    fn a_refused_save_leaves_the_file_and_the_running_configuration_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let stack_before = slot.current();
        let mut client = Client::new(config.clone(), path.clone(), keys, slot.clone());

        let mut edited = config.clone();
        edited.listening.silence_ms = 4321;
        edited.server.token = "pasted-by-hand".into();
        open_with_edits(&mut client, &edited);

        // A config path the process may not write: read-only is the cheapest way to make the write
        // itself fail — after every row parsed and every connector built, which is where the
        // interesting part of the order is.
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, &permissions).unwrap();

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);

        assert_eq!(std::fs::read(&path).unwrap(), before, "a refused save must not touch the file");
        assert_eq!(client.config, config, "the running configuration is the one it was");
        assert!(
            std::sync::Arc::ptr_eq(&stack_before.chat, &slot.current().chat),
            "the running provider stack must not have been swapped"
        );
        let form = client.state.config_form.as_ref().unwrap();
        let (kind, text) = form.notice.clone().expect("a refused save must say so");
        assert_eq!(kind, NoticeKind::Error);
        assert!(text.contains("config.toml"), "the notice must name the file: {text}");
        assert!(form.dirty, "the edits are still unsaved: {form:?}");
        // And the file still holds the value a next launch must read.
        assert!(!std::fs::read_to_string(&path).unwrap().contains("4321"));

        // Let the temp directory go away.
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, &permissions).unwrap();
    }

    /// The other half of Review Focus 1, and Review Focus 5's "no key": a configuration whose
    /// connectors cannot be built is never written at all. Writing it would leave a `config.toml`
    /// the app cannot start from — a fresh launch fails with exit code 2 — so the key is checked
    /// before the file is touched.
    #[test]
    fn a_save_whose_providers_cannot_be_built_never_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let stack_before = slot.current();
        let mut client = Client::new(config.clone(), path.clone(), keys, slot.clone());

        // The user typed a key name nobody holds. Everything else about this save is valid, and the
        // write itself would have succeeded.
        let mut edited = config.clone();
        edited.providers.chat.key = "not-in-the-store".into();
        open_with_edits(&mut client, &edited);

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);

        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "a configuration the app cannot start from must never be written"
        );
        assert_eq!(client.config, config);
        assert!(std::sync::Arc::ptr_eq(&stack_before.chat, &slot.current().chat));
        let (kind, text) = client.state.config_form.as_ref().unwrap().notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Error);
        assert!(
            text.contains("not-in-the-store") && text.contains("TIPSY_KEY_NOT_IN_THE_STORE"),
            "the notice must name the key and how to provide it: {text}"
        );
    }

    /// A save that works: the file holds the edits, the running configuration is the one that was
    /// written, the connectors are rebuilt *in place*, and the session is told the fields the
    /// channel carries — in that order.
    #[test]
    fn a_saved_configuration_is_written_and_swapped_into_the_running_stack() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let stack_before = slot.current();
        let mut client = Client::new(config.clone(), path.clone(), keys, slot.clone());

        let mut edited = config.clone();
        edited.listening.silence_ms = 4321;
        edited.listening.mode = ListeningMode::Auto;
        edited.active_profile = "senyor".into();
        open_with_edits(&mut client, &edited);

        let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("silence_ms = 4321"), "{text}");
        assert!(text.contains("mode = \"auto\""), "{text}");
        assert!(text.contains("active_profile = \"senyor\""), "{text}");
        assert_eq!(store::load_or_create(&path).unwrap().config, edited, "the file is the whole form");
        assert_eq!(client.config, edited, "the running configuration is what was written");
        assert!(!std::sync::Arc::ptr_eq(&stack_before.chat, &slot.current().chat), "rebuilt in place");
        assert!(!client.state.config_form.as_ref().unwrap().dirty, "saved means nothing unsaved");
        assert!(client.state.config_form.as_ref().unwrap().notice.is_none());

        // The session learns the mode and the profile over the channel it already uses.
        assert_eq!(
            commands.try_recv().unwrap(),
            ToServer::Command(Command::SetListeningMode { mode: ListeningMode::Auto })
        );
        assert_eq!(
            commands.try_recv().unwrap(),
            ToServer::Command(Command::SetProfile { id: "senyor".into() })
        );
    }

    /// Regenerating fills the row the screen shows and writes nothing: the file changes on save,
    /// only on save, and only through `store::save`.
    #[test]
    fn regenerating_the_token_fills_the_row_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, path.clone(), keys, slot);
        client.open_screen();
        assert!(client.state.config_form.as_ref().unwrap().server.token.is_empty());

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::RegenerateToken), &to_server, &events);

        let form = client.state.config_form.as_ref().unwrap();
        assert_eq!(form.server.token.len(), 32, "16 bytes as lowercase hex: {:?}", form.server.token);
        assert!(
            form.server.token.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "the token's alphabet is 0-9a-f: {:?}",
            form.server.token
        );
        assert!(form.dirty, "a fresh token is an unsaved edit like any other");
        assert_eq!(std::fs::read(&path).unwrap(), before, "nothing is written until the user saves");
    }
```

The existing test at line 402-420 constructs a client without providers and calls `on_action` with two
arguments; change line 404 to

```rust
        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);
```

and line 418 to

```rust
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Send(Command::Ask { text: "¿y eso?".into() }), &to_server, &events);
```

- [ ] **Step 10: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --lib client::`

Expected: FAIL — the target does not compile:

```
error[E0061]: this function takes 1 argument but 4 arguments were supplied
   --> crates\tipsy-compadre\src\client.rs:404:24
error[E0599]: no method named `open_screen` found for struct `Client`
error[E0599]: no method named `open_config` found for struct `UiState`
error[E0591]... / E0004: non-exhaustive patterns: `UiAction::OpenConfig` and `UiAction::Config(_)` not covered
   --> crates\tipsy-compadre\src\client.rs:242:9
```

(`UiAction`'s two new variants come from Task 5; this task is what exhaustively matches them.)

- [ ] **Step 11: Implement the client side**

**Imports** (lines 17-25):

```rust
use crate::audio::device::{Mic, Player};
use crate::audio::pcm::{envelope, i16_to_f32, rms};
use crate::brain::BrainConfig;
use crate::config::form::{config_from, form_of};
use crate::config::store;
use crate::config::Config;
use crate::keys::KeyResolver;
use crate::providers::{build_one, build_providers, ProviderSlot};
use crate::session::spawn_with_events;
use crate::sysmon;
```

and in the `tipsy_ui` import (line 14), `ConfigAction, ConfigEffect, NoticeKind` join the list. Two
names are added by Task 7 instead, because nothing in this task uses them: `build_one` in this import
block, and `provider_config` in `crate::config::form`'s. Leave them out here so no import is unused.

**`run`** (lines 35-54) builds the resolver and the slot once and hands both to the client:

```rust
pub fn run(config_path: PathBuf) -> Result<(), String> {
    let loaded = store::load_or_create(&config_path).map_err(|e| format!("{}: {e}", config_path.display()))?;
    let config = loaded.config;
    let keys = KeyResolver::system();
    // One stack, shared with the session from here on: a save swaps the connectors inside it.
    let providers = ProviderSlot::new(build_providers(&config.providers, &keys).map_err(|e| e.to_string())?);

    let mut client = Client::new(config, config_path, keys, providers);
    // `init` enters the alternate screen, enables raw mode and installs the panic hook that
    // restores the terminal; `restore` undoes all of it.
    let mut terminal = ratatui::init();
    // Mouse capture is not part of a bare terminal: without it the clickable titles do nothing.
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    if let Some(warning) = loaded.warning {
        client.state.notify(warning);
    }
    client.open_player();
    let outcome = client.frame_loop(&mut terminal);
    let _ = execute!(std::io::stdout(), event::DisableMouseCapture);
    ratatui::restore();
    outcome
}
```

**`Client`** (lines 56-88) gains the path, the resolver and the slot:

```rust
struct Client {
    state: UiState,
    /// The configuration the running session was built from: a save that worked replaces it, so the
    /// client always holds the configuration the file holds.
    config: Config,
    /// Where the screen writes, and where it reads every time it opens.
    config_path: PathBuf,
    /// The resolver the connectors are built with: the one the app started with.
    keys: KeyResolver,
    /// The live provider stack, shared with the session.
    providers: ProviderSlot,
    /// `None` when there is no output device: the HUD still works, silently.
    player: Option<Player>,
    /// The audio of the reply being spoken, kept for `Replay` until a new turn starts.
    reply: Vec<i16>,
    /// Sample rate announced by the last `SpeechChunk`.
    rate: u32,
    mic_gain: f32,
    mic: Option<Mic>,
    mic_frames: std::sync::mpsc::Receiver<Vec<i16>>,
    mic_tx: std::sync::mpsc::SyncSender<Vec<i16>>,
}

impl Client {
    fn new(config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot) -> Self {
        let (mic_tx, mic_frames) = std::sync::mpsc::sync_channel(MIC_QUEUE);
        let mic_gain = config.listening.mic_gain;
        Self {
            state: UiState::new(),
            config,
            config_path,
            keys,
            providers,
            player: None,
            reply: Vec::new(),
            rate: 24000,
            mic_gain,
            mic: None,
            mic_frames,
            mic_tx,
        }
    }
```

**`frame_loop`** (lines 99-163) takes no providers any more — the client holds them — and keeps a
sender for the Test results:

```rust
    fn frame_loop(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<(), String> {
        let ends = spawn_with_events(self.config.clone(), self.providers.clone(), BrainConfig::default());
        let to_server = ends.to_server;
        let mut from_server = ends.from_server;
        // A second sender for the same client channel: the system monitor and the screen's Test
        // results (Task 7) both arrive with the session's own events.
        let events = ends.events.clone();
        sysmon::spawn(ends.events);
```

and the action loop at lines 149-154 passes it:

```rust
            let mut quit = false;
            for key in keys {
                for action in self.state.on_key(key) {
                    quit |= self.on_action(action, &to_server, &events);
                }
            }
```

**`on_action`** (lines 240-267) gains the two screen arms, and `events` alongside `to_server`. The
signature, then the two new arms — the `UiAction::Send` and `UiAction::Replay` arms keep every line
they have today:

```rust
    fn on_action(
        &mut self,
        action: UiAction,
        to_server: &UnboundedSender<ToServer>,
        events: &UnboundedSender<ToClient>,
    ) -> bool {
        match action {
            UiAction::Send(command) => { /* as today */ }
            UiAction::Replay(_) => { /* as today */ }
            // The screen asks for a form it cannot build: `tipsy-ui` owns no native type. The file
            // is read here and now, which is what keeps a hand-edited `config.toml` from being
            // served out of a copy taken at startup.
            UiAction::OpenConfig => {
                self.open_screen();
                false
            }
            // Every other screen action goes through the form's own reducer, and the effects it asks
            // for are performed here, on the native side.
            UiAction::Config(action) => {
                let effects = match self.state.config_form.as_mut() {
                    Some(form) => form.apply(action),
                    None => Vec::new(),
                };
                for effect in effects {
                    self.on_config_effect(effect, to_server, events);
                }
                false
            }
            UiAction::Quit => true,
        }
    }

    /// Opens the configuration screen on a copy read from the file *now* (Review Focus 3): a
    /// `config.toml` edited while the app runs is what the screen shows. Opening changes nothing
    /// that is running — `Esc` discards the edits, and only a save applies them.
    fn open_screen(&mut self) {
        match store::load_or_create(&self.config_path) {
            Ok(loaded) => {
                let mut form = form_of(&loaded.config);
                // A file that had to be backed up and reset is worth saying in the notice line: the
                // user's own file is gone, and the form no longer holds what they wrote.
                if let Some(warning) = loaded.warning {
                    form.notice = Some((NoticeKind::Warning, warning));
                }
                self.state.open_config(form);
            }
            Err(e) => self.state.notify(format!("{}: {e}", self.config_path.display())),
        }
    }

    /// Performs one effect the screen asked for.
    ///
    /// `_events` is the client's own event sender: Task 7 uses it to report a Test result, and
    /// renames it to `events` then. It is underscored here so the compiler is not told about an
    /// argument this task has nothing to send.
    fn on_config_effect(
        &mut self,
        effect: ConfigEffect,
        to_server: &UnboundedSender<ToServer>,
        _events: &UnboundedSender<ToClient>,
    ) {
        match effect {
            ConfigEffect::None => {}
            ConfigEffect::Save => self.save(to_server),
            // Task 7 fills this in; the variant exists from Task 3 on, so the match is exhaustive.
            ConfigEffect::Test(_) => {}
            ConfigEffect::RegenerateToken => self.regenerate_token(),
        }
    }

    /// Writes the screen's configuration and applies it to the running session.
    ///
    /// The order is the whole of Review Focus 1: the connectors are built first, then the file is
    /// written through the one writer, and only then does anything running change. Nothing is
    /// written until every row parses and every key resolves, because a `config.toml` the app cannot
    /// start from is damage rather than a saved edit; and a refused write leaves the file, the
    /// running stack and the session exactly as they were.
    fn save(&mut self, to_server: &UnboundedSender<ToServer>) {
        // 1. The form in the file's shape.
        let parsed = self.state.config_form.as_ref().map(config_from);
        let config = match parsed {
            None => return,
            Some(Err(message)) => return self.config_notice(NoticeKind::Error, message),
            Some(Ok(config)) => config,
        };
        // 2. The connectors that configuration names.
        let rebuilt = match build_providers(&config.providers, &self.keys) {
            Ok(providers) => providers,
            Err(e) => return self.config_notice(NoticeKind::Error, e.to_string()),
        };
        // 3. The file itself.
        if let Err(e) = store::save(&self.config_path, &config) {
            return self.config_notice(NoticeKind::Error, format!("{}: {e}", self.config_path.display()));
        }
        // 4. What is running: the stack in place, then the fields the channel carries.
        self.mic_gain = config.listening.mic_gain;
        self.providers.replace(rebuilt);
        let _ = to_server.send(ToServer::Command(Command::SetListeningMode { mode: config.listening.mode }));
        let _ = to_server.send(ToServer::Command(Command::SetProfile { id: config.active_profile.clone() }));
        self.config = config;
        if let Some(form) = self.state.config_form.as_mut() {
            form.dirty = false;
            form.notice = None;
        }
    }

    /// Fills the Server tab's token row with a fresh token from the OS generator. This is an edit
    /// like any other: the file changes when the user saves, never here.
    fn regenerate_token(&mut self) {
        match store::new_token() {
            Ok(token) => {
                if let Some(form) = self.state.config_form.as_mut() {
                    form.server.token = token;
                    form.dirty = true;
                }
            }
            Err(e) => self.config_notice(NoticeKind::Error, e.to_string()),
        }
    }

    /// Puts a configuration message in the screen's notice line. The text is the low-level truth —
    /// the path and its OS error, the row that would not parse, the provider's own words — because
    /// the form carries no language of its own (the plan's `ConfigForm` has no `lang` field) and
    /// must not invent copy it cannot translate. Falls back to the HUD's notice when no screen is
    /// open.
    fn config_notice(&mut self, kind: NoticeKind, text: impl Into<String>) {
        let text = text.into();
        if let Some(form) = self.state.config_form.as_mut() {
            form.notice = Some((kind, text));
            return;
        }
        self.state.notify(text);
    }
```

Two scope notes, stated so nobody has to guess: `mic_gain` is applied to the running client at once
because it is the client's own multiplier, while `mic_device` and the edited *texts* of a profile are
written and shown now and take effect at the next launch — the device because `Mic::start`
(`audio/device.rs:99`) takes no device yet, and the texts because `Command` carries a profile id and
not a prompt.

- [ ] **Step 12: Run the client tests and watch them pass**

Run: `cargo test -p tipsy-compadre --lib client::`

Expected: PASS, 8 tests:

```
running 8 tests
test client::tests::a_finished_reply_keeps_its_audio_until_a_new_turn_starts ... ok
test client::tests::a_refused_save_leaves_the_file_and_the_running_configuration_alone ... ok
test client::tests::a_save_whose_providers_cannot_be_built_never_writes_the_file ... ok
test client::tests::a_saved_configuration_is_written_and_swapped_into_the_running_stack ... ok
test client::tests::only_an_interrupt_with_audio_left_to_drop_flushes ... ok
test client::tests::opening_the_screen_again_reads_the_file_again ... ok
test client::tests::regenerating_the_token_fills_the_row_and_writes_nothing ... ok
test client::tests::the_commands_that_end_a_turn_are_the_ones_that_interrupt_it ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

- [ ] **Step 13: Prove the whole workspace is still whole**

Run: `cargo test -p tipsy-compadre` — the lib, the bin and every integration test (`session`,
`brain`, `openai_compat`, `gemini`, `elevenlabs`) green.

Run: `cargo test --workspace` — `tipsy-protocol` and `tipsy-ui` included. The counts depend on
Tasks 1–5; what must be there is `config::form`'s 5, `client::tests`' 8 and `tests/session.rs`'s 9.

Run: `cargo clippy --workspace --all-targets` — no new warnings. The `render` and `state` suites are
the regression net for Task 5's `UiAction` variants: a wrong arm (a `Config` action silencing a reply
in progress, `open_config` swallowing the HUD) shows up there.

- [ ] **Step 14: Commit**

```bash
git add crates/tipsy-compadre/src/config/form.rs crates/tipsy-compadre/src/config/mod.rs crates/tipsy-compadre/src/providers/mod.rs crates/tipsy-compadre/src/brain.rs crates/tipsy-compadre/src/session.rs crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/tests/session.rs crates/tipsy-compadre/tests/brain.rs
git commit -m "feat(config): save the screen's form and apply it without restarting

Config and ConfigForm convert both ways with a round-trip test, so the two shapes
cannot drift. A save builds the connectors, writes the file through store::save
and only then changes what runs: a refused write or a missing key leaves the file
and the running stack untouched, and the notice line says why. The providers live
in a shared slot the session reads once per turn, so a rebuilt connector answers
the next turn; SetProfile and SetListeningMode re-announce the session in a fresh
Welcome so the header follows a save.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 7: The Test button `[mid]`

**Files:**
- Modify: `crates/tipsy-compadre/src/providers/mod.rs` — `Connector`, `build_one`, `Probe` and `Connector::probe` after `build_providers` (line 241), and their tests inside the existing `#[cfg(test)] mod build_tests` (`:326-519`) or in a new `mod probe_tests` beside it
- Modify: `crates/tipsy-compadre/src/client.rs` — the `ConfigEffect::Test(_)` arm of `on_config_effect` (as Task 6 leaves it), the new `fn test_provider(&mut self, capability: Capability, events: &UnboundedSender<ToClient>)`, the `Event::TestResult` arm in `on_message` (`:165-179`), the `build_one`/`Capability` imports (`:17-25`)
- Modify: `crates/tipsy-ui/src/config_form.rs` — `ConfigForm::test_result` at the end of `impl ConfigForm`, and its test in the module's test module (this file is created by Task 3; line numbers are Task 3's to fix, so this task is anchored by item names)
- Test: `crates/tipsy-compadre/src/providers/mod.rs` (`mod probe_tests`), `crates/tipsy-compadre/src/client.rs` (`mod tests`), `crates/tipsy-ui/src/config_form.rs` (its test module)
- Not modified: `crates/tipsy-protocol/src/lib.rs` (Task 2 landed the pair), `crates/tipsy-ui/src/state.rs` (the `Event::TestResult` arm stays a no-op: the client intercepts the event and gives it to the form)

**Interfaces:**
- Consumes (read from the committed code, not recalled):
  - Task 2's protocol pair: `pub enum Capability { Chat, Speech, Transcribe }` with `Display` ("chat" / "speech" / "transcription") and `#[serde(rename_all = "snake_case")]` — `tipsy-protocol/src/lib.rs:66-90`; `Command::TestProvider { capability: Capability }` and `Event::TestResult { capability: Capability, ok: bool, latency_ms: u64, message: String }` — the last variants of `Command` and `Event`
  - `pub enum Providers { chat: Arc<dyn ChatProvider>, speech: Arc<dyn SpeechProvider>, transcribe: Arc<dyn TranscribeProvider> }` — `providers/mod.rs:152-157`; the traits, their exact signatures and `pub type TextStream = futures::stream::BoxStream<'static, Result<String, ProviderError>>` — `:129-150`
  - The three private builders `fn chat_provider(&ProviderConfig, &KeyResolver) -> Result<Arc<dyn ChatProvider>, BuildError>` (`:191-199`), `fn speech_provider(…)` (`:201-209`), `fn transcribe_provider(…)` (`:211-221`), and `const fn supports(kind: ProviderKind, capability: Capability) -> bool` (`:183-189`)
  - `pub enum BuildError { MissingKey { name, env }, UnsupportedCapability { capability, kind }, Key(KeyError) }` — `providers/mod.rs:159-170`; its `Display` texts are what the screen shows for "no key" ("no API key named \`x\`: run \`tipsy-compadre key set x\` or set TIPSY_KEY_X")
  - `pub enum ProviderError { Auth, RateLimited, Timeout, Network, BadResponse, Cancelled }` and `ProviderError::from_status` — `providers/mod.rs:62-105`; the connectors map an HTTP error body to these — `providers/openai_compat.rs:50-57`
  - `pub fn build_providers(&ProvidersConfig, &KeyResolver) -> Result<Providers, BuildError>` — `providers/mod.rs:223-241`; `pub struct ProviderSlot` and `pub fn current(&self) -> Providers` — Task 6
  - `pub struct ProviderConfig { pub kind: ProviderKind, pub base_url: String, pub model: String, pub key: String }` — `config/model.rs:74-82`; `pub fn provider_config(row: &ProviderForm) -> Result<ProviderConfig, String>` and `pub fn form_of(&Config) -> ConfigForm` — Task 6's `config/form.rs`
  - `pub fn transcribe(&self, pcm: Pcm, language: &str) -> Result<String, ProviderError>` — `providers/mod.rs:149`; `pub struct Pcm { pub samples: Vec<f32>, pub sample_rate: u32 }` — `audio/pcm.rs:6-9`
  - The OpenAI-compatible connector the probe tests drive: `POST {base}/chat/completions` with `{"model", "messages", "stream": true}` (`openai_compat.rs:61-73`), `POST {base}/audio/speech` with `{"model", "input", "response_format"}` (`:133-158`), `POST {base}/audio/transcriptions` as multipart with `model`, `file` and `language` (`:162-185`); the `google/` model prefix means raw 24 kHz PCM (`:122-130`)
  - `wiremock` as the connector tests use it: `Mock::given(method("POST")).and(path(…))`, `ResponseTemplate::new(200).set_delay(Duration)`, `set_body_raw(body, "text/event-stream")`, `set_body_bytes`, `server.received_requests().await` — `tests/openai_compat.rs:29-56`, `:150-168`; `set_delay` is unconditional in wiremock 0.6.5
  - `pub fn notify(&mut self, text: impl Into<String>)` — `tipsy-ui/src/state.rs:136-138`; `pub fn apply(&mut self, ev: &Event)` — `:165-236`
  - Assumed from Tasks 3, 5 and 6 (reconcile before implementing):
    - Task 3's `ConfigForm` with `pub providers: Vec<ProviderForm>`, `pub notice: Option<(NoticeKind, String)>`, `pub tab: ConfigTab`, `pub field: usize`, and `NoticeKind::{Ok, Warning, Error}`; `ConfigForm::apply(ConfigAction::Test)` yields `ConfigEffect::Test(capability)` for the *focused* Providers row, and `ConfigTab::Providers` with `field == 0` is chat's row (the first of three, in `Capability` order)
    - Task 5's `UiState::config_form: Option<ConfigForm>` and `UiAction::Config(ConfigAction)`
    - Task 6's `Client::new(config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot)`, `fn config_notice(&mut self, kind: NoticeKind, text: impl Into<String>)`, `fn on_action(&mut self, action: UiAction, to_server: &UnboundedSender<ToServer>, events: &UnboundedSender<ToClient>) -> bool`, and the `events` sender in `frame_loop` (a second sender for the client's own `ToClient` channel)
- Produces:
  - `pub enum Connector { Chat(Arc<dyn ChatProvider>), Speech(Arc<dyn SpeechProvider>), Transcribe(Arc<dyn TranscribeProvider>) }` — `providers/mod.rs`
  - `pub fn build_one(cfg: &ProviderConfig, capability: Capability, keys: &KeyResolver) -> Result<Connector, BuildError>` — builds exactly one connector, so a key another capability is missing cannot fail this one's Test
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct Probe { pub ok: bool, pub latency_ms: u64, pub message: String }` — the three fields map one to one onto `Event::TestResult`, so the client never reinterprets them
  - `pub async fn Connector::probe(self, language: &str) -> Probe` — `language` is the BCP 47 tag the transcription probe uses; it is ignored by the other two
  - `pub fn ConfigForm::test_result(&mut self, capability: Capability, ok: bool, latency_ms: u64, message: String)` — `tipsy-ui/src/config_form.rs`; the screen's notice line for a Test, `Ok` when the provider answered and `Error` when it refused
  - Private to `client.rs`: `fn test_provider(&mut self, capability: Capability, events: &UnboundedSender<ToClient>)`

- [ ] **Step 1: Write the failing probe tests** — append a new test module to `crates/tipsy-compadre/src/providers/mod.rs`, after the existing `mod build_tests`:

```rust
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
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --lib providers::probe_tests`

Expected: FAIL — the target does not compile:

```
error[E0433]: failed to resolve: use of undeclared type `Connector`
error[E0425]: cannot find function `build_one` in this scope
error[E0599]: no method named `probe` found for opaque type `impl Future<Output = Result<…>>`
```

- [ ] **Step 3: Implement the probe** — append to `crates/tipsy-compadre/src/providers/mod.rs`, after `build_providers` (line 241), and add `use futures::StreamExt;` to the imports at the top:

```rust
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

impl Connector {
    /// The smallest real call this connector allows (spec §9's Test button).
    ///
    /// * chat: one request whose only message is the single word `ping`, of which exactly one delta
    ///   is read and the stream then dropped. The connector's own defaults bound the reply, since
    ///   `ChatRequest` carries no token cap. A 200 that streams no text at all is *not* a success: a
    ///   test that believed the status code would lie to the user.
    /// * speech: one word, with the connector's own default voice and no instructions.
    /// * transcription: half a second of silence at 16 kHz, which is the shortest buffer every
    ///   connector's endpoint accepts. An empty transcript is an answer — silence has no words.
    pub async fn probe(self, language: &str) -> Probe {
        match self {
            Connector::Chat(chat) => probe_chat(chat.as_ref()).await,
            Connector::Speech(speech) => probe_speech(speech.as_ref()).await,
            Connector::Transcribe(transcribe) => probe_transcribe(transcribe.as_ref(), language).await,
        }
    }
}

async fn probe_chat(chat: &dyn ChatProvider) -> Probe {
    let started = std::time::Instant::now();
    let outcome = async {
        let mut stream = chat.stream(ChatRequest { messages: vec![ChatMessage::user("ping")] }).await?;
        match stream.next().await {
            Some(Ok(text)) => Ok(format!("{} bytes", text.len())),
            Some(Err(e)) => Err(e),
            // The provider took the request and said nothing, which is a refusal, not an answer.
            None => Err(ProviderError::BadResponse("the provider answered with no text".into())),
        }
    }
    .await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(summary) => Probe::answered(latency_ms, summary),
        Err(e) => Probe::refused(latency_ms, e.to_string()),
    }
}

async fn probe_speech(speech: &dyn SpeechProvider) -> Probe {
    let started = std::time::Instant::now();
    let request = SpeechRequest { text: "hola".into(), instructions: String::new(), voice: None };
    let outcome = speech.synthesize(request).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(pcm) if !pcm.samples.is_empty() => {
            Probe::answered(latency_ms, format!("{} samples at {} Hz", pcm.samples.len(), pcm.sample_rate))
        }
        // A 200 that synthesised nothing is a refusal: nothing can be played from it.
        Ok(_) => Probe::refused(latency_ms, "the provider returned no audio"),
        Err(e) => Probe::refused(latency_ms, e.to_string()),
    }
}

async fn probe_transcribe(transcribe: &dyn TranscribeProvider, language: &str) -> Probe {
    let started = std::time::Instant::now();
    let pcm = Pcm { samples: vec![0.0; 8_000], sample_rate: 16_000 };
    let outcome = transcribe.transcribe(pcm, language).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(text) if text.trim().is_empty() => Probe::answered(latency_ms, "silence, no words"),
        Ok(text) => Probe::answered(latency_ms, format!("{} characters", text.chars().count())),
        Err(e) => Probe::refused(latency_ms, e.to_string()),
    }
}
```

- [ ] **Step 4: Run the probe tests and watch them pass**

Run: `cargo test -p tipsy-compadre --lib providers::probe_tests`

Expected: PASS, 7 tests:

```
running 7 tests
test providers::probe_tests::a_capability_a_connector_does_not_offer_is_refused_without_a_call ... ok
test providers::probe_tests::a_missing_key_is_refused_before_any_call ... ok
test providers::probe_tests::chat_probe_never_calls_an_empty_answer_a_success ... ok
test providers::probe_tests::chat_probe_refuses_an_error_the_provider_sent_inside_the_stream ... ok
test providers::probe_tests::chat_probe_reports_the_measured_latency_of_a_real_answer ... ok
test providers::probe_tests::speech_probe_reads_the_samples_and_the_providers_words ... ok
test providers::probe_tests::transcribe_probe_answers_a_silent_buffer_in_the_profiles_language ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

- [ ] **Step 5: Write the failing tests for the screen and the client** — one test in `crates/tipsy-ui/src/config_form.rs` and two in `crates/tipsy-compadre/src/client.rs`.

In `crates/tipsy-ui/src/config_form.rs`, append to the test module (its `use super::*;` and
`use tipsy_protocol::{Capability, ListeningMode};` are already there):

```rust
    /// A form nobody has touched, so a notice can be read on its own. If Task 3's module already
    /// has such a helper, use that one and drop this.
    fn form_for_test() -> ConfigForm {
        let provider = |capability| ProviderForm {
            capability,
            kind: "openai_compat".into(),
            base_url: "http://127.0.0.1:1/v1".into(),
            model: "m".into(),
            key_name: "openrouter".into(),
        };
        ConfigForm {
            tab: ConfigTab::Providers,
            field: 0,
            editing: None,
            active_profile: "compae".into(),
            providers: vec![provider(Capability::Chat), provider(Capability::Speech), provider(Capability::Transcribe)],
            listening: ListeningForm { mode: ListeningMode::Push, silence_ms: "1000".into(), mic_gain: "12".into(), mic_device: None },
            profiles: vec![ProfileForm {
                id: "compae".into(),
                name: "El Compae".into(),
                language: "es-ES".into(),
                prompt: "p".into(),
                voice_instructions: "v".into(),
                greeting: "g".into(),
                wake_word: "oye compae".into(),
                voices: vec![("gemini".into(), "Charon".into())],
            }],
            server: ServerForm { bind: "127.0.0.1:7070".into(), lan: false, token: String::new() },
            notice: None,
            dirty: false,
        }
    }

    /// Review Focus 5, as the screen shows it: an answer is green and carries the measured latency;
    /// a refusal is red and carries the provider's own words, with no latency dressed up as one.
    #[test]
    fn a_test_result_lands_in_the_notice_and_never_dresses_up_a_failure() {
        let mut form = form_for_test();

        form.test_result(Capability::Chat, true, 412, "18 bytes".into());
        let (kind, text) = form.notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Ok);
        assert_eq!(text, "chat: 412 ms — 18 bytes");

        form.test_result(Capability::Chat, false, 0, "no API key named `openrouter`: run `tipsy-compadre key set openrouter` or set TIPSY_KEY_OPENROUTER".into());
        let (kind, text) = form.notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Error);
        assert_eq!(
            text,
            "chat: no API key named `openrouter`: run `tipsy-compadre key set openrouter` or set TIPSY_KEY_OPENROUTER",
            "a refusal carries the answer as it came, and no latency"
        );

        // The third capability reads as a person says it, which is the protocol's `Display`.
        form.test_result(Capability::Transcribe, false, 30, "HTTP 500: boom".into());
        let (kind, text) = form.notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Error);
        assert_eq!(text, "transcription: HTTP 500: boom");
    }
```

In `crates/tipsy-compadre/src/client.rs`, append to `mod tests`, with the wiremock imports the two
tests need (Task 6's tests never make a call, so they did not import it):

```rust
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Review Focus 5: the result of a Test comes back through the client's own event channel and
    /// lands in the screen's notice line with the latency the call really took.
    #[tokio::test]
    async fn the_test_button_reports_a_real_answer_with_its_latency() {
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

        let config = mock_config(&server.uri());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config.clone(), path, keys, slot);
        client.state.open_config(form_of(&config));
        // Task 3's Providers tab lists the three capabilities in order, so its first row is chat's.
        // The assertion on `capability` below is what catches a different layout.
        client.state.config_form.as_mut().unwrap().field = 0;

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, mut results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Test), &to_server, &events);

        // The probe runs on a task of its own — the HUD never waits for a provider.
        let message = tokio::time::timeout(std::time::Duration::from_secs(5), results.recv())
            .await
            .expect("the probe must answer within five seconds")
            .expect("the client must still hold the channel");
        let ToClient::Event(Event::TestResult { capability, ok, latency_ms, message: text }) = &message else {
            panic!("a Test must answer with TestResult, got {message:?}");
        };
        // Copied out of the borrowed event: the message itself is moved into `on_message` below.
        let (capability, ok, latency_ms, text) = (*capability, *ok, *latency_ms, text.clone());
        assert_eq!(capability, Capability::Chat);
        assert!(ok, "the provider answered: {text}");
        assert!(latency_ms >= 20, "the latency must be the measured one, got {latency_ms}");

        client.on_message(message);
        let (kind, notice) = client.state.config_form.as_ref().unwrap().notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Ok);
        assert!(notice.starts_with("chat: "), "{notice}");
        assert!(notice.contains(&format!("{latency_ms} ms")), "{notice}");
    }

    /// "No key" is not "the provider refused": the row names a key nobody holds, so the answer comes
    /// back with no latency at all and no request is ever made.
    #[tokio::test]
    async fn a_test_for_a_row_with_no_key_answers_without_calling_anything() {
        let server = MockServer::start().await; // nothing may reach it
        let config = mock_config(&server.uri());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let keys = resolver(); // holds `openrouter` only
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config.clone(), path, keys, slot);

        let mut edited = config.clone();
        edited.providers.chat.key = "not-in-the-store".into();
        client.state.open_config(form_of(&edited));
        client.state.config_form.as_mut().unwrap().field = 0;

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, mut results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Test), &to_server, &events);

        let message = results.try_recv().expect("a row with no key is answered at once, with no call");
        let ToClient::Event(Event::TestResult { capability, ok, latency_ms, message: text }) = &message else {
            panic!("a Test must answer with TestResult, got {message:?}");
        };
        let (capability, ok, latency_ms, text) = (*capability, *ok, *latency_ms, text.clone());
        assert_eq!(capability, Capability::Chat);
        assert!(!ok);
        assert_eq!(latency_ms, 0, "no call was made, so there is no latency to report: {text}");
        assert!(
            text.contains("not-in-the-store") && text.contains("TIPSY_KEY_NOT_IN_THE_STORE"),
            "the answer names the key and how to provide it: {text}"
        );
        assert!(server.received_requests().await.unwrap().is_empty(), "no key must not become a request");

        client.on_message(message);
        let (kind, notice) = client.state.config_form.as_ref().unwrap().notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Error);
        assert!(notice.contains("not-in-the-store"), "{notice}");
    }
```

- [ ] **Step 6: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --lib client::tests::the_test_button` and
`cargo test -p tipsy-ui --lib config_form::tests::a_test_result`

Expected: FAIL — the targets do not compile:

```
error[E0599]: no method named `test_result` found for struct `ConfigForm`
   --> crates\tipsy-ui\src\config_form.rs
error[E0599]: no method named `test_provider` found for struct `Client`   (in the failing client tests)
error[E0004]: non-exhaustive patterns: `ConfigEffect::Test(_)` not covered
   --> crates\tipsy-compadre\src\client.rs
```

- [ ] **Step 7: Implement the notice and the client wiring**

In `crates/tipsy-ui/src/config_form.rs`, at the end of `impl ConfigForm`:

```rust
    /// Shows the answer to a `Test` request in the screen's notice line: green when the provider
    /// answered, red when it refused.
    ///
    /// The text is the capability's own name and what the probe said; the form adds no copy of its
    /// own, because it does not know which of the two string tables is in use (the plan's
    /// `ConfigForm` has no language field).
    pub fn test_result(&mut self, capability: Capability, ok: bool, latency_ms: u64, message: String) {
        let text = if ok {
            format!("{capability}: {latency_ms} ms — {message}")
        } else {
            format!("{capability}: {message}")
        };
        self.notice = Some((if ok { NoticeKind::Ok } else { NoticeKind::Error }, text));
    }
```

In `crates/tipsy-compadre/src/client.rs`:

- add `use tipsy_protocol::Capability;` to the protocol import (line 13) and `build_one` to the
  providers import;
- rename `on_config_effect`'s `_events` parameter to `events` (it is used from here on, and the
  underscore was only there to keep Task 6 warning-free), and replace the `ConfigEffect::Test(_) => {}`
  arm with `ConfigEffect::Test(capability) => self.test_provider(capability, events),`
- in `on_message`, before `self.state.apply(&event);` (line 174):

```rust
                // A Test result belongs to the screen's notice line, not to the HUD's log: the
                // answer stays beside the rows it is about. `UiState::apply` deliberately has
                // nothing to say about it.
                if let Event::TestResult { capability, ok, latency_ms, message } = &event {
                    if let Some(form) = self.state.config_form.as_mut() {
                        form.test_result(*capability, *ok, *latency_ms, message.clone());
                    }
                }
```

- and the method itself, beside `regenerate_token`:

```rust
    /// Runs the screen's Test against the row on screen and reports the answer into the client's own
    /// event channel, so it is rendered like anything else the session says.
    ///
    /// The connector is built *here*, before the call: a row that names a key nobody holds is then
    /// answered at once, with no latency and without a single request — which is what makes "no key"
    /// a different answer from "the provider refused". The probe itself runs on a task of its own,
    /// so the HUD never waits for a provider.
    fn test_provider(&mut self, capability: Capability, events: &UnboundedSender<ToClient>) {
        // The row the user is looking at, converted the way a save converts it: a Test can never
        // quietly use a connector other than the one on screen.
        let converted = self
            .state
            .config_form
            .as_ref()
            .and_then(|form| form.providers.iter().find(|row| row.capability == capability))
            .map(provider_config);
        let cfg = match converted {
            None => return,
            Some(Err(message)) => return self.config_notice(NoticeKind::Error, message),
            Some(Ok(cfg)) => cfg,
        };
        let connector = match build_one(&cfg, capability, &self.keys) {
            Ok(connector) => connector,
            Err(e) => {
                let _ = events.send(ToClient::Event(Event::TestResult {
                    capability,
                    ok: false,
                    latency_ms: 0,
                    message: e.to_string(),
                }));
                return;
            }
        };
        let language = self.config.active().language.clone();
        let events = events.clone();
        tokio::spawn(async move {
            let probe = connector.probe(&language).await;
            let _ = events.send(ToClient::Event(Event::TestResult {
                capability,
                ok: probe.ok,
                latency_ms: probe.latency_ms,
                message: probe.message,
            }));
        });
    }
```

- [ ] **Step 8: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --lib client::tests` and `cargo test -p tipsy-ui --lib config_form`

Expected: PASS — `client::tests` 10 tests (Task 6's 8 plus these two), `config_form`'s tests plus
`a_test_result_lands_in_the_notice_and_never_dresses_up_a_failure`, with the two client tests
appearing as

```
test client::tests::a_test_for_a_row_with_no_key_answers_without_calling_anything ... ok
test client::tests::the_test_button_reports_a_real_answer_with_its_latency ... ok
```

- [ ] **Step 9: Prove the whole workspace is still whole**

Run: `cargo test --workspace` — every crate green, `providers::probe_tests`' 7 and the two client
tests included. The `tipsy-ui` suite is the regression net for the notice: the reducer's own tests
still pass untouched, and nothing about `UiState::apply`'s `Event::TestResult` arm changed.

Run: `cargo clippy --workspace --all-targets` — no new warnings. No new dependency was added:
`wiremock`, `tempfile`, `futures` and `tokio` were all in the workspace already, and neither
`tipsy-ui` nor `tipsy-protocol` gained an import.

- [ ] **Step 10: Commit**

```bash
git add crates/tipsy-compadre/src/providers/mod.rs crates/tipsy-compadre/src/client.rs crates/tipsy-ui/src/config_form.rs
git commit -m "feat(config): the Test button probes a real call and tells the truth

Connector::probe makes the smallest call each capability allows — one delta of a
one-word chat request, a one-word synthesis, half a second of silence — and
answers with the latency it really measured, the provider's own words on a
refusal, and no latency at all when the row names a key nobody holds. A 200 whose
body carries no text is never a success. ConfigForm::test_result renders it in
the screen's notice line, green only when a provider answered.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 8: The detector `[mid]`

**Files:**
- Modify: `crates/tipsy-compadre/Cargo.toml:8-29` — one new line under `[dependencies]`, next to `futures = "0.3"`: `earshot = "1.2"`. It is the only dependency this plan adds, and the version is the one the VAD research note recommends (`.superpowers/sdd/plan-2-research/rust-vad.md:5`: "Use **`earshot = "1.2"`** (current release **1.2.2**, MIT OR Apache-2.0, edition 2024, rust-version 1.87)"). No feature is asked for: `alloc` is a default feature of 1.2.2.
- Create: `crates/tipsy-compadre/src/vad.rs` — `VadEvent`, `VadState` and its whole state machine
- Modify: `crates/tipsy-compadre/src/lib.rs:1-19` — one line, `pub mod vad;`, after `pub mod session;` (see "Why this task touches `lib.rs`")
- Test: `crates/tipsy-compadre/src/vad.rs` — the `#[cfg(test)] mod tests` this task creates
- Lock: `Cargo.lock` — commit it with the change: the new edge rewrites the `tipsy-compadre` entry and adds one `[[package]]` (`earshot` 1.2.2). Nothing else may move.
- Not touched: `crates/tipsy-compadre/src/session.rs`. Task 9 is where the session builds a `VadState` and gives it a say; this task stops at the module's boundary, so `vad.rs` is testable on its own (`cargo test -p tipsy-compadre --lib vad::` covers Steps 2–5 in full).

**Interfaces:**
- Consumes (read from the committed code, not recalled):
  - `pub struct Pcm { pub samples: Vec<f32>, pub sample_rate: u32 }` — `crates/tipsy-compadre/src/audio/pcm.rs:5-9`
  - `pub const MIC_SAMPLE_RATE: u32 = 16000;` — `crates/tipsy-compadre/src/audio/device.rs:17`
  - `pub const MIC_FRAME_SAMPLES: usize = (MIC_SAMPLE_RATE as usize) / 50;` = **320** samples (20 ms at 16 kHz) — `crates/tipsy-compadre/src/audio/device.rs:19`. **The client's `Frame::Mic` frames are 320 samples, not the 256 the plan's Interfaces section computes.** `VadState::push` therefore accepts any frame length and re-chunks it into earshot's 256-sample windows itself; see "The frame the client really sends".
  - The published crate, pinned exactly by Step 1: `Detector::<DefaultPredictor>::default_boxed() -> Box<Detector>` (`alloc` is a default feature), `Detector::predict_i16(&mut self, frame: &[i16]) -> f32` requiring **exactly 256 samples** and returning a score in `[0, 1]` (and `-1.0` for a wrong length), and `Detector::predict_f32(&mut self, frame: &[f32]) -> f32` with the same 256-sample rule. There is **no generic `predict` method, no `Frame` trait and no `FRAME_SIZE` constant in 1.2.2** — see "What Step 1 settles".
  - `pub fn spawn_with_events(config: Config, providers: Providers, brain_config: BrainConfig) -> SessionEnds` — `crates/tipsy-compadre/src/session.rs:49-89`
  - `pub struct SessionEnds { pub to_server: UnboundedSender<ToServer>, pub from_server: UnboundedReceiver<ToClient>, pub events: UnboundedSender<ToClient> }` — `session.rs:39-45`
  - `pub struct ListeningConfig { pub mode: ListeningMode, pub silence_ms: u32, pub mic_gain: f32 }`, `Default { mode: ListeningMode::Push, silence_ms: 1000, mic_gain: 12.0 }` — `crates/tipsy-compadre/src/config/model.rs:51-63`. Plan 2B Task 1 adds `mic_device: Option<String>` beside them; this task reads only `silence_ms`.
  - `pub enum ListeningMode { Push, Auto, HandsFree }` with `#[serde(rename_all = "snake_case")]` — `crates/tipsy-protocol/src/lib.rs:17-24`
- Produces (exactly the boundary the plan's "Interfaces this plan fixes" gives, with one documented refinement — `VadState` is generic over its scorer, and the default parameter makes `VadState` alone mean the real `earshot` detector):

```rust
/// What one pushed frame decided. `None` is every frame that changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent { None, SpeechStarted, SpeechEnded }

/// The score of one 256-sample window: `[0, 1]`, voice from 0.5 up.
pub type Scorer = fn(&[i16]) -> f32;

/// The real scorer: earshot's embedded network, built on first use.
#[derive(Debug, Default)]
pub struct Earshot { /* detector: Option<earshot::Detector> */ }

pub struct VadState<S = Earshot> { /* scorer, counters, hangover_frames, in_speech, paused, carry */ }

impl VadState {
    pub fn new(silence_ms: u32) -> VadState<Earshot>;
}

impl<S: FnMut(&[i16]) -> f32> VadState<S> {
    fn with_scorer(scorer: S, silence_ms: u32) -> Self;   // private: the tests reach it in-crate
    pub fn pause(&mut self);
    pub fn resume(&mut self);
    pub fn push(&mut self, frame: &[i16]) -> VadEvent;
    pub fn in_speech(&self) -> bool;
    pub fn silence_frames(&self) -> u32;                  // the hangover in frames — Task 9's tests count with it
}
```

  - `VadState` with no type parameter is the default scorer, exactly as the plan's Interfaces section writes it; the generic parameter exists only so Step 2's tests can hand in a scripted one, and it has a default, so no caller ever names it. Task 9 relies on: `VadState::new(config.listening.silence_ms)`, `push` for every `ToServer::Audio(Frame::Mic(..))` while the microphone is open, `pause`/`resume` around a turn and its 300 ms guard, and `silence_frames()` to know how many frames must reach the transcriber.

**What Step 1 settles, and the fallback it records**

The VAD research note recommends `earshot` but flags that it could not confirm the published API (`.superpowers/sdd/plan-2-research/rust-vad.md:89-91`): "**`predict_i16` / `predict_f32` are `#[deprecated]` on `main`** in favour of `predict<F: Frame>(frame) -> Score`, yet the 1.2.2 rustdoc page lists only the deprecated pair … **The plan must verify that `Frame` is implemented for `&[i16]` in the published 1.2.2**; if not, fall back to the present-but-deprecated `predict_i16`."

Step 1 settles it against the real crate, and the answer is in this task body:

- **1.2.2 has no `predict`, no `Frame`, no `Score` and no `FRAME_SIZE`.** Its whole public surface is `DefaultPredictor`, `Detector` and `Predictor` (docs.rs `all.html` for 1.2.2 lists exactly those three), and `Detector` exposes `const_default`, `default_boxed`, `new`, `new_boxed`, `reset`, `predict_i16`, `predict_f32` (docs.rs `Detector` for 1.2.2). So there is nothing to fall back *from*: **the deprecated pair the note worried about is neither deprecated nor a fallback in the published version — `predict_i16(&mut self, frame: &[i16]) -> f32` is the only i16 entry point and it is not deprecated.** The wrapper is one method, `Earshot::score`, whose body is `self.detector().predict_i16(frame)`, and no `#[allow(deprecated)]` is needed. (`Earshot` deliberately does **not** implement `Predictor`: that trait's three methods are `#[doc(hidden)]` and its own docs call the API "unstable and subject to change" — the free function `Earshot::score` is the whole adapter instead.)
- **If the implementer's checkout does carry the newer API** (the crate is at 1.2.2 today, but `cargo add earshot@1.2` could resolve a newer 1.2.x, and 1.3 could exist by execution time), `Earshot::score` is the one line that changes: if `predict`/`Frame`/`FRAME_SIZE` exist, call them there; if `predict_i16` is present but marked `#[deprecated]` in that version, put `#[allow(deprecated)]` on `Earshot::score` only and record it in the task report. Nothing else in this task and nothing in Task 9 depends on which of the two it is, because only `score` ever names the crate.
- **The hand-written energy/zero-crossing detector stays the documented fallback and is not implemented here.** `.superpowers/sdd/plan-2-research/rust-vad.md:29-41` describes what it would have to get right (adaptive noise floor updated only in silence, two-counter hysteresis, pre-roll so the first syllable survives, a DC block before the zero-crossing rate). It is only reached if Step 1 and Step 3 show that 1.2.2 cannot be fetched or does not classify a plainly voiced frame; a missing network at Step 2 is an environment problem to report, not a reason to write it.

**What this task decides, and why**

- **The crate answers only "voice or not" for one frame; everything else is ours.** `predict_i16` returns a score per 256-sample window and nothing else (`.superpowers/sdd/plan-2-research/rust-vad.md:5`: "It gives **per-frame scores only**: the 1000 ms hangover, the pause-while-speaking and the 300 ms post-TTS guard stay our own state machine (`earshot::segments` is for offline buffers, not this live gate)"). So `VadState` owns the onset counter, the hangover that turns `silence_ms` into `SpeechEnded`, the `paused` flag, and — because the client sends 320 samples and earshot demands 256 — the carry buffer that re-chunks them.
- **`push` accepts any frame length; the plan's "exactly one `Frame::Mic` worth" is what it means.** The plan's Interfaces section computes that frame as 256 samples, but the committed client is explicit: `MIC_FRAME_SAMPLES = MIC_SAMPLE_RATE / 50 = 320` (`audio/device.rs:19`), and `Mic::start` hands out exactly those frames (`device.rs:164-183`). A strict 256 would therefore never be satisfied by the real client and would need the same carry logic in the session instead — in the wrong layer, since the crate's window is the crate's business. `push` keeps the fixed signature `&[i16] -> VadEvent`, buffers the remainder of each frame, and calls earshot once per full 256-sample window. The wrong-length case is a *debug assertion inside earshot* (`predict_i16`, 1.2.2: `debug_assert_eq!(frame.len(), 256, "frame should be exactly 256 samples")`), so a test that feeds 255 samples and then one more is what proves we never hand it anything else.
- **A short frame is normal, not an error.** 320 = 256 + 64: the first `Frame::Mic` of a stream scores one window and carries 64 samples, the next one carries 128, and so on. A frame shorter than the 256 it takes to complete a window returns `VadEvent::None` without scoring — that is the `wrong_frame_size` test's first half — and must never panic.
- **`silence_frames` is part of the boundary because Task 9's tests need it.** A session test asserts how many samples reach the transcriber, and that number is `hangover + 1` frames after the last voiced one. Hard-coding it in the test would duplicate the arithmetic; asking the detector is the only way to keep the test honest if `silence_ms` changes.
- **The detector is generic over its scorer for exactly one reason: the state machine's tests are synthetic.** `VadState<S>` with `S = Earshot` by default changes no caller — `VadState::new(silence_ms)` is still the whole public constructor — and lets the six tests of the onset, the hangover, the pause and the re-chunking drive known scores through the state machine instead of waiting on a model's behaviour. The tests are pure and need no microphone, no device and no file.
- **The counter rules, stated so the tests are not guesses.** `onset_frames = 3` (48 ms): a single voiced frame is a chair creak, not speech, and `a_lone_loud_frame_never_becomes_an_utterance` pins that. `hangover_frames = silence_ms.div_ceil(16)`: the ceiling is what makes the budget *at least* `silence_ms` (1000 ms → 63 frames, 32 ms → 2 frames) and the tests use values where the ceiling is exact. `speech_run` is decremented rather than zeroed, so one dropped frame does not restart the confirmation (`.superpowers/sdd/plan-2-research/rust-vad.md:37`). `SpeechEnded` is emitted once, on the frame where the counter reaches the budget.
- **A pause throws away the half-heard utterance and does not pretend it ended.** `pause` sets `paused`, clears `in_speech` and both counters, and keeps the carry buffer: the samples that were waiting for a window are not a window, and they must not be scored against a stream the detector is not listening to. `resume` clears the counters again, so the frame after a resume starts a fresh onset search. `push` while paused consumes nothing and returns `None` — the crate is never called, because the assistant's own voice must not move a counter that decides when the user stopped talking. The 300 ms tail the spec asks for (§6: "Detection is paused while the session is speaking and for 300 ms afterwards") is the session's timer, not the detector's: `pause`/`resume` are the whole interface and Task 9 owns the 300 ms.

**Why this task touches `lib.rs`, not only `vad.rs`**

`lib.rs`'s module list (`crates/tipsy-compadre/src/lib.rs:1-19`) has no `vad` line, and without one `vad.rs` is a dead file — not compiled, not tested, and `cargo test --lib vad::` finds no test to fail. The declaration `pub mod vad;` is one line and it is what makes Steps 2–5 possible; Task 9 then reaches the same module through `use crate::vad::{VadEvent, VadState};`. **Nothing else changes in this task**: no mode behaves differently, no existing test moves, and `tests/session.rs`'s frozen assertions (`:234-263` especially) are untouched because nothing in the session consults the detector until Task 9 wires it in.

- [ ] **Step 1: Settle the published API against the real crate and add the dependency**

Run, from the workspace root:

```bash
cargo add earshot@1.2 -p tipsy-compadre
cargo fetch -p earshot
ls ~/.cargo/registry/src/*/earshot-1.2.*/src/
```

Expected: `Cargo.toml` gains exactly one line under `[dependencies]` — `earshot = "1.2"` — and `Cargo.lock` gains one `[[package]]` entry, `earshot 1.2.2`. `cargo fetch` needs the network once: `earshot` is **not** in this machine's registry cache and not in the lock today, so unlike Plan 2B Task 1's `getrandom` this step cannot be done offline. Then read the crate's own source, not its README and not this plan, and answer three questions in writing in the task report:

```bash
grep -n "pub fn predict\|pub fn default_boxed\|pub struct Detector\|deprecated" ~/.cargo/registry/src/*/earshot-1.2.*/src/lib.rs
grep -n "FRAME_SIZE\|pub trait Frame\|pub struct Score" ~/.cargo/registry/src/*/earshot-1.2.*/src/lib.rs
```

1. Which score entry point does the fetched version expose: `predict_i16`/`predict_f32`, or a generic `predict<F: Frame>`?
2. Is the entry point it exposes marked `#[deprecated]`?
3. Does the version expose `FRAME_SIZE` (256) or a `Frame`/`Score` pair?

The expected answers for 1.2.2 are `predict_i16`/`predict_f32`, **no `#[deprecated]` anywhere in the file**, and **no `FRAME_SIZE`, no `Frame`, no `Score`** — a `grep` that prints nothing for the second and third questions is the proof, and it is the finding the plan asked for. Record it; do not silently choose one and move on. If the fetched version *does* have the generic entry point, note that too and keep the same wrapper (`Earshot::score` is the only place that changes, as described above).

- [ ] **Step 2: Write the failing tests** — create `crates/tipsy-compadre/src/vad.rs` holding only the crate import and the test module below, and add `pub mod vad;` to `lib.rs` after `pub mod session;`. The type it names does not exist yet; that is the point of the step.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The synthetic frame every state-machine test pushes: a square wave with the amplitude of
    /// speech, so a real detector would call it voiced. Nothing here reads audio from a device.
    fn loud() -> Vec<i16> {
        (0..FRAME).map(|i| if i % 2 == 0 { 12_000 } else { -12_000 }).collect()
    }

    /// The frame with the amplitude of a quiet room.
    fn silent() -> Vec<i16> {
        vec![0; FRAME]
    }

    /// What the scorer was asked to score, so the re-chunking is observable. A `Vec` inside a
    /// `RefCell` and a `Cell` keep the scorer a plain `fn` pointer, which is what `type Scorer`
    /// is: the counters live beside the closure, not inside it.
    #[derive(Debug, Default)]
    struct Calls {
        scores: Vec<f32>,
        next: usize,
        sizes: std::cell::RefCell<Vec<usize>>,
    }

    impl Calls {
        /// The next scripted score, or `None` once the script runs out.
        fn next(&mut self) -> Option<f32> {
            let score = self.scores.get(self.next).copied();
            self.next += 1;
            score
        }
    }

    fn scripted(calls: &std::cell::RefCell<Calls>, frame: &[i16]) -> f32 {
        let mut calls = calls.borrow_mut();
        calls.sizes.borrow_mut().push(frame.len());
        calls.next().unwrap_or(0.0)
    }

    fn state(calls: &std::cell::RefCell<Calls>, scores: &[f32], silence_ms: u32) -> VadState<Scorer> {
        calls.borrow_mut().scores = scores.to_vec();
        VadState::<Scorer>::with_scorer(scripted, silence_ms)
    }

    /// `hangover_frames` is the silence budget in frames and `silence_frames()` reports it, so
    /// Task 9's tests can count what must reach the transcriber without duplicating the rounding.
    #[test]
    fn the_silence_budget_is_reported_in_frames() {
        assert_eq!(VadState::new(1000).silence_frames(), 63, "1000 ms of 16 ms frames");
        assert_eq!(VadState::new(32).silence_frames(), 2);
        assert_eq!(VadState::new(0).silence_frames(), 0, "zero means the frame that ends speech ends it");
    }

    /// Review Focus 2's first gate: a run of silent frames opens nothing and ends nothing, however
    /// long it lasts. It also proves the scorer is never asked for a verdict it should not give.
    #[test]
    fn silence_alone_never_speaks() {
        let calls = std::cell::RefCell::new(Calls::default());
        let mut vad = state(&calls, &[0.0, 0.1, 0.4], 32);
        for _ in 0..10 {
            assert_eq!(vad.push(&silent()), VadEvent::None);
        }
        assert!(!vad.in_speech(), "silence must not open an utterance");
        assert_eq!(calls.borrow().next, 3, "every full window is scored once: {calls:#?}");
    }

    /// Onset confirmation: one loud frame is a chair creak. `SpeechStarted` arrives once, on the
    /// third consecutive voiced frame, and the frames before it are not part of any utterance.
    #[test]
    fn speech_starts_once_on_the_third_voiced_frame() {
        let calls = std::cell::RefCell::new(Calls::default());
        let mut vad = state(&calls, &[0.9, 0.9, 0.9], 32);
        assert_eq!(vad.push(&loud()), VadEvent::None, "one frame is not speech yet");
        assert!(!vad.in_speech());
        assert_eq!(vad.push(&loud()), VadEvent::None, "two frames are not speech yet");
        assert_eq!(vad.push(&loud()), VadEvent::SpeechStarted, "the third confirms it");
        assert!(vad.in_speech());
        // The run is confirmed: further voiced frames are not another onset.
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(calls.borrow().sizes.borrow().len(), 4, "one score per full window");
    }

    /// The lone blip: an unconfirmed loud frame followed by silence never becomes an utterance and
    /// never ends one either — there was nothing to end.
    #[test]
    fn a_lone_loud_frame_never_becomes_an_utterance() {
        let calls = std::cell::RefCell::new(Calls::default());
        let mut vad = state(&calls, &[0.9, 0.0, 0.0], 32);
        let events: Vec<VadEvent> = vec![vad.push(&loud())]
            .into_iter()
            .chain(std::iter::repeat_with(|| vad.push(&silent())).take(2))
            .collect();
        assert_eq!(events, vec![VadEvent::None, VadEvent::None, VadEvent::None]);
        assert!(!vad.in_speech());
    }

    /// The hangover: `silence_ms` of 16 ms frames (32 ms -> 2 frames) after the last voiced frame
    /// turns into exactly one `SpeechEnded`, and the frame that ends it was scored.
    #[test]
    fn speech_ends_once_after_the_silence_budget() {
        let calls = std::cell::RefCell::new(Calls::default());
        let mut vad = state(&calls, &[0.9, 0.9, 0.9, 0.9, 0.0, 0.0, 1.0], 32);
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::SpeechStarted);
        assert_eq!(vad.push(&loud()), VadEvent::None, "still inside the utterance");
        assert_eq!(vad.push(&silent()), VadEvent::None, "1 of 2 silent frames");
        assert_eq!(vad.push(&silent()), VadEvent::SpeechEnded, "2 of 2 ends it");
        assert!(!vad.in_speech());
        assert_eq!(vad.push(&loud()), VadEvent::None, "a new utterance starts over from nothing");
        assert_eq!(vad.silence_frames(), 2);
    }

    /// The pause the spec asks for: frames are consumed, nothing is reported, and a half-heard
    /// utterance is abandoned rather than ended — the assistant's own voice must not produce a
    /// `SpeechEnded` that would close a turn. A resume starts the onset search again.
    #[test]
    fn a_pause_swallows_frames_and_a_resume_startsclean() {
        let calls = std::cell::RefCell::new(Calls::default());
        let mut vad = state(&calls, &[0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9], 32);
        assert_eq!(vad.push(&loud()), VadEvent::None);
        vad.pause();
        for _ in 0..5 {
            assert_eq!(vad.push(&loud()), VadEvent::None, "a paused detector never speaks");
            assert_eq!(vad.push(&silent()), VadEvent::None, "and never ends an utterance");
        }
        assert!(!vad.in_speech(), "the half-heard utterance was thrown away, not ended");
        let scored_before = calls.borrow().next;
        assert!(scored_before > 0, "the unpaused part of the run was scored");
        vad.resume();
        // After the resume the same three frames are what confirms speech, so nothing carried over.
        assert_eq!(vad.push(&loud()), VadEvent::SpeechStarted);
    }

    /// Frames are not always 256 samples — the client's are 320 (`audio/device.rs:19`) — so the
    /// detector does its own re-chunking: earshot is handed exactly 256 samples at a time, and a
    /// frame that does not complete a window is carried, not dropped and not scored.
    #[test]
    fn a_frame_that_is_not_a_whole_window_is_carried() {
        let calls = std::cell::RefCell::new(Calls::default());
        let mut vad = state(&calls, &[0.0, 0.0, 0.0, 0.0], 32);
        assert_eq!(vad.push(&vec![1000; 255]), VadEvent::None);
        assert_eq!(calls.borrow().sizes.borrow().len(), 0, "255 samples are not a window");
        assert_eq!(vad.push(&vec![1000; 1]), VadEvent::None, "the 256th completes it");
        assert_eq!(calls.borrow().sizes.borrow().as_slice(), &[256]);
        let filled = vad.push(&vec![1000; 320]);
        assert_eq!(filled, VadEvent::None);
        assert_eq!(calls.borrow().sizes.borrow().as_slice(), &[256, 256], "320 samples are one window plus a carry");
        assert!(calls.borrow().sizes.borrow().iter().all(|n| *n == 256), "earshot only ever sees 256");
    }

    /// The crate itself, on a window of the wrong length: this is what would panic inside
    /// `earshot::Detector::predict_i16` (a `debug_assert_eq!` on 256) if the re-chunking above
    /// were wrong. The whole state machine is by definition voice-then-silence, so the only thing
    /// asserted is that a run of it never panics and never ends more than once per silence budget.
    #[test]
    fn the_real_detector_survives_a_run_of_frames() {
        let mut vad = VadState::new(32);
        for _ in 0..4 {
            vad.push(&loud());
        }
        let mut endings = 0;
        for _ in 0..vad.silence_frames() + 2 {
            if vad.push(&silent()) == VadEvent::SpeechEnded {
                endings += 1;
            }
        }
        assert!(endings <= 1, "one budget of silence ends one utterance, got {endings}");
    }
}
```

- [ ] **Step 3: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --lib vad::`

Expected: FAIL — the module does not compile, because `vad.rs` names the type it will contain:

```
error[E0433]: failed to resolve: use of undeclared crate or module `earshot`
error[E0412]: cannot find type `VadState` in this scope
error[E0412]: cannot find type `Scorer` in this scope
error[E0599]: no function or associated item named `with_scorer` found for struct `VadState`
```

(`use super::*` is empty until the module's items exist; that is the expected failure. If `cargo add` fetched a version whose API differs from 1.2.2, Step 1's three answers are what the implementation below is adjusted against — nothing else.)

- [ ] **Step 4: Implement the detector**

Replace the top of `crates/tipsy-compadre/src/vad.rs` — everything above `#[cfg(test)]` — with this, keeping the module's tests exactly as Step 2 wrote them:

```rust
//! Voice activity detection over the microphone frames the client streams.
//!
//! `earshot` answers one question per 256-sample window — "is this voice?" — and nothing else. The
//! onset confirmation, the hangover that turns `silence_ms` into `SpeechEnded`, the pause around the
//! assistant's own speech and the re-chunking of the client's 320-sample frames into the crate's
//! 256-sample windows all live here, so the crate's API appears in exactly one function:
//! [`Earshot::score`].

/// Samples earshot scores per call: 256 = 16 ms at 16 kHz. Not a public constant of the crate in
/// 1.2.2, so it is ours; `predict_i16` rejects any other length.
const FRAME: usize = 256;

/// Frames at 16 kHz in one millisecond, or rather how many 16 ms frames a millisecond budget
/// needs: `silence_ms.div_ceil(16)`.
const FRAME_MS: u32 = 16;

/// Consecutive voiced frames before we call it speech: 3 x 16 ms = 48 ms. One frame is a chair
/// creak, a keyboard, a door; three in a row is a person starting a sentence.
const ONSET_FRAMES: u32 = 3;

/// What one pushed frame decided. `None` is every frame that changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent {
    /// A confirmed onset: the first frame of an utterance that is now open.
    SpeechStarted,
    /// The silence budget elapsed since the last voiced frame: the utterance is over.
    SpeechEnded,
    /// Nothing changed.
    None,
}

/// The score of one [`FRAME`]-sample window: `[0, 1]`, voice from 0.5 up.
///
/// A plain function pointer, so a test can drive the state machine with a known script — see
/// `VadState::with_scorer`. The default, [`Earshot`], is the real detector.
pub type Scorer = fn(&[i16]) -> f32;

/// The real scorer: earshot's embedded network.
///
/// The detector is about 8 KiB and is built on the first window it is asked for, so constructing a
/// `VadState` — which every session does, in every mode — costs a `None` and no stack.
#[derive(Debug, Default)]
pub struct Earshot {
    detector: Option<Box<earshot::Detector>>,
}

impl Earshot {
    /// earshot 1.2.2 exposes `predict_i16` and `predict_f32` and no generic `predict` (Step 1
    /// settles this against the fetched crate; the research note's `Frame`/`Score` pair exists only
    /// on `main`). This is the one place the crate is named, so a future version is a change here.
    fn score(&mut self, frame: &[i16]) -> f32 {
        // `default_boxed` keeps the detector's ~8 KiB off the stack; `alloc` is a default feature
        // of 1.2.2. `get_or_insert_with` is the lazy-init shape, so no `clippy::box_default`.
        let detector = self.detector.get_or_insert_with(earshot::Detector::default_boxed);
        detector.predict_i16(frame)
    }
}

/// Per-session detector state. Owned by the session task; not shared and not `Sync`.
///
/// The default scorer is the real detector, so a caller only ever writes `VadState::new(ms)`.
pub struct VadState<S = Earshot> {
    scorer: S,
    /// Consecutive voiced frames seen in the current run.
    speech_run: u32,
    /// Consecutive unvoiced frames seen since the last voiced one.
    silence_run: u32,
    /// The hangover, in frames: `silence_ms.div_ceil(16)`, at least as long as asked for.
    hangover_frames: u32,
    /// True while an utterance is open.
    in_speech: bool,
    /// True while the assistant speaks and while its 300 ms tail runs (Task 9 owns the timer).
    paused: bool,
    /// Samples that do not yet fill a [`FRAME`] window. The client sends 320 at a time, so this is
    /// 64 after the first frame and refills unevenly; dropping it would lose 64 samples of speech
    /// per frame.
    carry: Vec<i16>,
}

impl VadState {
    /// A detector for a session configured with `silence_ms` of silence to end an utterance.
    pub fn new(silence_ms: u32) -> Self {
        Self::with_scorer(Earshot::default(), silence_ms)
    }
}

impl<S: FnMut(&[i16]) -> f32> VadState<S> {
    /// The same detector over any scorer: the real one, or a script a test hands in. Private
    /// because `VadState::new` is the whole public surface; the tests reach it in-crate.
    fn with_scorer(scorer: S, silence_ms: u32) -> Self {
        Self {
            scorer,
            speech_run: 0,
            silence_run: 0,
            // The ceiling is what makes the budget *at least* `silence_ms`: 1000 ms becomes 63
            // frames = 1008 ms, and any budget shorter than one frame becomes one frame.
            hangover_frames: silence_ms.div_ceil(FRAME_MS),
            in_speech: false,
            paused: false,
            carry: Vec::with_capacity(FRAME),
        }
    }

    /// Stops deciding: the assistant is speaking, or its 300 ms tail has not elapsed. Frames are
    /// consumed and nothing is reported, and a half-heard utterance is abandoned rather than ended —
    /// a `SpeechEnded` here would close a turn on the assistant's own voice.
    pub fn pause(&mut self) {
        self.paused = true;
        self.in_speech = false;
        self.speech_run = 0;
        self.silence_run = 0;
    }

    /// Re-arms the detector and starts the onset search from nothing.
    pub fn resume(&mut self) {
        self.paused = false;
        self.in_speech = false;
        self.speech_run = 0;
        self.silence_run = 0;
    }

    /// True while an utterance is open: the samples pushed since `SpeechStarted` are its audio.
    pub fn in_speech(&self) -> bool {
        self.in_speech
    }

    /// The silence budget in frames: how many silent frames after the last voiced one report
    /// `SpeechEnded`. Task 9's tests count with this instead of repeating `div_ceil(16)`.
    pub fn silence_frames(&self) -> u32 {
        self.hangover_frames
    }

    /// Scores as many whole [`FRAME`]-sample windows as `frame` completes and reports what they
    /// decided. Any frame length is accepted: the client sends 320 samples (`MIC_FRAME_SAMPLES`),
    /// earshot wants 256, and the remainder waits here for the next frame.
    ///
    /// A pause swallows frames — it returns before the crate is called, so the assistant's own
    /// voice cannot move the counters that decide when the user stopped talking — and does not
    /// buffer them: they are not part of any utterance.
    pub fn push(&mut self, frame: &[i16]) -> VadEvent {
        if self.paused {
            return VadEvent::None;
        }
        self.carry.extend_from_slice(frame);
        let mut event = VadEvent::None;
        let mut done = false;
        let mut offset = 0;
        while self.carry.len() - offset >= FRAME && !done {
            let score = (self.scorer)(&self.carry[offset..offset + FRAME]);
            offset += FRAME;
            match self.decide(score) {
                // `SpeechEnded` wins the frame: it is the one event that closes a turn, and a
                // window cannot both start and end one.
                VadEvent::None => {}
                decided => {
                    event = decided;
                    done = decided == VadEvent::SpeechEnded;
                }
            }
        }
        self.carry.drain(..offset);
        event
    }

    /// The state machine for one window's score: confirmation, then the hangover.
    fn decide(&mut self, score: f32) -> VadEvent {
        // 0.5 is the crate's own documented split ("Scores over 0.5 can generally be considered
        // voice"). The scripted tests pass 0.0 and 0.9, so only a score exactly on the boundary
        // could tell the two conventions apart.
        let voiced = score >= 0.5;
        if self.in_speech {
            if voiced {
                self.silence_run = 0;
                return VadEvent::None;
            }
            self.silence_run += 1;
            if self.silence_run >= self.hangover_frames {
                self.in_speech = false;
                self.silence_run = 0;
                self.speech_run = 0;
                return VadEvent::SpeechEnded;
            }
            return VadEvent::None;
        }
        if !voiced {
            // Decrement rather than zero: one dropped frame in an otherwise voiced run must not
            // restart the confirmation.
            self.speech_run = self.speech_run.saturating_sub(1);
            self.silence_run += 1;
            return VadEvent::None;
        }
        self.speech_run += 1;
        self.silence_run = 0;
        if self.speech_run >= ONSET_FRAMES {
            self.in_speech = true;
            self.speech_run = 0;
            return VadEvent::SpeechStarted;
        }
        VadEvent::None
    }
}
```

Two notes for the implementer, both already reflected above: `VadState::with_scorer` is private but in-crate, which is why Step 2's `state()` helper can call it and the public API stays `VadState::new(silence_ms)`; and the `impl` block is bounded on `S: FnMut(&[i16]) -> f32` so that a closure and a plain `fn` pointer are both accepted — `VadState<Scorer>` in the tests names the function-pointer instantiation.
- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --lib vad::`

Expected: PASS, 8 tests:

```
running 8 tests
test vad::tests::a_frame_that_is_not_a_whole_window_is_carried ... ok
test vad::tests::a_lone_loud_frame_never_becomes_an_utterance ... ok
test vad::tests::a_pause_swallows_frames_and_a_resume_starts_clean ... ok
test vad::tests::silence_alone_never_speaks ... ok
test vad::tests::speech_ends_once_after_the_silence_budget ... ok
test vad::tests::speech_starts_once_on_the_third_voiced_frame ... ok
test vad::tests::the_real_detector_survives_a_run_of_frames ... ok
test vad::tests::the_silence_budget_is_reported_in_frames ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

`the_real_detector_survives_a_run_of_frames` is the one test that calls the crate. If it panics inside `predict_i16`'s `debug_assert_eq!(frame.len(), 256, ...)`, the re-chunking is wrong, not the crate. If Step 1 found a version whose API differs, adapt only `Earshot::score` and say so in the report.

- [ ] **Step 6: Run the whole crate**

Run: `cargo test -p tipsy-compadre`

Expected: all green — the new 8 plus every existing unit and integration test, `tests/session.rs`'s frozen assertions included, which is the proof that adding the field changes no existing behaviour.

Run: `cargo clippy -p tipsy-compadre --all-targets`

Expected: no warnings. `earshot` is a new dependency: `cargo tree -p tipsy-compadre -i earshot` must show exactly one edge, from `tipsy-compadre`, and `git diff Cargo.toml` exactly one added line.

- [ ] **Step 7: Commit the detector**

```bash
git add crates/tipsy-compadre/Cargo.toml Cargo.lock crates/tipsy-compadre/src/lib.rs crates/tipsy-compadre/src/vad.rs
git commit -m "feat(vad): add the earshot-backed detector behind our own state machine

earshot scores one 256-sample window and nothing else, so vad.rs owns the onset
confirmation, the hangover that turns silence_ms into SpeechEnded, the pause the
assistant's own speech needs, and the re-chunking of the client's 320-sample mic
frames. The published 1.2.2 exposes predict_i16/predict_f32 and no generic predict:
the wrapper is one function either way. The tests are synthetic and need no device.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: `auto` mode end to end `[mid]`

**Files:** (the line numbers are the committed `session.rs` as this task body was written — Tasks 1–8 land before this one, so re-read the file and match the function, not the number)
- Modify: `crates/tipsy-compadre/src/session.rs:9` — the import line gains `use std::time::{Duration, Instant};`
- Modify: `crates/tipsy-compadre/src/session.rs:20-26` — the `GUARD` constant beside `HISTORY_MAX_MESSAGES`
- Modify: `crates/tipsy-compadre/src/session.rs:68-80` — the `Session` literal gains `vad`, `heard`, `utterance` and `guard`
- Modify: `crates/tipsy-compadre/src/session.rs:101-115` — `Session` gains the four fields
- Modify: `crates/tipsy-compadre/src/session.rs:124-151` — the `select!` gains the guard timer's branch
- Modify: `crates/tipsy-compadre/src/session.rs:156-163` — `settle_finished_turn` arms the 300 ms guard, and the new `guard_elapsed` disarms it
- Modify: `crates/tipsy-compadre/src/session.rs:202-213` — `dispatch` routes a `Frame::Mic` by mode, and `detect`/`utterance_ended` are added below it
- Modify: `crates/tipsy-compadre/src/session.rs:251-264` — `listen_start` resets the detector's utterance state, `listen_stop` transcribes what the detector heard
- Modify: `crates/tipsy-compadre/src/session.rs:272-281` — `reset` clears the utterance, `heard` and the guard
- Modify: `crates/tipsy-compadre/src/session.rs:302-317` — `start_turn` pauses the detector for the length of the turn
- Test: `crates/tipsy-compadre/tests/session.rs:1-14, 189` — the `use` block gains nothing and the three new `#[tokio::test]`s are appended after `a_long_conversation_keeps_the_prompt_bounded` (`:303-332`); the fakes and helpers at `:16-187` are reused unchanged and the six existing tests are not touched
- Note: **`crates/tipsy-compadre/src/client.rs` needs no change and gets none.** Its `drain_mic` (`client.rs:211-216`) already forwards every captured 320-sample frame as `ToServer::Audio(Frame::Mic(..))` on the same channel the session reads, so the frames Task 8's detector wants are already arriving; `on_message`'s `ToClient::Audio(Frame::Mic(_)) => {}` arm (`client.rs:177`) is about the other direction. The 300 ms guard is a session timer, not a client one. This is the plan's File Structure line for `client.rs` being wrong, not a licence to edit the file.

**Interfaces:**
- Consumes:
  - `pub enum VadEvent { None, SpeechStarted, SpeechEnded }`, `pub struct VadState`, `VadState::new(silence_ms: u32)`, `pause(&mut self)`, `resume(&mut self)`, `push(&mut self, frame: &[i16]) -> VadEvent`, `in_speech(&self) -> bool`, `silence_frames(&self) -> u32` — Task 8's `crates/tipsy-compadre/src/vad.rs`
  - `pub fn spawn_session(config: Config, providers: Providers, brain_config: BrainConfig) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>)` — `session.rs:29-36`. The mode comes from `config.listening.mode`, so a test sets `config.listening.mode = ListeningMode::Auto` and the detector follows.
  - `fn dispatch(&mut self, message: ToServer)` — `session.rs:202-213`; today every `Frame::Mic` while `self.listening` goes into `self.mic`
  - `fn listen_stop(&mut self)` — `session.rs:257-264`; today it transcribes the whole buffer through `TurnInput::Utterance(Pcm { samples: i16_to_f32(&samples), sample_rate: MIC_SAMPLE_RATE })`
  - `fn start_turn(&mut self, input: TurnInput) -> Turn` — `session.rs:302-317`; the one place a turn begins, so the one place the detector can be told the assistant is speaking
  - `pub enum TurnInput { Ask(String), Utterance(Pcm), Say(String) }` — `crates/tipsy-compadre/src/brain.rs:45-49`; `TurnInput::Utterance` sends `AssistantState::Transcribing`, then transcribes, and an empty transcript is `ErrorCode::NothingHeard` (`brain.rs:75-94`)
  - `pub fn spawn_with_events(...) -> SessionEnds` and `pub struct SessionEnds { to_server, from_server, events }` — `session.rs:39-89`
  - `pub fn spawn_session(config: Config, providers: Providers, brain_config: BrainConfig)` and the fakes `FakeChat`/`FakeSpeech`/`FakeTranscribe` — `tests/session.rs:16-125`, especially `FakeTranscribe::pcms: Mutex<Vec<Pcm>>` (`:83-97`) and `send_mic(&tx, vec![100; 320])` (`:131-133`). **A new test may not weaken `listen_stop_transcribes_buffered_mic_audio` (`:234-263`)**: in `push` mode ten 320-sample frames must still reach the transcriber as one 3200-sample utterance.
- Produces: no new public API. The session's own, private shape:

```rust
struct Session {
    // ... as today ...
    /// The detector `auto` mode consults: Task 8's state machine and earshot behind it.
    vad: VadState,
    /// True from `VadEvent::SpeechStarted` until the frame that reports `SpeechEnded`: the frames
    /// in between are the utterance.
    heard: bool,
    /// The audio the detector confirmed, in arrival order. Only ever fed while `heard`.
    utterance: Vec<i16>,
    /// When the 300 ms guard after the assistant's speech expires; `None` when it is not running.
    guard: Option<Instant>,
}
```

  - Task 9 changes no wire shape, no protocol message and no dependency. `ListeningMode::HandsFree` keeps today's behaviour exactly (a wake word is Plan 4's): `ListenStart` opens the stream and `ListenStop` transcribes the buffer, and the detector is not consulted for it.

**What this task decides, and why**

- **`auto` mode transcribes what the detector confirmed, and nothing else.** `dispatch` routes a `Frame::Mic` by mode (`session.rs:202-213` is the whole routing):

```rust
            ToServer::Audio(Frame::Mic(samples)) => {
                if !self.listening {
                    // Frames arriving before `ListenStart` — or after `ListenStop` — belong to no turn.
                    return;
                }
                match self.listening_mode {
                    ListeningMode::Auto => self.detect(&samples),
                    // `push` and the not-yet-implemented `hands_free` (Plan 4) both hold everything
                    // the microphone sent and transcribe it when the user stops.
                    ListeningMode::Push | ListeningMode::HandsFree => self.mic.extend_from_slice(&samples),
                }
            }
```

  - `detect` is where Review Focus 2 is enforced, and it is deliberately one-directional: **until `SpeechStarted`, nothing is kept**, so the silence and the room noise before the first syllable can never reach the transcriber; **after `SpeechEnded`, `heard` goes false**, so the assistant's own voice and the silence it leaves behind cannot open a turn. The carry buffer inside `VadState` and this buffer are not the same thing — the first is samples waiting for a 256-sample window, the second is audio that has been confirmed as the user's.
- **`SpeechEnded` ends the utterance by handing the brain exactly what the detector heard.** `utterance_ended` takes `self.utterance`, converts it with the same `i16_to_f32` and `MIC_SAMPLE_RATE` `listen_stop` uses, and starts the turn the brain already understands: `TurnInput::Utterance(Pcm { .. })`. It is the same call `listen_stop` makes, so the fake `FakeTranscribe` sees the same kind of `Pcm` and `tests/session.rs`'s helper needs no new API.
- **The microphone stays open, and `Tab` still closes it.** The plan's task table says so ("`Tab` still stops"), and spec §6's table makes `ListenStart`/`Tab` the way `auto` starts: the detector ends the *utterance*, not the mode. So `listen_stop` keeps `self.listening = false` and keeps transcribing whatever the detector has confirmed but not yet ended — which is what a user pressing `Tab` mid-utterance means. The frozen test's ten frames arrive while `self.listening` is true in `push` mode into `self.mic`, exactly as today.
- **`VadState::pause` is called when a turn starts, not when the assistant starts speaking.** The speaker is audible from the moment a turn's first sentence is synthesised, and the turn is the only interval the session can see without the audio device's state (the client owns the device; the session never learns when it drained). Pausing at `start_turn` also covers synthesis and transcription, which is stricter than the spec and never lets a leak through. **The cost is deliberate and recorded here: a turn that is only thinking also mutes the microphone, so `auto` mode cannot capture a question asked while the assistant is composing a reply.** Barge-in is unaffected — `Tab` and a typed question both still work, and `Ask` still cancels the running turn.
- **The 300 ms guard is a real timer, because the turn ends before the speaker does.** The spec's tail (spec §6: "Detection is paused while the session is speaking and for 300 ms afterwards") starts when the turn's last `Frame::Speech` has gone to the client, and the reply's `Done` arrives while the device is still speaking it (`client.rs:269-279` records the same fact for playback). A deadline is therefore kept in `guard: Option<Instant>` — **`std::time::Instant` is fine here: the constraint that forbids it is `tipsy-ui`'s, and `session.rs` already uses `std::time`** — and the run loop gets one more `select!` branch: when the deadline passes, `resume()` is called once and the guard is cleared. `ListenStart` does **not** clear it: a `Tab` pressed inside the 300 ms must not let the assistant's voice into the utterance, so the microphone is open and the detector stays paused until the guard elapses. A command arriving at the same moment as the deadline is handled after it, which is correct — the guard has elapsed either way.
- **`reset` clears the utterance, the guard and `heard`, and resumes nothing.** A `Reset` while the assistant is silent leaves the detector paused until the next turn's guard elapses; that is one extra `settle_finished_turn` away and not worth a special case. A `Reset` is a fresh conversation, not a listener.

- [ ] **Step 1: Write the failing tests** — append these three tests, and the matcher they use, to `crates/tipsy-compadre/tests/session.rs`. Everything above the marker comment is untouched; `use` at `:1-14` already brings in `store`, `Config`, `spawn_session`, `Frame`, `ToServer`, `Event` and `Pcm`.

```rust
// ---------- auto mode ----------

/// A real config with the listening mode the test needs; the built-in `compae` profile and its
/// greeting are still present, which is why every test waits for the greeting's `Done` first.
fn auto_config(silence_ms: u32) -> Config {
    let dir = tempfile::tempdir().unwrap();
    let mut config = store::load_or_create(&dir.path().join("config.toml")).unwrap().config;
    config.listening.mode = ListeningMode::Auto;
    config.listening.silence_ms = silence_ms;
    config
}

/// The session ends and the transcribe fake for an `auto` run.
fn auto_session(
    chat: Arc<FakeChat>,
    speech: Arc<FakeSpeech>,
    transcript: &'static str,
    silence_ms: u32,
) -> (mpsc::UnboundedSender<ToServer>, mpsc::UnboundedReceiver<ToClient>, Arc<FakeTranscribe>) {
    let transcribe = Arc::new(FakeTranscribe {
        text: transcript,
        languages: Mutex::new(Vec::new()),
        pcms: Mutex::new(Vec::new()),
    });
    let providers = Providers { chat, speech, transcribe: transcribe.clone() };
    let (tx, rx) = spawn_session(auto_config(silence_ms), providers, BrainConfig::default());
    (tx, rx, transcribe)
}

/// A frame the detector calls voiced: a full-scale square wave. The client's own frames are 320
/// samples (`audio/device.rs:19`), and so are these, so the path is the one the app takes.
fn loud_frame() -> Vec<i16> {
    (0..320).map(|i| if i % 2 == 0 { 12_000 } else { -12_000 }).collect()
}

/// `MIC_FRAME_SAMPLES`: 20 ms at 16 kHz.
fn silent_frame() -> Vec<i16> {
    vec![0; 320]
}

/// Waits out the 300 ms guard the assistant's speech leaves behind, so a `ListenStart` that
/// follows is not swallowed by it.
async fn past_the_guard() {
    tokio::time::sleep(Duration::from_millis(400)).await;
}

fn is_heard(m: &ToClient) -> bool {
    matches!(m, ToClient::Event(Event::Heard { .. }))
}

/// Review Focus 2, first half: silence before the first syllable never opens a turn. Ten silent
/// microphone frames, then `Tab` — in `auto` mode the detector is the only thing that may start a
/// turn, so nothing is transcribed and no provider is called.
#[tokio::test]
async fn auto_mode_never_transcribes_silence_before_speech() {
    let chat = FakeChat::new(vec![]);
    let (tx, mut rx, transcribe) = auto_session(chat.clone(), FakeSpeech::new(), "no debería oírse", 100);
    next_matching(&mut rx, is_done, 500).await; // the greeting
    past_the_guard().await;

    send(&tx, Command::ListenStart);
    for _ in 0..10 {
        send_mic(&tx, silent_frame());
    }
    send(&tx, Command::ListenStop);

    let seen = drain(&mut rx, 300).await;
    assert!(
        !seen.iter().any(is_heard),
        "silence must not reach transcription: {seen:#?}"
    );
    assert!(transcribe.pcms.lock().unwrap().is_empty(), "no audio the detector did not hear may be transcribed");
    assert_eq!(chat.calls(), 0, "and nothing may ask the chat provider");
}

/// Review Focus 2, second half, end to end: a voiced run followed by enough silence ends the
/// utterance by itself, and exactly the audio the detector confirmed — the voiced run plus the
/// hangover, and not one sample of the silence before it or after it — is what reaches the
/// transcriber. `Tab` afterwards must not transcribe the same audio twice.
#[tokio::test]
async fn auto_mode_transcribes_only_the_audio_the_detector_heard() {
    let chat = FakeChat::new(vec![Ok(vec![S1])]);
    let (tx, mut rx, transcribe) = auto_session(chat, FakeSpeech::new(), "¿y el ticket?", 200);
    next_matching(&mut rx, is_done, 500).await; // the greeting
    past_the_guard().await;

    send(&tx, Command::ListenStart);

    // Ten frames of silence: nothing to hear, and nothing to keep.
    for _ in 0..10 {
        send_mic(&tx, silent_frame());
    }
    // A full second of voice: far past the three frames of onset confirmation.
    for _ in 0..50 {
        send_mic(&tx, loud_frame());
    }
    // The hangover, in frames, exactly what `VadState` says it needs — no magic number here.
    let budget = tipsy_compadre::vad::VadState::new(200).silence_frames() as usize;
    for _ in 0..budget {
        send_mic(&tx, silent_frame());
    }
    // Trailing silence after the utterance ended: it belongs to no turn and must be dropped.
    for _ in 0..5 {
        send_mic(&tx, silent_frame());
    }

    let heard = next_matching(&mut rx, is_heard, 1000).await;
    assert_eq!(heard, ToClient::Event(Event::Heard { text: "¿y el ticket?".into() }));

    let pcms = transcribe.pcms.lock().unwrap();
    assert_eq!(pcms.len(), 1, "one utterance, one transcription: {pcms:#?}");
    // The utterance is the voiced frames plus the frame that reports the end of the silence
    // budget — the detector hears the frame that ends it — and the ten leading silent frames and
    // the five trailing ones are not in it.
    let voiced = 50 * silent_frame().len();
    let hangover = budget * silent_frame().len();
    assert_eq!(
        pcms[0].samples.len(),
        voiced + hangover,
        "only the confirmed utterance may be transcribed: {pcms:#?}"
    );
    assert_eq!(pcms[0].sample_rate, 16000);
    assert!(
        pcms[0].samples[..voiced].iter().any(|s| s.abs() > 0.1),
        "the voiced frames are the bulk of it"
    );
    assert!(
        pcms[0].samples[voiced..].iter().all(|s| s.abs() < 0.01),
        "and the hangover is the silence that ended it"
    );
    drop(pcms);

    // `Tab` still stops the microphone by hand, and there is nothing left for it to add.
    send(&tx, Command::ListenStop);
    let after = drain(&mut rx, 300).await;
    assert!(!after.iter().any(is_heard), "the same audio must not be transcribed twice: {after:#?}");
    assert_eq!(transcribe.pcms.lock().unwrap().len(), 1);
}

/// The assistant's own voice never re-opens the microphone. A typed question is answered while the
/// stream is open; the detector is paused for the whole turn and for the 300 ms guard after it, so
/// no amount of noisy "room" during the reply becomes an utterance.
#[tokio::test]
async fn the_assistants_own_voice_never_reopens_the_microphone() {
    let chat = FakeChat::new(vec![Ok(vec![S1])]);
    let (tx, mut rx, transcribe) = auto_session(chat, FakeSpeech::new(), "no debería oírse", 100);
    next_matching(&mut rx, is_done, 500).await; // the greeting
    past_the_guard().await;

    send(&tx, Command::ListenStart);
    send(&tx, Command::Ask { text: "¿qué tal, compadre?".into() });
    // The reply is being spoken; the microphone picks up the room, and the room must be ignored.
    for _ in 0..40 {
        send_mic(&tx, loud_frame());
    }
    next_matching(&mut rx, is_done, 2000).await; // the reply ends
    let during = drain(&mut rx, 100).await;
    assert!(
        !during.iter().any(is_heard),
        "the assistant's own voice must not become an utterance: {during:#?}"
    );
    assert!(transcribe.pcms.lock().unwrap().is_empty());
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --test session auto_mode`

Expected: FAIL — in `auto` mode `dispatch` still appends every frame to `self.mic`, so `ListenStop` transcribes the silence and the assistant's reply re-opens nothing only by accident. The first two tests fail on their assertions, not at the compiler:

```
---- auto_mode_never_transcribes_silence_before_speech stdout ----
thread '...' panicked at crates\tipsy-compadre\tests\session.rs:...
silence must not reach transcription: [Event(Heard { text: "no debería oírse" }), ...]

---- auto_mode_transcribes_only_the_audio_the_detector_heard stdout ----
thread '...' panicked at ...:
no message matched within 1000 ms; received [..., Status { state: Listening, ... }]
```

The second failure is the honest one: nothing ends the utterance today, so no `Heard` ever arrives. `the_assistants_own_voice_never_reopens_the_microphone` may pass or fail depending on how the frames land; either way it is a test the mode must keep passing.

- [ ] **Step 3: Route the microphone frames by mode** — in `crates/tipsy-compadre/src/session.rs`, replace `dispatch` (`:202-213`) with:

```rust
    fn dispatch(&mut self, message: ToServer) {
        match message {
            ToServer::Command(command) => self.command(command),
            ToServer::Audio(Frame::Mic(samples)) => {
                // Frames arriving before `ListenStart` — or after `ListenStop` — belong to no turn.
                if !self.listening {
                    return;
                }
                match self.listening_mode {
                    // `auto`: the detector decides what is an utterance (spec §6), and only what it
                    // confirmed is ever transcribed.
                    ListeningMode::Auto => self.detect(&samples),
                    // `push`, and `hands_free` until Plan 4 implements it: the client's frames are
                    // the utterance, and `ListenStop` transcribes them.
                    ListeningMode::Push | ListeningMode::HandsFree => self.mic.extend_from_slice(&samples),
                }
            }
            ToServer::Audio(Frame::Speech { .. }) => {}
        }
    }

    /// Feeds one microphone frame to the detector and keeps only what it confirms.
    ///
    /// Review Focus 2 is this function: nothing before `SpeechStarted` is kept, so the silence and
    /// the room noise before the first syllable cannot reach the transcriber; and `SpeechEnded`
    /// closes the utterance and starts it, so the assistant's own voice — paused away by
    /// `start_turn` — cannot open one either.
    fn detect(&mut self, samples: &[i16]) {
        match self.vad.push(samples) {
            VadEvent::SpeechStarted => {
                self.heard = true;
                self.utterance.clear();
                self.utterance.extend_from_slice(samples);
            }
            VadEvent::SpeechEnded => {
                self.heard = false;
                // The frame that reports the end is the last of the hangover: it is part of the
                // utterance, which is why it is pushed before the turn starts.
                self.utterance.extend_from_slice(samples);
                self.utterance_ended();
            }
            VadEvent::None => {
                if self.heard {
                    self.utterance.extend_from_slice(samples);
                }
            }
        }
    }

    /// The detector says the user stopped talking: the audio it confirmed becomes the next turn.
    fn utterance_ended(&mut self) {
        let samples = std::mem::take(&mut self.utterance);
        if samples.is_empty() {
            // Nothing confirmed and nothing kept: no turn, and no transcription request for audio
            // nobody heard.
            return;
        }
        self.start(TurnInput::Utterance(Pcm {
            samples: i16_to_f32(&samples),
            sample_rate: MIC_SAMPLE_RATE,
        }));
    }
```

- [ ] **Step 4: Pause the detector for a turn and its 300 ms tail** — in the same file:

`Session` (`:101-115`) gains four fields and `spawn_with_events`'s literal (`:68-80`) their initial values:

```rust
struct Session {
    brain: Arc<Brain>,
    out: UnboundedSender<ToClient>,
    history: Vec<ChatMessage>,
    current: Option<Turn>,
    generation: u64,
    /// True between `ListenStart` and `ListenStop`.
    listening: bool,
    /// Mono 16 kHz samples gathered while listening, in `push` mode.
    mic: Vec<i16>,
    /// The `auto` mode detector, paused while the assistant speaks and for 300 ms after.
    vad: VadState,
    /// True from the frame that reports `SpeechStarted` until the one that reports `SpeechEnded`.
    heard: bool,
    /// The audio the detector confirmed: what `SpeechEnded` transcribes.
    utterance: Vec<i16>,
    /// When the 300 ms guard after the assistant's speech expires, if it is running.
    guard: Option<Instant>,
    profile: ProfileView,
    ui_language: UiLanguage,
    listening_mode: ListeningMode,
    greeting: String,
}
```

with, in `spawn_with_events` (`:68-80`), `vad: VadState::new(listening_silence_ms)` — `let listening_silence_ms = config.listening.silence_ms;` next to `let listening_mode = config.listening.mode;` (`:56`) — and `heard: false, utterance: Vec::new(), guard: None,`. The greeting is a turn like any other, so `start_turn` pauses the detector before the first frame could ever arrive.

`start_turn` (`:302-317`) pauses it, at the top:

```rust
    fn start_turn(&mut self, input: TurnInput) -> Turn {
        // The assistant is about to speak, and the speaker is what the detector would otherwise
        // hear. Pausing at the start of the turn also covers synthesis and transcription, which is
        // stricter than spec §6's "while the session is speaking" and never lets a leak through.
        self.vad.pause();
        self.heard = false;
        self.generation += 1;
        // ... unchanged ...
    }
```

`settle_finished_turn` (`:156-163`) starts the guard when a turn ends, and the run loop (`:124-151`) gets the timer's branch:

```rust
    /// Applies the history of a turn that has ended, if the running one has ended, and arms the
    /// 300 ms guard the detector must stay paused for afterwards.
    async fn settle_finished_turn(&mut self) {
        if self.current.as_ref().is_some_and(|turn| turn.handle.is_finished()) {
            let mut turn = self.current.take().expect("checked just above");
            let history = (&mut turn.done).await.ok();
            self.finish_turn(turn, history);
            self.guard = Some(Instant::now() + GUARD);
        }
    }

    /// The guard elapsed: the speaker is quiet, so the microphone is the user's again.
    fn guard_elapsed(&mut self) {
        self.guard = None;
        self.vad.resume();
    }
```

and, in `run`'s `select!` (`:125-147`), one branch after the command branch:

```rust
                deadline = async {
                    match self.guard {
                        Some(guard) => tokio::time::sleep_until(guard.into()).await,
                        // No guard runs: this branch can never win.
                        None => std::future::pending().await,
                    }
                } => {
                    let _ = deadline;
                    self.guard_elapsed();
                }
```

with the constant beside `HISTORY_MAX_MESSAGES` (`:26`):

```rust
/// The tail the detector stays paused for after the assistant stops speaking: spec §6's 300 ms.
/// The turn's `Done` arrives while the device is still speaking the last sentence, which is what
/// this is for.
const GUARD: Duration = Duration::from_millis(300);
```

and `use std::time::{Duration, Instant};` at the top of the file. `ListenStart` deliberately does not clear the guard: a `Tab` inside the 300 ms opens the microphone while the detector stays paused until the tail is over.

- [ ] **Step 5: Make `Tab` and `Reset` agree with the detector** — in the same file, replace `listen_start`, `listen_stop` and `reset` (`:251-281`) with:

```rust
    fn listen_start(&mut self) {
        self.listening = true;
        self.mic.clear();
        self.utterance.clear();
        self.heard = false;
        send(&self.out, status(AssistantState::Listening));
    }

    fn listen_stop(&mut self) {
        self.listening = false;
        self.mic.clear();
        // What the detector confirmed and has not ended yet is the utterance the user just closed
        // by hand. In `push` mode that is every frame the microphone sent.
        let samples = if self.listening_mode == ListeningMode::Auto {
            let confirmed = std::mem::take(&mut self.utterance);
            self.heard = false;
            confirmed
        } else {
            std::mem::take(&mut self.mic)
        };
        if samples.is_empty() {
            // Nothing was heard: no turn, and no transcription request for silence. Pressing `Tab`
            // twice in `auto` mode is the case this keeps free.
            self.idle();
            return;
        }
        self.start(TurnInput::Utterance(Pcm {
            samples: i16_to_f32(&samples),
            sample_rate: MIC_SAMPLE_RATE,
        }));
    }
```

`reset` (`:272-281`) additionally clears what the detector confirmed and the guard the turn left behind:

```rust
    fn reset(&mut self) {
        self.cancel_running();
        self.history.clear();
        self.mic.clear();
        self.utterance.clear();
        self.heard = false;
        self.guard = None;
        self.listening = false;
        // A turn that ignored the cancellation must not restore what this just cleared.
        self.generation += 1;
        self.idle();
    }
```

`cancel` (`:266-270`) is unchanged: the turn's own `settle_finished_turn` is what arms the guard, so a cancelled turn is followed by the same 300 ms as a finished one. **No `vad` call is needed in either `listen_stop` or `reset`**: `pause` and `resume` already clear the state machine's counters, `reset` clears the samples the detector confirmed, and a detector that resumes after a `Reset` simply re-adapts its score over the next frames — a fresh conversation is not a fresh audio stream.

- [ ] **Step 6: Run the new tests and watch them pass**

Run: `cargo test -p tipsy-compadre --test session`

Expected: PASS, 9 tests — the six that were already frozen, plus the three new ones:

```
running 9 tests
test a_long_conversation_keeps_the_prompt_bounded ... ok
test ask_produces_reply_and_done ... ok
test auto_mode_never_transcribes_silence_before_speech ... ok
test auto_mode_transcribes_only_the_audio_the_detector_heard ... ok
test barge_in_cancels_running_turn ... ok
test listen_stop_transcribes_buffered_mic_audio ... ok
test reset_clears_history ... ok
test the_assistants_own_voice_never_reopens_the_microphone ... ok
test welcome_then_greeting ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.05s
```

`listen_stop_transcribes_buffered_mic_audio` passing unchanged is the check that `push` mode is untouched — ten 320-sample frames, one 3200-sample `Pcm` at 16 kHz. If `auto_mode_transcribes_only_the_audio_the_detector_heard` fails on the sample count, the two likely causes are a `push` that keeps frames before `SpeechStarted` or a `VadState` whose `silence_frames()` the test did not use; neither is a reason to loosen the assertion, which is Review Focus 2.

- [ ] **Step 7: Run the workspace**

Run: `cargo test --workspace`

Expected: all green, the detector's 8 and the session's 9 included, with no test left `#[ignore]`d and no microphone opened. Nothing in this task needs an audio device: the frames are `Vec<i16>`s the test constructs and the fakes are `tests/session.rs`'s own.

Run: `cargo clippy --workspace --all-targets`

Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/tipsy-compadre/src/session.rs crates/tipsy-compadre/tests/session.rs
git commit -m "feat(session): make auto mode end the utterance by itself

In auto mode the detector decides what is an utterance: silence before the first
voiced frame is never kept, SpeechEnded hands the confirmed audio to the brain as
a TurnInput::Utterance, and the detector is paused for the whole turn plus spec 6's
300 ms guard afterwards, so the assistant's own voice cannot re-open the microphone.
Tab still stops the microphone by hand. The client needs no change: it already
forwards every mic frame the detector wants.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: The README and the plan's own review `[small]`

**Files:**
- Modify: `README.md`
- Modify: `crates/tipsy-ui/src/state.rs` (the notice for a mode that is not implemented yet)
- Modify: `crates/tipsy-ui/src/strings.rs` (that notice's copy, in both tables)
- Test: the `#[cfg(test)]` module of `crates/tipsy-ui/src/state.rs`

**Interfaces:**
- Consumes: the key binding and the screen from Task 5, the save/apply path from Task 6, the Test button from Task 7, `auto` mode from Task 9, and the strings from Task 3.
- Produces: nothing code-facing beyond the notice; this task closes the plan's documentation.

- [ ] **Step 1: Write the failing test for the unimplemented mode**

The configuration accepts three listening modes, and this plan implements two: `push` and `auto`. A user who writes `mode = "hands_free"` today gets whatever the session does with an enum value nothing handles — silence, or push behaviour with no explanation. That is the gap this step closes, with the honest answer: the wake word arrives in a later plan, and until then the client says so.

```rust
#[test]
fn hands_free_says_it_is_not_implemented_instead_of_going_quiet() {
    let mut state = UiState::new();
    state.apply(&Event::Welcome {
        session_id: 1,
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: UiLanguage::Es,
        listening_mode: ListeningMode::HandsFree,
        web_bundle: false,
    });
    assert!(state.notice.is_some(), "an unimplemented mode must say so");
    let (notice, _) = state.notice.as_ref().expect("set above");
    assert!(notice.contains("manos libres") || notice.contains("hands free"), "{notice}");
    assert!(!state.mic_open, "and it must not pretend to be listening");
}

#[test]
fn the_other_two_modes_raise_no_notice() {
    for mode in [ListeningMode::Push, ListeningMode::Auto] {
        let mut state = UiState::new();
        state.apply(&Event::Welcome {
            session_id: 1,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: UiLanguage::En,
            listening_mode: mode,
            web_bundle: false,
        });
        assert!(state.notice.is_none(), "{mode:?} is implemented and must stay quiet");
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p tipsy-ui state::tests::`
Expected: FAIL — `an unimplemented mode must say so` (`notice` is `None`).

- [ ] **Step 3: Set the notice and write the README**

In `tipsy-ui/src/state.rs`'s `Welcome` handling, set the notice (through the string table, so both languages carry it — add the key to `Strings` in Task 3's two tables if it is not there yet) when `listening_mode` is `HandsFree`; leave the other two alone.

Then update `README.md`: the quickstart gains the configuration screen (`F9` or the header's ⚙, its four tabs, "changes apply on save"), the keys table gains the screen's keys and the save key, the modes are described truthfully — `push` and `auto` work, `hands_free` says what it is waiting for — and nothing claims a feature the code does not have. **It must also say which changes apply live and which wait for the next launch**: the active profile, the listening mode and the connectors switch immediately, while a profile's edited texts and the microphone device take effect when the app is restarted (the protocol carries a profile *id*, not its prompt, and `Mic::start` takes no device). Keep the keys table's shape and the document's voice; the API-key rule (names, never values) and the per-minute warning patterns elsewhere in the README stay as they are.

- [ ] **Step 4: Run everything and check the README against the code**

Run: `cargo test --workspace`
Expected: all green, no warnings.

Then read your own README change against `crates/tipsy-compadre/src/main.rs`, `client.rs`, `tipsy-ui/src/strings.rs` and `config/model.rs`: every command, key and default it names must exist as written. Say in your report which lines you checked against which file.

- [ ] **Step 5: Commit**

```bash
git add README.md crates/tipsy-ui/src/state.rs crates/tipsy-ui/src/strings.rs
git commit -m "docs: document the configuration screen and the listening modes

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Then the process's single whole-branch review runs before merging (subagent-driven-development's final review, with the standing ruling of no per-task reviews).

---

## Coordinator notes

- **Task 1 also updates the design doc.** `server.token` and `listening.mic_device` change the configuration file's shape, so §8.1's example in `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md` gains both keys, each with a one-line comment. Task 1's Files block and its commit's staged paths include that file; nothing else in the spec changes in this plan.
- **`Capability` already exists.** Plan 2A's Task 5 landed it in `tipsy-protocol` (commit `cb4d469`): `pub enum Capability { Chat, Speech, Transcribe }` with the crate's serde style, `Display` beside it, and the wire spelling `transcribe` for the third variant (the human-facing word is "transcription"). Task 2 consumes it — it adds `Command::TestProvider` and `Event::TestResult` that *use* `Capability` — and must not declare a second one.
- **The detector's crate API is settled.** Task 8's first step checked the published `earshot` 1.2.2 against docs.rs: its public surface is `DefaultPredictor`, `Detector` and `Predictor`, with `predict_i16(&mut self, &[i16]) -> f32` and no generic `predict`, no `Frame` trait, no `Score` and nothing deprecated — so the research note's API fear was unfounded and `predict_i16` is called directly, with the one-method adapter the fallback if a later release changes it. `earshot` is **not** in this machine's cargo registry cache, and a sandboxed shell **cannot reach crates.io at all** — measured, not assumed: `cargo add earshot@1.2 --dry-run` in the sandbox fails with `schannel: AcquireCredentialsHandle failed: SEC_E_NO_CREDENTIALS`. So the fetch happens **before Task 8 is dispatched**, in one of two ways: the controller runs `cargo add earshot@1.2 -p tipsy-compadre` under a one-shot full-access approval (which also warms the registry cache, so the confined build afterwards resolves offline), or the user runs the same command in their own terminal. Hand-writing an energy detector to dodge this is the last resort the note documents, never the first.
- **Two corrections to this document from that task's author, recorded here so the plan does not lie:** the detector re-chunks frames internally (the client's are 320 samples, fixed above), and Task 8 also declares `pub mod vad;` in `lib.rs` — without it the new file would not be compiled. Task 9 needs no change in `client.rs`, because the mic bridge already forwards every frame and the 300 ms guard is a session timer.
- **Task 2 touches four files, not one.** `Command` and `Event` are matched exhaustively in `session.rs`, `client.rs` and `tipsy-ui/src/state.rs`; Task 2 adds one no-op arm per site so the workspace keeps compiling, and Task 7 replaces the client/session arms if the session ends up owning the probe.
- **Interface decisions Tasks 3–5 had to make, now pinned by their tests** (the plan left them open and they are the contract for Tasks 6–10): the Providers tab is fifteen rows — three capabilities × (connector, base URL, model, key name, Test), capability-major — and the Test button is one of those rows, because the click areas come from the row list; `Ctrl+S` and `Ctrl+R` arrive as `KeyInput::Char('\u{13}')` and `Char('\u{12}')` (the client maps any CONTROL+char to `c & 0x1f`), so `KeyInput` gains no variant and the token can still be regenerated from the keyboard; a row's `y` is `3 + its index`; and `ConfigForm::apply` returns an empty vector rather than ever constructing `ConfigEffect::None`, which therefore stays unused in that enum.
- **Task 2's body is stale on one point, and its dispatch must say so:** it was written before Plan 2A landed `Capability` in `tipsy-protocol`, so its Steps still tell the implementer to declare the enum. It already exists (`tipsy-protocol/src/lib.rs`, with `Display` beside it and the wire spelling `transcribe`); Task 2 adds only `Command::TestProvider` and `Event::TestResult` and must not declare a second one. The same body's quoted line numbers also predate the tree.
- **Shared names across Task 3 and Task 4:** Task 4's test module imports `SERVER_ROW_SAVE` and `VOICE_LABEL_ROW` from Task 3, so renaming either means editing both in lockstep. And `ConfigForm::string_keys()`'s non-blankness check only covers the keys it returns: a screen string added to `Strings` and forgotten there slips through silently.
- **Ruling on Tasks 6–7's three deviations, all accepted:** (1) Task 6 leaves `config/store.rs` alone — `save`'s existing serialise-then-write order is exactly what proves Review Focus 1, and a temp-file-and-rename hardening would make a read-only *file* succeed on Unix and quietly invalidate that test; the hardening goes to the release plan, whose test must inject the failure with a read-only **directory** instead, since this plan's read-only-file injection assumes a non-root user and would behave differently in the Linux CI. (2) Task 6 adds `ProviderSlot` to `providers/mod.rs`, which the task table had given to Task 7: the slot belongs beside `Providers`, the two tasks run in sequence and there is no conflict — the table's attribution was a modelling slip, not a scope change. (3) The screen's notices are built from data (a config path, an OS error, the row that would not parse, the provider's own words) and the fixed `ConfigForm` carries no language field, so their prefix is not localised yet: the labels are localised and the data is data, and the localised prefixes arrive when a later pass adds the `Strings` field — cost if wrong: a Spanish user reads a short English prefix before the data.
- **A deviation from spec §9's "changes apply on save, without restarting", recorded on purpose:** the active profile, the listening mode and the connectors apply live, while a profile's edited *texts* and the microphone device take effect at the next launch, because the protocol carries a profile id and not its prompt, and `Mic::start` takes no device. The plan's own decision scoped it exactly that way, and Task 10's README states which change applies when, so the user is never surprised. — cost if wrong: someone edits a prompt or picks a device and has to restart to hear it, which the README says out loud.
- **Also carried into the dispatches:** `SetProfile` and `SetListeningMode` now re-announce `Welcome`, so Task 5's UI must tolerate a second `Welcome` (the state reducer must not treat it as a fresh session); and the Test probe's "one-token chat request" is in practice a one-word prompt, because `ChatRequest` carries no token cap and the probe reads a single delta.
