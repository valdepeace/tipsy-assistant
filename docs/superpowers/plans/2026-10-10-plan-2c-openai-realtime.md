# Plan 2C — OpenAI Realtime voice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a realtime voice mode: one duplex WebSocket session with OpenAI's Realtime API in which the microphone audio goes up and the model's speech comes down as it is generated, the server segments the turns, the user can interrupt it mid-sentence, and the terminal HUD shows the transcript of what was heard and answered.

**Architecture:** A new `crates/tipsy-compadre/src/realtime/` module — a socket client over `tokio-tungstenite`, a pure mapping between the wire's events and our actions, and a session façade the client drives. While a realtime *call* is active the client keeps one long-lived socket open; the existing `Player` plays the audio deltas, the existing protocol events (`Heard`, `ReplyDelta`, `Reply`, `Status`) carry the transcripts, and `tipsy-ui` gains only a live indicator. Realtime is a **mode of the client, not a fourth provider capability**: the `[realtime]` section of `config.toml` configures it, and one key starts and ends a call.

**Tech Stack:** the workspace as Plans 2A and 2B leave it, plus **`tokio-tungstenite` 0.30** with its `rustls-tls-native-roots` feature — the project's first WebSocket client, chosen over `native-tls` to avoid a native dependency on Windows — and the existing `audio::pcm` helpers for the 16 kHz ↔ 24 kHz conversion. The HTTP connectors keep `wiremock`; the realtime tests use a **local fake socket server** built from the same crate.

**Spec:** `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md` — §5 (the turn pipeline this mode sits beside), §6 (listening modes), §7 (providers and the connector table), §9 (the interface and its keyboard/mouse rule), §12 (errors).

## Research inputs

- `.superpowers/sdd/plan-2c-research/openai-realtime.md` — endpoints, auth, model ids, the event `type` strings, the audio format, the interruption bookkeeping, the crate recommendation and the cost model. **It also marks a payload uncertainty this plan must respect:** `response.output_audio.delta` and `response.output_audio_transcript.delta` are confirmed *type strings* whose exact payload fields were inferred from a documentation page that truncated, so every task pins them with a fixture and says so out loud, and a live-key smoke test of one spoken exchange is a release-blocking item.

## Decisions this plan makes, and why

- **Realtime is a client mode, not a provider kind.** `Capability` stays `chat`/`speech`/`transcribe` and `build_providers` is untouched: the pipeline's three capabilities describe request/response text and audio, while this is a duplex conversation that replaces the whole pipeline for as long as it lasts. Making it a kind would force the brain's turn model to pretend it is a chat stream.
- **Auth uses the ordinary key resolution.** The `[realtime]` section names a key (`key = "openai"`), and `KeyResolver` finds it in `TIPSY_KEY_OPENAI` or the credential store. The ephemeral-secret exchange the research describes exists for browsers that must not see the key; a desktop process that already holds it does not need it.
- **One socket per call**, opened by the key binding and closed by it, by `Esc`, or when the call's own limit is reached. The API caps a session at 60 minutes, so a long call reopens rather than dying.
- **The cost is never silent.** A streaming socket bills audio input while it is open — the research's rates put it near $0.32 per minute of listening and $1.28 per minute of speech — so the mode defaults to disabled, the HUD shows an unmistakable live indicator for as long as a call lasts, `[realtime] max_minutes` (default 10) ends a call by itself, and the README states the rates. The cheaper push-to-talk variant (framing the input by hand instead of streaming an open microphone) is documented as a future option, not implemented here.
- **Server-side turn detection**, configured with `[listening] silence_ms`: the server decides when the user stopped speaking. The local VAD Plan 2B adds is deliberately *not* a dependency of this plan.
- **Barge-in is signalled by the server and acted on locally.** On `input_audio_buffer.speech_started` the client stops playback, computes the milliseconds actually played (48 bytes per millisecond of 24 kHz mono PCM16, per the research), and sends `conversation.item.truncate` with `item_id`, `content_index: 0` and `audio_end_ms`; the arithmetic is a pure function with its own test, because telling the model it heard more than the user heard corrupts the conversation.
- **The transcript is the HUD's text**: the input transcript arrives as `Heard`, the output transcript as `ReplyDelta` and then `Reply`, so everything downstream of the session — the log, the tabs, the selection — behaves exactly as in the text pipeline. `tipsy-ui` gains the indicator and the strings for it, and nothing else about it changes.
- **The profile supplies `instructions`** (its `prompt`) and the *voice* comes from `[realtime] voice` rather than from the profile's `voices` table, because that table maps speech connectors to voices and this is not one of them. The plan records that as a deliberate asymmetry.
- **An unanticipated event is ignored, never fatal.** The research's page truncated; a realtime API that grows an event we do not know must not end the call. A malformed *known* event is a typed error that ends the call with a readable message.

## Global Constraints

- Toolchain pinned by `rust-toolchain.toml` to `1.96.1`; every crate uses `edition = "2024"`.
- All code, identifiers, comments, docs and commit messages are in **English**. Interface strings keep the two static es/en tables.
- `tipsy-protocol` and `tipsy-ui` must NOT depend on `tokio`, `cpal`, `rodio`, `keyring`, `sysinfo`, `reqwest` or `crossterm`, and must keep compiling to `wasm32-unknown-unknown`: the socket lives in `tipsy-compadre`, and anything the HUD learns arrives as a protocol event.
- **One new dependency, named:** `tokio-tungstenite` 0.30 with `rustls-tls-native-roots`, for the duplex socket. Nothing else is added.
- API keys never appear in logs, events or test fixtures; the socket's handshake carries the key and nothing prints it. The `Authorization` header must not be logged even at debug level.
- Mic audio is mono 16 kHz; the Realtime API takes and returns mono 24 kHz PCM16. The conversion is explicit at the boundary and its two directions have their own tests.
- Timeouts: the existing 30 s chat / 30 s speech / 60 s transcription values do not apply to a duplex socket; a realtime call has an explicit open timeout and a keepalive, and its limits come from `[realtime] max_minutes` and the API's own 60-minute cap.
- Every commit message ends with the trailer line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A socket that dies mid-answer must not leave the user in the dark.** The HUD must leave its live state, playback must stop, the log must show a readable error, and the session must return to a state where the next turn works — whether the death is a clean close, an error event, or a transport failure. Task 2 and Task 4 own the tests.
2. **Truncation must never overstate what the user heard.** The milliseconds sent to the model are derived from bytes actually played since the last spoken item began; an off-by-48 error silently corrupts the conversation's memory. Task 5 owns the arithmetic's tests and the boundary cases (nothing played yet, a truncated delta, a second barge-in).
3. **The mode must never be silently on.** A call can only start from an explicit key press with `[realtime] enabled = true`, the indicator is visible for the whole call, and the call ends by itself at `max_minutes`. Task 1 and Task 6 own those tests.
4. **An unknown event is data, not death.** The socket's reader must ignore an event type it does not know, and a *known* event with a malformed payload must produce a readable error rather than a panic. Task 3 owns both.
5. **16 kHz and 24 kHz must never be confused.** The microphone's frames are resampled up before they are sent, the model's audio is decoded at its own rate, and the player is told the rate that arrives — a silent rate mismatch is inaudible in tests and obvious to the user's ears. Task 4 owns the round-trip test.

## File Structure

```
crates/tipsy-protocol/src/lib.rs          MODIFY: the live-call event the HUD shows during a call
crates/tipsy-compadre/src/config/model.rs MODIFY: the [realtime] section, its defaults and its serde defaults
crates/tipsy-compadre/src/realtime/
├── mod.rs                                 NEW: the call façade (open, close, turn plumbing, the public API)
├── socket.rs                              NEW: the tokio-tungstenite client, auth, keepalive, reopen, clean close
└── events.rs                              NEW: the pure mapping between wire events and our actions
crates/tipsy-compadre/src/client.rs        MODIFY: the key binding, the mic routing, delta playback, the indicator
crates/tipsy-compadre/src/audio/device.rs  MODIFY: only if playing a 24 kHz stream needs a note or a parameter
crates/tipsy-ui/src/state.rs               MODIFY: the live flag and its reducer
crates/tipsy-ui/src/strings.rs             MODIFY: the live indicator's es/en copy
crates/tipsy-ui/src/render.rs              MODIFY: the header indicator
crates/tipsy-compadre/tests/realtime.rs    NEW: the local fake socket server and the scripted exchanges
```

## Tasks

| # | Task `[tier]` | Deliverable | Files | Depends on |
|---|---|---|---|---|
| 1 | Config and protocol `[small]` | The `[realtime]` section (enabled, model, key name, voice, max_minutes) with defaults for old files, plus the protocol's live variant, both with tests | `config/model.rs`, `tipsy-protocol/src/lib.rs` | — |
| 2 | The socket client `[mid]` | Connect with Bearer and the model query, send `session.update`, read typed frames, keepalive, reopen at the 60-minute cap, close cleanly | `realtime/socket.rs` | 1 |
| 3 | The event mapping `[mid]` | Pure JSON ⇄ action for the events the research confirms, unknown types ignored, malformed known types typed as errors | `realtime/events.rs` | — |
| 4 | The turn plumbing `[mid]` | A call: mic audio in (16→24 kHz), audio deltas out through `Player`, transcripts as `Heard`/`ReplyDelta`/`Reply`, status in the HUD | `realtime/mod.rs`, `client.rs` | 2, 3 |
| 5 | Barge-in `[mid]` | Stop playback on `speech_started`, the truncate arithmetic, `response.cancel` on a local interrupt, the item-id bookkeeping | `realtime/mod.rs`, `realtime/events.rs` | 4 |
| 6 | The interface, the docs and the review `[small]` | The live indicator with its strings, the key binding, the README's cost warning and the release checklist item | `tipsy-ui/*`, `client.rs`, `README.md` | 1–5 |

**After Task 6:** the process's single whole-branch review runs before merging (subagent-driven-development's final review, with the standing ruling of no per-task reviews), and the live-key smoke test of one spoken exchange is added to the release checklist rather than attempted here.

### Task 1: Config and protocol `[small]`

**Files:**
- Modify: `crates/tipsy-compadre/src/config/model.rs:8-36` — `Config` gains `realtime`, `Config::default()` fills it
- Modify: `crates/tipsy-compadre/src/config/model.rs:113-129` — insert `RealtimeConfig` after `ProvidersConfig` (line 129) and before `Profile` (line 131)
- Modify: `crates/tipsy-protocol/src/lib.rs:113-132` — `Event` gains `Live { active: bool }` after `Error`
- Modify: `crates/tipsy-ui/src/state.rs:225` — one behaviour-free arm in `UiState::apply`, the one exhaustive match over `Event` (Step 6); `session.rs:216` and `client.rs:286` are *not* touched, because their matches are over `Command`
- Test: `crates/tipsy-compadre/src/config/model.rs:179-220` — the existing `#[cfg(test)] mod tests`
- Test: `crates/tipsy-compadre/src/config/store.rs:59-211` — the existing `#[cfg(test)] mod tests`
- Test: `crates/tipsy-protocol/tests/roundtrip.rs:28-46` — appends one test after `events_roundtrip`
- Modify: `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md:244-285` — §8.1's example file gains the section (Step 8)

**Interfaces:**
- Consumes (read from the committed code, not recalled):
  - `pub struct Config { pub active_profile: String, pub server: ServerConfig, pub listening: ListeningConfig, pub providers: ProvidersConfig, pub profiles: Vec<Profile> }` with `#[serde(default)]`, and `Config::default()` with `active_profile: "compae".into()` — `config/model.rs:8-36`
  - `pub struct ListeningConfig { pub mode: ListeningMode, pub silence_ms: u32, pub mic_gain: f32 }`, `Default { mode: ListeningMode::Push, silence_ms: 1000, mic_gain: 12.0 }` — `config/model.rs:51-63`. This `silence_ms` is what turn detection reuses.
  - `pub struct ProviderConfig { pub kind: ProviderKind, pub base_url: String, pub model: String, pub key: String }` with the comment `/// Name of the key, never the key itself.` — `config/model.rs:74-82`
  - `pub fn load_or_create(path: &Path) -> std::io::Result<LoadOutcome>` and `pub fn save(path: &Path, config: &Config) -> std::io::Result<()>` — `config/store.rs:20-30`; `LoadOutcome { config, warning }` at `:7-11`
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")] pub enum AssistantState { Idle, Listening, Transcribing, Thinking, Speaking }` — `tipsy-protocol/src/lib.rs:26-34`. It keeps exactly these five variants: a call is not a turn.
  - `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(tag = "type", rename_all = "snake_case")] pub enum Event { … , Error { code, message, recoverable } }` — `tipsy-protocol/src/lib.rs:113-132`. The tag is `type` and the variant names are snake_case, so the variant this task adds is spelled `{"type":"live","active":true}` on the wire — the same convention Plan 2B's `Event::TestResult` follows.
  - `fn roundtrip_event(e: Event)` and `fn roundtrip_command(c: Command)` — `tipsy-protocol/tests/roundtrip.rs:3-13`, `serde_json::to_string` → `from_str` → `assert_eq!` with the JSON in the panic message
- Produces:

```rust
/// `[realtime]` in `config.toml` (spec §6/§7, plan 2C). Every field defaults in, so a file written
/// before this plan is a valid file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RealtimeConfig {
    pub enabled: bool,
    pub model: String,
    /// Name of the key, never the key itself — the same rule as `ProviderConfig::key`.
    pub key: String,
    pub voice: String,
    pub max_minutes: u32,
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: "gpt-realtime-2.1".into(),
            key: "openai".into(),
            voice: "marin".into(),
            max_minutes: 10,
        }
    }
}

// in Config, after `providers` and before `profiles`:
pub realtime: RealtimeConfig,

// in Event, after `Error` (the last variant, which is where the crate's own
// tagged-event style puts a later addition):
/// The server's answer to "is a realtime call live?": `active: true` when a call
/// opens its socket and `false` when that call ends, by any of its endings. An
/// event rather than a new `AssistantState`, because the server owns the socket
/// (so the indicator cannot be on while no socket is open) and because "the call
/// ended" is then explicit rather than inferred from a status transition.
Live { active: bool },
```

  - Task 2 reads `RealtimeConfig` (`model`, `key`, `voice`) through `crate::config::RealtimeConfig`; Task 4 builds its `CallSpec` from `RealtimeConfig` plus `ListeningConfig::silence_ms` and the active profile's `prompt`/`language`; Task 6 gates the key binding on `enabled` and ends the call at `max_minutes`; `Event::Live { active: true }` is what turns the HUD's indicator on for the whole call, and `Event::Live { active: false }` is what turns it off.

**What this task decides, and why**

- **The plan's task-table row is stale: there is no `[realtime] silence_ms`.** The table (`docs/superpowers/plans/2026-10-10-plan-2c-openai-realtime.md:69`) lists the section as "enabled, model, key name, voice, silence_ms, max_minutes", but the plan's own Decision is "Server-side turn detection, configured with `[listening] silence_ms`" (`:23`), and the Decisions section wins. Adding a second silence field would give the user two numbers that mean the same thing and let them disagree. `RealtimeConfig` therefore has **five** fields, and Task 2's `session_update` reads `listening.silence_ms` through `CallSpec::silence_ms`.
- **`key` is a name.** The Decision at `:20` is that auth uses the ordinary key resolution, so `[realtime] key = "openai"` is what `KeyResolver` looks up in `TIPSY_KEY_OPENAI` or the credential store. The struct carries no key value and serialises no secret.
- **The model id and the voice are quoted from the research note**, not invented: `gpt-realtime-2.1` is the note's "current default choice" with the best interruption behaviour, and `marin` is one of the two voices its `session.update` example recommends. The voice comes from `[realtime]` rather than the profile's `voices` table, which maps *speech connectors* — the plan records that asymmetry at `:26`, and a test pins both values so a later "improvement" has to be deliberate.
- **`max_minutes = 10` is a cost limit, not a technical one.** The plan's Decision (`:22`) makes the cost never silent; the note's arithmetic puts a listening minute near $0.32 and a speaking minute near $1.28, so ten minutes is the default ceiling and Task 6 ends the call there.
- **`Live` is one event variant and nothing else.** The HUD needs to show that a call is live for as long as it lasts (Review Focus 3), and the plan's File Structure gives `tipsy-ui` the indicator and the strings in Task 6. `AssistantState` is left alone: it describes what the assistant is doing during a turn, and a call is not a turn. `draw_header`'s match over `state.status` keeps its `_ => strings.thinking` fallback (`tipsy-ui/src/render.rs:84-88`) and `UiState::apply` keeps comparing the state to `AssistantState::Idle` (`tipsy-ui/src/state.rs:172-178`), so no *rendering* changes here. What the new variant does break is exhaustiveness — `UiState::apply` matches `Event` without a wildcard, so Step 6 adds one behaviour-free arm there after showing the exact `E0004`. An `Event` variant reaches no further than that: the two other exhaustive matches Plan 2B Task 2 treats are over `Command` (`session.rs:216`, `client.rs:286`), which this task does not touch.

- [ ] **Step 1: Write the failing tests**

In `crates/tipsy-compadre/src/config/model.rs`, append inside the existing `mod tests` (after `a_profile_without_a_voice_for_the_connector_falls_back_to_the_provider_default`, line 219):

```rust
    /// The section's defaults, quoted from the plan and the research note: off, the note's current
    /// model, one key *name*, one of the two voices the note recommends, and the plan's own limit.
    #[test]
    fn realtime_defaults_are_off_and_named() {
        let realtime = RealtimeConfig::default();
        assert!(!realtime.enabled, "the mode must never be silently on");
        assert_eq!(realtime.model, "gpt-realtime-2.1", "the note's current default choice");
        assert_eq!(realtime.key, "openai", "the name `KeyResolver` looks up in TIPSY_KEY_OPENAI");
        assert_eq!(realtime.voice, "marin", "one of the two voices the note recommends");
        assert_eq!(realtime.max_minutes, 10, "the plan's default call limit");

        // `[realtime]` has no `silence_ms`: turn detection reuses `[listening] silence_ms` (the
        // plan's Decisions section), which is why the two are asserted together here and why
        // `RealtimeConfig` has no such field to drift from it.
        let config = Config::default();
        assert_eq!(config.listening.silence_ms, 1000);
        assert_eq!(config.realtime, realtime, "a default config carries the default section");
    }
```

In `crates/tipsy-compadre/src/config/store.rs`, append inside the existing `mod tests` (after `voice_key_and_voice_lookup`, line 210). It uses `Config`, `save` and `load_or_create`, all already imported through `use super::*;`:

```rust
    /// Review Focus 3, the config half: a `config.toml` written before this plan has no `[realtime]`
    /// section at all, and it is a valid file, not a corrupt one. Every field of the new section
    /// defaults in and nothing else moves.
    #[test]
    fn a_config_written_before_this_plan_gets_the_realtime_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"
active_profile = "compae"

[providers.chat]
kind = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
model = "google/gemini-2.5-flash"
key = "openrouter"

[listening]
mode = "auto"
silence_ms = 1500
mic_gain = 9.5
"#).unwrap();

        let out = load_or_create(&path).unwrap();
        assert!(out.warning.is_none(), "an old file is not a corrupt file: {:?}", out.warning);
        let realtime = &out.config.realtime;
        assert!(!realtime.enabled, "the mode must never be silently on");
        assert_eq!(realtime.model, "gpt-realtime-2.1");
        assert_eq!(realtime.key, "openai", "a key *name*: the resolver looks the value up");
        assert_eq!(realtime.voice, "marin");
        assert_eq!(realtime.max_minutes, 10);
        // Turn detection is not a `[realtime]` field: it reuses the listening silence, which the old
        // file already carries and which must not be touched by this task.
        assert_eq!(out.config.listening.silence_ms, 1500);
        // Loading is not a migration: the user's file is left exactly as it was.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("[realtime]"), "loading must not rewrite the file, got:\n{text}");
    }

    /// The section survives a save and a load with every field, and it is written as its own table.
    #[test]
    fn the_realtime_section_survives_a_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.realtime.enabled = true;
        config.realtime.model = "gpt-realtime-2.1-mini".into();
        config.realtime.key = "openai-realtime".into();
        config.realtime.voice = "cedar".into();
        config.realtime.max_minutes = 25;
        save(&path, &config).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        for line in [
            "[realtime]",
            "enabled = true",
            "model = \"gpt-realtime-2.1-mini\"",
            "key = \"openai-realtime\"",
            "voice = \"cedar\"",
            "max_minutes = 25",
        ] {
            assert!(text.contains(line), "the file must contain `{line}`, got:\n{text}");
        }
        assert!(!text.contains("silence_ms = 25"), "silence is not a realtime field:\n{text}");
        assert_eq!(load_or_create(&path).unwrap().config.realtime, config.realtime);
    }
```

In `crates/tipsy-protocol/tests/roundtrip.rs`, append after `events_roundtrip` (line 46):

```rust
/// What the HUD shows for the whole of a realtime call (Plan 2C Task 1): one more `Event`, tagged
/// with `type` and spelled snake_case like every other variant of the crate's tagged events.
#[test]
fn the_live_event_roundtrips() {
    // The server sends `active: true` when a call opens and `active: false` when it ends; both are
    // round-tripped, and both are then pinned by the JSON the HUD's own clients will see.
    for active in [true, false] {
        roundtrip_event(Event::Live { active });
    }
    let json = serde_json::to_string(&Event::Live { active: true }).unwrap();
    assert_eq!(json, r#"{"type":"live","active":true}"#);
    assert_eq!(serde_json::from_str::<Event>(r#"{"type":"live","active":false}"#).unwrap(), Event::Live { active: false });
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --lib config::model::tests::realtime_defaults`

Expected: FAIL — the run stops at the compiler, one error per missing item:

```
error[E0433]: failed to resolve: use of undeclared type `RealtimeConfig`
error[E0609]: no field `realtime` on type `Config`
error[E0609]: no field `realtime` on type `&Config`
```

Run: `cargo test -p tipsy-protocol --test roundtrip the_live_event_roundtrips`

Expected: FAIL — the test target does not compile:

```
error[E0599]: no variant or associated item named `Live` found for enum `Event` in the current scope
```

(The `config::store` tests fail the same way; rustc reports one missing item at a time and gives up after a handful. No test in either module reaches the harness yet.)

- [ ] **Step 3: Add the section to the configuration model**

`crates/tipsy-compadre/src/config/model.rs` — in `Config` (lines 8-24), after `pub providers: ProvidersConfig,` (line 18) and before the `profiles` doc comment (line 19):

```rust
    /// The realtime voice mode (spec §6, plan 2C). A section of its own because a call is a *mode of
    /// the client*: `Capability` stays chat/speech/transcribe and `build_providers` never sees it.
    pub realtime: RealtimeConfig,
```

and in `Config::default()` (lines 26-36), after `providers: ProvidersConfig::default(),`:

```rust
            realtime: RealtimeConfig::default(),
```

then insert after `ProvidersConfig`'s `Default` (line 129), before `pub struct Profile` (line 131):

```rust
/// `[realtime]`: one duplex WebSocket session with OpenAI's Realtime API.
///
/// There is deliberately no `silence_ms` here. The plan's Decision is that server-side turn
/// detection is configured with `[listening] silence_ms`, so two numbers never describe one
/// behaviour; the socket reads that field through `CallSpec`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RealtimeConfig {
    /// Off unless the user turns it on: a streaming socket bills audio while it is open, so the
    /// mode is never silently on (plan 2C, Review Focus 3).
    pub enabled: bool,
    /// A session-acceptable Realtime model id (the note's table). `gpt-realtime-2.1` is its current
    /// default choice; `gpt-realtime-2.1-mini` is the cheaper one.
    pub model: String,
    /// Name of the key, never the key itself — resolved by `KeyResolver` like every provider key.
    pub key: String,
    /// One of the note's session voices (`alloy, ash, ballad, coral, echo, sage, shimmer, verse,
    /// marin, cedar`). Deliberately *not* the profile's `voices` table: that table maps speech
    /// connectors, and realtime is not one of them (plan 2C, "Decisions this plan makes").
    pub voice: String,
    /// A call ends by itself after this many minutes (plan 2C's cost Decision). The API's own cap is
    /// 60 minutes; this is the number the user chose.
    pub max_minutes: u32,
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: "gpt-realtime-2.1".into(),
            key: "openai".into(),
            voice: "marin".into(),
            max_minutes: 10,
        }
    }
}
```

- [ ] **Step 4: Add the protocol's live-call event**

`crates/tipsy-protocol/src/lib.rs` — in `Event` (lines 113-132), after `Error` (line 131) and before the closing brace:

```rust
    /// The server's answer to "is a realtime call live?": `active: true` when a call opens its
    /// socket and `false` when that call ends, by any of its endings (Plan 2C). An event rather
    /// than a new `AssistantState`: the server owns the socket, so the indicator cannot be on
    /// while no socket is open, "the call ended" is explicit rather than inferred from a status
    /// transition back to `Idle`, and `AssistantState` keeps meaning exactly one thing — what the
    /// assistant is doing during a turn. Nothing else about the protocol changes: a call's
    /// transcripts still arrive as `Heard`, `ReplyDelta` and `Reply`.
    Live { active: bool },
```

`AssistantState` (lines 26-34) is **not** touched: it keeps its five variants.

The tag is its own: the enum's `#[serde(tag = "type", rename_all = "snake_case")]` spells this variant `{"type":"live","active":true}`, which is what Step 1's JSON assertion pins.

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --lib config::model::tests::realtime_defaults`

Expected: PASS.

```
running 1 test
test config::model::tests::realtime_defaults_are_off_and_named ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Run: `cargo test -p tipsy-compadre --lib config::store::tests::realtime`

Expected: PASS, 2 tests — the filter matches both new names (`…gets_the_realtime_defaults` and `the_realtime_section_survives…`) and nothing else in the module:

```
running 2 tests
test config::store::tests::a_config_written_before_this_plan_gets_the_realtime_defaults ... ok
test config::store::tests::the_realtime_section_survives_a_save_load_roundtrip ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

Run: `cargo test -p tipsy-protocol --test roundtrip the_live_event_roundtrips`

Expected: PASS, 1 test.

The protocol crate's own suite is green now, but the workspace is not: `cargo check --workspace --all-targets` reports the three exhaustive matches below, and Step 6 clears them. Run

```
cargo test -p tipsy-protocol --test roundtrip
```

here and stop: it passes, and that is the right moment to prove the wire shape before the dependents are touched (Plan 2B Task 2's Step 4 is the same green-before-the-arms point).

- [ ] **Step 6: Watch the dependent break, and add its one arm**

```
cargo check -p tipsy-ui
```

Expected: FAIL with exactly one `E0004`, and this is the shape of it as rustc really prints it (a `-p tipsy-compadre` check stops at the same error, because `tipsy-ui` is its dependency):

```
error[E0004]: non-exhaustive patterns: `&tipsy_protocol::Event::Live { .. }` not covered
   --> crates\tipsy-ui\src\state.rs:166:15
    |
166 |         match ev {
    |               ^^ pattern `&tipsy_protocol::Event::Live { .. }` not covered
    |
note: `tipsy_protocol::Event` defined here
   --> crates\tipsy-protocol\src\lib.rs:115:1
    |
115 | pub enum Event {
    | ^^^^^^^^^^^^^^
...
133 |     Live { active: bool },
    |     ---- not covered
    = note: the matched value is of type `&tipsy_protocol::Event`
help: ensure that all possible cases are being handled by adding a match arm with a wildcard pattern or an explicit pattern as shown
    |
234 ~             },
235 +             &tipsy_protocol::Event::Live { .. } => todo!()
    |

For more information about this error, try `rustc --explain E0004`.
error: could not compile `tipsy-ui` (lib) due to 1 previous error
```

The quote is exact, path prefixes and all (rustc names the variant `tipsy_protocol::Event::Live` rather than the bare `Event::Live`; the elided form is what this plan writes in prose). The line in the `note:` block moves with whatever the enum's last variant is when this task lands, and `133` is where the new variant sits today. Any *other* error — or a second `E0004` — means this task is wrong, not the dependents.

**`Event` breaks one exhaustive match here, not three.** This task adds an `Event` variant, so only a match *over `Event`* can stop being exhaustive: `UiState::apply`'s (no wildcard, `state.rs:166-235`) is the only one in the workspace, and the check above is the proof. The contrast with Plan 2B Task 2 is worth stating so the two plans are not read as contradicting each other: that task adds `Command::TestProvider` **and** `Event::TestResult`, and the `Command` half is what breaks `session.rs:216` and `client.rs:286` as well. Neither of those two matches is over `Event` — `session.rs:216` is `fn command`'s match over `Command` and `client.rs:286` is `starts_turn`'s — so when this plan adds no `Command` variant, they keep compiling and this task touches **one** file beyond the protocol crate. (The precise test, if a later task ever adds a `Command` variant, is Plan 2B Task 2's: one arm in each of the three.)

That arm goes in `crates/tipsy-ui/src/state.rs`, in `UiState::apply` (lines 166-235), after `Event::Stats { … }` (line 224) and immediately before `Event::Error { … }` (line 225):

```rust
            // A live-call event is raised by the call, not by a turn: the status rings and the log
            // say nothing about it (Plan 2C Task 6 owns the indicator and its own tests), so the arm
            // records nothing. It is here to keep the match exhaustive and this task behaviour-free.
            Event::Live { active: _ } => {}
```

`Event::Live` is a struct variant, so the arm binds nothing and does nothing; `active: _` is spelled out so nobody has to wonder whether the flag is read here. Task 6 replaces the arm with the reducer that copies the flag into the indicator's own field (`tipsy-ui/src/state.rs:4633`, below), which is where the variant finally means something to the UI.

- [ ] **Step 7: Prove the workspace is whole again**

Run: `cargo check --workspace --all-targets`

Expected: `Finished` for all three crates, no warnings.

Run: `cargo test --workspace`

Expected: all green. This is the real regression net for the new variant: every match over `Event` — the one this task armed, and the wildcarded ones further down the pipeline — must keep compiling, and the protocol's round-trip test is what pins the wire shape. The test counts of `config::store` and `roundtrip` depend on whether Plan 2B's Tasks 1-2 have landed (they add six and two tests respectively), so this step quotes only the four tests it adds. Finish with `cargo clippy --workspace --all-targets` (no new warnings).

- [ ] **Step 8: Document the section in the spec's example file**

In `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md`, §8.1's example file, insert between `[providers.transcribe]` (ends line 272) and `[[profiles]]` (line 274), in the doc's own trailing-comment style, and change nothing else in the spec:

```toml
[realtime]
enabled = false          # off by default: a streaming call bills audio while it is open
model = "gpt-realtime-2.1"
key = "openai"           # key name, never the key itself
voice = "marin"
max_minutes = 10         # a call ends by itself after this long
# turn detection reuses [listening] silence_ms: there is no silence_ms here on purpose
```

- [ ] **Step 9: Commit**

```bash
git add crates/tipsy-compadre/src/config/model.rs crates/tipsy-compadre/src/config/store.rs crates/tipsy-protocol/src/lib.rs crates/tipsy-protocol/tests/roundtrip.rs crates/tipsy-ui/src/state.rs docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md
git commit -m "feat(config): add the realtime section and Event::Live

[realtime] carries enabled, model, key name, voice and max_minutes, all of them
defaulting in for a file written before this plan; there is no silence_ms field
because turn detection reuses [listening] silence_ms, as the plan's Decisions say.
The protocol gains one event for the whole of a live call, sent by the server when
the call opens and when it ends, serde round-tripped and spelled snake_case like
the rest of the crate's tagged events. AssistantState is untouched: it is about a
turn. UiState::apply gets the one behaviour-free arm the new variant needs.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The socket client `[mid]`

**Files:**
- Modify: `crates/tipsy-compadre/Cargo.toml:8-29` — one new line under `[dependencies]`: `tokio-tungstenite = { version = "0.30", features = ["rustls-tls-native-roots"] }`
- Create: `crates/tipsy-compadre/src/realtime/mod.rs` — the module header and `pub mod socket;` (the call façade is Task 4 and lands in this file)
- Create: `crates/tipsy-compadre/src/realtime/socket.rs`
- Modify: `crates/tipsy-compadre/src/lib.rs:15-17` — `pub mod realtime;` between `providers` and `session`
- Test: `crates/tipsy-compadre/tests/realtime.rs` — NEW: the local fake socket server and this task's handshake, reader, close, timeout and reopen exchanges (Task 4 appends the turn-plumbing exchanges to the same file)
- Lock: `Cargo.lock` — commit it with the change (Step 1 adds a package tree; Step 5's `cargo tree` proves no `native-tls`, no `openssl` and no `schannel` comes with it)

**Interfaces:**
- Consumes (read from the committed code, not recalled):
  - `pub struct RealtimeConfig { pub enabled: bool, pub model: String, pub key: String, pub voice: String, pub max_minutes: u32 }` and `Default { enabled: false, model: "gpt-realtime-2.1", key: "openai", voice: "marin", max_minutes: 10 }` — Task 1
  - `pub struct KeyResolver` with `pub fn resolve(&self, name: &str) -> Result<Option<(String, KeySource)>, KeyError>` — `keys.rs:97-102`; `pub fn env_var_name(name: &str) -> String` — `keys.rs:65-71`; `pub enum KeyError { Store(String) }` — `keys.rs:8-12`
  - `MemoryStore` and the `SecretStore` trait (`get`/`set`/`delete`) — `keys.rs:14-18`, `:47-62`, used by the tests exactly as `providers/mod.rs`'s `build_tests` use them (`providers/mod.rs:333-348`)
  - The connectors' error shape, which this task mirrors: `#[derive(Debug, Clone, PartialEq, thiserror::Error)] pub enum ProviderError { Auth(String), RateLimited(String), Timeout, Network(String), BadResponse(String), Cancelled }` with `pub fn code(&self) -> ErrorCode` and `pub fn from_status(status: u16, body: &str) -> Self` — `providers/mod.rs:62-117`; and `BuildError::MissingKey { name, env }` whose message is `` no API key named `{name}`: run `tipsy-compadre key set {name}` or set {env} `` — `providers/mod.rs:159-170`
  - `pub enum ErrorCode { Auth, RateLimited, Timeout, Network, BadResponse, NoKey, Mic, Config, NothingHeard, Internal }` — `tipsy-protocol/src/lib.rs:51-64`
  - `tokio` (with `full`, already a dependency), `serde_json`, `futures` (`SinkExt`, `StreamExt`) — `crates/tipsy-compadre/Cargo.toml:11-29`
- Produces (the exact shapes Task 3, 4, 5 and 6 build on):

```rust
pub const REALTIME_URL: &str = "wss://api.openai.com/v1/realtime";
pub const PCM_RATE: u32 = 24_000;
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(15);
pub const KEEPALIVE: Duration = Duration::from_secs(30);
pub const SESSION_CAP: Duration = Duration::from_secs(60 * 60);
pub const TRANSCRIBE_MODEL: &str = "gpt-4o-transcribe";

pub struct CallSpec { pub instructions: String, pub language: String, pub silence_ms: u32 }

pub struct CallOptions { pub base_url: String, pub cap: Duration, pub open_timeout: Duration }
impl Default for CallOptions { /* REALTIME_URL, SESSION_CAP, OPEN_TIMEOUT */ }
impl CallOptions {
    pub fn at(base_url: impl Into<String>) -> Self;
    pub fn with_cap(self, cap: Duration) -> Self;
    pub fn with_open_timeout(self, open_timeout: Duration) -> Self;
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RealtimeError {
    MissingKey { name: String, env: String },
    Store(String),
    Auth(String),
    RateLimited(String),
    Timeout,
    Network(String),
    BadResponse(String),
}
impl RealtimeError {
    pub fn from_status(status: u16, body: &str) -> Self;
    pub fn code(&self) -> ErrorCode;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketEvent {
    Text(String),
    Closed { code: Option<u16>, reason: String },
}

pub struct RealtimeSocket { /* stream, options, config, spec, url, opened_at, pending */ }
impl std::fmt::Debug for RealtimeSocket { /* never prints a key, an instruction or a header */ }

impl RealtimeSocket {
    pub async fn connect(keys: &KeyResolver, config: &RealtimeConfig, spec: &CallSpec) -> Result<Self, RealtimeError>;
    pub async fn connect_with(options: CallOptions, keys: &KeyResolver, config: &RealtimeConfig, spec: &CallSpec) -> Result<Self, RealtimeError>;
    pub fn url(&self) -> &str;
    pub fn session_expired(&self) -> bool;
    pub async fn reopen(&mut self, keys: &KeyResolver) -> Result<(), RealtimeError>;
    pub async fn send(&mut self, event: &serde_json::Value) -> Result<(), RealtimeError>;
    pub async fn ping(&mut self) -> Result<(), RealtimeError>;
    pub async fn recv(&mut self) -> Result<SocketEvent, RealtimeError>;
    pub async fn close(&mut self) -> Result<(), RealtimeError>;
}

pub fn realtime_url(base_url: &str, model: &str) -> String;
pub fn session_update(config: &RealtimeConfig, spec: &CallSpec) -> serde_json::Value;
```

  - Task 4's loop is written against exactly these: `recv()` gives `SocketEvent::Text` to `events::map`, `send` carries `input_audio_buffer.append`, `ping` is ticked from `KEEPALIVE`, `session_expired` plus `reopen` implement the 60-minute reopen, `close` ends the call, `PCM_RATE` is the rate the microphone's 16 kHz is resampled to and the rate the `Player` is told, and `RealtimeError::code()` is what `Event::Error` carries.
  - Task 5 sends `conversation.item.truncate` and `response.cancel` through `send` unchanged.
  - Task 6 reads `enabled` itself: `connect` does **not** look at it, because Review Focus 3 makes starting a call the key binding's decision and a second copy of that rule inside the socket would be a second place to get it wrong.

**What this task decides, and why**

- **The one new dependency, named.** `tokio-tungstenite` 0.30 with `rustls-tls-native-roots`, which the research note recommends ("pure Rust — TLS via `rustls` + `rustls-native-certs` + `tokio-rustls`"; `native-tls` variants "pull a native dependency (Schannel/OpenSSL)" on Windows/MSVC, and vendored additionally wants a C toolchain). Its defaults already include `connect` and `handshake`, and `native-tls` is not a default feature, so the plain dependency line is correct. Nothing else is added: base64 decoding reuses the hand-written decoder Plan 2A wrote for Gemini (Task 3 makes it `pub(crate)`), and `serde_json`/`futures`/`tokio` are already here. `tipsy-protocol` and `tipsy-ui` gain nothing.
- **The endpoint is the note's, with the model as a query parameter.** `wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1` (the note's "Endpoints" section). The model appears twice on purpose: in the URL and in `session.update`'s `session.model`, exactly as the note's example does.
- **Auth is the ordinary bearer header and nothing else.** The note's WebSocket auth line: `Authorization: Bearer $OPENAI_API_KEY` on the handshake, with `OpenAI-Safety-Identifier` optional. GA needs no `OpenAI-Beta: realtime=v1` (the note's deprecations page removed the beta interface on 2026-05-12), so **no** beta header is sent and a test asserts it is absent. The ephemeral-secret path exists for browsers and is not used (plan Decision at `:20`).
- **`session.update` waits for `session.created`**, because that is the note's ordering ("Client → server, immediately after `session.created`"). Frames that arrive first are parked in `pending` and handed back by `recv`, so the wait loses nothing. The wait is bounded by the open timeout, so a silent server is a `Timeout` rather than a hang.
- **The session is configured for what the plan's architecture actually needs.** PCM16 24 kHz both ways is the note's only supported PCM rate; `turn_detection` is `server_vad` with `silence_duration_ms` taken from `[listening] silence_ms`, `create_response: true` so the server starts the turn, and `interrupt_response: true` so the server cancels its own in-flight response on a barge-in (both from the note's example and its interruption section). **Input transcription is configured too** — the note marks `conversation.item.input_audio_transcription.*` as arriving "only if `transcription` configured", and without it `Event::Heard` could never carry anything. Threshold, padding, noise reduction and every other knob the note documents are left at the API's defaults: nothing in this plan decides them.
- **Keepalive and reopen are ours, because the note documents neither.** The close codes and reasons, the reconnect semantics for the 60-minute cap and any proactive idle close are all listed under the note's Uncertainty, so: `recv` reports a close's code and reason **uninterpreted** (no code is special-cased anywhere — the reopen is decided by `session_expired`, never by a code), the caller pings every `KEEPALIVE` because an idle socket's fate is unknown, and `reopen` opens a fresh socket *before* the API's own cap can end the conversation, re-resolving the key and re-sending `session.update`.
- **Nothing in this type can leak the key.** `connect_with` resolves the key, puts it in the handshake and drops it; `reopen` resolves it again instead of keeping it, so the struct holds no key value at all. `Debug` is hand-written and prints the URL, the model, the voice and the parked-frame count — never `spec.instructions` and never a header — which is what the Global Constraint "the `Authorization` header must not be logged even at debug level" asks for; a test asserts the printed form contains neither the resolved value nor the word `Bearer`.
- **The reader is protocol-blind on purpose.** `recv` returns text; only `configure` looks at one field (`type == "session.created"`) because the handshake ordering needs it. The mapping — including the two events whose payloads the note marks inferred — is Task 3's, so a live-key surprise in `response.output_audio.delta` changes one file and its fixture, not the transport.

- [ ] **Step 1: Name the dependency this task needs**

In `crates/tipsy-compadre/Cargo.toml`, under `[dependencies]` next to the other pinned versions (after `clap = { version = "4.6", features = ["derive"] }`, line 29):

```toml
tokio-tungstenite = { version = "0.30", features = ["rustls-tls-native-roots"] }
```

Run: `cargo check -p tipsy-compadre --lib`

Expected: `Finished` for `tipsy-compadre`. This is the plan's one new dependency and the first run fetches it from crates.io, so the output also shows `Updating crates.io index` and a `Compiling …` list that names `tungstenite v0.30.0`, `rustls v0.23.x`, `rustls-native-certs v0.8.x`, `tokio-rustls v0.26.x` and `rustls-pki-types v1.x`. `git diff --stat Cargo.lock` must show only added packages — no version of a package the workspace already builds must move.

Run: `cargo tree -p tipsy-compadre --prefix none`

Expected: the tree lists `tokio-tungstenite`, `tungstenite`, `rustls`, `rustls-native-certs`, `tokio-rustls`, `rustls-pki-types` — and **no** `native-tls`, **no** `openssl`, **no** `schannel`, **no** `webpki-roots`. That is the whole point of the `rustls-tls-native-roots` feature choice (the note's "Rust client" section: the `native-tls` variants are the ones that drag a native dependency into a Windows/MSVC build). If the machine has no network, the crates.io fetch is the blocker to report — do not substitute another TLS backend and do not vendor anything into the tree.

- [ ] **Step 2: Create the module skeleton and write the failing tests**

`crates/tipsy-compadre/src/realtime/mod.rs`:

```rust
//! Realtime voice: one duplex WebSocket session with OpenAI's Realtime API (spec §6, plan 2C).
//!
//! `socket` is the transport — connect, configure, read, keep alive, reopen, close. `events` (Task
//! 3) is the pure mapping from the wire's JSON to the actions the rest of the module performs. The
//! call façade that ties both to the session, the microphone and the `Player` is Task 4 and lands
//! in this file.

pub mod socket;
```

`crates/tipsy-compadre/src/lib.rs` — add between `pub mod providers;` (line 15) and `pub mod session;` (line 17):

```rust
pub mod realtime;
```

Then create `crates/tipsy-compadre/tests/realtime.rs` with the harness and this task's tests, verbatim:

```rust
//! The realtime socket against a local fake server: the handshake it makes, the frames it reads,
//! the two ways a call ends, and the reopen at the session cap.
//!
//! `wiremock` cannot help here — it serves HTTP, and this is a duplex WebSocket — so the fake server
//! is a `TcpListener` plus `tokio_tungstenite::accept_hdr_async`, which is enough to see the request
//! URI and every header the client sends, and to script what comes back.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tipsy_compadre::config::RealtimeConfig;
use tipsy_compadre::keys::{KeyResolver, MemoryStore, SecretStore};
use tipsy_compadre::realtime::socket::{CallOptions, CallSpec, RealtimeError, RealtimeSocket, SocketEvent};
use tipsy_protocol::ErrorCode;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

/// The one key value any test here uses. It is a fixture, not a key: the workspace rule is that keys
/// appear by name and never in a file, a log or a fixture, and this string exists only to prove that
/// the header is built from the *resolved* value.
const TEST_KEY: &str = "fake-key-for-tests-only";

/// Frames the tests script. The payload shapes are Task 3's business — the socket treats every text
/// frame as opaque — so these carry only the note's `type` strings and a field or two.
const SESSION_CREATED: &str = r#"{"type":"session.created","session":{"id":"sess_fixture"}}"#;
const SESSION_UPDATED: &str = r#"{"type":"session.updated","session":{"id":"sess_fixture"}}"#;
const AUDIO_DELTA: &str = r#"{"type":"response.output_audio.delta","delta":"AAECAw=="}"#;
const TRANSCRIPT_DELTA: &str = r#"{"type":"response.output_audio_transcript.delta","delta":"Ozú"}"#;
const SPEECH_STARTED: &str = r#"{"type":"input_audio_buffer.speech_started","item_id":"item_0","audio_start_ms":1200}"#;

/// A line that is exactly this, instead of JSON, is sent as a WebSocket **ping**: keepalive traffic
/// is real traffic, and the reader must skip it without disturbing the order of the events.
const PING: &str = "__ping__";

fn resolver() -> KeyResolver {
    let store = MemoryStore::default();
    store.set("openai", TEST_KEY).unwrap();
    KeyResolver::new(Box::new(store), Box::new(|_| None))
}

fn empty_resolver() -> KeyResolver {
    KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None))
}

fn spec() -> CallSpec {
    CallSpec {
        instructions: "Habla como un compae andaluz, con cariño y sin prisa.".into(),
        language: "es-ES".into(),
        silence_ms: 1500,
    }
}

/// What the fake server does on one accepted connection. Every variant starts by sending the frames
/// the test scripts — always `session.created` first, because the client waits for it before it
/// sends `session.update` (the note's ordering).
#[derive(Clone)]
enum Script {
    /// Send, then keep the socket open and record what the client sends.
    Open(Vec<String>),
    /// Send, wait for the client's next frame, then close with this code and reason.
    Close(Vec<String>, u16, String),
    /// Send, wait for the client's next frame, then drop the TCP connection with no close frame.
    Drop(Vec<String>),
}

#[derive(Clone, Default)]
struct Handshake {
    target: String,
    headers: Vec<(String, String)>,
}

impl Handshake {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

struct FakeServer {
    addr: SocketAddr,
    handshakes: Arc<Mutex<Vec<Handshake>>>,
    texts: Arc<Mutex<Vec<String>>>,
}

impl FakeServer {
    /// Starts the listener on a random loopback port. The `n`-th accepted connection runs
    /// `scripts[n]`; a connection with no script is accepted and left silent.
    async fn start(scripts: Vec<Script>) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handshakes: Arc<Mutex<Vec<Handshake>>> = Arc::new(Mutex::new(Vec::new()));
        let texts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (seen, recorded) = (handshakes.clone(), texts.clone());
        tokio::spawn(async move {
            let mut accepted = 0usize;
            while let Ok((stream, _)) = listener.accept().await {
                let script = scripts.get(accepted).cloned().unwrap_or(Script::Open(Vec::new()));
                accepted += 1;
                let slot: Arc<Mutex<Handshake>> = Arc::new(Mutex::new(Handshake::default()));
                let callback_slot = slot.clone();
                let seen = seen.clone();
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let callback = move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
                        let mut handshake = callback_slot.lock().unwrap();
                        handshake.target = request.uri().to_string();
                        handshake.headers = request
                            .headers()
                            .iter()
                            .map(|(name, value)| {
                                (name.as_str().to_string(), value.to_str().unwrap_or_default().to_string())
                            })
                            .collect();
                        Ok(response)
                    };
                    let Ok(socket) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
                        return;
                    };
                    seen.lock().unwrap().push(slot.lock().unwrap().clone());
                    serve(socket, script, recorded).await;
                });
            }
        });
        FakeServer { addr, handshakes, texts }
    }

    /// The URL `CallOptions::at` takes: the fake server serves the real path.
    fn url(&self) -> String {
        format!("ws://{}/v1/realtime", self.addr)
    }

    /// Waits for the `index`-th handshake. The accept loop records it just after the 101 response, so
    /// a test that returns from `connect` may still be a scheduling hop ahead of it.
    async fn handshake(&self, index: usize) -> Handshake {
        for _ in 0..200 {
            if let Some(handshake) = self.handshakes.lock().unwrap().get(index).cloned() {
                return handshake;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the fake server never accepted connection {index}");
    }

    async fn connections_at_least(&self, wanted: usize) {
        for _ in 0..200 {
            if self.handshakes.lock().unwrap().len() >= wanted {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the fake server accepted {} connections, wanted {wanted}", self.handshakes.lock().unwrap().len());
    }

    /// Waits for at least `n` client frames and returns everything recorded so far.
    async fn texts(&self, n: usize) -> Vec<String> {
        for _ in 0..200 {
            let recorded = self.texts.lock().unwrap().clone();
            if recorded.len() >= n {
                return recorded;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the fake server recorded {} client frames, wanted {n}", self.texts.lock().unwrap().len());
    }
}

/// How a scripted connection ends, once the client's first frame has been seen.
enum Ending {
    KeepOpen,
    Close(u16, String),
    /// The abrupt death: no close frame, so the client can only learn of it from the transport.
    Drop,
}

async fn serve(mut socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>, script: Script, texts: Arc<Mutex<Vec<String>>>) {
    let (lines, ending) = match script {
        Script::Open(lines) => (lines, Ending::KeepOpen),
        Script::Close(lines, code, reason) => (lines, Ending::Close(code, reason)),
        Script::Drop(lines) => (lines, Ending::Drop),
    };
    for line in lines {
        let message =
            if line == PING { Message::Ping(Vec::<u8>::new().into()) } else { Message::text(line) };
        if socket.send(message).await.is_err() {
            return;
        }
    }
    // Wait for one client frame before ending the connection: `session.update` is what the client
    // sends the moment it has seen `session.created`, and waiting for it turns "the call is
    // configured, then the socket dies" into a deterministic sequence instead of a race.
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                texts.lock().unwrap().push(text.as_str().to_string());
                break;
            }
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return,
        }
    }
    match ending {
        Ending::KeepOpen => {
            while let Some(Ok(message)) = socket.next().await {
                if let Message::Text(text) = message {
                    texts.lock().unwrap().push(text.as_str().to_string());
                }
            }
        }
        Ending::Close(code, reason) => {
            let frame = CloseFrame { code: CloseCode::from(code), reason: reason.into() };
            let _ = socket.send(Message::Close(Some(frame))).await;
            // Complete the close handshake: read the client's own close reply, so its `close()` has
            // a live socket to say goodbye to and does not fail on a connection we already dropped.
            while let Some(Ok(message)) = socket.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
            let _ = socket.close(None).await;
        }
        Ending::Drop => drop(socket),
    }
}

async fn connect_to(server: &FakeServer) -> RealtimeSocket {
    RealtimeSocket::connect_with(CallOptions::at(server.url()), &resolver(), &RealtimeConfig::default(), &spec())
        .await
        .expect("the fake server accepts the handshake and sends session.created")
}

/// The handshake the client makes, and the session it configures: the URL, the query, the header and
/// the `session.update` that follows `session.created`.
#[tokio::test]
async fn the_handshake_carries_the_model_query_and_the_bearer_key() {
    let server = FakeServer::start(vec![Script::Open(vec![SESSION_CREATED.into()])]).await;
    let socket = connect_to(&server).await;

    let handshake = server.handshake(0).await;
    assert_eq!(
        handshake.target, "/v1/realtime?model=gpt-realtime-2.1",
        "the note's endpoint with the model as a query parameter"
    );
    assert_eq!(
        handshake.header("authorization"), Some("Bearer fake-key-for-tests-only"),
        "the note: `Authorization: Bearer $OPENAI_API_KEY` on the handshake"
    );
    assert_eq!(handshake.header("openai-beta"), None, "GA needs no `OpenAI-Beta: realtime=v1`");
    assert_eq!(handshake.header("openai-safety-identifier"), None, "optional, and this process has no hashed user id");

    let first: serde_json::Value = serde_json::from_str(&server.texts(1).await[0]).unwrap();
    assert_eq!(first["type"], "session.update");
    assert_eq!(first["session"]["audio"]["input"]["format"], serde_json::json!({ "type": "audio/pcm", "rate": 24_000 }));
    assert_eq!(first["session"]["audio"]["output"]["format"], serde_json::json!({ "type": "audio/pcm", "rate": 24_000 }));
    assert_eq!(first["session"]["audio"]["output"]["voice"], "marin");
    assert_eq!(first["session"]["audio"]["input"]["turn_detection"]["type"], "server_vad");
    assert_eq!(first["session"]["audio"]["input"]["turn_detection"]["silence_duration_ms"], serde_json::json!(1500));

    // The key never reaches a log: the type's `Debug` prints neither the resolved value nor the word
    // `Bearer` (the Global Constraint says the header must not be logged even at debug level).
    let printed = format!("{socket:?}");
    assert!(!printed.contains(TEST_KEY), "{printed}");
    assert!(!printed.contains("Bearer"), "{printed}");
    assert!(!printed.contains(&spec().instructions), "the profile's own text is not printed either: {printed}");
}

/// A scripted exchange, frame by frame: text is handed back byte for byte (the socket parses
/// nothing), and keepalive traffic never appears as an event.
#[tokio::test]
async fn a_scripted_exchange_is_read_frame_by_frame_in_order() {
    let server = FakeServer::start(vec![Script::Open(vec![
        SESSION_CREATED.into(),
        PING.into(),
        SESSION_UPDATED.into(),
        AUDIO_DELTA.into(),
        TRANSCRIPT_DELTA.into(),
        SPEECH_STARTED.into(),
    ])])
    .await;
    let mut socket = connect_to(&server).await;

    // `connect` consumed `session.created` and sent `session.update`; the ping that follows it in the
    // script is skipped, so the next event is the one after it, unchanged.
    assert_eq!(socket.recv().await.unwrap(), SocketEvent::Text(SESSION_UPDATED.into()));
    assert_eq!(socket.recv().await.unwrap(), SocketEvent::Text(AUDIO_DELTA.into()));

    // Our own keepalive: a ping the API's side answers with a pong, which the reader also skips.
    socket.ping().await.unwrap();
    assert_eq!(socket.recv().await.unwrap(), SocketEvent::Text(TRANSCRIPT_DELTA.into()));
    assert_eq!(socket.recv().await.unwrap(), SocketEvent::Text(SPEECH_STARTED.into()));
}

/// A close the server sends arrives with its code and reason; nothing interprets them (the note says
/// the codes are undocumented), and closing a socket that is already closed is not a failure.
#[tokio::test]
async fn a_clean_close_is_reported_with_its_code_and_reason() {
    let server = FakeServer::start(vec![Script::Close(
        vec![SESSION_CREATED.into(), SESSION_UPDATED.into()],
        1000,
        "call ended".into(),
    )])
    .await;
    let mut socket = connect_to(&server).await;

    assert_eq!(socket.recv().await.unwrap(), SocketEvent::Text(SESSION_UPDATED.into()));
    assert_eq!(
        socket.recv().await.unwrap(),
        SocketEvent::Closed { code: Some(1000), reason: "call ended".into() }
    );
    // The socket reports, the caller decides: reopening at the cap is `session_expired`'s decision,
    // never a close code's (the note documents no code for the 60-minute limit). Closing twice is
    // fine, because a call that is already over is over.
    socket.close().await.unwrap();
    socket.close().await.unwrap();
}

/// The other death: no close frame at all. It must surface as a readable error, not as a hang.
#[tokio::test]
async fn an_abrupt_death_is_a_network_error_not_a_hang() {
    let server = FakeServer::start(vec![Script::Drop(vec![SESSION_CREATED.into()])]).await;
    let mut socket = connect_to(&server).await;

    let error = tokio::time::timeout(Duration::from_secs(5), socket.recv())
        .await
        .expect("recv must return when the peer dies without a close frame")
        .expect_err("a death with no close frame is an error, never a silent Ok");
    assert!(matches!(error, RealtimeError::Network(_)), "{error:?}");
    assert_eq!(error.code(), ErrorCode::Network, "the HUD log shows a network error: {error}");
}

/// The 60-minute cap. The test cannot wait an hour, so `CallOptions::with_cap` gives it 200 ms — the
/// production value (`SESSION_CAP`) is the note's "maximum duration of a Realtime session is 60
/// minutes" and reconnection is app-managed.
#[tokio::test]
async fn a_call_reopens_when_the_session_cap_is_reached() {
    let server = FakeServer::start(vec![
        Script::Open(vec![SESSION_CREATED.into()]),
        Script::Open(vec![SESSION_CREATED.into()]),
    ])
    .await;
    let options = CallOptions::at(server.url()).with_cap(Duration::from_millis(200));
    let mut socket =
        RealtimeSocket::connect_with(options, &resolver(), &RealtimeConfig::default(), &spec()).await.unwrap();

    assert!(!socket.session_expired(), "a fresh socket has its whole cap ahead of it");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(socket.session_expired(), "the cap is what the caller watches, never a close code");

    socket.reopen(&resolver()).await.unwrap();
    assert!(!socket.session_expired(), "a reopened socket starts a new session");
    server.connections_at_least(2).await;

    let texts = server.texts(2).await;
    assert_eq!(texts.len(), 2, "each connection is configured once: {texts:?}");
    for text in &texts {
        let event: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(event["type"], "session.update", "the reopened call is configured like the first");
        assert_eq!(event["session"]["audio"]["input"]["turn_detection"]["silence_duration_ms"], serde_json::json!(1500));
    }
}

/// A server that accepts the handshake and then says nothing must not hold the call forever: the open
/// timeout is what makes a dead endpoint an error instead of a spinner.
#[tokio::test]
async fn a_silent_server_times_out_instead_of_hanging() {
    let server = FakeServer::start(vec![Script::Open(Vec::new())]).await;
    let options = CallOptions::at(server.url()).with_open_timeout(Duration::from_millis(150));
    let error = RealtimeSocket::connect_with(options, &resolver(), &RealtimeConfig::default(), &spec())
        .await
        .expect_err("a server that never opens a session is a failure, not a wait");
    assert_eq!(error, RealtimeError::Timeout);
    assert_eq!(error.code(), ErrorCode::Timeout);
}

/// A key *name* the resolver cannot find is the connectors' `MissingKey`, and it fails before the
/// network is touched: no server is started in this test.
#[tokio::test]
async fn a_missing_key_names_the_key_and_the_variable() {
    let error = RealtimeSocket::connect(&empty_resolver(), &RealtimeConfig::default(), &spec())
        .await
        .expect_err("the default key name `openai` is not in an empty resolver");
    assert_eq!(error, RealtimeError::MissingKey { name: "openai".into(), env: "TIPSY_KEY_OPENAI".into() });
    assert!(error.to_string().contains("tipsy-compadre key set openai"), "{error}");
    assert!(error.to_string().contains("TIPSY_KEY_OPENAI"), "{error}");
    assert_eq!(error.code(), ErrorCode::NoKey);
    assert!(!error.to_string().contains("sk-"), "the message names the key, never a value: {error}");
}

/// An empty key name means "no key", exactly as `providers::key_for` treats it: the handshake goes
/// out with no `Authorization` header at all, so a loopback stub can be pointed at without one.
#[tokio::test]
async fn an_empty_key_name_sends_no_authorization_header() {
    let server = FakeServer::start(vec![Script::Open(vec![SESSION_CREATED.into()])]).await;
    let config = RealtimeConfig { key: String::new(), ..RealtimeConfig::default() };
    let _socket = RealtimeSocket::connect_with(CallOptions::at(server.url()), &resolver(), &config, &spec())
        .await
        .unwrap();

    let handshake = server.handshake(0).await;
    assert_eq!(handshake.header("authorization"), None, "no key name, no header");
    assert_eq!(handshake.target, "/v1/realtime?model=gpt-realtime-2.1");
}
```

- [ ] **Step 3: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --test realtime`

Expected: FAIL — the test crate does not compile, one error per missing item:

```
error[E0432]: unresolved import `tipsy_compadre::realtime::socket`
error[E0433]: failed to resolve: could not find `socket` in `realtime`
error[E0412]: cannot find type `RealtimeSocket` in module `socket`
error[E0599]: no function or associated item named `connect_with` found for struct `RealtimeSocket` in the current scope
```

(Rustc reports these one occurrence at a time and gives up after a handful; that is the expected failure, and no test in the file reaches the harness yet.)

- [ ] **Step 4: Implement the socket**

`crates/tipsy-compadre/src/realtime/socket.rs`, complete:

```rust
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
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{self, Error, Message};
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
    #[error("no API key named `{name}`: run `tipsy-compadre key set {name}` or set {env}")]
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
    pub async fn close(&mut self) -> Result<(), RealtimeError> {
        match self.stream.close(None).await {
            Ok(()) => Ok(()),
            Err(Error::AlreadyClosed) | Err(Error::ConnectionClosed) => Ok(()),
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
```

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --lib realtime::socket`

Expected: PASS, 6 tests:

```
running 6 tests
test realtime::socket::tests::a_profile_without_a_language_gets_no_language_field ... ok
test realtime::socket::tests::a_rejected_handshake_maps_like_the_connectors ... ok
test realtime::socket::tests::error_codes_mirror_the_connectors ... ok
test realtime::socket::tests::primary_subtags ... ok
test realtime::socket::tests::realtime_url_is_the_notes_endpoint_with_the_model_as_a_query ... ok
test realtime::socket::tests::the_session_update_has_the_notes_shape ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Run: `cargo test -p tipsy-compadre --test realtime`

Expected: PASS, 8 tests:

```
running 8 tests
test a_call_reopens_when_the_session_cap_is_reached ... ok
test a_clean_close_is_reported_with_its_code_and_reason ... ok
test a_missing_key_names_the_key_and_the_variable ... ok
test a_scripted_exchange_is_read_frame_by_frame_in_order ... ok
test a_silent_server_times_out_instead_of_hanging ... ok
test an_abrupt_death_is_a_network_error_not_a_hang ... ok
test an_empty_key_name_sends_no_authorization_header ... ok
test the_handshake_carries_the_model_query_and_the_bearer_key ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.61s
```

(The suite takes about half a second because of the cap test's two short sleeps; if it runs much longer, the fake server's wait loop is spinning on something that never arrives.)

Finish with `cargo test --workspace` (all green — nothing outside `tipsy-compadre` changes, and `tipsy-protocol`/`tipsy-ui` still depend on neither `tokio` nor the new crate) and `cargo clippy --workspace --all-targets` (no new warnings).

- [ ] **Step 6: Commit**

```bash
git add crates/tipsy-compadre/Cargo.toml Cargo.lock crates/tipsy-compadre/src/lib.rs crates/tipsy-compadre/src/realtime/mod.rs crates/tipsy-compadre/src/realtime/socket.rs crates/tipsy-compadre/tests/realtime.rs
git commit -m "feat(realtime): add the socket client, its reopen and its keepalive

tokio-tungstenite 0.30 with rustls-tls-native-roots is the plan's one new
dependency: the endpoint carries the model as a query, the handshake carries the
resolved key as a bearer header and neither is ever logged. The client waits for
session.created before session.update, pings on KEEPALIVE, reopens on its own
session cap because the API's close codes are undocumented, and reports a clean
close and an abrupt one differently. The tests are a local fake server built from
the same crate, because wiremock only speaks HTTP.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The event mapping `[mid]`

**Files:**
- Create: `crates/tipsy-compadre/src/realtime/events.rs` — the mapping, its fixtures and its tests
- Modify: `crates/tipsy-compadre/src/realtime/mod.rs:1-9` — add `pub mod events;` under `pub mod socket;`
- Modify: `crates/tipsy-compadre/src/providers/gemini.rs:327-329` — `fn decode_base64` becomes `pub(crate) fn decode_base64`, one word and its comment
- Test: `crates/tipsy-compadre/src/realtime/events.rs:1-…` — the module's own `#[cfg(test)] mod tests`, which is where the note's two inferred payloads are pinned

**Interfaces:**
- Consumes (read from the committed code, not recalled):
  - `fn decode_base64(input: &str) -> Result<Vec<u8>, String>` — `providers/gemini.rs:327-357`. Plan 2A wrote it "because this plan adds no dependency for it"; it skips `=`, line breaks and spaces, and its error already reads `invalid base64 byte {byte} in the audio data`. This task widens it to `pub(crate)` and reuses it, so the one new dependency stays the one Task 2 named.
  - `serde_json::{Value, json}` and `serde_json::from_str` — `serde_json` is already a dependency (`crates/tipsy-compadre/Cargo.toml:12`), as is `thiserror` (`:13`)
  - `pub enum ErrorCode { Auth, RateLimited, Timeout, Network, BadResponse, NoKey, … }` — `tipsy-protocol/src/lib.rs:51-64`
  - Task 1 by name: `Event::Live { active: bool }` is the live event this mapping's `SessionConfigured` leads to. **The mapping itself consumes nothing from Task 1** — it is deliberately wire-shape-only, so that changing the configuration cannot change how a frame is read — and Task 4 is what turns these actions into the protocol events Task 1 added.
- Produces (what Tasks 4 and 5 build on):

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SessionConfigured { session_id: Option<String> },   // → Event::Live { active: true }
    AudioDelta { pcm: Vec<u8> },                        // PCM16 LE, 24 kHz, ready for Player
    Heard { text: String },                             // → Event::Heard
    ReplyDelta { text: String },                        // → Event::ReplyDelta
    SpeechStarted,                                      // stop playback now (Task 5 truncates there)
    ResponseDone { status: String },                    // → Event::Reply + Event::Done
    ServerError { message: String },                    // → Event::Error
    Ignored,                                            // an unknown `type`: data, not death
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventError {
    NotJson(String),
    MissingField { kind: String, name: &'static str },
    NotAString { kind: String, name: &'static str, found: &'static str },
    BadAudio { kind: String, detail: String },
}
impl EventError { pub fn code(&self) -> ErrorCode; }

pub fn map(frame: &str) -> Result<Action, EventError>;
```

  - Task 4's loop is `Ok(SocketEvent::Text(text)) => match events::map(&text) { … }`, and the error arm is `Err(e) => Event::Error { code: e.code(), message: e.to_string(), recoverable: false }`. Task 5 adds the assistant item id to this file (the note takes it from `conversation.item.added` / `response.output_item.added`, neither of which this task maps) and reads the item id from the truncate bookkeeping.

**What this task decides, and why**

- **The mapping is pure.** `map(&str) -> Result<Action, EventError>` touches no socket, no clock, no file and no configuration, so every wire shape this plan relies on has a test that needs nothing but a string. That is what makes the two uncertain events safe to pin.
- **Two fixtures are the plan's one payload uncertainty, and they say so.** The research note (`Uncertainty`, first bullet) confirms the *type strings* `response.output_audio.delta` and `response.output_audio_transcript.delta` verbatim in the beta→GA migration list, but the page documenting their fields truncated, so **the field names in this task's two fixtures are inferred** from sibling events: `delta`, plus `response_id`, `item_id`, `output_index`, `content_index`, which the mapping does not read. The fixtures, the two `field(&value, kind, "delta")` calls that consume them and the comment above them are the *single* place that shape is written down; if the live smoke test (a release-checklist item, not attempted here) shows a different field, those three change together and nothing else in the module does. A wrong guess cannot panic: it becomes an `EventError` whose message names the event and the missing field.
- **A third uncertainty gets a fixture too.** The note cannot establish whether `response.output_audio.done` / `response.output_audio_transcript.done` exist (only the `_text_` done events are confirmed), so this task maps **no** `_done` sibling and its tests use `response.output_audio_transcript.done` as the unknown-type example: whether or not the API sends it, the answer is `Action::Ignored`, and the HUD's `Reply` still comes from the deltas Task 4 accumulates.
- **An unknown `type` is data, not death.** Review Focus 4: `Action::Ignored` is a normal outcome, and a test pins both a plausible-but-unconfirmed type and one that does not exist.
- **A malformed *known* event is a typed error.** Missing field, wrong type, unreadable JSON and undecodable base64 each get a variant whose `Display` is readable, so the HUD's log line says "`response.output_audio.delta` carried no `delta`" rather than "bad response".
- **`conversation.item.input_audio_transcription.completed` is deliberately not mapped**, even though the note confirms it. `Event::Heard` *inserts* a log line (`tipsy-ui/src/state.rs:179-186`), so mapping both the deltas and the completed transcript would show the user's sentence twice; the deltas are enough and Task 4 owns the accumulation.
- **`rate_limits.updated` and `response.usage` are ignored here on purpose.** The plan's cost story is the HUD's live indicator plus `max_minutes` (Decisions, `:22`); a usage mapping is a later task's call, and today it must not end a call.
- **The base64 decoder is borrowed, not copied.** The Global Constraint is one new dependency and `base64` is not it, so this task reuses Plan 2A's hand-written decoder. Gemini's comment already explains why it exists; the new call site adds the realtime reason.

- [ ] **Step 1: Write the failing tests**

Add `pub mod events;` to `crates/tipsy-compadre/src/realtime/mod.rs`, under `pub mod socket;`:

```rust
pub mod events;
```

Then create `crates/tipsy-compadre/src/realtime/events.rs` with **only** the module header, the fixtures and the tests below — no `Action`, no `EventError`, no `map`. That is what makes Step 2 fail for the right reason.

```rust
//! Wire → action: the pure half of the realtime module.
//!
//! `map` reads one JSON text frame from the Realtime socket and says what the rest of the module
//! should do about it. It is pure on purpose: it touches no socket, no clock, no file and no
//! configuration, so every shape this plan trusts is pinned by a string in the tests below.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::gemini::decode_base64;

    // ---- Fixtures -----------------------------------------------------------------------------
    //
    // Verbatim from `.superpowers/sdd/plan-2c-research/openai-realtime.md`, except where marked.

    const SESSION_CREATED: &str = r#"{"type":"session.created","event_id":"evt_1","session":{"id":"sess_fixture","object":"realtime.session","model":"gpt-realtime-2.1","voice":"marin","turn_detection":{"type":"server_vad","silence_duration_ms":1000},"expires_at":1780000000}}"#;
    const SESSION_UPDATED: &str = r#"{"type":"session.updated","event_id":"evt_2","session":{"id":"sess_fixture","object":"realtime.session","model":"gpt-realtime-2.1","voice":"marin","turn_detection":{"type":"server_vad","silence_duration_ms":1500},"expires_at":1780000000}}"#;

    /// **INFERRED PAYLOAD.** The note confirms this `type` verbatim (its Migration list) but the page
    /// that documents the fields truncated, so `delta` and the sibling indices below are inferred
    /// from neighbouring events and are *unverified*. This fixture is where the guess lives: if the
    /// live smoke test shows another field, this string and the one `field(...)` call that reads
    /// `delta` change together, and the rest of the module does not move.
    const AUDIO_DELTA: &str = r#"{"type":"response.output_audio.delta","event_id":"evt_3","response_id":"resp_1","item_id":"item_1","output_index":0,"content_index":0,"delta":"AAECAw=="}"#;

    /// **INFERRED PAYLOAD**, as above: the `type` is confirmed, the fields are not.
    const TRANSCRIPT_DELTA: &str = r#"{"type":"response.output_audio_transcript.delta","event_id":"evt_4","response_id":"resp_1","item_id":"item_1","output_index":0,"content_index":0,"delta":"Ozú, quillo"}"#;

    /// Confirmed: `item_id`, `content_index`, `delta` (the note's event table, step 9). It arrives
    /// only because the session asked for `transcription` (Task 2's `session.update`).
    const INPUT_TRANSCRIPT_DELTA: &str = r#"{"type":"conversation.item.input_audio_transcription.delta","event_id":"evt_5","item_id":"item_0","content_index":0,"delta":"oye compae"}"#;

    /// Confirmed: `item_id`, `audio_start_ms` (step 5). The payload carries more than the mapping
    /// reads, which is the point — stopping playback needs no field at all.
    const SPEECH_STARTED: &str = r#"{"type":"input_audio_buffer.speech_started","event_id":"evt_6","item_id":"item_0","audio_start_ms":1200}"#;

    /// Confirmed: `response.status` (`completed`/`cancelled`), `response.output[]` and `response.usage`
    /// (step 17). The usage is carried by the payload and deliberately not mapped.
    const RESPONSE_DONE: &str = r#"{"type":"response.done","event_id":"evt_7","response":{"id":"resp_1","status":"completed","output":[],"usage":{"input_tokens":42,"output_tokens":7,"input_token_details":{"audio_tokens":30,"text_tokens":12,"cached_tokens":0},"output_token_details":{}}}}"#;

    /// Confirmed: `error{message, type, code, param, event_id}` plus the server's own `event_id`.
    const SERVER_ERROR: &str = r#"{"type":"error","event_id":"evt_8","error":{"message":"Invalid value for 'session.audio.output.voice'","type":"invalid_request_error","code":"invalid_value","param":"session.audio.output.voice","event_id":"evt_3"}}"#;

    /// **UNCONFIRMED EXISTENCE.** The note could not establish whether the `_done` siblings of the
    /// audio and transcript deltas exist (only the `_text_` done events are confirmed). Whichever way
    /// that goes, this build does not map them: the deltas are what the HUD accumulates, and an event
    /// we do not know must never end a call.
    const UNCONFIRMED_TRANSCRIPT_DONE: &str = r#"{"type":"response.output_audio_transcript.done","event_id":"evt_9","response_id":"resp_1","item_id":"item_1","output_index":0,"content_index":0,"transcript":"Ozú, quillo."}"#;

    #[test]
    fn session_events_configure_the_call() {
        assert_eq!(
            map(SESSION_CREATED).unwrap(),
            Action::SessionConfigured { session_id: Some("sess_fixture".into()) }
        );
        assert_eq!(
            map(SESSION_UPDATED).unwrap(),
            Action::SessionConfigured { session_id: Some("sess_fixture".into()) }
        );
        // The session object is optional in the frame: an id we cannot read is not a failure.
        assert_eq!(
            map(r#"{"type":"session.updated"}"#).unwrap(),
            Action::SessionConfigured { session_id: None }
        );
        assert_eq!(
            map(r#"{"type":"session.updated","session":[]}"#).unwrap(),
            Action::SessionConfigured { session_id: None }
        );
    }

    #[test]
    fn the_audio_delta_is_decoded_to_pcm() {
        // `AAECAw==` is the four bytes 0,1,2,3: the audio deltas are base64 inside JSON text frames
        // (the note's "Audio" section), PCM16 little-endian mono at 24 kHz (`Task 2`'s `PCM_RATE`).
        assert_eq!(map(AUDIO_DELTA).unwrap(), Action::AudioDelta { pcm: vec![0, 1, 2, 3] });
    }

    #[test]
    fn the_output_transcript_delta_is_the_reply_text() {
        assert_eq!(map(TRANSCRIPT_DELTA).unwrap(), Action::ReplyDelta { text: "Ozú, quillo".into() });
    }

    #[test]
    fn the_input_transcript_delta_is_what_was_heard() {
        assert_eq!(map(INPUT_TRANSCRIPT_DELTA).unwrap(), Action::Heard { text: "oye compae".into() });
    }

    #[test]
    fn speech_started_asks_for_playback_to_stop() {
        // The note's interruption section: "The WebSocket client MUST stop playback locally" on this
        // event. The milliseconds and the item id it also carries are Task 5's business.
        assert_eq!(map(SPEECH_STARTED).unwrap(), Action::SpeechStarted);
    }

    #[test]
    fn response_done_reports_the_servers_status() {
        assert_eq!(map(RESPONSE_DONE).unwrap(), Action::ResponseDone { status: "completed".into() });
        // The other status the note names: with `interrupt_response: true` the server cancels an
        // interrupted response itself and says so here.
        let cancelled = r#"{"type":"response.done","response":{"id":"resp_2","status":"cancelled"}}"#;
        assert_eq!(map(cancelled).unwrap(), Action::ResponseDone { status: "cancelled".into() });
    }

    #[test]
    fn a_server_error_carries_its_message() {
        assert_eq!(
            map(SERVER_ERROR).unwrap(),
            Action::ServerError { message: "Invalid value for 'session.audio.output.voice'".into() }
        );
    }

    #[test]
    fn an_unknown_type_is_ignored_never_fatal() {
        // Review Focus 4: an event type this build has never heard of is data, not death.
        assert_eq!(map(r#"{"type":"session.resumed"}"#).unwrap(), Action::Ignored);
        assert_eq!(map(r#"{"type":"rate_limits.updated","rate_limits":[]}"#).unwrap(), Action::Ignored);
    }

    #[test]
    fn the_unconfirmed_done_events_are_ignored_too() {
        assert_eq!(map(UNCONFIRMED_TRANSCRIPT_DONE).unwrap(), Action::Ignored);
    }

    #[test]
    fn a_known_event_with_a_malformed_payload_is_a_typed_error() {
        // The inferred audio payload is the riskiest shape in this plan: the message names the event
        // and the field, so a live mismatch is a readable log line rather than a panic.
        let error = map(r#"{"type":"response.output_audio.delta","response_id":"resp_1","item_id":"item_1"}"#)
            .unwrap_err();
        assert_eq!(
            error,
            EventError::MissingField { kind: "response.output_audio.delta".into(), name: "delta" }
        );
        assert_eq!(error.to_string(), "response.output_audio.delta carried no `delta`");
        assert_eq!(error.code(), ErrorCode::BadResponse);

        // Same event family, field present with the wrong type.
        let error = map(r#"{"type":"response.output_audio_transcript.delta","delta":7}"#).unwrap_err();
        assert_eq!(
            error.to_string(),
            "response.output_audio_transcript.delta carried `delta` as a number, not a string"
        );

        // The completion's status lives one level down, and its absence is just as typed.
        let error = map(r#"{"type":"response.done","response":{"id":"resp_1"}}"#).unwrap_err();
        assert_eq!(error.to_string(), "response.done carried no `response.status`");

        // A malformed error event is still readable: the HUD shows the missing piece, not a blank.
        let error = map(r#"{"type":"error","error":{"type":"server_error"}}"#).unwrap_err();
        assert_eq!(error.to_string(), "error carried no `error.message`");

        // Base64 that does not decode keeps the decoder's own words, prefixed with the event.
        let error = map(r#"{"type":"response.output_audio.delta","delta":"not base64!"}"#).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("response.output_audio.delta carried audio that is not base64: invalid base64 byte"),
            "{error}"
        );
        assert_eq!(error.code(), ErrorCode::BadResponse);
    }

    #[test]
    fn frames_that_are_not_events_at_all_are_typed_errors() {
        for (frame, expected) in [
            ("", "the frame is not JSON"),
            ("not json at all", "the frame is not JSON"),
            ("[]", "carried no `type`"),
            ("null", "carried no `type`"),
            ("0", "carried no `type`"),
            (r#"{"type":42}"#, "carried no `type`"),
            (r#"{"type":"response.done","response":"cancelled"}"#, "carried no `response.status`"),
            (r#"{"type":"response.output_audio.delta","delta":null}"#, "carried `delta` as null"),
            (r#"{"type":"response.output_audio.delta","delta":[]}"#, "carried `delta` as an array"),
        ] {
            let error = map(frame).expect_err(frame);
            assert!(error.to_string().contains(expected), "{frame} → {error}");
            assert_eq!(error.code(), ErrorCode::BadResponse, "{frame}");
        }
    }

    /// The decoder this module borrows is the one Plan 2A wrote for Gemini; the mapping's behaviour
    /// for a well-formed payload is asserted once, here, so the reuse is visible rather than implied.
    #[test]
    fn the_borrowed_base64_decoder_is_the_one_the_connectors_use() {
        assert_eq!(decode_base64("AAECAw==").unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(decode_base64("T3rDug==").unwrap(), "Ozú".as_bytes());
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --lib realtime::events`

Expected: FAIL — the module does not compile, one error per missing item:

```
error[E0433]: failed to resolve: use of undeclared type `Action`
error[E0425]: cannot find function `map` in this scope
error[E0412]: cannot find type `EventError` in this scope
error[E0599]: no variant or associated item named `SessionConfigured` found for enum `Action`
```

(Rustc reports these one occurrence at a time and gives up after a handful; that is the expected failure. `use crate::providers::gemini::decode_base64;` fails too until Step 3, which is the same step's second file.)

- [ ] **Step 3: Implement the mapping, and widen the decoder it borrows**

In `crates/tipsy-compadre/src/providers/gemini.rs`, replace the doc comment and signature at lines 327-329:

```rust
/// Standard base64 (`A–Z a–z 0–9 + /`, `=` padding), written here because this plan adds no
/// dependency for it. Padding and line breaks are ignored; any other byte is an error.
///
/// `pub(crate)` because Plan 2C's realtime audio deltas are base64 too and that plan adds no base64
/// dependency either: one decoder, one set of tests, two call sites.
pub(crate) fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
```

Then replace `crates/tipsy-compadre/src/realtime/events.rs` with the file in full — the fixtures and the `#[cfg(test)] mod tests` from Step 1 stay byte for byte as written, and the implementation is prepended above them:

```rust
//! Wire → action: the pure half of the realtime module.
//!
//! `map` reads one JSON text frame from the Realtime socket and says what the rest of the module
//! should do about it. It is pure on purpose: it touches no socket, no clock, no file and no
//! configuration, so every shape this plan trusts is pinned by a string in its tests — including the
//! two payloads the research note marks inferred.

use serde_json::Value;
use tipsy_protocol::ErrorCode;

use crate::providers::gemini::decode_base64;

/// What one server frame asks the rest of the module to do. Task 4 turns these into protocol events,
/// which is why they are named after the events they become (`Heard`, `ReplyDelta`) rather than after
/// the wire's names.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// `session.created` / `session.updated`: the session is configured and the call is live. Task 4
    /// raises `Event::Live { active: true }` for it — Task 1's event — for the whole of the call, and
    /// `Event::Live { active: false }` when the call ends.
    SessionConfigured { session_id: Option<String> },
    /// `response.output_audio.delta`: PCM16 little-endian mono at 24 kHz (`socket::PCM_RATE`),
    /// base64-decoded and ready for the `Player`.
    AudioDelta { pcm: Vec<u8> },
    /// `conversation.item.input_audio_transcription.delta`: what the server heard, as it transcribes
    /// it. `Event::Heard` is one log line, so the deltas are accumulated and the *completed* event is
    /// deliberately not mapped: it would show the same sentence twice.
    Heard { text: String },
    /// `response.output_audio_transcript.delta`: the assistant's words, as it speaks them.
    ReplyDelta { text: String },
    /// `input_audio_buffer.speech_started`: the user talked over the model, so playback stops now
    /// (the note's interruption section: "The WebSocket client MUST stop playback locally"). The
    /// item-id bookkeeping and the truncate arithmetic are Task 5's.
    SpeechStarted,
    /// `response.done`: the turn is over, with the server's own status (`completed`/`cancelled`).
    ResponseDone { status: String },
    /// `error`: the API refused or failed something. The message is the server's own words.
    ServerError { message: String },
    /// A `type` this build does not know — or one whose existence the research note could not
    /// confirm. Ignored on purpose, never fatal: a Realtime API that grows an event we have never
    /// heard of must not end the call.
    Ignored,
}

/// A *known* event whose payload is not the shape this build expects. Every variant's `Display` names
/// the event and what was missing or wrong, so the HUD's log line is readable and the call ends with
/// a reason rather than a panic (Review Focus 4).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventError {
    #[error("the frame is not JSON: {0}")]
    NotJson(String),
    #[error("{kind} carried no `{name}`")]
    MissingField { kind: String, name: &'static str },
    #[error("{kind} carried `{name}` as {found}, not a string")]
    NotAString { kind: String, name: &'static str, found: &'static str },
    #[error("{kind} carried audio that is not base64: {detail}")]
    BadAudio { kind: String, detail: String },
}

impl EventError {
    /// What the HUD shows for a frame we could not read (`Event::Error.code`): always
    /// `BadResponse`, because that is what it is, and the message says what was wrong.
    pub fn code(&self) -> ErrorCode {
        ErrorCode::BadResponse
    }
}

/// The one mapping the module runs on. Unknown types are `Action::Ignored`; a known type with a
/// payload we cannot read is an `EventError`. Note what is *not* here: the `_done` siblings of the
/// audio and transcript deltas, whose existence the note could not confirm, and `rate_limits.updated`
/// / `response.usage`, which nothing in this plan acts on yet.
pub fn map(frame: &str) -> Result<Action, EventError> {
    let value: Value = serde_json::from_str(frame).map_err(|error| EventError::NotJson(error.to_string()))?;
    let kind = value.get("type").and_then(Value::as_str).ok_or_else(|| EventError::MissingField {
        kind: "<frame>".into(),
        name: "type",
    })?;

    match kind {
        // The first server event and the acknowledgement of our own `session.update`.
        "session.created" | "session.updated" => Ok(Action::SessionConfigured {
            session_id: value.pointer("/session/id").and_then(Value::as_str).map(str::to_string),
        }),

        // INFERRED FIELD (`delta`): the note confirms this `type` verbatim but its payload page
        // truncated, so this arm and its fixture are the single place the guessed shape lives.
        "response.output_audio.delta" => {
            let delta = field(&value, kind, "delta")?;
            let pcm = decode_base64(delta)
                .map_err(|detail| EventError::BadAudio { kind: kind.to_string(), detail })?;
            Ok(Action::AudioDelta { pcm })
        }

        // INFERRED FIELD (`delta`), exactly as above.
        "response.output_audio_transcript.delta" => {
            Ok(Action::ReplyDelta { text: field(&value, kind, "delta")?.to_string() })
        }

        "conversation.item.input_audio_transcription.delta" => {
            Ok(Action::Heard { text: field(&value, kind, "delta")?.to_string() })
        }

        // Stopping playback needs no field at all: the event *is* the instruction.
        "input_audio_buffer.speech_started" => Ok(Action::SpeechStarted),

        "response.done" => {
            let status = value.pointer("/response/status").and_then(Value::as_str).ok_or_else(|| {
                EventError::MissingField { kind: kind.to_string(), name: "response.status" }
            })?;
            Ok(Action::ResponseDone { status: status.to_string() })
        }

        "error" => {
            let message = value.pointer("/error/message").and_then(Value::as_str).ok_or_else(|| {
                EventError::MissingField { kind: kind.to_string(), name: "error.message" }
            })?;
            Ok(Action::ServerError { message: message.to_string() })
        }

        _ => Ok(Action::Ignored),
    }
}

/// A flat string field of a known event, with the two ways it can be wrong kept apart.
fn field<'a>(value: &'a Value, kind: &str, name: &'static str) -> Result<&'a str, EventError> {
    match value.get(name) {
        None => Err(EventError::MissingField { kind: kind.to_string(), name }),
        Some(found) => found.as_str().ok_or_else(|| EventError::NotAString {
            kind: kind.to_string(),
            name,
            found: json_kind(found),
        }),
    }
}

/// What a JSON value is, in words a person reads in the log line.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --lib realtime::events`

Expected: PASS, 11 tests:

```
running 11 tests
test realtime::events::tests::a_known_event_with_a_malformed_payload_is_a_typed_error ... ok
test realtime::events::tests::a_server_error_carries_its_message ... ok
test realtime::events::tests::an_unknown_type_is_ignored_never_fatal ... ok
test realtime::events::tests::frames_that_are_not_events_at_all_are_typed_errors ... ok
test realtime::events::tests::response_done_reports_the_servers_status ... ok
test realtime::events::tests::session_events_configure_the_call ... ok
test realtime::events::tests::speech_started_asks_for_playback_to_stop ... ok
test realtime::events::tests::the_audio_delta_is_decoded_to_pcm ... ok
test realtime::events::tests::the_borrowed_base64_decoder_is_the_one_the_connectors_use ... ok
test realtime::events::tests::the_input_transcript_delta_is_what_was_heard ... ok
test realtime::events::tests::the_output_transcript_delta_is_the_reply_text ... ok

test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Then `cargo test -p tipsy-compadre` (the whole crate: the socket's six, the realtime integration file's eight and the config/model/store suites must all still be green — the one-word change to `gemini::decode_base64` is covered by the Gemini connector's own decoder tests at `providers/gemini.rs:537-542`) and `cargo clippy -p tipsy-compadre --all-targets` (no new warnings).

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/realtime/events.rs crates/tipsy-compadre/src/realtime/mod.rs crates/tipsy-compadre/src/providers/gemini.rs
git commit -m "feat(realtime): map the wire events to the module's actions

One pure map from a JSON text frame to an Action: the session-configured
acknowledgement, the audio delta decoded through the base64 helper Plan 2A wrote
for Gemini, both transcript deltas, speech_started, the response completion, the
server's error event, and an unknown type ignored rather than fatal. A known event
with a malformed payload becomes a typed error with a readable message.

The two payloads the research note marks inferred are pinned here and nowhere
else: the fixtures, the comments above them and the two field() calls that read
`delta` change together if the live smoke test shows another spelling. The
_unconfirmed_ `_done` siblings get a fixture too, as an ignored type.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The turn plumbing `[mid]`

**Files:**
- Create: `crates/tipsy-compadre/src/realtime/mod.rs` — the call façade: the socket task, the
  16↔24 kHz boundary, the transcripts, the call's own deadline
- Modify: `crates/tipsy-compadre/src/lib.rs:15-17` — one line, `pub mod realtime;` (between
  `providers` and `session`)
- Modify: `crates/tipsy-compadre/src/client.rs:56-96` — the `call` field and `Client::new`
- Modify: `crates/tipsy-compadre/src/client.rs:99-163` — `frame_loop`'s exit: the call goes with the
  terminal
- Modify: `crates/tipsy-compadre/src/client.rs:165-216` — `on_message` (the end of a call) and
  `drain_mic` (the microphone belongs to the call while one is live)
- Modify: `crates/tipsy-compadre/src/client.rs:219-238` — `sync_mic` opens the microphone for a call
- Modify: `crates/tipsy-ui/src/state.rs:141-202` — `reply_entry`, used by the `ReplyDelta` and
  `Reply` arms of `UiState::apply`
- Test: `crates/tipsy-compadre/tests/realtime.rs` (new: the local fake socket server and the
  scripted exchanges), `crates/tipsy-compadre/src/realtime/mod.rs` (the pure tests),
  `crates/tipsy-compadre/src/client.rs` (the `mod tests` at `:357-421`),
  `crates/tipsy-ui/src/state.rs` (the `mod tests` at `:359-643`)

**Interfaces:**

- Consumes (read from the committed code, quoted with its lines):
  - `spawn_with_events(config: Config, providers: Providers, brain_config: BrainConfig) -> SessionEnds`
    with `pub struct SessionEnds { pub to_server: UnboundedSender<ToServer>, pub from_server: UnboundedReceiver<ToClient>, pub events: UnboundedSender<ToClient> }`
    — `session.rs:39-49`. A call reports into the same `UnboundedSender<ToClient>` the session uses,
    so the client reads one stream.
  - `pub struct Player` with `pub fn play(&self, samples: Vec<i16>, sample_rate: u32)` (`device.rs:42-54`),
    `pub fn stop(&self)` (`:64-67`), `pub fn playing(&self) -> bool` (`:73-75`);
    `pub const MIC_SAMPLE_RATE: u32 = 16000`, `pub const MIC_FRAME_SAMPLES: usize = 320`
    (`device.rs:17-19`).
  - `pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32>` (`audio/pcm.rs:36-55`),
    `pub fn i16_to_f32(samples: &[i16]) -> Vec<f32>` (`:24-26`),
    `pub fn f32_to_i16(samples: &[f32]) -> Vec<i16>` (`:20-22`),
    `pub fn pcm_s16le_to_pcm(bytes: &[u8], sample_rate: u32) -> Pcm` (`:87-90`),
    `pub struct Pcm { pub samples: Vec<f32>, pub sample_rate: u32 }` with `duration_ms` (`:6-18`).
  - `Client` as it stands: `state`, `player: Option<Player>`, `reply: Vec<i16>`, `rate: u32`,
    `mic_frames`, `mic_tx` (`client.rs:56-88`); `on_message` (`:165-179`), `playing` (`:185-187`),
    `flush_playback` (`:191-195`), `on_speech` (`:198-208`), `drain_mic` (`:211-216`),
    `sync_mic` (`:219-238`), `flush_on_interrupt` (`:277-279`).
  - `fn reply_pending(entry: &LogEntry) -> bool` (`state.rs:41-43`),
    `fn pending_assistant(&mut self) -> Option<&mut LogEntry>` (`:141-143`),
    `LogEntry::pending_assistant(text)` (`:33-35`), the `ReplyDelta`/`Reply` arms of `apply`
    (`:187-202`).
  - `Event::SpeechChunk { seq: u32, sample_rate: u32 }` and `Frame::Speech { seq: u32, samples: Vec<i16> }`
    (`tipsy-protocol/src/lib.rs:128`, `:139`), the pair `brain.rs:435-436` sends and
    `client.rs:167-171` + `:203-205` turn into `Player::play(samples, self.rate)`.
  - `Event::Heard { text }` (`brain.rs:84`), `Event::ReplyDelta { text }` (`brain.rs:215`),
    `Event::Reply { text }` + `Event::Done { first_audio_ms, total_ms }` (`brain.rs:110-115`),
    `Status { state, detail }` (`brain.rs:465-467`).
  - **Assumed from Task 1** (it is being written now, and this task is written against these names;
    the plan author reconciles them): `Event::Live { active: bool }` in `tipsy-protocol`, with a
    behaviour-free arm already present in `UiState::apply` so the workspace compiles; and
    `Config::realtime: RealtimeConfig` with `pub struct RealtimeConfig { pub enabled: bool, pub model: String, pub key: String, pub voice: String, pub max_minutes: u32 }`
    whose defaults are `enabled: false`, `model: "gpt-realtime-2.1"`, `key: "openai"`,
    `voice: "marin"`, `silence_ms: 1000`, `max_minutes: 10`. If Task 1 named the live state
    `AssistantState::Live` instead, the only change here is `call_ended`'s body and every
    `Event::Live { active }` below becomes `Event::Status { state: AssistantState::Live, .. }`; the
    rest of Tasks 4–6 is unchanged.
  - **Assumed from Task 2** (`realtime/socket.rs`; reconcile these five signatures):
    ```rust
    pub struct SessionConfig { pub model: String, pub instructions: String, pub voice: String, pub silence_ms: u32, pub language: String }
    /// Opens `url` with `Authorization: Bearer <key>`, sends the `session.update` built from
    /// `session` (24 kHz `audio/pcm` in and out, `server_vad` at `silence_ms`, input transcription at
    /// `language` so the HUD can show what was heard), and returns once the socket is up. The key is
    /// never logged, never echoed in an error and never put in an event.
    pub async fn connect(url: &str, key: &str, session: &SessionConfig) -> Result<Socket, String>;
    impl Socket {
        /// The next JSON text frame from the server; `None` when the peer closed. A transport
        /// failure is an `Err` whose message is for the log, never for a banner with the key in it.
        pub async fn next_text(&mut self) -> Result<Option<String>, String>;
        pub async fn send_text(&mut self, json: &str) -> Result<(), String>;
        pub async fn close(self) -> Result<(), String>;   // the clean close, with a close frame
    }
    ```
    This task never inspects a close code: the research note marks close codes and reconnect
    semantics as **undocumented**, so any close — clean or not — ends the call, and Task 2 owns
    reopening at the API's 60-minute cap.
  - **Assumed from Task 3** (`realtime/events.rs`; reconcile these names):
    ```rust
    pub enum ModelEvent {
        SessionCreated, SessionUpdated,
        SpeechStarted { item_id: String }, SpeechStopped { item_id: String },
        InputTranscript { transcript: String }, InputTranscriptDelta { delta: String },
        ResponseCreated { response_id: String },
        ResponseAudioDelta { item_id: String, content_index: u32, delta: String },
        OutputTranscriptDelta { delta: String },
        ResponseDone { status: String },
        Error { message: String },
    }
    /// `None` for an event type this crate does not know (an unanticipated event is data, not
    /// death); `Err` for a *known* event whose payload is malformed.
    pub fn parse_event(text: &str) -> Result<Option<ModelEvent>, EventError>;
    /// `input_audio_buffer.append`, with the audio base64 inside the JSON.
    pub fn append_audio_message(pcm_s16le: &[u8]) -> String;
    /// RFC 4648 standard base64, `+`/`/` with `=` padding — the wire's audio encoding in both
    /// directions, hand-written because this plan adds exactly one dependency and it is the socket.
    pub fn base64_encode(bytes: &[u8]) -> String;
    pub fn base64_decode(text: &str) -> Result<Vec<u8>, EventError>;
    ```
    `ResponseAudioDelta`'s `item_id`, `content_index` and `delta` field names are the ones the
    research note marks **inferred, not verified** (its documentation page truncated before the
    Response section). This task pins them with a fixture in `tests/realtime.rs` and says so in the
    fixture's comment; the release checklist's live smoke test is what proves the real API agrees.

- Produces:
  - `pub const API_SAMPLE_RATE: u32 = 24000`
  - `pub fn url(model: &str) -> String`
  - `pub struct CallSetup { pub url: String, pub key: String, pub model: String, pub instructions: String, pub voice: String, pub silence_ms: u32, pub language: String, pub limit: Duration }`
    with `pub fn CallSetup::from_config(config: &Config, key: String) -> CallSetup`
  - `pub fn mic_to_api(frame: &[i16]) -> Vec<i16>` and `pub fn pcm16_le(bytes: &[u8]) -> Vec<i16>`
  - `pub struct Heard` with `pub fn Heard::new() -> Arc<Heard>`, `pub fn begin(&self, item_id: &str, content_index: u32)`, `pub fn clear(&self)` — Task 5 adds the count and the barge-in handshake to this type
  - `pub struct Call` with `pub fn Call::open(setup: CallSetup, heard: Arc<Heard>, out: UnboundedSender<ToClient>) -> Call`, `pub fn push_mic(&self, samples: Vec<i16>)`, `pub fn heard(&self) -> &Arc<Heard>`, `pub fn stop(self)`; `impl Drop for Call`
  - `pub fn call_ended(event: &Event) -> bool` and `pub fn mic_wanted(mic_open: bool, live: bool) -> bool` in `client.rs`
  - `UiState::reply_entry(&mut self) -> &mut LogEntry` in `tipsy-ui`
  - Tasks 5 and 6 build on all of these: Task 5 adds the count to `Heard`, the client's
    `on_speech` bookkeeping and `Call::interrupt`; Task 6 calls `CallSetup::from_config` from the
    key binding.

**What this task decides, and why**

- **A call reports like a session, so nothing downstream learns a new vocabulary.** The socket task
  emits `Event::SpeechChunk` + `Frame::Speech` for each audio delta and `Heard` / `ReplyDelta` /
  `Reply` / `Done` / `Status` for the transcripts, into the same `UnboundedSender<ToClient>` the
  session uses (`session.rs:49-89`). The client's `on_message` therefore plays the call's audio and
  the HUD logs its transcripts through code that already exists, which is the plan's decision —
  "the transcript is the HUD's text" — taken literally.
- **The audio boundary is two functions, and both are tested.** `mic_to_api` is the workspace's own
  resampler over its own conversions (`pcm.rs:24-26`, `:36-55`): 320 samples of 16 kHz become 480
  of 24 kHz, and 20 ms stays 20 ms. `pcm16_le` decodes the API's PCM16 without a float round trip,
  because the samples the device is handed must be the samples the model sent: 960 bytes is 480
  samples at 24 kHz, i.e. 20 ms, and it is announced as `SpeechChunk { sample_rate: 24000 }` so the
  `Player` is told the rate that arrived (Review Focus 5).
- **`Heard::new()` returns `Arc<Heard>` and `Call::open` takes it, rather than owning it.** The
  client fills the count in as it plays (Task 5) on the frame loop while the socket task reads it
  when the server reports a barge-in; the tests need to fill it in by hand, which is only possible
  if the caller keeps a handle.
- **The call has its own deadline, `CallSetup.limit`.** The plan's `[realtime] max_minutes` (default
  10) ends a call by itself and Review Focus 3 says the mode must never be silently on, so `run`
  races the socket against `tokio::time::sleep(limit)` and exits through the same path as every
  other ending. The limit is a `Duration` in `CallSetup` and not a number of minutes so a test can
  use 50 ms instead of waiting ten minutes.
- **`[realtime] enabled = false` is refused before anything else happens.** `start_call` (Task 6)
  checks it first and turns a refusal into a notice; this task only guarantees that the *only* way
  into a call is `Call::open`, which the client calls from one place.
- **The microphone belongs to the call while one is live.** `mic_wanted(mic_open, live)` opens the
  capture stream for a call and `drain_mic` sends those frames to the socket instead of the session,
  so the session's `ListenStart`/`ListenStop` bookkeeping (`session.rs:205-210`) never sees frames
  the call took.
- **A `ReplyDelta` with no placeholder opens one.** The session path always has a placeholder by the
  time a delta arrives (`state.rs:307-325`, and the `Tab` arm at `:249-259`); a call has no key
  press to push it, because the socket starts its own turns. `reply_entry` opens the entry instead of
  dropping the transcript, which is the only behaviour in this task that changes a code path the
  text pipeline also uses — and it changes it for the better: no reply is ever silently dropped.
- **`Esc` ends a call by ending the client.** `frame_loop` drops `self.call` on its way out, and
  `Drop for Call` cancels the socket task, which is how the plan's "closed by it, by `Esc`, or when
  the call's own limit is reached" is satisfied without a second key.

- [ ] **Step 1: Write the failing tests**

In `crates/tipsy-compadre/tests/realtime.rs`, a new file: the local fake server and the nine tests.

```rust
//! The realtime call, driven against a local socket that speaks the same wire.
//!
//! Nothing here touches the real API and no key is ever a fixture: the client is pointed at a
//! loopback port with a placeholder bearer token, and every event is one the research note
//! (`.superpowers/sdd/plan-2c-research/openai-realtime.md`) describes.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tipsy_compadre::realtime::{self, events, Call, CallSetup, Heard};
use tipsy_protocol::{AssistantState, Event, Frame, ToClient};
use tokio::net::TcpListener;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::Message;

/// One scripted server: what the tests make it say, and what they read back.
struct Fake {
    socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
}

/// Binds a loopback port, hands back the `ws://` URL the client is pointed at, and a task that
/// accepts the one connection the call opens.
async fn fake_server() -> (String, tokio::task::JoinHandle<Fake>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let url = format!("ws://{}/v1/realtime?model=gpt-realtime-2.1", listener.local_addr().unwrap());
    let accepting = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("the call connects");
        Fake { socket: tokio_tungstenite::accept_async(stream).await.expect("the handshake") }
    });
    (url, accepting)
}

impl Fake {
    async fn send(&mut self, json: impl Into<String>) {
        self.socket.send(Message::Text(json.into().into())).await.expect("the client is listening");
    }

    /// The next JSON text frame the client sent, or `None` when it closed the socket.
    async fn recv_json(&mut self) -> Option<Value> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    return Some(serde_json::from_str(text.as_str()).expect("JSON"))
                }
                Some(Ok(Message::Close(_))) | None => return None,
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("the socket failed: {e}"),
            }
        }
    }

    /// The handshake both sides agree on: the client sends the session configuration, the server
    /// answers. Its shape is Task 2's; what this task asserts is the 24 kHz the audio format pins,
    /// which is Review Focus 5 — the API's `audio/pcm` supports that one rate and no other.
    async fn handshake(&mut self) {
        let update = self.recv_json().await.expect("session.update");
        assert_eq!(update["type"], "session.update", "the first frame is the session's configuration");
        assert_eq!(update["session"]["audio"]["input"]["format"]["type"], "audio/pcm");
        assert_eq!(update["session"]["audio"]["input"]["format"]["rate"], 24000);
        assert_eq!(update["session"]["audio"]["output"]["format"]["rate"], 24000);
        self.send(r#"{"type":"session.updated","session":{}}"#).await;
    }
}

/// The call the tests open: loopback, a placeholder that is not a key, and a deadline long enough
/// that it never fires unless a test asks for one.
fn setup(url: String) -> CallSetup {
    CallSetup {
        url,
        key: "not-a-key".into(),
        model: "gpt-realtime-2.1".into(),
        instructions: "eres un compae".into(),
        voice: "marin".into(),
        silence_ms: 1000,
        language: "es-ES".into(),
        limit: Duration::from_secs(600),
    }
}

/// One `response.output_audio.delta`, as the research note's event table describes it: base64 PCM16
/// inside a JSON text frame, with the item and content indices a later truncate has to name.
///
/// **The note marks this event's field names as inferred, not verified** — its documentation page
/// truncated before the Response section — so this fixture is where the inference is pinned: the
/// parser must accept `item_id`, `content_index` and `delta`. The release checklist's live smoke
/// test is what proves the real API agrees.
fn delta(item_id: &str, content_index: u32, samples: &[i16]) -> String {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    let audio = events::base64_encode(&pcm);
    format!(
        r#"{{"type":"response.output_audio.delta","response_id":"resp_1","item_id":"{item_id}","output_index":0,"content_index":{content_index},"delta":"{audio}"}}"#
    )
}

/// One `response.output_audio_transcript.delta` — the same inferred payload shape, pinned the same
/// way.
fn transcript_delta(text: &str) -> String {
    format!(
        r#"{{"type":"response.output_audio_transcript.delta","response_id":"resp_1","item_id":"item_a","output_index":0,"content_index":0,"delta":"{text}"}}"#
    )
}

fn response_created(id: &str) -> String {
    format!(r#"{{"type":"response.created","response":{{"id":"{id}"}}}}"#)
}

fn response_done(id: &str, status: &str) -> String {
    format!(r#"{{"type":"response.done","response":{{"id":"{id}","status":"{status}"}}}}"#)
}

/// Waits until the call has reported something `until` is happy with, and returns everything it
/// said. A call that says nothing for five seconds is a failure, not a hang.
async fn collect_until(
    from_call: &mut UnboundedReceiver<ToClient>,
    mut until: impl FnMut(&[ToClient]) -> bool,
) -> Vec<ToClient> {
    let mut seen = Vec::new();
    while !until(&seen) {
        match tokio::time::timeout(Duration::from_secs(5), from_call.recv()).await {
            Ok(Some(message)) => seen.push(message),
            Ok(None) => break,
            Err(_) => panic!("the call reported nothing for 5 s: {seen:#?}"),
        }
    }
    seen
}

fn events_of(seen: &[ToClient]) -> Vec<Event> {
    seen.iter()
        .filter_map(|message| match message {
            ToClient::Event(event) => Some(event.clone()),
            ToClient::Audio(_) => None,
        })
        .collect()
}

fn audio_of(seen: &[ToClient]) -> Vec<(u32, Vec<i16>)> {
    seen.iter()
        .filter_map(|message| match message {
            ToClient::Audio(Frame::Speech { seq, samples }) => Some((*seq, samples.clone())),
            _ => None,
        })
        .collect()
}

fn error_of(seen: &[ToClient]) -> Option<String> {
    events_of(seen).into_iter().find_map(|event| match event {
        Event::Error { message, .. } => Some(message),
        _ => None,
    })
}

/// The last live-state the call reported.
fn live_of(seen: &[ToClient]) -> Option<bool> {
    events_of(seen)
        .into_iter()
        .filter_map(|event| match event {
            Event::Live { active } => Some(active),
            _ => None,
        })
        .last()
}

#[tokio::test]
async fn a_mic_frame_is_appended_as_twenty_milliseconds_of_24k_audio() {
    let (url, accepting) = fake_server().await;
    let (out, _from_call) = tokio::sync::mpsc::unbounded_channel();
    let call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    // One frame as the microphone delivers it: 20 ms of 16 kHz mono PCM16, i.e. 320 samples.
    let frame: Vec<i16> = (0..320).map(|i| ((i as f32 * 0.3).sin() * 12000.0) as i16).collect();
    call.push_mic(frame);

    let append = server.recv_json().await.expect("input_audio_buffer.append");
    assert_eq!(append["type"], "input_audio_buffer.append");
    let audio = append["audio"].as_str().expect("the audio is a base64 string");
    // 24 kHz mono PCM16 is 48 bytes per millisecond (the research note's Interruption 4), so 20 ms
    // of it is 960 bytes: 480 samples, one and a half times the 320 the microphone gave us. A send
    // that skipped the resample would be 320 samples, i.e. 640 bytes and 856 base64 characters.
    assert_eq!(audio.len(), 1280, "960 bytes in base64, i.e. 480 samples at 24 kHz: {audio:?}");
    assert_eq!(audio.len() % 4, 0, "base64 is four characters per three bytes");
}

#[tokio::test]
async fn an_audio_delta_of_twenty_milliseconds_reaches_the_device_at_24000() {
    let (url, accepting) = fake_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    // 20 ms of the API's own audio: 480 samples, 960 bytes.
    let samples: Vec<i16> = (0..480).map(|i| (i as i16 % 200) - 100).collect();
    server.send(response_created("resp_1")).await;
    server.send(delta("item_a", 0, &samples)).await;

    let seen = collect_until(&mut from_call, |seen| audio_of(seen).len() == 1).await;
    let played = audio_of(&seen);
    assert_eq!(played[0].1, samples, "the samples arrive whole, in order and signed");
    assert_eq!(played[0].1.len(), 480, "20 ms at 24 kHz, not 20 ms at 16 kHz");
    // The header the client keeps the device's rate from: `on_message` copies `SpeechChunk.sample_rate`
    // into `Client::rate` (`client.rs:167-171`) and `on_speech` hands it straight to
    // `Player::play(samples, self.rate)` (`:203-205`), so this pair *is* the rate the device is told.
    let header = events_of(&seen).into_iter().find_map(|event| match event {
        Event::SpeechChunk { sample_rate, .. } => Some(sample_rate),
        _ => None,
    });
    assert_eq!(header, Some(24000), "the API's rate, never the microphone's 16000");
}

#[tokio::test]
async fn a_delta_for_another_item_is_not_played() {
    let (url, accepting) = fake_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    server.send(delta("item_a", 0, &[7i16; 480])).await;
    server.send(delta("item_b", 0, &[9i16; 480])).await;

    let seen = collect_until(&mut from_call, |seen| audio_of(seen).len() == 1).await;
    assert_eq!(audio_of(&seen)[0].1, vec![7i16; 480], "the item this response named is played");
    // The second delta names an item this response is not playing — a cancelled response still
    // arriving, or a race between two of them. Its bytes must not be heard, and they must not be
    // counted either: they would be added to the item a later truncate names.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(from_call.try_recv().is_err(), "nothing of `item_b` may reach the device");
}

#[tokio::test]
async fn the_transcripts_of_a_call_reach_the_hud() {
    let (url, accepting) = fake_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    // What the user said, then what the model answered, in the lifecycle order the research note's
    // event table gives: the input transcript completes (step 10) before the response (step 11).
    server
        .send(r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"item_1","content_index":0,"transcript":"¿y el ticket de la cena?"}"#)
        .await;
    server.send(response_created("resp_1")).await;
    server.send(transcript_delta("Aquí")).await;
    server.send(transcript_delta(" andamos.")).await;
    server.send(delta("item_a", 0, &[5i16; 480])).await;
    server.send(response_done("resp_1", "completed")).await;

    let seen = collect_until(&mut from_call, |seen| {
        events_of(seen).iter().any(|event| matches!(event, Event::Done { .. }))
    })
    .await;

    // The three events the text pipeline also sends, in order: `Heard`, then `ReplyDelta`s, then
    // `Reply` — so the log, the tabs and the selection behave the same way in both modes.
    let kinds: Vec<&str> = events_of(&seen)
        .iter()
        .filter_map(|event| match event {
            Event::Heard { .. } => Some("heard"),
            Event::ReplyDelta { .. } => Some("delta"),
            Event::Reply { .. } => Some("reply"),
            Event::Done { .. } => Some("done"),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, ["heard", "delta", "delta", "reply", "done"], "{:#?}", events_of(&seen));

    // And they land in the HUD's own state, which is the point: a call's turns are drawn exactly
    // like the text pipeline's.
    let mut hud = tipsy_ui::UiState::new();
    for message in &seen {
        if let ToClient::Event(event) = message {
            hud.apply(event);
        }
    }
    assert_eq!(hud.log.len(), 2, "{:#?}", hud.log);
    assert_eq!(hud.log[0].who, tipsy_ui::Who::User);
    assert_eq!(hud.log[0].text, "¿y el ticket de la cena?");
    assert_eq!(hud.log[1].who, tipsy_ui::Who::Assistant);
    assert_eq!(hud.log[1].text, "Aquí andamos.");
    assert!(!hud.log[1].pending, "the reply was said: the spinner stops");
    assert!(hud.log[1].first_audio_ms.is_some(), "the reply's timing is a real measurement");
}

#[tokio::test]
async fn a_socket_that_dies_mid_answer_leaves_the_hud_out_of_the_live_state() {
    let (url, accepting) = fake_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    server.send(transcript_delta("Aquí anda")).await;
    server.send(delta("item_a", 0, &[3i16; 480])).await;
    // Mid-answer the socket goes, the way a dead peer leaves it.
    drop(server);

    let seen = collect_until(&mut from_call, |seen| live_of(seen) == Some(false)).await;

    // Review Focus 1: the flag goes down, so the indicator goes down (`UiState::live`, Task 6), and
    // the status comes back to rest so nothing is left spinning.
    assert_eq!(live_of(&seen), Some(false), "the call is over: {seen:#?}");
    assert!(
        events_of(&seen)
            .iter()
            .any(|event| matches!(event, Event::Status { state: AssistantState::Idle, .. })),
        "an ending status must follow: {seen:#?}"
    );
    // A readable error, never an empty string and never a panic.
    let why = error_of(&seen).expect("the user is told what happened");
    assert!(why.contains("socket"), "the message says which side went quiet: {why}");
    // What the reply had said before the socket died is still on the wire: the answer was cut off,
    // not erased.
    assert!(
        events_of(&seen).iter().any(|event| matches!(event, Event::ReplyDelta { text } if text == "Aquí anda")),
        "the partial transcript survives the failure: {seen:#?}"
    );
}

#[tokio::test]
async fn an_error_event_ends_the_call_with_its_own_message() {
    let (url, accepting) = fake_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    // The research note's `error` event: the server's own words, which are what the user gets.
    server
        .send(r#"{"type":"error","error":{"message":"Invalid API key provided","type":"invalid_request_error","code":"invalid_api_key"}}"#)
        .await;

    let seen = collect_until(&mut from_call, |seen| live_of(seen) == Some(false)).await;
    assert_eq!(error_of(&seen).as_deref(), Some("Invalid API key provided"));
    assert!(error_of(&seen).is_some_and(|why| !why.trim().is_empty()));
}

#[tokio::test]
async fn a_closed_port_is_a_readable_error_and_the_call_never_goes_live() {
    // A port nobody is listening on: bind one, note the URL, drop the listener.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let url = format!("ws://{}/v1/realtime?model=gpt-realtime-2.1", listener.local_addr().unwrap());
    drop(listener);

    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);

    let seen = collect_until(&mut from_call, |seen| error_of(seen).is_some()).await;
    assert!(
        !events_of(&seen).iter().any(|event| matches!(event, Event::Live { active: true })),
        "a call that never opened is never live: {seen:#?}"
    );
    // `Live { active: false }` still goes out, so a client holding the handle lets go of it.
    assert_eq!(live_of(&seen), Some(false));
    assert!(error_of(&seen).is_some_and(|why| !why.trim().is_empty()));
}

#[tokio::test]
async fn stopping_the_call_closes_the_socket() {
    let (url, accepting) = fake_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    call.stop();

    assert!(server.recv_json().await.is_none(), "a stopped call closes its socket");
    let seen = collect_until(&mut from_call, |seen| live_of(seen) == Some(false)).await;
    assert_eq!(live_of(&seen), Some(false), "the indicator goes down: {seen:#?}");
    assert!(
        events_of(&seen)
            .iter()
            .any(|event| matches!(event, Event::Status { state: AssistantState::Idle, .. })),
        "and the status comes back to rest: {seen:#?}"
    );
}

#[tokio::test]
async fn a_call_that_reaches_its_limit_ends_itself() {
    let (url, accepting) = fake_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let mut wanted = setup(url);
    // A limit a test can wait for; `[realtime] max_minutes` is the same deadline in minutes.
    wanted.limit = Duration::from_millis(50);
    let _call = Call::open(wanted, Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    let seen = collect_until(&mut from_call, |seen| live_of(seen) == Some(false)).await;
    let why = error_of(&seen).expect("the user is told why the call ended");
    assert!(why.contains("limit"), "the message names the limit: {why}");
    assert!(server.recv_json().await.is_none(), "a call at its limit closes its socket");
}
```

In `crates/tipsy-compadre/src/realtime/mod.rs` — the file does not exist yet, so these five pure
tests go in the `#[cfg(test)] mod tests` this task creates:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::pcm::Pcm;

    #[test]
    fn the_endpoint_carries_the_model_as_a_query() {
        // The research note's Endpoints section, verbatim: the socket a native client opens, with
        // the model as a query parameter. The id is Task 1's default for `[realtime] model`.
        assert_eq!(url("gpt-realtime-2.1"), "wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1");
    }

    #[test]
    fn a_microphone_frame_becomes_one_and_a_half_times_as_many_samples() {
        // 20 ms of 16 kHz mono is 320 samples; the same 20 ms at 24 kHz is 480. Review Focus 5:
        // nothing may be sent at the wrong rate, and a missed resample is a 1.0 ratio.
        let frame = vec![1000i16; 320];
        let up = mic_to_api(&frame);
        assert_eq!(up.len(), 480, "16 kHz → 24 kHz is 3/2 the samples");
        assert_eq!(Pcm { samples: i16_to_f32(&up), sample_rate: API_SAMPLE_RATE }.duration_ms(), 20);
        // A constant stays constant within one least-significant bit (the f32 conversion rounds).
        for sample in &up {
            assert!((i32::from(*sample) - 1000).abs() <= 1, "the resampler invented a signal: {sample}");
        }
        assert!(mic_to_api(&[]).is_empty());
    }

    #[test]
    fn the_apis_audio_is_sixteen_bit_little_endian() {
        // The API's PCM16, decoded without the f32 round trip `pcm_s16le_to_pcm` takes
        // (`audio/pcm.rs:87-90`), because the samples the device is handed must be the ones the
        // model sent.
        assert_eq!(pcm16_le(&[0x00, 0x40, 0x00, 0xC0]), vec![16384, -16384]);
        assert_eq!(pcm16_le(&[1, 2, 3]), vec![513], "an odd trailing byte is not a sample");
        assert!(pcm16_le(&[]).is_empty());
    }

    #[test]
    fn the_audio_the_client_appends_is_base64_of_little_endian_samples() {
        // RFC 4648, pinned against a hand-written vector rather than against the crate's own
        // encoder: a codec that only agrees with itself is caught here. `-1, 0` as little-endian
        // PCM16 is `ff ff 00 00`.
        assert_eq!(events::base64_encode(&[0xff, 0xff, 0x00, 0x00]), "//8AAA==");
        assert_eq!(events::base64_decode("//8AAA==").unwrap(), vec![0xff, 0xff, 0x00, 0x00]);
        let mut bytes = Vec::new();
        for sample in [-1i16, 0] {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        assert_eq!(events::base64_encode(&bytes), "//8AAA==");
    }

    #[test]
    fn the_call_asks_for_the_configured_model_voice_and_prompt() {
        let config = Config::default();
        let wanted = CallSetup::from_config(&config, "not-a-key".into());
        assert_eq!(wanted.url, "wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1");
        // The profile's own prompt is the session's `instructions`, exactly as the brain sends it as
        // the system message (`brain.rs:318-324`).
        assert_eq!(wanted.instructions, config.active().prompt);
        // The voice comes from `[realtime] voice`, deliberately *not* from the profile's `voices`
        // table, which maps speech connectors to voices and this mode is not one of them.
        assert_eq!(wanted.voice, "marin");
        assert_eq!(wanted.language, config.active().language, "input transcription in the profile's language");
        assert_eq!(wanted.limit, Duration::from_secs(600), "`[realtime] max_minutes`, 10 by default");
    }
}
```

In `crates/tipsy-compadre/src/client.rs`, inside the existing `mod tests`, three tests (and the
`AssistantState` import is already there at `:360`):

```rust
    #[test]
    fn the_microphone_runs_when_the_user_opened_it_or_when_a_call_is_live() {
        assert!(mic_wanted(true, false), "the microphone key still opens it");
        assert!(mic_wanted(false, true), "a call streams the microphone for as long as it lasts");
        assert!(mic_wanted(true, true));
        assert!(!mic_wanted(false, false));
    }

    #[test]
    fn a_chunk_header_sets_the_rate_the_device_is_told() {
        let mut client = Client::new(Config::default());
        client.on_message(ToClient::Event(Event::SpeechChunk { seq: 0, sample_rate: 24000 }));
        client.on_message(ToClient::Audio(Frame::Speech { seq: 0, samples: vec![5, 6] }));
        // There is no output device in a test — `Player::new` needs one — so the playback itself is
        // skipped; what is pinned is the rate it would be handed, which is the announced one and
        // never a guess about which mode is running.
        assert_eq!(client.rate, 24000);
        assert_eq!(client.reply, vec![5, 6], "the reply is kept for `F2` as it always was");
    }

    #[test]
    fn the_client_lets_go_of_a_call_when_the_socket_says_it_is_over() {
        // A `Call` needs a socket, so what this pins is the rule: a call's ending is a live-flag
        // event and nothing else. A status change must never be mistaken for it — that is what
        // stops a finished reply from being cut off mid-sentence (`client.rs:269-279`).
        assert!(call_ended(&Event::Live { active: false }));
        assert!(!call_ended(&Event::Live { active: true }));
        assert!(!call_ended(&Event::Status { state: AssistantState::Idle, detail: None }));
        let mut client = Client::new(Config::default());
        client.on_message(ToClient::Event(Event::Live { active: false }));
        assert!(client.call.is_none(), "there was no call, and none is invented");
    }
```

In `crates/tipsy-ui/src/state.rs`, inside the existing `mod tests`:

```rust
    #[test]
    fn a_transcript_with_no_placeholder_still_lands_in_the_log() {
        let mut s = UiState::new();
        // A call: the socket starts and ends its own turns, so no key press pushed the placeholder
        // the session path pushes in `submit` (`:307-325`) and in the microphone key (`:249-259`).
        s.apply(&Event::Heard { text: "¿y el ticket?".into() });
        s.apply(&Event::ReplyDelta { text: "Aquí" });
        s.apply(&Event::ReplyDelta { text: " andamos." });
        assert_eq!(s.log.len(), 2, "{:#?}", s.log);
        assert_eq!(s.log[0].who, Who::User);
        assert_eq!(s.log[1].who, Who::Assistant);
        assert_eq!(s.log[1].text, "Aquí andamos.");
        assert!(s.log[1].pending, "the reply is still being said");

        s.apply(&Event::Done { first_audio_ms: Some(300), total_ms: 1200 });
        assert!(!s.log[1].pending);
        assert_eq!(s.log[1].first_audio_ms, Some(300));
        assert!(!s.busy());
    }
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --test realtime`

Expected: FAIL — the test target does not compile:

```
error[E0432]: unresolved import `tipsy_compadre::realtime`
 --> crates\tipsy-compadre\tests\realtime.rs:10:32
error[E0432]: unresolved import `tipsy_compadre::realtime::events`
error[E0599]: no variant or associated item named `Live` found for enum `Event` in the current scope
```

Run: `cargo test -p tipsy-compadre --lib realtime`

Expected: FAIL, `error[E0583]: file not found for module `realtime`` (or, if only `lib.rs`'s line is
added first, `error[E0425]: cannot find function `url`` and one `E0433` per missing item).

Run: `cargo test -p tipsy-ui --lib a_transcript_with_no_placeholder`

Expected: FAIL — the test compiles but asserts nothing that holds yet:

```
---- state::tests::a_transcript_with_no_placeholder_still_lands_in_the_log stdout ----
thread '...' panicked at crates\tipsy-ui\src\state.rs:...
assertion `left == right` failed: [
    LogEntry { who: User, ... },
]
  left: 1
 right: 2
```

(`ReplyDelta` with no pending entry is dropped by `apply`'s `pending_assistant()`, so no reply row
is ever opened.)

- [ ] **Step 3: Create the call façade**

`crates/tipsy-compadre/src/lib.rs` — one line between `providers` (line 15) and `session` (line 17):

```rust
pub mod realtime;
```

`crates/tipsy-compadre/src/realtime/mod.rs` — the new file. Keep Task 2's and Task 3's `pub mod`
lines at the top if they left a skeleton here:

```rust
//! The realtime call: one duplex socket's turn plumbing, beside the session's pipeline.
//!
//! While a call is live the microphone goes up as mono 24 kHz PCM16 and the model's audio comes down
//! as the same `SpeechChunk`/`Frame::Speech` pair a spoken sentence arrives in
//! (`brain.rs:435-436`), so the client plays it with the `Player` it already has and the HUD logs
//! the same transcript events the text pipeline sends. This module owns the call's own bookkeeping:
//! the socket task, the frames in, the events out, the item the device is playing and the call's
//! deadline.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tipsy_protocol::{AssistantState, ErrorCode, Event, Frame, ToClient};
use tokio::sync::mpsc::{Receiver, Sender, UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::audio::device::MIC_SAMPLE_RATE;
use crate::audio::pcm::{f32_to_i16, i16_to_f32, resample_linear};
use crate::config::Config;
use crate::realtime::events::{self, ModelEvent};
use crate::realtime::socket::{SessionConfig, Socket};

pub mod events;
pub mod socket;

/// The rate the Realtime API takes and returns audio at: mono PCM16, 24 kHz, and the only rate its
/// `audio/pcm` format accepts (research note, Audio).
pub const API_SAMPLE_RATE: u32 = 24000;

/// Microphone frames that may wait for the socket before the oldest are dropped: about a fifth of a
/// second, the same trade the client's own microphone bridge makes (`client.rs:223-227`).
const MIC_QUEUE: usize = 16;

/// The endpoint one call opens: the model is a query parameter, not part of the path (research note,
/// Endpoints) — `wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1`.
pub fn url(model: &str) -> String {
    format!("wss://api.openai.com/v1/realtime?model={model}")
}

/// One call, as the client asks for it.
///
/// `[realtime]` supplies everything but the key, which the client resolves and keeps: the key is
/// carried to the handshake and appears nowhere else — not in an event, not in an error message, not
/// in a fixture.
#[derive(Debug, Clone, PartialEq)]
pub struct CallSetup {
    pub url: String,
    pub key: String,
    pub model: String,
    pub instructions: String,
    pub voice: String,
    pub silence_ms: u32,
    pub language: String,
    /// The call's own deadline, from `[realtime] max_minutes`: a call ends by itself here, whatever
    /// the socket is doing (Review Focus 3).
    pub limit: Duration,
}

impl CallSetup {
    /// From the configuration and the active profile. The *voice* deliberately comes from
    /// `[realtime] voice` and not from `Profile::voices`, which maps speech connectors to voices and
    /// this mode is not one of them.
    pub fn from_config(config: &Config, key: String) -> CallSetup {
        let realtime = &config.realtime;
        CallSetup {
            url: url(&realtime.model),
            key,
            model: realtime.model.clone(),
            instructions: config.active().prompt.clone(),
            voice: realtime.voice.clone(),
            silence_ms: realtime.silence_ms,
            language: config.active().language.clone(),
            limit: Duration::from_secs(u64::from(realtime.max_minutes) * 60),
        }
    }
}

/// One microphone frame as the API takes it: mono 16 kHz in, mono 24 kHz out, PCM16 either way.
///
/// The microphone delivers 16 kHz (`device.rs:17`) and the API's `audio/pcm` takes 24 kHz and
/// nothing else, so the conversion happens here, once, with the workspace's own resampler over its
/// own i16 conversion.
pub fn mic_to_api(frame: &[i16]) -> Vec<i16> {
    f32_to_i16(&resample_linear(&i16_to_f32(frame), MIC_SAMPLE_RATE, API_SAMPLE_RATE))
}

/// The API's audio as samples: little-endian PCM16, one sample per two bytes, in order.
pub fn pcm16_le(bytes: &[u8]) -> Vec<i16> {
    bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}

/// What the call knows about the audio the device is playing.
///
/// This task keeps the item it belongs to — the id and content index a truncate would have to name.
/// Task 5 adds the count of what has actually been heard and the handshake with the client's frame
/// loop, which is the only place the device's own queue is visible.
#[derive(Debug, Default)]
pub struct Heard {
    item: Mutex<Option<(String, u32)>>,
}

impl Heard {
    pub fn new() -> Arc<Heard> {
        Arc::new(Heard::default())
    }

    /// The first audio delta of a response names the item the device is playing.
    pub fn begin(&self, item_id: &str, content_index: u32) {
        *self.item.lock().expect("the item lock is never held across a panic") =
            Some((item_id.to_string(), content_index));
    }

    /// Forgets the item: the call ended, or a barge-in took it away.
    pub fn clear(&self) {
        *self.item.lock().expect("the item lock is never held across a panic") = None;
    }
}

/// One call's handle, held by the client.
pub struct Call {
    mic: Sender<Vec<i16>>,
    heard: Arc<Heard>,
    cancel: CancellationToken,
}

impl Call {
    /// Opens one call.
    ///
    /// The socket is opened on a task of its own, because the client's frame loop must never wait
    /// for a handshake, and everything the model produces is reported through `out`, exactly as a
    /// session reports its own.
    pub fn open(setup: CallSetup, heard: Arc<Heard>, out: UnboundedSender<ToClient>) -> Call {
        let (mic, frames) = tokio::sync::mpsc::channel(MIC_QUEUE);
        let cancel = CancellationToken::new();
        tokio::spawn(run(setup, Arc::clone(&heard), frames, out, cancel.clone()));
        Call { mic, heard, cancel }
    }

    /// Queues one microphone frame for the socket. Bounded and lossy: a busy client drops a frame
    /// rather than stalling the device.
    pub fn push_mic(&self, samples: Vec<i16>) {
        let _ = self.mic.try_send(samples);
    }

    /// The bookkeeping the client fills in as it plays: Task 5's barge-in count.
    pub fn heard(&self) -> &Arc<Heard> {
        &self.heard
    }

    /// Ends the call: the socket is closed and the task finishes. Dropping the handle does the same,
    /// which is how `Esc` ends a call by ending the client.
    pub fn stop(self) {
        self.cancel.cancel();
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// One response in flight: what the HUD is told when it ends.
#[derive(Default)]
struct Turn {
    started: Option<Instant>,
    first_audio_ms: Option<u64>,
    text: String,
    /// True once any of this response's audio reached the device.
    spoke: bool,
    /// True once the HUD was told the assistant is speaking.
    speaking: bool,
    /// The sequence number of the next `SpeechChunk`/`Frame::Speech` pair.
    seq: u32,
    /// The audio item this response is playing, once its first delta named one.
    item: Option<(String, u32)>,
}

/// What one server event did to the call.
enum Step {
    Continue,
    Stop,
}

/// The sentence the user reads when the socket goes quiet, on whichever side it happened.
const SOCKET_CLOSED: &str = "the call's socket closed while the model was speaking";

async fn run(
    setup: CallSetup,
    heard: Arc<Heard>,
    mut frames: Receiver<Vec<i16>>,
    out: UnboundedSender<ToClient>,
    cancel: CancellationToken,
) {
    let session = SessionConfig {
        model: setup.model,
        instructions: setup.instructions,
        voice: setup.voice,
        silence_ms: setup.silence_ms,
        language: setup.language,
    };
    let mut socket = match Socket::connect(&setup.url, &setup.key, &session).await {
        Ok(socket) => socket,
        Err(e) => {
            // Nothing was ever live, and the client is told why; `Live { active: false }` still goes
            // out so a client holding the handle lets go of it.
            report(&out, ErrorCode::Network, &format!("the call could not open: {e}"));
            ended(&out);
            return;
        }
    };
    // The key has done its job: it is in the handshake and nowhere else.
    send(&out, ToClient::Event(Event::Live { active: true }));
    send(&out, status(AssistantState::Listening));

    // `[realtime] max_minutes`: a call ends by itself here (Review Focus 3), through the same exit
    // as every other ending.
    let deadline = tokio::time::sleep(setup.limit);
    tokio::pin!(deadline);
    let mut turn = Turn::default();

    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            () = &mut deadline => {
                report(&out, ErrorCode::Timeout, "the call reached its limit ([realtime] max_minutes)");
                break;
            }
            frame = frames.recv() => {
                let Some(frame) = frame else { break };
                let pcm = mic_to_api(&frame);
                if pcm.is_empty() {
                    continue;
                }
                if socket.send_text(&events::append_audio_message(&pcm)).await.is_err() {
                    report(&out, ErrorCode::Network, SOCKET_CLOSED);
                    break;
                }
            }
            text = socket.next_text() => {
                match text {
                    Ok(Some(text)) => {
                        if let Step::Stop = on_event(&text, &mut socket, &mut turn, &heard, &out).await {
                            break;
                        }
                    }
                    // A close, clean or not, ends the call: the research note marks close codes and
                    // reconnect semantics as undocumented, so no code is read and Task 2 owns
                    // reopening at the API's 60-minute cap.
                    Ok(None) => {
                        report(&out, ErrorCode::Network, SOCKET_CLOSED);
                        break;
                    }
                    Err(e) => {
                        report(&out, ErrorCode::Network, &format!("the call's socket failed: {e}"));
                        break;
                    }
                }
            }
        }
    }
    let _ = socket.close().await;
    ended(&out);
}

/// One text frame from the server.
async fn on_event(
    text: &str,
    socket: &mut Socket,
    turn: &mut Turn,
    heard: &Arc<Heard>,
    out: &UnboundedSender<ToClient>,
) -> Step {
    // An event this crate does not know is data, not death (Review Focus 4): `parse_event` answers
    // `None` for a type we have never seen. Only a *known* event with a malformed payload is an
    // error, and then it is a typed one that ends the call in words the user can read.
    let event = match events::parse_event(text) {
        Ok(Some(event)) => event,
        Ok(None) => return Step::Continue,
        Err(e) => {
            report(out, ErrorCode::BadResponse, &format!("the call sent an event we cannot read: {e}"));
            return Step::Stop;
        }
    };
    match event {
        // The user talked over the model. Task 5 acts on this; the plumbing that matters here is
        // that the item the device is playing is the call's own bookkeeping, not the event's.
        ModelEvent::SpeechStarted { .. } => {
            heard.clear();
        }
        ModelEvent::ResponseCreated { .. } => {
            *turn = Turn { started: Some(Instant::now()), ..Turn::default() };
            send(out, status(AssistantState::Thinking));
        }
        ModelEvent::ResponseAudioDelta { item_id, content_index, delta } => {
            // The first delta of a response names the item it belongs to.
            if turn.item.is_none() {
                turn.item = Some((item_id.clone(), content_index));
                heard.begin(&item_id, content_index);
            }
            // A delta naming anything else belongs to a response we are not playing — a cancelled
            // one still arriving — and is dropped rather than played: its bytes must reach neither
            // the device nor the count a later truncate is derived from.
            if turn.item.as_ref().is_some_and(|(id, _)| *id != item_id) {
                return Step::Continue;
            }
            let samples = pcm16_le(&delta);
            if samples.is_empty() {
                return Step::Continue;
            }
            turn.spoke = true;
            if turn.first_audio_ms.is_none() {
                turn.first_audio_ms = Some(elapsed_ms(turn.started));
            }
            if !turn.speaking {
                turn.speaking = true;
                send(out, status(AssistantState::Speaking));
            }
            // The rate header and the audio itself: the same pair a spoken sentence arrives as, so
            // the client hands the device the rate that actually arrived (Review Focus 5).
            send(out, ToClient::Event(Event::SpeechChunk { seq: turn.seq, sample_rate: API_SAMPLE_RATE }));
            send(out, ToClient::Audio(Frame::Speech { seq: turn.seq, samples }));
            turn.seq += 1;
        }
        ModelEvent::OutputTranscriptDelta { delta } => {
            turn.text.push_str(&delta);
            send(out, ToClient::Event(Event::ReplyDelta { text: delta }));
        }
        // What the user said, once, whole: the HUD's `Heard` inserts a row per event
        // (`state.rs:179-186`), so the growing `...input_audio_transcription.delta` of step 9 is
        // deliberately not forwarded.
        ModelEvent::InputTranscript { transcript } => {
            send(out, ToClient::Event(Event::Heard { text: transcript }));
        }
        ModelEvent::ResponseDone { .. } => {
            finish(turn, out);
            // The call is still live and the microphone is still open: the resting state of a call
            // is listening, never idle.
            send(out, status(AssistantState::Listening));
        }
        ModelEvent::Error { message } => {
            report(out, ErrorCode::BadResponse, &message);
            return Step::Stop;
        }
        ModelEvent::SessionCreated
        | ModelEvent::SessionUpdated
        | ModelEvent::SpeechStopped { .. }
        | ModelEvent::InputTranscriptDelta { .. } => {}
    }
    Step::Continue
}

/// Says what a response said, in the events the text pipeline also sends, so the HUD's entry stops
/// waiting and keeps its timing (`brain.rs:110-115`). A response that neither spoke nor transcribed
/// leaves the log alone.
fn finish(turn: &mut Turn, out: &UnboundedSender<ToClient>) {
    let total_ms = elapsed_ms(turn.started);
    if !turn.spoke && turn.text.is_empty() {
        return;
    }
    send(out, ToClient::Event(Event::Reply { text: std::mem::take(&mut turn.text) }));
    send(out, ToClient::Event(Event::Done { first_audio_ms: turn.first_audio_ms, total_ms }));
}

fn elapsed_ms(started: Option<Instant>) -> u64 {
    started.map_or(0, |at| at.elapsed().as_millis() as u64)
}

/// The call is over, whatever ended it: a clean close, an error event, a transport failure, the
/// deadline or the key. The HUD leaves its live state in every one of those cases (Review Focus 1).
fn ended(out: &UnboundedSender<ToClient>) {
    send(out, ToClient::Event(Event::Live { active: false }));
    send(out, status(AssistantState::Idle));
}

fn report(out: &UnboundedSender<ToClient>, code: ErrorCode, message: &str) {
    send(out, ToClient::Event(Event::Error { code, message: message.to_string(), recoverable: true }));
}

fn status(state: AssistantState) -> ToClient {
    ToClient::Event(Event::Status { state, detail: None })
}

fn send(out: &UnboundedSender<ToClient>, message: ToClient) {
    let _ = out.send(message);
}
```

- [ ] **Step 4: Wire the client and the HUD's reply entry**

`crates/tipsy-compadre/src/client.rs` — the imports gain the call's names:

```rust
use crate::realtime::{Call, Heard};
```

`struct Client` (`:57-71`) gains one field:

```rust
    /// The live call, when one is running: the microphone feeds it instead of the session and the
    /// audio the device plays arrives from it.
    call: Option<Call>,
```

`Client::new` (`:74-88`) gains `call: None,` in the initialiser, and `frame_loop` ends with one
more line before `Ok(())`:

```rust
        // Silence the device and stop capturing before the terminal comes back.
        self.flush_playback();
        self.mic = None;
        // And close the call with the terminal: `Esc` ends a call by ending the client.
        self.call = None;
```

`on_message` (`:165-179`):

```rust
    fn on_message(&mut self, message: ToClient) {
        match message {
            ToClient::Event(event) => {
                if let Event::SpeechChunk { sample_rate, .. } = &event {
                    self.rate = *sample_rate;
                }
                // A call reports that it is over through `Live { active: false }`, whatever ended
                // it. The client lets go of the handle and silences the device here, because the
                // device is only reachable from this thread and the audio still queued belongs to a
                // conversation that is gone (Review Focus 1).
                if call_ended(&event) {
                    self.end_call();
                }
                // No event ever touches the queued audio or the audio kept for `Replay`: the status
                // that ends a reply arrives while the device is still speaking it, so the only
                // interruption is one the user asks for, in `on_action`.
                self.state.apply(&event);
            }
            ToClient::Audio(Frame::Speech { samples, .. }) => self.on_speech(samples),
            ToClient::Audio(Frame::Mic(_)) => {}
        }
    }
```

`drain_mic` (`:211-216`) routes a live call's microphone away from the session:

```rust
    /// Forwards every captured frame and shows its loudness in the oscilloscope.
    fn drain_mic(&mut self, to_server: &UnboundedSender<ToServer>) {
        while let Ok(samples) = self.mic_frames.try_recv() {
            match &self.call {
                // A call owns the microphone while it is live: the session's own
                // `ListenStart`/`ListenStop` bookkeeping must not see frames it never asked for.
                Some(call) => call.push_mic(samples.clone()),
                None => {
                    let _ = to_server.send(ToServer::Audio(Frame::Mic(samples.clone())));
                }
            }
            self.state.push_mic_level(rms(&i16_to_f32(&samples)) * self.mic_gain);
        }
    }
```

`sync_mic` (`:219-238`) opens the microphone for a call too:

```rust
    /// Opens or closes the microphone so it matches what wants it: the microphone key, or a live
    /// call, which streams until it ends.
    fn sync_mic(&mut self) {
        let wanted = mic_wanted(self.state.mic_open, self.call.is_some());
        match (wanted, self.mic.is_some()) {
```

and the two new items go beside `flush_on_interrupt` (`:277`):

```rust
/// Whether a live-state event ends the call here.
///
/// The socket task decides when a call is over — it says so with `Live { active: false }` — and the
/// client's only job is to let the handle go and quiet the device. A `Live { active: true }` is a
/// call starting, which `start_call` already holds the handle for.
fn call_ended(event: &Event) -> bool {
    matches!(event, Event::Live { active: false })
}

/// Whether the microphone stream should be open: the user asked for it, or a call is live and
/// streams it for as long as it lasts.
fn mic_wanted(mic_open: bool, live: bool) -> bool {
    mic_open || live
}
```

`Client` gains the method `end_call` beside `flush_playback` (`:191-195`):

```rust
    /// Lets go of the call and silences the device: the audio still queued belongs to a call that is
    /// over, which is the same rule the user's own barge-in follows (`flush_on_interrupt`, `:277-279`).
    fn end_call(&mut self) {
        self.call = None;
        if flush_on_interrupt(true, self.playing()) {
            self.flush_playback();
        }
    }
```

`crates/tipsy-ui/src/state.rs` — replace `pending_assistant`'s two callers with a method that opens
the entry, and add the method after `pending_assistant` (`:141-143`):

```rust
    /// The entry a streamed reply fills: the pending one if there is one, and a fresh placeholder if
    /// there is not.
    ///
    /// The session's own path always has a placeholder by the time a delta arrives — `submit`
    /// (`:307-325`) and the microphone key (`:249-259`) push it — but a live call has no key press
    /// to push it with, because the socket starts and ends its turns on its own. Without this the
    /// first line of a call's reply would be dropped on the floor, and a reply nobody pressed
    /// anything for still has to be readable.
    fn reply_entry(&mut self) -> &mut LogEntry {
        if !self.log.iter().any(reply_pending) {
            self.log.push(LogEntry::pending_assistant("…"));
        }
        self.pending_assistant().expect("an entry was just ensured")
    }
```

and in `apply`, the two arms (`:187-202`) become:

```rust
            Event::ReplyDelta { text } => {
                let e = self.reply_entry();
                if e.text == "…" {
                    e.text.clear();
                }
                e.text.push_str(text);
            }
            Event::Reply { text } => {
                let e = self.reply_entry();
                e.text = text.clone();
                // A recoverable error is advisory: the reply it sat in front of has arrived, and the
                // entry is the reply, not the warning that preceded it.
                e.error = None;
            }
```

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --test realtime`

Expected: PASS, 9 tests:

```
running 9 tests
test a_call_that_reaches_its_limit_ends_itself ... ok
test a_closed_port_is_a_readable_error_and_the_call_never_goes_live ... ok
test a_delta_for_another_item_is_not_played ... ok
test a_mic_frame_is_appended_as_twenty_milliseconds_of_24k_audio ... ok
test a_socket_that_dies_mid_answer_leaves_the_hud_out_of_the_live_state ... ok
test an_audio_delta_of_twenty_milliseconds_reaches_the_device_at_24000 ... ok
test an_error_event_ends_the_call_with_its_own_message ... ok
test stopping_the_call_closes_the_socket ... ok
test the_transcripts_of_a_call_reach_the_hud ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.42s
```

Run: `cargo test -p tipsy-compadre --lib realtime`

Expected: PASS, 5 tests:

```
running 5 tests
test realtime::tests::a_microphone_frame_becomes_one_and_a_half_times_as_many_samples ... ok
test realtime::tests::the_apis_audio_is_sixteen_bit_little_endian ... ok
test realtime::tests::the_audio_the_client_appends_is_base64_of_little_endian_samples ... ok
test realtime::tests::the_call_asks_for_the_configured_model_voice_and_prompt ... ok
test realtime::tests::the_endpoint_carries_the_model_as_a_query ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Run: `cargo test -p tipsy-compadre --lib client`

Expected: PASS, 6 tests:

```
running 6 tests
test client::tests::a_chunk_header_sets_the_rate_the_device_is_told ... ok
test client::tests::a_finished_reply_keeps_its_audio_until_a_new_turn_starts ... ok
test client::tests::only_an_interrupt_with_audio_left_to_drop_flushes ... ok
test client::tests::the_client_lets_go_of_a_call_when_the_socket_says_it_is_over ... ok
test client::tests::the_commands_that_end_a_turn_are_the_ones_that_interrupt_it ... ok
test client::tests::the_microphone_runs_when_the_user_opened_it_or_when_a_call_is_live ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

Run: `cargo test -p tipsy-ui --lib a_transcript_with_no_placeholder`

Expected: PASS, `1 passed`.

- [ ] **Step 6: Prove the workspace is whole**

Run: `cargo test --workspace`

Expected: all green — the protocol's, the UI's render/state suites and the compadre's
session/brain/client/provider suites, which are the regression net for `reply_entry`: `Reply`,
`ReplyDelta` and `Done` are exercised there for the text pipeline, so an entry opened twice or a
timing lost shows up immediately.

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: `Finished`, no warnings. `Heard`'s mutex is `std::sync::Mutex` and is never held across an
`.await`, so no `await_holding_lock`; the call's task is a `tokio::spawn`, so nothing in `tipsy-ui`
or `tipsy-protocol` gained a `tokio` dependency (a Global Constraint of this plan).

- [ ] **Step 7: Commit**

```bash
git add crates/tipsy-compadre/src/lib.rs crates/tipsy-compadre/src/realtime/mod.rs crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/tests/realtime.rs crates/tipsy-ui/src/state.rs
git commit -m "feat(realtime): plumb a live call's turns

The microphone's 16 kHz frames are resampled to the API's 24 kHz and appended,
the model's deltas are decoded as PCM16 and handed to the existing Player behind
a SpeechChunk that names 24000, and the transcripts are reported as the heard,
delta, reply and done events the text pipeline already sends. A call reports
through the session's own channel, so the HUD needs only a reply entry that
opens itself when no key press pushed one. The socket's death, an error event,
the call's own limit and the key all end a call through the same exit, which
takes the HUD out of its live state.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: Barge-in `[mid]`

**Files:**
- Modify: `crates/tipsy-compadre/src/realtime/mod.rs` — `BYTES_PER_MS`, `ms_of`, `played_bytes`, the
  count and the interrupt handshake on `Heard`, the `Control` channel with `Call::interrupt`, the
  `SpeechStarted` and `ResponseCreated` paths of `on_event`, and the `silenced` flag on `Turn`
- Modify: `crates/tipsy-compadre/src/realtime/events.rs` — add `truncate_message` and
  `cancel_message` (Task 3 owns the rest of the file)
- Modify: `crates/tipsy-compadre/src/audio/device.rs:24-82` — `Player::progress`, and
  `use std::time::Duration`
- Modify: `crates/tipsy-compadre/src/client.rs:99-163` — call `sync_call` once per frame
- Modify: `crates/tipsy-compadre/src/client.rs:191-267` — `on_speech` fills the count in,
  `sync_call`, and `on_action`'s live diversion with `takes_the_floor`
- Test: `crates/tipsy-compadre/tests/realtime.rs` (four more tests), `crates/tipsy-compadre/src/realtime/mod.rs`
  (five more pure tests), `crates/tipsy-compadre/src/client.rs` (one more test),
  `crates/tipsy-compadre/src/audio/device.rs` (one more test)

**Interfaces:**

- Consumes (from Task 4, as it was committed): `pub struct Heard` with `new()->Arc<Heard>`,
  `begin`, `clear`; `pub struct Call` with `open`, `push_mic`, `heard() -> &Arc<Heard>`, `stop`;
  `pub struct CallSetup`; `pub const API_SAMPLE_RATE: u32 = 24000`; `struct Turn` (whose `item` field
  is what names the item a truncate may carry); `async fn on_event`; `const SOCKET_CLOSED`;
  `report`, `status`, `send`, `ended`. Task 4 deliberately left the interrupt path out — the arm
  would have needed this task's `cancel_message` and an enum variant nothing constructs yet — so this
  task adds `Control`, the channel that carries it and `Call::interrupt` itself.
  From the real code: `pub fn play(&self, samples: Vec<i16>, sample_rate: u32)` (`device.rs:42-54`),
  `pub fn playing(&self) -> bool` (`:73-75`), `flush_on_interrupt` (`client.rs:277-279`),
  `flush_playback` (`:191-195`), `on_action` (`:241-266`), `frame_loop` (`:99-163`).
- Consumes — **assumed from Task 3** (`realtime/events.rs`), and this task is written against these
  two names; if Task 3 already added them, they are used unchanged and this task does not declare a
  second pair:
  ```rust
  /// `conversation.item.truncate`, exactly the shape the research note quotes: the assistant item,
  /// `content_index: 0`, and the inclusive milliseconds of audio that was played.
  pub fn truncate_message(item_id: &str, content_index: u32, audio_end_ms: u64) -> String;
  /// `response.cancel`, with no `response_id`: the research note marks it safe to send when nothing
  /// is running ("cancelling nothing is safe — it errors but the session survives").
  pub fn cancel_message() -> String;
  ```
  Task 3's `ModelEvent::SpeechStarted { item_id }` carries the **user's** input item, not the
  assistant's (research note, Interruption 1 and 4): the item a truncate may name is the one this
  task has been counting, which is why `Heard::truncation` takes no item id from the event.
- Produces:
  - `pub const BYTES_PER_MS: u64 = 48` — 24 kHz mono PCM16 is 48 000 bytes per second (research
    note, Interruption 4), so an off-by-48 error is an off-by-one-millisecond lie to the model.
  - `pub fn played_bytes(appended: &[usize], remaining: usize, pos_ms: u64) -> u64` and
    `pub fn ms_of(bytes: u64) -> u64` — the pure arithmetic, tested on its own
  - `Heard::push(&self, chunk_bytes: usize, remaining: usize, pos_ms: u64)`,
    `Heard::heard_ms(&self) -> Option<u64>`, `Heard::truncation(&self) -> Option<(String, u32, u64)>`,
    `Heard::interrupt(&self)`, `Heard::take_interrupted(&self) -> bool`
  - `Call::interrupt(&self)`
  - `Player::progress(&self) -> (usize, Duration)`
  - `fn takes_the_floor(command: &Command) -> bool` and `Client::sync_call(&mut self)` in `client.rs`
  - Task 6 relies on none of these directly; the release checklist's live smoke test exercises the
    whole path.

**What this task decides, and why**

- **The number is derived from the device, never from the bytes sent.** A response's audio is
  generated faster than it is played, so "bytes handed to rodio" overstates what the user heard — and
  the research note says over-reporting `audio_end_ms` returns an error, while telling the model it
  heard more than it did makes its memory of the conversation wrong. `played_bytes` therefore takes
  rodio's own two numbers (`Player::len` — the sources still queued or playing, `player.rs:325-330` —
  and `Player::get_pos` — the position inside the one playing, `:332-342`) and counts the chunks that
  have finished plus the part of the one that is playing.
- **Every way of being unsure rounds the number down, and that asymmetry is deliberate.** A partial
  millisecond floors to zero (`ms_of`), a position past the end of its own chunk is capped at that
  chunk's length, a stale position is not added once the queue is empty, and a chunk of something
  else in the sink can only make `remaining` too large, which *understates* the count. Nothing here
  can overstate it.
- **Nothing played means nothing is sent.** The research note does not say whether
  `audio_end_ms: 0` is a legal truncate, and a zero would be a claim about an item the user has not
  heard a millisecond of, so `truncation()` answers `None` and no message goes out. The note's own
  rule — only assistant message items can be truncated, and over-reporting is an error — makes
  silence the safe side.
- **Stopping the device is the client's job, and the socket task asks for it.** The `Player` lives on
  the client's thread, so `speech_started` cannot touch it. The task sends the truncate, then raises
  `Heard::interrupt`; the client's frame loop — which runs every 33 ms, and whose last published
  count is therefore always `<=` the truth, another safe direction — flushes the device, clears the
  count and flushes again if it is asked twice.
- **A barge-in silences the rest of the response even if it is still in flight.** The server cancels
  its own response when `interrupt_response` is true, but deltas already on the wire would be played
  by a naive client. `Turn::silenced` is set by `speech_started` and cleared by the next
  `response.created`, and a silenced delta is dropped before it reaches the device or the count.
- **A local interrupt cancels instead of truncating.** A key press cannot know how much the user
  heard the way the server's VAD can, so `Call::interrupt` sends `response.cancel` (research note,
  Interruption 5) and clears the count rather than inventing a number. Which keys do it: while a call
  is live, the verbs that mean "the user is taking the floor" (`Ask`, `Say`, the microphone key in
  either direction and `Cancel`) — the call replaces the whole pipeline, so the session is told
  nothing while it lasts, and a `Reset` or a profile change could not reach a socket whose
  `instructions` were fixed when it opened anyway.
- **Sending the truncate clears the bookkeeping, so a second barge-in counts from zero.** The task
  calls `clear()` before raising the interrupt flag, so the next response's deltas start a new item
  and its count starts from nothing; `take_interrupted` answers true exactly once, so a frame loop
  cannot flush twice for one barge-in.

- [ ] **Step 1: Write the failing tests**

Append to `crates/tipsy-compadre/tests/realtime.rs` (the helpers of Task 4 are reused unchanged;
add `use std::sync::Arc;` and `tipsy_compadre::realtime::Heard` is already imported):

```rust
#[tokio::test]
async fn a_barge_in_truncates_the_item_at_the_milliseconds_that_were_heard() {
    let (url, accepting) = fake_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let heard = Heard::new();
    let _call = Call::open(setup(url), Arc::clone(&heard), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    for _ in 0..3 {
        server.send(delta("item_a", 0, &[7i16; 480])).await;
    }
    let _ = collect_until(&mut from_call, |seen| audio_of(seen).len() == 3).await;

    // The client's side of the count, stated by hand: in the running client these three numbers come
    // from `Player::progress` after each chunk was appended (`client.rs`, `on_speech`). Three 20 ms
    // chunks are queued, two have finished, and 12 ms of the third have played.
    heard.begin("item_a", 0);
    heard.push(960, 3, 0);
    heard.push(960, 2, 0);
    heard.push(960, 1, 12);

    // The research note's barge-in event names the *user's* item; the truncate must name the
    // assistant's, which is the one the device was playing.
    server
        .send(r#"{"type":"input_audio_buffer.speech_started","item_id":"item_1","audio_start_ms":3200}"#)
        .await;

    let truncate = server.recv_json().await.expect("the truncate");
    assert_eq!(truncate["type"], "conversation.item.truncate");
    assert_eq!(truncate["item_id"], "item_a", "the assistant's item, never the user's `item_1`");
    assert_eq!(truncate["content_index"], 0, "the note fixes the content index at 0");
    // 2 finished chunks of 960 bytes plus 12 ms × 48 bytes: 2496 bytes ÷ 48 = 52 ms.
    assert_eq!(truncate["audio_end_ms"], 52);
    // And the device is asked to stop, once.
    assert!(heard.take_interrupted(), "the client is told to stop the device on its next frame");
    assert!(!heard.take_interrupted(), "a second frame must not flush again for the same barge-in");
}

#[tokio::test]
async fn what_arrives_after_a_barge_in_is_not_played() {
    let (url, accepting) = fake_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    for _ in 0..3 {
        server.send(delta("item_a", 0, &[7i16; 480])).await;
    }
    let seen = collect_until(&mut from_call, |seen| audio_of(seen).len() == 3).await;
    assert_eq!(audio_of(&seen).len(), 3);

    server
        .send(r#"{"type":"input_audio_buffer.speech_started","item_id":"item_1","audio_start_ms":3200}"#)
        .await;
    // The tail of the response the user talked over is still in flight, both before and after the
    // server's own cancellation.
    server.send(delta("item_a", 0, &[9i16; 480])).await;
    server.send(response_done("resp_1", "cancelled")).await;
    server.send(delta("item_a", 0, &[9i16; 480])).await;

    let seen = collect_until(&mut from_call, |seen| {
        events_of(seen).iter().any(|event| matches!(event, Event::Done { .. }))
    })
    .await;
    assert!(audio_of(&seen).is_empty(), "none of it may reach the device: {:#?}", audio_of(&seen));
}

#[tokio::test]
async fn a_local_interrupt_cancels_the_response() {
    let (url, accepting) = fake_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let heard = Heard::new();
    let call = Call::open(setup(url), Arc::clone(&heard), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    server.send(delta("item_a", 0, &[7i16; 480])).await;
    let _ = collect_until(&mut from_call, |seen| audio_of(seen).len() == 1).await;

    call.interrupt();

    let cancel = server.recv_json().await.expect("the cancel");
    assert_eq!(cancel["type"], "response.cancel");
    assert!(
        cancel.get("response_id").is_none(),
        "the research note's client event is `{type, response_id?}`: nothing is named, because \
         cancelling nothing is safe: {cancel:#?}"
    );
    assert!(heard.take_interrupted(), "a local interrupt stops the device too");
}
```

Append to the `mod tests` in `crates/tipsy-compadre/src/realtime/mod.rs`:

```rust
    #[test]
    fn the_bytes_per_millisecond_are_the_notes_forty_eight() {
        // The research note's Interruption 4: 24 kHz mono PCM16 is 24000 samples × 2 bytes per
        // second, i.e. 48 bytes per millisecond. An off-by-48 error is a whole millisecond of the
        // model's memory, every barge-in (Review Focus 2).
        assert_eq!(BYTES_PER_MS, u64::from(API_SAMPLE_RATE) * 2 / 1000);
        assert_eq!(BYTES_PER_MS, 48);
        assert_eq!(ms_of(960), 20);
        assert_eq!(ms_of(47), 0, "a partial millisecond is never rounded up");
    }

    #[test]
    fn nothing_played_yet_is_nothing_to_truncate() {
        let heard = Heard::new();
        assert_eq!(played_bytes(&[], 0, 0), 0);
        assert_eq!(heard.heard_ms(), None);
        assert_eq!(heard.truncation(), None, "no item, no truncate");

        heard.begin("item_a", 0);
        assert_eq!(heard.heard_ms(), None, "an item the user has not heard is not a truncation");
        assert_eq!(heard.truncation(), None);

        // A chunk that is still entirely in the queue has not been heard. The note does not say
        // whether `audio_end_ms: 0` is legal, so nothing is sent rather than a zero that claims
        // something about an item nobody heard.
        heard.push(960, 1, 0);
        assert_eq!(heard.heard_ms(), None, "queued is not played");
        assert_eq!(heard.truncation(), None);
    }

    #[test]
    fn a_partially_played_chunk_counts_only_what_left_the_buffer() {
        let heard = Heard::new();
        heard.begin("item_a", 0);
        heard.push(960, 1, 12);
        assert_eq!(heard.heard_ms(), Some(12), "12 × 48 of the chunk's 960 bytes");
        heard.push(960, 1, 20);
        assert_eq!(heard.heard_ms(), Some(20));
        heard.push(960, 1, 100);
        assert_eq!(heard.heard_ms(), Some(40), "a position cannot report more than its own chunk");
    }

    #[test]
    fn the_chunks_that_finished_are_counted_and_the_one_playing_is_added() {
        let heard = Heard::new();
        heard.begin("item_a", 0);
        heard.push(960, 3, 0);
        assert_eq!(heard.heard_ms(), None, "all three are still queued");
        heard.push(960, 2, 0);
        assert_eq!(heard.heard_ms(), Some(20), "the first has finished");
        heard.push(960, 1, 12);
        assert_eq!(heard.heard_ms(), Some(52), "two finished plus 12 ms of the third");
        heard.push(960, 1, 20);
        assert_eq!(heard.heard_ms(), Some(60), "three finished");
        heard.push(960, 0, 999);
        assert_eq!(heard.heard_ms(), Some(80), "an empty queue: the stale position is not added");
        assert_eq!(heard.truncation(), Some(("item_a".to_string(), 0, 80)));
    }

    #[test]
    fn a_delta_for_another_item_never_counts_towards_this_one() {
        let heard = Heard::new();
        heard.begin("item_a", 0);
        heard.push(960, 1, 20);
        assert_eq!(heard.truncation(), Some(("item_a".to_string(), 0, 20)));

        // The next response's item takes the count's place: the old item's bytes were never played
        // as part of it, and truncating the wrong item's transcript is the one failure this whole
        // task exists to prevent. (The socket-level half of the same rule is
        // `a_delta_for_another_item_is_not_played` in `tests/realtime.rs`.)
        heard.begin("item_b", 7);
        assert_eq!(heard.heard_ms(), None);
        assert_eq!(heard.truncation(), None);
        heard.push(960, 1, 20);
        assert_eq!(heard.truncation(), Some(("item_b".to_string(), 7, 20)));

        // A second barge-in counts from the new item only: `clear` is what the task sends before it
        // raises the interrupt flag, so nothing of the first one survives into the second.
        heard.clear();
        assert_eq!(heard.heard_ms(), None);
        heard.begin("item_c", 0);
        heard.push(960, 1, 20);
        assert_eq!(heard.truncation(), Some(("item_c".to_string(), 0, 20)));
    }

    #[test]
    fn the_truncate_and_cancel_messages_are_the_ones_the_note_quotes() {
        // Research note, Interruption 3 and 5, verbatim.
        assert_eq!(
            events::truncate_message("item_a", 0, 1500),
            r#"{"type":"conversation.item.truncate","item_id":"item_a","content_index":0,"audio_end_ms":1500}"#
        );
        assert_eq!(events::cancel_message(), r#"{"type":"response.cancel"}"#);
    }
```

Append to `crates/tipsy-compadre/src/client.rs`'s `mod tests`:

```rust
    #[test]
    fn the_verbs_that_take_the_floor_are_the_ones_a_call_answers_locally() {
        for command in [
            Command::Ask { text: "¿qué pasa?".into() },
            Command::Say { text: "hola compadre".into() },
            Command::ListenStart,
            Command::ListenStop,
            Command::Cancel,
        ] {
            assert!(takes_the_floor(&command), "{command:?} means the user is taking the floor");
        }
        for command in [
            Command::Reset,
            Command::SetProfile { id: "meigo".into() },
            Command::SetListeningMode { mode: ListeningMode::Push },
            Command::Hello { token: None, client: ClientKind::Tui, protocol_version: 1 },
        ] {
            assert!(!takes_the_floor(&command), "{command:?} is not an interruption");
        }
    }
```

(That test needs `use tipsy_protocol::{ClientKind, ListeningMode};` in the module; `Command` is
already in scope through `use super::*`.)

Append to `crates/tipsy-compadre/src/audio/device.rs`'s `mod tests`:

```rust
    /// The rodio behaviour `Player::progress` is built on, pinned here for the same reason
    /// `a_cleared_rodio_player_stays_paused_until_it_is_resumed` is: the barge-in count is these two
    /// numbers, and an empty sink is the boundary the count must read as "nothing has been heard".
    #[test]
    fn an_empty_rodio_player_has_nothing_queued_and_no_position() {
        let (player, _queue) = RodioPlayer::new();
        assert_eq!(player.len(), 0, "rodio counts a source until it finishes (player.rs:325-330)");
        assert_eq!(player.get_pos(), Duration::ZERO, "nothing is playing, so there is no position");
    }
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-compadre --test realtime`

Expected: FAIL — the target does not compile:

```
error[E0599]: no method named `push` found for struct `Heard` in the current scope
error[E0599]: no method named `truncation` found for struct `Heard` in the current scope
error[E0599]: no method named `take_interrupted` found for struct `Heard` in the current scope
error[E0599]: no method named `interrupt` found for struct `Call` in the current scope
```

Run: `cargo test -p tipsy-compadre --lib realtime`

Expected: FAIL, `error[E0425]: cannot find function `played_bytes``, `ms_of`, `BYTES_PER_MS` and
`events::truncate_message` / `events::cancel_message`.

Run: `cargo test -p tipsy-compadre --lib audio::device`

Expected: FAIL, `error[E0599]: no method named `progress` found for struct `Player``.

- [ ] **Step 3: The count**

`crates/tipsy-compadre/src/realtime/events.rs` — add the two serialisers (Task 3 owns the rest of
the file):

```rust
/// `conversation.item.truncate`: remove the model's unplayed audio and its transcript, so what the
/// session remembers is what the user actually heard (research note, Interruption 3). `audio_end_ms`
/// is inclusive milliseconds of audio played and `content_index` is fixed at 0 by the note; only an
/// assistant message item may be named here, which is why the caller passes the item the *device*
/// was playing and never the item `input_audio_buffer.speech_started` names.
pub fn truncate_message(item_id: &str, content_index: u32, audio_end_ms: u64) -> String {
    serde_json::json!({
        "type": "conversation.item.truncate",
        "item_id": item_id,
        "content_index": content_index,
        "audio_end_ms": audio_end_ms,
    })
    .to_string()
}

/// `response.cancel`: stop the response in progress (research note, Interruption 5). No
/// `response_id` is sent: the client event is `{type, response_id?}` and cancelling nothing is safe —
/// it errors, and the session survives.
pub fn cancel_message() -> String {
    serde_json::json!({ "type": "response.cancel" }).to_string()
}
```

`crates/tipsy-compadre/src/audio/device.rs` — `use std::time::Duration;` beside the existing imports,
and the accessor in `impl Player` (after `playing`, `:73-75`):

```rust
    /// The queue as rodio reports it: how many chunks are still queued or playing, and how far into
    /// the one playing we are.
    ///
    /// `len` counts a source until it finishes (`player.rs:325-330`, the same rule `playing()` is
    /// built on) and `get_pos` is the position inside the sound playing now (`:332-342`). The
    /// barge-in count is these two numbers and never the bytes merely appended: telling the model it
    /// heard more than the user heard is what corrupts the conversation silently (Review Focus 2).
    pub fn progress(&self) -> (usize, Duration) {
        (self.player.len(), self.player.get_pos())
    }
```

`crates/tipsy-compadre/src/realtime/mod.rs` — the arithmetic, the count and the byte constant,
placed after `pcm16_le`:

```rust
/// Bytes of one millisecond of the API's audio: 24 kHz mono PCM16 is 48 000 bytes per second
/// (research note, Interruption 4). The one number a barge-in must not get wrong.
pub const BYTES_PER_MS: u64 = 48;

/// Whole milliseconds in `bytes`. Flooring, never rounding: a partial millisecond is a claim about
/// audio the user has not heard.
pub fn ms_of(bytes: u64) -> u64 {
    bytes / BYTES_PER_MS
}

/// Bytes of an item's audio that have left the playback buffer.
///
/// `appended` is every chunk handed to the device for that item, in order; `remaining` is how many
/// of them the device still holds (`Player::progress`); `pos_ms` is the position inside the one
/// playing. The chunks that have finished are the first `appended.len() - remaining` of them, and
/// the one playing contributes only what it has actually played, never more than its own length.
///
/// Every uncertain term here resolves downwards: a stale position is not counted once nothing is
/// queued, and a chunk of something else in the sink can only make `remaining` too large, which
/// understates. Overstating is the direction that corrupts the conversation (Review Focus 2).
pub fn played_bytes(appended: &[usize], remaining: usize, pos_ms: u64) -> u64 {
    let finished = appended.len().saturating_sub(remaining);
    let done: u64 = appended[..finished].iter().map(|bytes| *bytes as u64).sum();
    let playing = match appended.get(finished) {
        Some(length) if remaining > 0 => (pos_ms * BYTES_PER_MS).min(*length as u64),
        _ => 0,
    };
    done + playing
}
```

and the count on `Heard` — replace Task 4's struct and `impl` with:

```rust
/// What the call knows about the audio the device is playing.
///
/// Two threads touch it, and neither waits for the other: the client's frame loop writes what has
/// actually left the playback buffer — the device's queue is visible nowhere else — and the socket
/// task reads it when the server reports that the user started speaking. No lock here is ever held
/// across an `await`, and none of this touches the audio thread.
#[derive(Debug, Default)]
pub struct Heard {
    item: Mutex<Option<(String, u32)>>,
    /// Every chunk handed to the device for the current item, in order, in bytes.
    appended: Mutex<Vec<usize>>,
    /// How much of the current item has been heard, in bytes: the client publishes it from
    /// `Player::progress` after each chunk, and the socket task reads it at a barge-in.
    played: AtomicU64,
    /// Set by a barge-in: the client's next frame silences the device and clears the count.
    interrupted: AtomicBool,
}

impl Heard {
    pub fn new() -> Arc<Heard> {
        Arc::new(Heard::default())
    }

    /// The first audio delta of a response names the item the device is playing, and the count
    /// starts at nothing.
    pub fn begin(&self, item_id: &str, content_index: u32) {
        *self.item.lock().expect("the item lock is never held across a panic") =
            Some((item_id.to_string(), content_index));
        self.forget();
    }

    /// One chunk handed to the device, and what the device says about the queue afterwards.
    pub fn push(&self, chunk_bytes: usize, remaining: usize, pos_ms: u64) {
        let played = {
            let mut appended = self.appended.lock().expect("the chunk lock is never held across a panic");
            appended.push(chunk_bytes);
            played_bytes(&appended, remaining, pos_ms)
        };
        self.played.store(played, Ordering::Relaxed);
    }

    /// The milliseconds of the current item the user has actually heard. `None` while none of it has
    /// been played: a partial millisecond is not rounded up to one, and nothing is claimed about an
    /// item the device has not started.
    pub fn heard_ms(&self) -> Option<u64> {
        if self.item.lock().ok()?.is_none() {
            return None;
        }
        match ms_of(self.played.load(Ordering::Relaxed)) {
            0 => None,
            ms => Some(ms),
        }
    }

    /// What a barge-in must tell the model: the item the device is playing, its content index and
    /// the milliseconds of it the user heard — or `None` when there is nothing safe to send.
    ///
    /// The item comes from the bookkeeping and not from the event: `speech_started` names the user's
    /// input item (research note, Interruption 1), and a truncate cuts the transcript of whatever
    /// item it names (Interruption 3), so naming the wrong one is the failure this task exists to
    /// prevent.
    pub fn truncation(&self) -> Option<(String, u32, u64)> {
        let (item_id, content_index) = self
            .item
            .lock()
            .expect("the item lock is never held across a panic")
            .clone()?;
        Some((item_id, content_index, self.heard_ms()?))
    }

    /// Asks the client's frame loop to silence the device: only the client owns the `Player`.
    pub fn interrupt(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }

    /// True exactly once per barge-in, for the client that acts on it.
    pub fn take_interrupted(&self) -> bool {
        self.interrupted.swap(false, Ordering::SeqCst)
    }

    /// Forgets the item and everything counted for it: the call ended, a barge-in took it away, or a
    /// new item began. What is left is the interrupted flag, which belongs to the frame loop that
    /// has not acted on it yet.
    pub fn clear(&self) {
        *self.item.lock().expect("the item lock is never held across a panic") = None;
        self.forget();
    }

    fn forget(&self) {
        self.appended.lock().expect("the chunk lock is never held across a panic").clear();
        self.played.store(0, Ordering::Relaxed);
    }
}
```

The imports gain `use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};`.

- [ ] **Step 4: The barge-in, and the local interrupt**

`crates/tipsy-compadre/src/realtime/mod.rs` — `Turn` gains the silencing flag:

```rust
    /// True from a barge-in until the next response: a response the user talked over must not be
    /// played even if its deltas are still in flight (research note, Interruption 1).
    silenced: bool,
```

the `SpeechStarted` arm of `on_event` becomes:

```rust
        // The user talked over the model. Playback stops on the client's next frame, and the model
        // is told how much of its own speech was actually heard, so its memory matches the user's
        // (research note, Interruption 2 and 3).
        ModelEvent::SpeechStarted { .. } => {
            if let Some((item_id, content_index, audio_end_ms)) = heard.truncation() {
                let message = events::truncate_message(&item_id, content_index, audio_end_ms);
                if socket.send_text(&message).await.is_err() {
                    report(out, ErrorCode::Network, SOCKET_CLOSED);
                    return Step::Stop;
                }
            }
            // The count is spent: a second barge-in counts from the next item, and nothing of this
            // one can be added to it by a delta that is still arriving.
            heard.clear();
            heard.interrupt();
            turn.silenced = true;
        }
```

`ResponseCreated` clears the silencing:

```rust
        ModelEvent::ResponseCreated { .. } => {
            *turn = Turn { started: Some(Instant::now()), ..Turn::default() };
            send(out, status(AssistantState::Thinking));
        }
```

(the `Turn::default()` reset already clears `silenced`), and `ResponseAudioDelta` drops a silenced
delta before it does anything else:

```rust
        ModelEvent::ResponseAudioDelta { item_id, content_index, delta } => {
            // The tail of a response the user talked over is not played, and is not counted: it is
            // audio nobody asked to hear any more.
            if turn.silenced {
                return Step::Continue;
            }
            // The first delta of a response names the item it belongs to.
            if turn.item.is_none() {
                turn.item = Some((item_id.clone(), content_index));
                heard.begin(&item_id, content_index);
            }
```

`Call` gains the way the client asks for an interrupt, which is the one thing the client cannot do
itself. The enum, above the struct:

```rust
/// What the client asks of a call while it lasts.
enum Control {
    /// The user is taking the floor by hand: the response is cancelled rather than truncated.
    Interrupt,
}
```

the field on `Call`:

```rust
    control: UnboundedSender<Control>,
```

the channel in `Call::open`:

```rust
        let (mic, frames) = tokio::sync::mpsc::channel(MIC_QUEUE);
        let (control, orders) = tokio::sync::mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        tokio::spawn(run(setup, Arc::clone(&heard), frames, orders, out, cancel.clone()));
        Call { mic, control, heard, cancel }
```

`run`'s signature, which gains the orders:

```rust
async fn run(
    setup: CallSetup,
    heard: Arc<Heard>,
    mut frames: Receiver<Vec<i16>>,
    mut orders: tokio::sync::mpsc::UnboundedReceiver<Control>,
    out: UnboundedSender<ToClient>,
    cancel: CancellationToken,
) {
```

and the two new pieces of `Call` and `run`:

```rust
    /// The user is taking the floor by hand: the server is told to cancel the response, and the
    /// device is silenced on the client's next frame.
    ///
    /// A local interrupt cancels instead of truncating (research note, Interruption 5): the number a
    /// truncate carries comes from the server's own report of what the user heard, and a key press
    /// has no such report to give.
    pub fn interrupt(&self) {
        let _ = self.control.send(Control::Interrupt);
    }
```

and the arm in `run`, which goes after the deadline arm and before the microphone's:

```rust
            Some(Control::Interrupt) = orders.recv() => {
                // Cancelling nothing is safe: the research note says the cancel errors on the server
                // and the session survives, so it is sent without checking what is running.
                if socket.send_text(&events::cancel_message()).await.is_err() {
                    report(&out, ErrorCode::Network, SOCKET_CLOSED);
                    break;
                }
                heard.interrupt();
            }
```

`crates/tipsy-compadre/src/client.rs` — `frame_loop` gains the call's frame work, beside the
microphone's (`:143-147`):

```rust
            while let Ok(message) = from_server.try_recv() {
                self.on_message(message);
            }
            self.drain_mic(&to_server);
            self.sync_mic();
            self.sync_call();
```

`on_speech` (`:198-208`) publishes the count as it plays:

```rust
    /// Plays one spoken sentence, and shows the envelope of the reply it belongs to.
    fn on_speech(&mut self, samples: Vec<i16>) {
        if samples.is_empty() {
            return;
        }
        self.reply.extend_from_slice(&samples);
        let bytes = samples.len() * 2;
        if let Some(player) = &self.player {
            player.play(samples, self.rate);
        }
        // While a call is live the device is asked what it has actually played — the queue it still
        // holds is subtracted, because a response is generated faster than it is spoken and the
        // bytes merely handed over would overstate what the user heard (Review Focus 2). The device
        // is reachable only from this thread, so this is where the number is worked out.
        if let (Some(call), Some(player)) = (&self.call, &self.player) {
            let (remaining, pos) = player.progress();
            call.heard().push(bytes, remaining, pos.as_millis() as u64);
        }
        // The oscilloscope shows the reply as it is heard, so it is rebuilt as it grows.
        self.state.start_voice(envelope(&i16_to_f32(&self.reply), self.rate));
    }

    /// Acts on what the call's socket task decided since the last frame.
    ///
    /// A barge-in is the one thing the socket task cannot do itself: stopping the device means
    /// touching the `Player`, which lives on this thread. The task marks it, the frame loop flushes,
    /// and the count starts again from nothing — the chunks it just dropped were never heard.
    fn sync_call(&mut self) {
        let Some(call) = &self.call else {
            return;
        };
        if call.heard().take_interrupted() {
            self.flush_playback();
            call.heard().clear();
        }
    }
```

`on_action` (`:241-266`) diverts a live call's verbs, and the rule joins the other two pure rules
beside `starts_turn` (`:285`):

```rust
    fn on_action(&mut self, action: UiAction, to_server: &UnboundedSender<ToServer>) -> bool {
        match action {
            UiAction::Send(command) => {
                // A call replaces the whole pipeline for as long as it lasts, so while one is live
                // the session is told nothing: a `Reset` or a profile change could not reach a
                // socket whose `instructions` were fixed when it opened, and a typed question would
                // be spoken over the call. The verbs that mean "the user is taking the floor" become
                // the local barge-in.
                if let Some(call) = &self.call {
                    if takes_the_floor(&command) {
                        self.flush_playback();
                        call.heard().clear();
                        call.interrupt();
                    }
                    return false;
                }
                // The only two things that quiet the device: the user starting a new turn over the
                // reply still being spoken (barge-in), and the cancel the user asked for. Reaching
                // the end of a turn is not an interruption — see `flush_on_interrupt`.
                if flush_on_interrupt(interrupts_playback(&command), self.playing()) {
                    self.flush_playback();
                }
                if starts_turn(&command) {
                    // What `Replay` replays is the last reply, so it goes when the next turn begins.
                    self.reply.clear();
                }
                let _ = to_server.send(ToServer::Command(command));
                false
            }
```

```rust
/// Whether a command means the user is taking the floor while a call is live.
///
/// In push mode these verbs are the microphone key and the cancel; in a call the microphone is
/// already open for the whole call, so both directions of that key mean the same thing — "stop
/// talking, I am talking" — and they are answered locally with `response.cancel` rather than sent to
/// a session that is not driving the conversation (Review Focus 2's safe side; the server's own VAD
/// is what reports a barge-in and what knows the number).
fn takes_the_floor(command: &Command) -> bool {
    matches!(
        command,
        Command::Ask { .. }
            | Command::Say { .. }
            | Command::ListenStart
            | Command::ListenStop
            | Command::Cancel
    )
}
```

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo test -p tipsy-compadre --test realtime`

Expected: PASS, 12 tests:

```
running 12 tests
test a_barge_in_truncates_the_item_at_the_milliseconds_that_were_heard ... ok
test a_call_that_reaches_its_limit_ends_itself ... ok
test a_closed_port_is_a_readable_error_and_the_call_never_goes_live ... ok
test a_delta_for_another_item_is_not_played ... ok
test a_local_interrupt_cancels_the_response ... ok
test a_mic_frame_is_appended_as_twenty_milliseconds_of_24k_audio ... ok
test a_socket_that_dies_mid_answer_leaves_the_hud_out_of_the_live_state ... ok
test an_audio_delta_of_twenty_milliseconds_reaches_the_device_at_24000 ... ok
test an_error_event_ends_the_call_with_its_own_message ... ok
test stopping_the_call_closes_the_socket ... ok
test the_transcripts_of_a_call_reach_the_hud ... ok
test what_arrives_after_a_barge_in_is_not_played ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.51s
```

Run: `cargo test -p tipsy-compadre --lib realtime`

Expected: PASS, 11 tests:

```
running 11 tests
test realtime::tests::a_delta_for_another_item_never_counts_towards_this_one ... ok
test realtime::tests::a_microphone_frame_becomes_one_and_a_half_times_as_many_samples ... ok
test realtime::tests::a_partially_played_chunk_counts_only_what_left_the_buffer ... ok
test realtime::tests::nothing_played_yet_is_nothing_to_truncate ... ok
test realtime::tests::the_apis_audio_is_sixteen_bit_little_endian ... ok
test realtime::tests::the_audio_the_client_appends_is_base64_of_little_endian_samples ... ok
test realtime::tests::the_bytes_per_millisecond_are_the_notes_forty_eight ... ok
test realtime::tests::the_call_asks_for_the_configured_model_voice_and_prompt ... ok
test realtime::tests::the_chunks_that_finished_are_counted_and_the_one_playing_is_added ... ok
test realtime::tests::the_endpoint_carries_the_model_as_a_query ... ok
test realtime::tests::the_truncate_and_cancel_messages_are_the_ones_the_note_quotes ... ok

test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

(Task 4's five are the other five of these; the filter `realtime` matches no other module.)

Run: `cargo test -p tipsy-compadre --lib client` and `cargo test -p tipsy-compadre --lib audio::device`

Expected: PASS, 7 and 3 tests respectively:

```
running 7 tests
test client::tests::a_chunk_header_sets_the_rate_the_device_is_told ... ok
test client::tests::a_finished_reply_keeps_its_audio_until_a_new_turn_starts ... ok
test client::tests::only_an_interrupt_with_audio_left_to_drop_flushes ... ok
test client::tests::the_client_lets_go_of_a_call_when_the_socket_says_it_is_over ... ok
test client::tests::the_commands_that_end_a_turn_are_the_ones_that_interrupt_it ... ok
test client::tests::the_microphone_runs_when_the_user_opened_it_or_when_a_call_is_live ... ok
test client::tests::the_verbs_that_take_the_floor_are_the_ones_a_call_answers_locally ... ok
```

```
running 3 tests
test audio::device::tests::a_cleared_rodio_player_stays_paused_until_it_is_resumed ... ok
test audio::device::tests::an_empty_rodio_player_has_nothing_queued_and_no_position ... ok
test audio::device::tests::frame_size_is_twenty_milliseconds ... ok
```

- [ ] **Step 6: Prove the workspace is whole**

Run: `cargo test --workspace`

Expected: all green. `client.rs`'s existing `only_an_interrupt_with_audio_left_to_drop_flushes`
and `the_commands_that_end_a_turn_are_the_ones_that_interrupt_it` are the regression net for the new
`on_action` arm: they pin that `flush_on_interrupt` still governs the *session's* playback, which the
live branch must not have changed.

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: `Finished`, no warnings — in particular no `await_holding_lock` in `realtime/mod.rs`
(no `Heard` lock is held across an `.await`) and no `dead_code` for `Control::Interrupt`.

- [ ] **Step 7: Commit**

```bash
git add crates/tipsy-compadre/src/realtime/mod.rs crates/tipsy-compadre/src/realtime/events.rs crates/tipsy-compadre/src/audio/device.rs crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/tests/realtime.rs
git commit -m "feat(realtime): barge in without overstating what was heard

speech_started stops the device on the client's next frame and sends
conversation.item.truncate with the milliseconds derived from the bytes that
left the playback buffer, at 48 bytes per millisecond: rodio's queue count and
position, never the bytes handed over. A position past its own chunk, a stale
position after an empty queue and a partial millisecond all resolve downwards,
and nothing is sent at all when nothing has been played. The rest of the
cancelled response is dropped even if it is still in flight, and a local
interrupt sends response.cancel instead of inventing a number.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: The interface, the docs and the review `[small]`

**Files:**
- Modify: `crates/tipsy-ui/src/state.rs:66-97` — `UiAction::ToggleLive` and the `live` flag
- Modify: `crates/tipsy-ui/src/state.rs:165-236` — the `Event::Live` arm of `UiState::apply`
- Modify: `crates/tipsy-ui/src/state.rs:238-301` — the `F10` arm of `UiState::on_key`
- Modify: `crates/tipsy-ui/src/strings.rs:5-69` — `live` and `live_off` in `Strings`, plus both
  `help` lines
- Modify: `crates/tipsy-ui/src/render.rs:78-97` — the header indicator
- Modify: `crates/tipsy-compadre/src/client.rs:56-163` — the `events` field, `start_call`,
  `toggle_call` and `frame_loop`'s handover
- Modify: `crates/tipsy-compadre/src/client.rs:241-266` — the `UiAction::ToggleLive` arm
- Modify: `README.md:47-59` — the `F10` row; `README.md:112` — a new `## Realtime voice mode`
  section; `README.md:186` — a new `## Release checklist` section at the end
- Test: `crates/tipsy-ui/src/strings.rs`, `crates/tipsy-ui/src/state.rs`,
  `crates/tipsy-ui/tests/render.rs:56-168`, `crates/tipsy-compadre/src/client.rs`,
  `crates/tipsy-compadre/tests/realtime.rs` (Task 4's death test gains the HUD's own assertions)

**Interfaces:**

- Consumes (from the committed code and from Tasks 4 and 5):
  - `UiState` (`state.rs:74-97`), `UiAction` (`:66-72`), `on_key` (`:238-301`), `apply` (`:165-236`),
    `end_pending_replies` (`:155-159`), `busy` (`:161-163`), `notify` (`:136-138`),
    `strings()` (`:132-134`).
  - `Strings` with its eight `repr`-free `&'static str` fields and the `help` line
    (`strings.rs:5-25`), `ES` (`:27-47`), `EN` (`:49-69`).
  - `draw_header(frame, state, area, title)` (`render.rs:78-97`) and the test helpers `state`,
    `render`, `text_at` (`tipsy-ui/tests/render.rs:6-34`).
  - Task 4's `Call::open`, `CallSetup`, `CallSetup::from_config`, `Heard::new`, `Call::stop`;
    `Config::realtime` and `Config::active()` (`config/model.rs:158-166`).
  - `KeyResolver::system()` and `KeyResolver::resolve(name) -> Result<Option<(String, KeySource)>, KeyError>`
    (`keys.rs:89-102`), `env_var_name(name) -> String` (`:65-71`) — the same resolution every
    provider's key goes through, so a key in `TIPSY_KEY_<NAME>` wins over the credential store.
  - **Assumed from Task 1, and this is where it has to be right**: `Event::Live { active: bool }`,
    kept as a behaviour-free arm in `UiState::apply` (`:165-236`) so the workspace compiled while
    Tasks 4 and 5 ran. This task replaces that arm. `RealtimeConfig`'s defaults are also assumed here
    for the README's example: `enabled = false`, `model = "gpt-realtime-2.1"`, `key = "openai"`,
    `voice = "marin"`, `silence_ms = 1000`, `max_minutes = 10`; if Task 1 chose different values, the
    README's block and `crates/tipsy-compadre/src/realtime/mod.rs`'s `the_call_asks_for_the_configured_model_voice_and_prompt`
    are the two places to change, and nothing else in this task moves.
- Produces: the HUD's `live` flag and its strings, the `F10` binding, `Client::start_call` and
  `Client::toggle_call`, and the two new README sections. Nothing later depends on these: Task 6 is
  the last task of the plan.

**What this task decides, and why**

- **`F10`, and this is the justification the plan's keyboard rule asks for.** The rule is that
  shortcuts avoid the keys browsers reserve (`F5` reloads, `F11` fullscreens, `F12` opens devtools,
  `Ctrl+W` closes the tab — spec §9). Of the function keys this HUD has not taken (`F9` opens the
  configuration screen, `F2`/`F3`/`F4`/`F6`/`F7`/`F8` are the chat and network bindings), `F10` is
  the only one no browser binds to a navigation or a destructive action: where a browser maps it to
  its own menu bar, that is a focus change a page can `preventDefault` in its key handler, and this
  plan ships the native terminal only (the browser client is a later milestone). `Esc` is *not* used
  for it: `Esc` quits, and quitting already ends the call because the handle drops with the frame
  loop.
- **No mouse equivalent, deliberately.** The spec's rule is that every action has both, and this is
  the one exception it is worth taking: Review Focus 3 says a call can only start from an explicit
  key press, and the indicator — the only part of the header that could plausibly be clicked — is
  copy, not a button. A stray click must never start a mode that bills per minute.
- **The key asks; the socket answers.** `F10` produces `UiAction::ToggleLive`; `UiState::live` is set
  by `Event::Live` from the call task, so the indicator cannot be on while no socket is open (the
  flag goes up when the handshake succeeded, `realtime/mod.rs`'s `run`) and cannot stay on
  after one died (Task 4 sends `Live { active: false }` on every exit). `toggle_call` still has to
  decide by `self.call.is_some()`, which is the client's own handle on the socket.
- **The indicator outranks everything in the header.** The header currently shows listening, a
  spinner or the last notice (`render.rs:80-94`); a live call is drawn first and in red, because the
  whole point of Review Focus 3 is that the mode is never silently on: a spinner must not be able to
  hide it.
- **The end of a call stops the spinner, and that is a new rule.** A call's turns are started by the
  socket, and a reply the user never got — the socket died, the call hit its limit, the model said
  nothing — would leave `busy()` true for ever, which is exactly the bug
  `an_idle_status_stops_the_spinner_on_a_cancelled_turn` was written for in the session's path.
  `Event::Live { active: false }` applies the same rule.
- **The README says what the mode costs, near the top of what it costs.** The plan's decision is
  that "the cost is never silent": the per-minute figures are arithmetic from the documented
  per-token prices (the research note's Cost section says so itself), so they are stated as
  arithmetic, with the rates they come from, and the `[realtime]` block is printed so that
  `enabled = false` is the first thing a reader sees.
- **The release checklist is where the release checklist goes.** The repo has no checklist file, and
  the spec's §13 states the manual check in prose ("Manual check before each release: real mic and
  speakers in the terminal…"). The README is the file this task owns and the file a reader opens
  before shipping, so the list is created there, and its realtime item names the two inferred payload
  fields it is the only proof of.

- [ ] **Step 1: Write the failing tests**

`crates/tipsy-ui/src/strings.rs` — a new `#[cfg(test)] mod tests` at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The indicator's copy exists in both languages — Review Focus 3's "the mode is never silently
    /// on" starts with a word for it in the table the user's profile selects.
    #[test]
    fn the_live_indicator_has_copy_in_both_languages() {
        assert_eq!(strings(UiLanguage::Es).live, "EN VIVO");
        assert_eq!(strings(UiLanguage::En).live, "LIVE");
        assert_ne!(strings(UiLanguage::Es).live_off, strings(UiLanguage::En).live_off);
        for table in [strings(UiLanguage::Es), strings(UiLanguage::En)] {
            assert!(!table.live_off.is_empty(), "a refusal has to say something");
            assert!(table.help.contains("F10"), "the help line names the key: {}", table.help);
        }
    }
}
```

`crates/tipsy-ui/src/state.rs` — inside the existing `mod tests`:

```rust
    #[test]
    fn f10_asks_for_the_call_and_the_socket_answers() {
        let mut s = UiState::new();
        assert_eq!(s.on_key(KeyInput::F(10)), vec![UiAction::ToggleLive]);
        // The key asks; the flag is the socket's to set, so it is untouched here.
        assert!(!s.live, "a key press is not a call: only `Event::Live` turns the indicator on");
        s.apply(&Event::Live { active: true });
        assert!(s.live);
        // And the same key again is the way out, asked for the same way.
        assert_eq!(s.on_key(KeyInput::F(10)), vec![UiAction::ToggleLive]);
        assert!(s.live, "the indicator goes down when the socket says so, not when the key is pressed");
    }

    #[test]
    fn the_end_of_a_call_takes_the_indicator_down_and_stops_the_spinner() {
        let mut s = UiState::new();
        s.apply(&Event::Live { active: true });
        // A call's turns are started by the socket: `Heard` and the deltas open the reply entry
        // (`reply_entry`), and no `Done` ever arrives for a call that dies mid-answer.
        s.apply(&Event::Heard { text: "¿y el ticket?".into() });
        s.apply(&Event::ReplyDelta { text: "Aquí andamos" });
        assert!(s.busy(), "the reply is being said");

        s.apply(&Event::Live { active: false });
        assert!(!s.live, "the indicator is the first thing to go");
        assert!(!s.busy(), "a call that ended is not still thinking: {:#?}", s.log);
        assert_eq!(s.log.last().unwrap().text, "Aquí andamos", "the partial reply stays readable");
    }
```

`crates/tipsy-ui/tests/render.rs` — append three tests; the helpers `state`, `render`, `clock`
(`:6-30`) are reused unchanged:

```rust
#[test]
fn the_header_shows_the_live_indicator_instead_of_anything_else() {
    let mut s = state(UiLanguage::Es);
    s.apply(&Event::Live { active: true });
    // A turn in flight is not a reason to hide it: the call is billing while it lasts, so the
    // spinner that would otherwise be there is exactly what must not cover the indicator.
    s.apply(&Event::Heard { text: "¿y el ticket?".into() });
    s.apply(&Event::ReplyDelta { text: "Aquí andamos" });
    s.apply(&Event::Status { state: AssistantState::Thinking, detail: None });
    assert!(s.busy(), "the test is only worth anything while the spinner would be drawn");
    let (text, _, _) = render(&s);
    assert!(text.contains("EN VIVO"), "missing the live indicator");
    assert!(!text.contains("Pensando"), "a spinner may not sit in front of it");
    assert!(!text.contains("Listo"), "the indicator is the header while a call lasts");
}

#[test]
fn english_header_uses_the_english_live_indicator() {
    let mut s = state(UiLanguage::En);
    s.apply(&Event::Live { active: true });
    let (text, _, _) = render(&s);
    assert!(text.contains("LIVE"));
    assert!(!text.contains("EN VIVO"));
    assert!(!text.contains("Ready"));
}

#[test]
fn the_live_indicator_disappears_when_the_call_ends() {
    let mut s = state(UiLanguage::Es);
    s.apply(&Event::Live { active: true });
    s.apply(&Event::Live { active: false });
    let (text, _, _) = render(&s);
    assert!(!text.contains("EN VIVO"), "the indicator is not left behind by a call that ended");
    assert!(text.contains("Listo"), "the header is the quiet one again");
}
```

`crates/tipsy-ui/tests/render.rs` needs `use tipsy_protocol::*;` (already there, `:3`) so
`AssistantState` resolves.

`crates/tipsy-compadre/src/client.rs` — inside `mod tests`:

```rust
    #[test]
    fn a_call_never_starts_while_the_mode_is_disabled() {
        let mut config = Config::default();
        config.realtime.enabled = false;
        let mut client = Client::new(config);
        let why = client.start_call().expect_err("a disabled mode starts nothing");
        assert!(why.contains("enabled"), "the refusal names the setting: {why}");
        assert!(client.call.is_none(), "Review Focus 3: the mode is never silently on");
    }
```

`crates/tipsy-compadre/tests/realtime.rs` — Task 4's
`a_socket_that_dies_mid_answer_leaves_the_hud_out_of_the_live_state` gains the HUD's own half, at
the end of the test, because `UiState::live` exists only now:

```rust
    // And the HUD itself leaves the call: the flag the indicator is drawn from is down, and the
    // partial reply is still on screen with its spinner stopped (Review Focus 1).
    let mut hud = tipsy_ui::UiState::new();
    for message in &seen {
        if let ToClient::Event(event) = message {
            hud.apply(event);
        }
    }
    assert!(!hud.live, "the HUD is out of its live state: {:#?}", hud.log);
    assert!(!hud.busy(), "and nothing is left spinning: {:#?}", hud.log);
    assert_eq!(hud.log.last().unwrap().text, "Aquí anda", "what was said before the death stays");
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test -p tipsy-ui --lib strings`

Expected: FAIL — the module does not compile:

```
error[E0609]: no field `live` on type `&'static Strings`
error[E0609]: no field `live_off` on type `&'static Strings`
```

Run: `cargo test -p tipsy-ui --lib call`

Expected: FAIL, `error[E0609]: no field `live` on type `UiState`` and
`error[E0599]: no variant or associated item named `ToggleLive` found for enum `UiAction``.

Run: `cargo test -p tipsy-ui --test render`

Expected: FAIL, the same two errors plus
`error[E0599]: no method named `apply`... ` halted at `Event::Live` (`no variant`), because the
three render tests are compiled with the library.

Run: `cargo test -p tipsy-compadre --lib client`

Expected: FAIL, `error[E0599]: no method named `start_call` found for struct `Client`` and
`error[E0609]: no field `enabled` on type `RealtimeConfig`` if Task 1 has not landed yet.

Run: `cargo test -p tipsy-compadre --test realtime`

Expected: FAIL, `error[E0609]: no field `live` on type `UiState``.

- [ ] **Step 3: The HUD's flag, its copy and its header**

`crates/tipsy-ui/src/state.rs` — `UiAction` (`:66-72`) gains one variant:

```rust
    /// Start a live call, or end the one that is running (Plan 2C). The client owns the socket and
    /// the HUD only asks: the indicator turns on when the socket says it is live, never here.
    ToggleLive,
```

`UiState` (`:74-97`) gains one field, after `status`:

```rust
    /// True for as long as a live call lasts (Plan 2C). The header says so for the whole call — the
    /// mode bills while it is open — and the flag is set by `Event::Live`, never by the key.
    pub live: bool,
```

its initialiser (`:106-130`) gains `live: false,`, `on_key` (`:238-301`) gains one arm before
`KeyInput::F(_) => Vec::new()`:

```rust
            KeyInput::F(10) => vec![UiAction::ToggleLive],
```

and `apply` (`:165-236`) replaces whatever arm Task 1 left for exhaustiveness:

```rust
            Event::Live { active } => {
                self.live = *active;
                if !self.live {
                    // A call that ended is not still thinking: its turns were started by the socket,
                    // so a reply that was cut off mid-answer never gets its `Done` and would leave
                    // the spinner running for ever — the same rule the session's `Idle` applies
                    // (`:174-177`).
                    self.end_pending_replies();
                }
            }
```

`crates/tipsy-ui/src/strings.rs` — two fields in `Strings` (after `speaking`), and their two
values:

```rust
    /// The header indicator for the whole of a live call (Plan 2C).
    pub live: &'static str,
    /// Why a call did not start while `[realtime] enabled` is false.
    pub live_off: &'static str,
```

```rust
    live: "EN VIVO",
    live_off: "El modo en vivo está apagado ([realtime] enabled = false)",
```

```rust
    live: "LIVE",
    live_off: "The live mode is off ([realtime] enabled = false)",
```

and both `help` lines gain the key, in the tables' own separator style:

```rust
    help: " Enter preguntar · !texto recitar · Tab micro · F4 borrar · F10 llamada · ←→ chat/red · Esc salir",
```

```rust
    help: " Enter ask · !text recite · Tab mic · F4 clear · F10 call · ←→ chat/network · Esc quit",
```

`crates/tipsy-ui/src/render.rs` — `draw_header` (`:78-97`) draws the call first:

```rust
fn draw_header(frame: &mut Frame, state: &UiState, area: Rect, title: &str) {
    let strings = state.strings();
    let spin = SPINNER[(state.elapsed_ms / 150) as usize % SPINNER.len()];
    let status: Span = if state.live {
        // A live call outranks everything else in the header: it bills for as long as it lasts, so
        // no spinner and no notice may hide it (Review Focus 3).
        Span::from(format!(" ⏺ {} ", strings.live)).fg(Color::Black).bg(Color::Red).bold()
    } else if state.mic_open {
```

- [ ] **Step 4: The key, the socket and the guard**

`crates/tipsy-compadre/src/client.rs` — the imports gain what a call needs:

```rust
use crate::config::Config;
use crate::keys::{env_var_name, KeyResolver};
use crate::realtime::{Call, CallSetup, Heard};
```

`struct Client` grows one field, beside `call`:

```rust
    /// Where a call reports: the same channel the session reports on, so the HUD reads one stream.
    /// `None` only before the frame loop has started.
    events: Option<UnboundedSender<ToClient>>,
```

`Client::new` initialises it with `events: None,`, and `frame_loop` (`:104-107`) hands the sender
over before the system monitor takes it:

```rust
        let ends = spawn_with_events(self.config.clone(), providers, BrainConfig::default());
        let to_server = ends.to_server;
        let mut from_server = ends.from_server;
        // A call reports through the session's own channel; the monitor keeps the other clone.
        self.events = Some(ends.events.clone());
        sysmon::spawn(ends.events);
```

`on_action` (`:241-266`) gains the arm:

```rust
            UiAction::ToggleLive => {
                self.toggle_call();
                false
            }
```

and `Client` gains the two methods after `end_call`:

```rust
    /// Starts a live call, or ends the one that is running.
    fn toggle_call(&mut self) {
        if let Some(call) = self.call.take() {
            call.stop();
            return;
        }
        if let Err(why) = self.start_call() {
            // The mode is never silently on (Review Focus 3): a refusal is a sentence the user
            // reads, never a key press that quietly did nothing.
            self.state.notify(why);
        }
    }

    /// Opens the socket, or explains why it will not.
    ///
    /// The key is resolved the way every provider's is (`KeyResolver::system`: the environment
    /// first, then the credential store) and is carried straight into `CallSetup` — it is never
    /// logged, never put in an event and never in an error message.
    fn start_call(&mut self) -> Result<(), String> {
        let realtime = self.config.realtime.clone();
        if !realtime.enabled {
            return Err(self.state.strings().live_off.to_string());
        }
        let key = KeyResolver::system()
            .resolve(&realtime.key)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                format!(
                    "no key named `{}`: set {} or store it",
                    realtime.key,
                    env_var_name(&realtime.key)
                )
            })?
            .0;
        let events = self.events.clone().ok_or("the client's frame loop is not running")?;
        // Whatever the session was still saying belongs to the pipeline the call replaces.
        self.flush_playback();
        self.call = Some(Call::open(CallSetup::from_config(&self.config, key), Heard::new(), events));
        Ok(())
    }
```

(`env_var_name` comes from `crate::keys`, beside `KeyResolver`.)

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo test -p tipsy-ui --lib strings`

Expected: PASS, `1 passed`:
`test strings::tests::the_live_indicator_has_copy_in_both_languages ... ok`

Run: `cargo test -p tipsy-ui --lib call`

Expected: PASS, `2 passed`:
`test state::tests::f10_asks_for_the_call_and_the_socket_answers ... ok` and
`test state::tests::the_end_of_a_call_takes_the_indicator_down_and_stops_the_spinner ... ok`

Run: `cargo test -p tipsy-ui --test render`

Expected: PASS, 12 tests:

```
running 12 tests
test chat_log_shows_user_and_assistant_lines ... ok
test clickable_areas_sit_on_their_labels ... ok
test english_header_uses_the_english_live_indicator ... ok
test english_hud_uses_english_strings ... ok
test input_box_shows_typed_text_or_placeholder ... ok
test listening_shows_red_indicator_text ... ok
test network_tab_shows_chart_legend ... ok
test rings_fit_inside_their_panel_when_it_is_not_wide_enough_for_them ... ok
test spanish_hud_shows_every_panel ... ok
test the_header_shows_the_live_indicator_instead_of_anything_else ... ok
test the_live_indicator_disappears_when_the_call_ends ... ok
test tiny_terminals_do_not_panic ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```

Run: `cargo test -p tipsy-compadre --lib client`

Expected: PASS, 8 tests — Task 5's seven plus
`test client::tests::a_call_never_starts_while_the_mode_is_disabled ... ok`.

Run: `cargo test -p tipsy-compadre --test realtime`

Expected: PASS, 12 tests, with `a_socket_that_dies_mid_answer_leaves_the_hud_out_of_the_live_state`
now asserting the HUD's own flag and spinner.

Run: `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`

Expected: all green, `Finished`, no warnings. The wasm boundary is untouched by construction:
`tipsy-ui` gains two `&'static str` fields, one `bool`, one `UiAction` variant and one match arm, and
no `tokio`, no socket and no `std` item that was not already there. The
`wasm32-unknown-unknown` target is not installed on this machine (`rustup target list --installed`
lists only `x86_64-pc-windows-msvc`), so there is no `--target` build in this step; if it is added
later, `cargo check -p tipsy-ui --target wasm32-unknown-unknown` is the check.

- [ ] **Step 6: The README — the key, the cost and the release checklist**

`README.md`'s Keys table (`:47-55`) gains one row between `F4` and the arrows:

```markdown
| `F10` | Start a live call, or end the one that is running (`[realtime]`, OpenAI Realtime). `Esc` also ends it, by quitting |
```

and a new section goes after the `[providers.transcribe]` example block, before `## Privacy`
(`:112`, i.e. after line 141's closing paragraph):

````markdown
## Realtime voice mode

`F10` starts and ends a **live call**: one duplex WebSocket session with OpenAI's Realtime API
(`gpt-realtime-2.1` by default) in which the microphone streams up, the model's speech comes down as
it is generated, the server segments the turns, and you can talk over it mid-sentence. The header
shows a red `EN VIVO` / `LIVE` for as long as the call lasts, and the call ends by itself at
`[realtime] max_minutes`.

**A call is not free, and it does not hide what it costs.** Realtime bills per token, and an open
microphone is billed while it streams. From the published rates for `gpt-realtime-2.1` ($32 per 1M
audio input tokens, $64 per 1M audio output tokens, one input token per 100 ms of audio and one
output token per 50 ms of it) that is roughly **$0.32 per minute of listening and $1.28 per minute of
the model speaking** — about **$0.10 and $0.40** on `gpt-realtime-2.1-mini` ($10/$20 per 1M). Those
figures are arithmetic from the per-token prices, not quoted per-minute prices, and every
`response.done` carries the session's own `usage` if you want to check them. Input transcription, if
you enable it, bills separately on the transcription model's rate card.

The mode is off until you turn it on, and it will not start while it is off: the key set the section
names (`openai` by default) has to resolve first.

```toml
[realtime]
enabled = false          # a call bills while it is open: this is the switch
model = "gpt-realtime-2.1"
key = "openai"           # the key *name*, never the key itself
voice = "marin"          # realtime voices are their own list, not the profile's `voices` table
silence_ms = 1000        # server-side turn detection: the silence that ends your turn
max_minutes = 10         # the call ends by itself here
```

The cheaper push-to-talk variant — framing the input by hand instead of streaming an open microphone
— is documented as a future option and is not implemented.
````

and a new `## Release checklist` section at the end of the file (after `## Building`):

````markdown
## Release checklist

The manual checks of the design spec (§13), terminal first. Nothing here replaces `cargo test`; it is
what a test cannot hear.

- [ ] The client starts, greets you out loud, and answers a typed question in the log.
- [ ] `Tab` opens the microphone, what you said appears as `tú …`, and `F2` replays the reply.
- [ ] `F9` opens the configuration screen, a change saves, and it takes effect without a restart.
- [ ] **A live call, one spoken exchange (`F10`).** With `[realtime] enabled = true` and a key stored
      or exported, press `F10`: the header shows a red `EN VIVO` / `LIVE`, say one sentence, see it
      in the log as `tú …`, hear the answer, watch its transcript appear while it is spoken, and
      press `F10` again to end the call — the indicator must go.
      This is the only check that exercises the real socket, the real 24 kHz audio path and the real
      barge-in. The payload fields of `response.output_audio.delta` and
      `response.output_audio_transcript.delta` are **inferred from sibling events in the research
      note, not verified** (its documentation page truncated before the Response section), so the
      fixtures in `crates/tipsy-compadre/tests/realtime.rs` and this one exchange are all that stand
      behind them. If the transcript stays empty while the audio plays, that inference is what is
      wrong, and the field name is what to change.
- [ ] Talk over the call mid-answer: the speech stops at once, the log keeps only what was said
      before the interruption, and the model does not carry on from the cut-off sentence.
- [ ] The real microphone and speakers in the browser, and the wake word in Spanish and English,
      once those milestones land.
````

- [ ] **Step 7: Commit**

```bash
git add crates/tipsy-ui/src/state.rs crates/tipsy-ui/src/strings.rs crates/tipsy-ui/src/render.rs crates/tipsy-ui/tests/render.rs crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/tests/realtime.rs README.md
git commit -m "feat(ui): show and switch a live call

F10 starts and ends a call, chosen because it is the one free function key no
browser binds to navigation; the red EN VIVO / LIVE indicator outranks every
other header state for as long as the call lasts, and the call's own end stops
the spinner a socket-started turn would otherwise leave running. The README
states what the mode costs per minute from the published per-token prices, and
the release checklist gains the one spoken exchange that is the only proof of
the API's inferred transcript fields.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

**After this task:** the process's single whole-branch review runs before merging (the standing
ruling is no per-task reviews), and the live-key smoke test stays on the release checklist rather
than being attempted by an implementer. Two things the review should look at first, because they are
the only places where a later API change lands: the inferred `item_id` / `content_index` / `delta`
field names of `response.output_audio.delta` and `response.output_audio_transcript.delta`, pinned by
the fixtures in `crates/tipsy-compadre/tests/realtime.rs`; and the assumed names Tasks 2 and 3 were
written against, listed in each task's Interfaces block above — `Socket::connect` / `next_text` /
`send_text` / `close`, `SessionConfig`, `ModelEvent` and its variant names, `parse_event`,
`append_audio_message`, `base64_encode`, `base64_decode`, `truncate_message` and `cancel_message`.

---

## Coordinator notes

- **The interface decisions Tasks 1–3 pinned** (the contract for Tasks 4–6): `RealtimeConfig { enabled: false, model: "gpt-realtime-2.1", key: "openai", voice: "marin", max_minutes: 10 }` with turn detection reusing `[listening] silence_ms` (there is no `silence_ms` in `[realtime]` — the task table said otherwise and is corrected); `Event::Live { active: bool }` for the HUD — the design the two halves were reconciled to, see the ruling below; it breaks exactly one exhaustive match (`tipsy-ui/src/state.rs`), measured, and gets a behaviour-free arm there; `RealtimeSocket::{connect, connect_with, recv, send, ping, close, session_expired, reopen}`, `SocketEvent`, `RealtimeError` (mirroring `ProviderError` plus a missing-key case) and `CallOptions::{at, with_cap, with_open_timeout}`; and `Action`/`EventError`/`map` from the event mapping.
- **Two structural wrinkles the plan's File Structure did not foresee:** Task 2 creates `src/realtime/mod.rs` (with `pub mod realtime;` in `lib.rs`) so `socket.rs` has a module at all, and Task 4 later fills that file with the call façade; and Task 1's last step also updates spec §8.1's example with the `[realtime]` section, the same precedent as Plan 2B's Task 1.
- **Task 3 reuses Plan 2A's base64 decoder** instead of adding a crate for it, so `crates/tipsy-compadre/src/providers/gemini.rs`'s `decode_base64` becomes `pub(crate)` and that file is in Task 3's commit. No base64 dependency is added.
- **The note's payload uncertainty lives in exactly three places:** Task 3's two fixtures and its two `field(..., "delta")` calls. If the live smoke test shows a different shape, those three change together — the bodies say so, so nobody rewrites the mapping around a guess.
- **Both this plan and Plan 2B need one crates.io fetch before the tasks that add their dependency run**, and a sandboxed shell cannot reach crates.io (`schannel: AcquireCredentialsHandle failed`, measured): this plan needs `tokio-tungstenite` 0.30 + its rustls tree, and 2B needs `earshot` 1.2. Warm the cache with one approved unconfined command — or the user runs the same `cargo add` in their own terminal — and have the affected tasks run cargo with `--offline` afterwards. `cargo tree` must then show no `native-tls`, no `openssl` and no `schannel`.
- **The interface conflict between the plan's two halves, ruled:** Tasks 4–6 were written against `Event::Live { active: bool }` (an event the server sends when a call opens and when it ends) while Tasks 1–3 had chosen a new `AssistantState` variant for the same job. **`Event::Live` wins**, for two reasons: the server owns the truth, so the indicator cannot be on while no socket is open (Review Focus 3), and "the call ended" is explicit instead of being inferred from a status transition — which also leaves `AssistantState` meaning exactly one thing, what the assistant is doing during a turn. The Tasks 1–3 half is being reconciled to it: adding an `Event` variant costs a behaviour-free arm in the same three exhaustive matches Plan 2B's Task 2 already documents, and Task 1's enum, tests, expected error text and commit message all change with it.
- **`silence_ms` belongs to `[listening]`**, not to `[realtime]`: Tasks 4–6 assumed the field on `RealtimeConfig` and are corrected. It travels to the socket through the call's own options, which Task 2 already threads.
- **One file the task table did not list:** `crates/tipsy-compadre/src/audio/device.rs` gains `Player::progress()` — how much audio has actually been played — because Review Focus 2's honesty requirement cannot be met from the queue's contents alone. It belongs to Task 5.
- **A trap Task 4 must not miss:** `tipsy-ui`'s state opens a reply placeholder for a transcript that arrives with no pending entry; without it a call's transcripts are silently dropped.
- **Task 4 deliberately leaves `Control`/`Call::interrupt` to Task 5**, so nothing is a dead variant. The call key is `F10` (justified against spec §9's browser-reserved list), `Tab` while live is the local interrupt, and there is deliberately no mouse equivalent: a call may only start from an explicit key press.
