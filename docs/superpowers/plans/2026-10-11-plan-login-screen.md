# Login screen Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status: integrated on `master`, and this document is short by two task bodies.** All six tasks were
executed, reviewed (one whole-branch review, one fix wave, one scoped re-review that closed every
finding) and merged. The written bodies below cover **tasks 1–4**; **tasks 5 (the widget and the veil)
and 6 (the client wiring and the documents) were executed from briefs and exist here only as their
reports** — `task-5-report.md` and `task-6-report.md` in
`.superpowers/sdd/2026-10-11-plan-login-screen/`. That directory is the plan's real record: read
**`coordinator-rulings.md`** there first, because `progress.md` in it was overwritten by another
session and does not tell the truth.

**Goal:** Ask who you are before the assistant lets you in: a widget with the core in the middle over a
veiled, unreadable HUD, which opens for the keywords the machine's owner listed in `config.toml`
(`andres`, `core`) and tells anyone else to go and get their hair combed somewhere else — by keyboard
or by voice, with the answer judged on the server and the whole thing documented as a ritual rather
than a lock.

**Architecture:** Four additions and no new dependency. `tipsy-protocol` gains three commands and two
events plus one field on `Welcome`; `crates/tipsy-compadre/src/config/model.rs` gains the `[login]`
section; a new pure module `crates/tipsy-compadre/src/login.rs` owns the matching rules; `session.rs`
gains a **login capture** — its own `VadState` in front of the existing transcribe provider, a
transcription that emits a verdict instead of starting an LLM turn; and `tipsy-ui` gains
`LoginState`, the widget and the veil, which is what makes the native TUI, the web terminal (same TUI
in a PTY) and the future web client share one screen. The token stays the only real door: nothing here
is authentication, and no task may describe it as such.

**Tech Stack:** the workspace as Plan 2C leaves it — Rust `edition = "2024"`, pinned `1.96.1`,
`tokio` actors, `ratatui`/`crossterm` in `tipsy-ui`, `earshot` behind `VadState`. **Nothing is added:**
the accent fold is a `match`, the veil is one pass over the frame's buffer, and the capture reuses the
detector and the `TranscribeProvider` that already exist.

**Spec:** `docs/superpowers/specs/2026-10-11-login-screen-design.md` (§1–§13; §4 is the wire, §5 the
rules, §6 the screen, §7 the veil, §8 the copy). The v1 spec
(`docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md`) is read for §9 (the interface and its
keyboard/mouse rule), §11 (security: the token) and §12 (errors).

## Inputs: what this plan read in the code

Line numbers are as of the commit this plan was written against; every task reads the file again
before it edits it.

- `crates/tipsy-protocol/src/lib.rs` — `PROTOCOL_VERSION = 1` (:7); `Command` is
  `#[serde(tag = "type", rename_all = "snake_case")]` and its last variant is
  `TestProvider { capability: Capability }` (:113); `Event` is tagged the same way and its last
  variant is `Live { active: bool }` (:146); `Welcome { session_id, profile, ui_language,
  listening_mode, web_bundle }` (:119-125); `ProfileView { id, name, language }` (:92-97). Plan 2B
  (`TestProvider`/`TestResult`) and Plan 2C (`Live`) both added variants **without** bumping the
  constant: this plan follows them (spec L10).
- `crates/tipsy-protocol/tests/roundtrip.rs:3-13` — `roundtrip_command(Command)` and
  `roundtrip_event(Event)`: `serde_json::to_string` → `from_str` → `assert_eq!` with the JSON in the
  panic message.
- `crates/tipsy-compadre/src/config/model.rs:8-40` — `Config { active_profile, server, listening,
  providers, realtime, profiles }` with `#[serde(default)]`; `ServerConfig` (:42-66) and
  `RealtimeConfig` (Plan 2C) are the two shapes `LoginConfig` is written after.
- `crates/tipsy-compadre/src/config/store.rs:20-30` — `load_or_create(&Path) -> io::Result<LoadOutcome>`
  and `save(&Path, &Config) -> io::Result<()>`; its tests use `tempfile::tempdir()` (:100-131) and
  pin that **loading never rewrites the file** (:267).
- `crates/tipsy-compadre/src/config/form.rs:17-102` — `form_of` / `config_from`. The precedent this
  plan copies is `Realtime` (`:41-50` and `:91-99`): a section no tab draws, carried through the form
  so a save of something else cannot reset it.
- `crates/tipsy-ui/src/config_form.rs:95-145` — `ServerForm`, `RealtimeForm` and `ConfigForm` with its
  **hand-written** `Default` (:195-226); `crates/tipsy-ui/src/lib.rs:9` re-exports `config_form::*`,
  so a new public form type needs no further export.
- `crates/tipsy-compadre/src/vad.rs` — `VadEvent { SpeechStarted, SpeechEnded, None }` (:21-29),
  `trait Scorer` (:37-40), `Earshot` (:46-62), `VadState<S = Earshot>` with `VadState::new(silence_ms)`
  (:86-88) and a **private** `with_scorer` (:95); the onset is 3 × 16 ms (:18) and the test module's
  `Scripted` scorer (:262-264) is what the session's own tests will reuse.
- `crates/tipsy-compadre/src/session.rs:255-287` — `handle`/`dispatch`: a `Frame::Mic` is dropped
  unless `self.listening` (:272-275), `Auto` feeds `detect` (:279), `Push`/`HandsFree` accumulate into
  `self.mic` (:282); `Session::new` builds `vad: VadState::new(listening_silence_ms)` (:78).
- `crates/tipsy-compadre/src/brain.rs:74-98` — `TurnInput::Utterance` transcribes, emits
  `Event::Heard`, and **then answers with the LLM** (`:87-90`, `:102-121`). That is exactly why a
  login answer needs its own path: it must produce a verdict, not a reply, and must leave the
  conversation history alone.
- `crates/tipsy-compadre/src/client.rs:505-520` — the command classification (`ListenStop`/`Reset`
  start a turn), which is where a login's commands must **not** land.
- `crates/tipsy-ui/src/render.rs:40-41,65-109,516-548` — `RING_VIEW = 50.0`; `draw`'s full-screen
  branch for the config form (`:74-91`, which draws the HUD *not at all*); `Hit { area, key, action }`
  (`:50-63`); `draw_rings` (:516-548) with its Braille canvas and `state.voice_level()` pulse.
- `crates/tipsy-ui/src/state.rs:100-190` — `UiState` (`config_form: Option<ConfigForm>` at :125),
  `KeyInput` (:70-81), `UiAction` (:83-98), `toggle_config_form` (:171) and `show_config_form` (:187).
- `crates/tipsy-ui/src/strings.rs` — the two static tables, keyed by `UiLanguage`; every string the UI
  draws lives there.
- `crates/tipsy-web` **does not exist** (`crates/` holds `tipsy-protocol`, `tipsy-ui`,
  `tipsy-compadre`): the web client is Plan 3's, and this plan's contract for it is §4 and §6 of the
  spec, not code.

## Decisions this plan makes, and why

- **The commit trailer changes.** Plan 2C's global constraints require
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` on every commit. The harness that wrote
  those plans is not the harness executing this one, so this plan's messages are Conventional Commits
  with a short body and no model trailer. A false co-author line is worse than a missing one.
- **The login capture gets a test seam in `vad.rs`, and it is declared.** The login path can only be
  tested by driving the detector through a script, and earshot cannot be asked to call synthetic audio
  "speech". So `VadState::with_scorer` becomes `pub(crate)`, the test module's `Scripted` moves to a
  `#[cfg(test)] pub(crate) mod scripted`, and the session's `login_vad` is a
  `VadState<Box<dyn Scorer>>` (with a one-line `impl Scorer for Box<dyn Scorer>`), which is how a test
  hands the capture a script. Production still boxes `Earshot`. This is one indirection per 16 ms
  window and it buys the whole capture its tests.
- **The capture lives in `session.rs`, next to the detector and the mic it shares state with.** A
  separate `login.rs` module owns the *rules* (Task 2) and knows nothing about audio, sockets or
  events: the two must not be confused, because one is a pure function with a table of examples and
  the other owns a buffer and a task.
- **`judge` answers `ok: true` when the screen does not apply.** One rule, in one place (spec §4.1):
  `applies` decides whether there is a screen at all, and a stray answer to a screen that is not there
  must never be refused.
- **The login transcription is a session task with a cancel token, like a turn.** A newer utterance
  cancels the transcription in flight (`session.rs` already replaces a running turn the same way,
  :254-256), so "the last thing said is the thing that counts" costs no new machinery.
- **`LoginState` adds exactly one `UiAction`.** `UiAction::LoginDone` is how the screen tells the
  client it is finished; every other control on the screen resolves to a `Command` the existing
  `UiAction::Send` already carries, and a click is the keystroke its control answers (`Hit.key`), the
  convention `render.rs:56-62` already documents.
- **The veil is drawn in `draw`, not in a wrapper widget.** `draw` already owns the frame and already
  branches for the config form; it gains one more branch that paints the HUD, veils
  `frame.buffer_mut()` outside the widget's rectangle, clears that rectangle and draws the widget.
- **The copy is data, and it is frozen.** Spec §8's strings are typed into `strings.rs` **verbatim**,
  punctuation and «guillemets» included. An implementer who would phrase it differently says so in
  their report; they do not reword it (CLAUDE.md: the profile copy is the user's to approve).

## Global Constraints

- Toolchain pinned by `rust-toolchain.toml`; every crate uses `edition = "2024"`.
- All code, identifiers, comments, docs and commit messages are in **English**. Interface copy keeps
  the two static es/en tables, and the login copy is the spec's §8 table word for word.
- `tipsy-protocol` and `tipsy-ui` must NOT depend on `tokio`, `cpal`, `rodio`, `keyring`, `sysinfo`,
  `reqwest`, `crossterm` or `earshot`, and must keep compiling to `wasm32-unknown-unknown`. The
  capture, the detector and the transcription live in `tipsy-compadre`; everything the screen learns
  arrives as a protocol event.
- **No new dependency.** The accent fold is a `match`, the veil is a buffer pass, the tokenizer is
  `char::is_alphanumeric`.
- API keys never appear in files, logs, events or fixtures, and the login answers a user types are
  **not** logged: an answer is a person's words, and the only thing that leaves the module is a
  verdict and a boolean. A `tracing::debug!` may name the command it ignored, never the text.
- Mono 16 kHz PCM16 in `Frame::Mic`, unchanged; a login answer is the same audio the microphone
  already sends.
- Every cargo command in this plan runs in the **login worktree** with this prelude, and never in the
  original checkout:

  ```powershell
  $env:CARGO_TARGET_DIR="E:\workspace\personal\tipsy-login\.target-sandbox"
  $env:TEMP="E:\workspace\personal\tipsy-login\.target-sandbox\tmp"
  $env:TMP=$env:TEMP
  cargo test -j 1 …
  ```

  Building into the configured `G:` target labels the artifacts low-integrity and breaks the app's
  config write (Plan 1's ledger); `-j 1` avoids this machine's `STATUS_STACK_BUFFER_OVERRUN`; the
  worktree has no `.cargo/config.toml` (it is excluded, never committed), so nothing lands on `G:` by
  accident; and `TEMP` must be overridden, because the shell's own `G:\dev\tmp` currently denies
  directory creation (`Acceso denegado`, os error 5) and makes every `tempfile::tempdir()` panic at its
  `.unwrap()` naming a path that has nothing to do with the code — 22 existing tests fail without it.
  The worktree's baseline before Task 1 was 314 tests and 0 failures.
- **Lint and format gates, restated for this baseline.** The repository is not clippy-clean and not
  rustfmt-clean at `e7bd82f`, in code no task of this plan touches: `cargo clippy --workspace
  --all-targets` fails with `-D warnings` on **seven pre-existing findings** (listed in the ledger's
  2026-10-11 ruling) and `cargo fmt --all -- --check` reports 36 files. So `-D warnings` and
  `fmt --check` are **not** usable gates here, and a task never runs `cargo fmt`. A task's lint gate
  is: run `cargo clippy --workspace --all-targets` and confirm the finding set is the same seven — line
  numbers may shift if the task edits one of those files, the findings themselves may not — with
  nothing new in the task's own files, and keep the task's own edits in the surrounding hand style. The
  baseline debt is not fixed from inside a task: it is a dedicated cleanup, parked as a follow-up.
- **Every task works in `E:\workspace\personal\tipsy-login`**, a git worktree on branch
  `plan-login-screen`, and never in the original checkout: another agent works the 2C plan there, and on
  2026-10-11 it stashed this plan's in-flight files to get its own tree clean (the ledger tells the
  story). The worktree is based on that branch's tip, so everything this plan reads is where it expects
  it.
- The client never reads `[login]` to decide whether to show the screen: `Welcome.login` is the only
  authority (spec §3). A test pins that, because it is the one rule that keeps a native client
  connected to a remote server from disagreeing with the server that will judge it.
- Commit messages are Conventional Commits with a short body, no model trailer.

## Review Focus

Each line is an input or failure the spec implies, with the test that pins it added to the owning
task. Most likely first.

1. **A keyword list that is on but unusable must not lock the owner out.** `enabled = true` with an
   entry whose `keywords` are missing, empty or whitespace must mean *no screen at all* (spec R1), and
   a `config.toml` carrying it must not be rewritten by a load. Task 2 owns the `applies` tests; Task 1
   owns the store's "loading never rewrites" test for the section.
2. **An answer that is only punctuation, emoji or spaces must be judged, not trusted.** `tokens("")`,
   `tokens("!!!")`, `tokens("🔥")` are empty vectors, and an empty keyword must never match them by
   accident (R4/R6). Task 2 owns it, including the non-ASCII case, because `char::is_alphanumeric` is
   where a naive implementation reaches for `is_ascii_alphanumeric` and starts splitting `á` in half.
3. **A `Welcome` from a server that does not know the field must parse, and a client that does not
   know the field must keep working.** An absent `login` is `None`, and the field is additive in both
   directions. Task 1 owns both tests; a plan that adds a variant to `Event` without them breaks every
   already-deployed client at the first handshake.
4. **The microphone can die mid-capture.** A device unplugged, a permission revoked or a stream that
   errors must close the capture, leave the screen typeable, and never feed login audio to the
   listening path — and the session must not panic on a frame that arrives after `LoginStop`. Task 3
   owns it; Task 5 owns the client half.
5. **The heard line is a person's text and can be cut anywhere.** Truncating «te he oído: …» to the
   widget's width must respect UTF-8 boundaries (a Naïve `&text[..w]` panics on `é`), and a frame too
   small for the widget must degrade to the input row instead of panicking in `Rect` arithmetic. Task 5
   owns both.

---

## File Structure

```
crates/tipsy-protocol/src/lib.rs             MODIFY: LoginView, three commands, two events, Welcome.login
crates/tipsy-protocol/tests/roundtrip.rs     MODIFY: the new messages, and Welcome with and without the field
crates/tipsy-ui/src/config_form.rs           MODIFY: LoginForm/AllowedPersonForm, carried not drawn
crates/tipsy-compadre/src/config/model.rs    MODIFY: LoginConfig, LoginEntry, Config.login
crates/tipsy-compadre/src/config/form.rs     MODIFY: form_of/config_from carry the section
crates/tipsy-compadre/src/config/store.rs    MODIFY: tests only — the section's round trip and an old file
crates/tipsy-compadre/src/login.rs           NEW: applies, tokens, judge, Verdict — the rules of spec §5
crates/tipsy-compadre/src/lib.rs             MODIFY: pub mod login
crates/tipsy-compadre/src/vad.rs             MODIFY: the pub(crate) seam and the shared scripted scorer
crates/tipsy-compadre/src/brain.rs           MODIFY: transcribe_once — a transcription with no turn
crates/tipsy-compadre/src/session.rs         MODIFY: the login capture, the verdict, the ignore rule
crates/tipsy-compadre/src/client.rs          MODIFY: Welcome.login, the mic, the keys, LoginDone
crates/tipsy-ui/src/state.rs                 MODIFY: LoginState, UiState.login, the key table, LoginDone
crates/tipsy-ui/src/render_login.rs          NEW: the widget, its borders, its geometry, the veil
crates/tipsy-ui/src/render.rs                MODIFY: draw's login branch; the ring canvas into a helper
crates/tipsy-ui/src/strings.rs               MODIFY: the ten strings of spec §8, es and en
crates/tipsy-ui/src/lib.rs                   MODIFY: pub mod render_login
crates/tipsy-compadre/tests/login.rs         NEW: the session's scripted capture, end to end
README.md                                    MODIFY: the [login] block and the ritual's honest paragraph
docs/architecture.md                         MODIFY: one sentence about the login's explicit recording
docs/superpowers/specs/2026-10-10-…-design.md MODIFY: §8.1's example file gains [login]
```

## Tasks

| # | Task `[tier]` | Deliverable | Files | Depends on |
|---|---|---|---|---|
| 1 | The wire and the section `[small]` | `LoginView`, `login_start`/`login_stop`/`login_answer`, `login_heard`/`login_verdict`, `Welcome.login`, and the `[login]` section with its defaults, its form passthrough and its tests | `tipsy-protocol/*`, `config/*`, `tipsy-ui/src/config_form.rs` | — |
| 2 | The matcher `[small]` | `applies`, `tokens`, `judge` and `Verdict`: the rules of spec §5, pure, with the spec's own example table as a test | `login.rs`, `lib.rs` | 1 |
| 3 | The capture and the verdict `[mid]` | A login capture with its own detector in front of the transcribe provider: `login_heard` then `login_verdict`, no reply, no history, a 15 s cap, and foreign commands ignored | `session.rs`, `vad.rs`, `brain.rs`, `tests/login.rs` | 1, 2 |
| 4 | The screen's state `[small]` | `LoginState`, `UiState.login`, the key table of spec §6.2, the attempt guard and `UiAction::LoginDone` | `tipsy-ui/src/state.rs` | 1 |
| 5 | The widget and the veil `[mid]` | The centred widget with the core, the single status line, the input box, the two borders, its degradation, and the veil with its two strengths | `render_login.rs`, `render.rs`, `strings.rs`, `lib.rs` | 4 |
| 6 | The client and the docs `[small]` | `Welcome.login` becomes a screen, the microphone opens and closes with it, the keys route to it, and the three documents say what this is | `client.rs`, `README.md`, `docs/*` | 1–5 |

**After Task 6:** the process's single whole-branch review runs before merging (the standing ruling is
**no per-task reviews**), then one fix wave and one scoped re-review of it — the same shape Plans 1,
2A and 2B ran.

### Task 1: The wire and the section `[small]`

**Files:**
- Modify: `crates/tipsy-protocol/src/lib.rs` — `Command` gains three variants after `TestProvider`
  (:113); `Event` gains two after `Live` (:146); `LoginView` goes beside `ProfileView` (:92-97);
  `Welcome` gains its field (:119-125)
- Modify: `crates/tipsy-protocol/tests/roundtrip.rs` — three tests appended after the existing ones
- Modify: `crates/tipsy-ui/src/config_form.rs` — `LoginForm` and `AllowedPersonForm` after
  `RealtimeForm` (:110-133); `ConfigForm` gains `pub login` (:134-145); its hand-written `Default`
  gains the field (:195-226)
- Modify: `crates/tipsy-compadre/src/config/model.rs` — `LoginConfig` and `LoginEntry` after
  `RealtimeConfig`, and `Config` gains `pub login` (:8-27)
- Modify: `crates/tipsy-compadre/src/config/form.rs` — `form_of` (:41-50) and `config_from` (:91-99)
  carry the section; its test module gains the round trip
- Modify: `crates/tipsy-compadre/src/config/store.rs` — tests only, appended to the existing module
- Modify: `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md` — §8.1's example file

**Interfaces:**
- Consumes (read from the committed code, not recalled):
  - `#[serde(tag = "type", rename_all = "snake_case")] pub enum Command { Hello { token, client, protocol_version }, Ask { text }, Say { text }, ListenStart, ListenStop, Cancel, Reset, SetProfile { id }, SetListeningMode { mode }, TestProvider { capability } }` — `tipsy-protocol/src/lib.rs:99-114`
  - `#[serde(tag = "type", rename_all = "snake_case")] pub enum Event { Welcome { … }, Status { … }, Heard { text }, …, Live { active } }` — `tipsy-protocol/src/lib.rs:116-147`
  - `pub struct Welcome { session_id: u64, profile: ProfileView, ui_language: UiLanguage, listening_mode: ListeningMode, web_bundle: bool }` — `:119-125`
  - `pub struct LoginConfig` does not exist yet; `RealtimeConfig` (`config/model.rs`, Plan 2C) is the shape to write after: `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(default)]` plus a hand-written `Default`
  - `pub fn form_of(config: &Config) -> ConfigForm` and `pub fn config_from(form: &ConfigForm) -> Result<Config, String>` — `config/form.rs:17`, `:66`
  - `roundtrip_command(c: Command)` / `roundtrip_event(e: Event)` — `tipsy-protocol/tests/roundtrip.rs:3-13`
- Produces:

```rust
// crates/tipsy-protocol/src/lib.rs

/// Which login screen a client should show before its HUD (spec `2026-10-11-login-screen-design.md`).
/// `Option::None` on `Welcome` means "no login screen": the client goes straight to the HUD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginView {
    /// Whether the microphone opens with the screen. Typing always works, and is always free.
    pub voice: bool,
}

// Command, after `TestProvider`:
    /// The login screen is up and the microphone is opening (`login_start` on the wire). The server
    /// answers nothing: what it does is start collecting the audio the client is about to stream.
    LoginStart,
    /// Stop capturing and discard whatever was buffered (`login_stop`). Idempotent.
    LoginStop,
    /// A typed answer (`login_answer`). Judged whether or not a capture is active, so a typing-only
    /// client never has to send `LoginStart` at all.
    LoginAnswer { text: String },

// Event, after `Live`:
    /// What the transcriber heard from a login utterance, before the verdict. The screen shows it so
    /// that a misheard accent is visible rather than mysterious.
    LoginHeard { text: String },
    /// The ruling: whether the door opens, and the matched entry's name when it does.
    LoginVerdict { ok: bool, name: Option<String> },

// Welcome, last field:
    /// `None` — no login screen. Present on every `Welcome` from this server, absent from an older
    /// one's, which `#[serde(default)]` turns into `None`.
    #[serde(default)]
    login: Option<LoginView>,
```

```rust
// crates/tipsy-compadre/src/config/model.rs

/// `[login]` in `config.toml` (spec §5, plan login-screen). Both switches default in, and an empty
/// `allowed` means the screen never appears — which is what makes the defaults safe for a file
/// written before this feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoginConfig {
    pub enabled: bool,
    /// The microphone opens with the screen; only speech the local detector confirms is transcribed.
    pub voice: bool,
    /// Ordered on purpose: the first entry whose keywords are all in the answer wins (spec R5).
    pub allowed: Vec<LoginEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoginEntry {
    /// What the screen greets them with. May be empty.
    pub name: String,
    /// All of them must be in the answer, as whole words, in the order they are written.
    pub keywords: Vec<String>,
}

impl Default for LoginConfig {
    fn default() -> Self {
        Self { enabled: true, voice: true, allowed: Vec::new() }
    }
}

impl Default for LoginEntry {
    fn default() -> Self {
        Self { name: String::new(), keywords: Vec::new() }
    }
}

// in Config, after `realtime` and before `profiles`:
pub login: LoginConfig,
```

```rust
// crates/tipsy-ui/src/config_form.rs

/// `[login]`, carried and never drawn, exactly as `RealtimeForm` is: no tab of the screen shows it,
/// but a save rebuilds the whole configuration from this form, so a hand-written allowlist has to
/// survive the trip or saving an unrelated row would erase it.
#[derive(Debug, Clone, PartialEq)]
pub struct LoginForm {
    pub enabled: bool,
    pub voice: bool,
    pub allowed: Vec<AllowedPersonForm>,
}

/// One `[[login.allowed]]` entry, as plain values.
#[derive(Debug, Clone, PartialEq)]
pub struct AllowedPersonForm {
    pub name: String,
    pub keywords: Vec<String>,
}

impl Default for LoginForm {
    /// The same defaults as `LoginConfig`, and a hand-written `Default` for the same reason
    /// `ConfigForm`'s is: `derive` would say `enabled: false`, which is a different section.
    fn default() -> Self {
        Self { enabled: true, voice: true, allowed: Vec::new() }
    }
}
```

- [ ] **Step 1: Write the failing protocol tests**

Append to `crates/tipsy-protocol/tests/roundtrip.rs`, after the last test in the file:

```rust
/// The login screen's three commands and two events (spec §4). They are additive, which is why
/// `PROTOCOL_VERSION` does not move: Plan 2B's `TestResult` and Plan 2C's `Live` did the same.
#[test]
fn login_commands_and_events_roundtrip() {
    roundtrip_command(Command::LoginStart);
    roundtrip_command(Command::LoginStop);
    roundtrip_command(Command::LoginAnswer { text: "yo soy Andrés del equipo del core".into() });
    roundtrip_event(Event::LoginHeard { text: "yo soy Andrés del equipo del core".into() });
    roundtrip_event(Event::LoginVerdict { ok: true, name: Some("Andrés".into()) });
    roundtrip_event(Event::LoginVerdict { ok: false, name: None });
}

/// The wire spellings are a contract: a client and a server that disagree about the tag do not fail
/// to compile, they fail to connect.
#[test]
fn the_login_messages_spell_their_tags_as_the_spec_says() {
    let json = serde_json::to_string(&Command::LoginAnswer { text: "andres".into() }).unwrap();
    assert!(json.contains(r#""type":"login_answer""#), "{json}");
    let json = serde_json::to_string(&Event::LoginVerdict { ok: false, name: None }).unwrap();
    assert!(json.contains(r#""type":"login_verdict""#), "{json}");
    assert!(json.contains(r#""ok":false"#), "{json}");
    assert!(json.contains(r#""name":null"#), "{json}");
}

/// Both directions of compatibility in one test: a server that predates the field sends a `Welcome`
/// without it, and a client that knows it must still parse the event (that is `#[serde(default)]`);
/// serde ignores unknown fields, which is what makes the other direction work.
#[test]
fn a_welcome_without_the_login_field_parses_as_no_login_screen() {
    let json = r#"{"type":"welcome","session_id":1,"profile":{"id":"compae","name":"El Compae","language":"es-ES"},"ui_language":"es","listening_mode":"push","web_bundle":false}"#;
    match serde_json::from_str::<Event>(json).expect("an older server's Welcome must parse") {
        Event::Welcome { login, .. } => assert_eq!(login, None, "an absent field is no screen"),
        other => panic!("expected a Welcome, got {other:?}"),
    }
    roundtrip_event(Event::Welcome {
        session_id: 1,
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: UiLanguage::Es,
        listening_mode: ListeningMode::Push,
        web_bundle: false,
        login: Some(LoginView { voice: true }),
    });
}
```

The test file's `use tipsy_protocol::{…}` list gains `LoginView`; every other name these three tests
need (`Command`, `Event`, `ProfileView`, `UiLanguage`, `ListeningMode`) is already imported there.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -j 1 -p tipsy-protocol`
Expected: FAIL to compile — `cannot find variant LoginStart`, `no field login on type Welcome`, and
`cannot find type LoginView`. A test that cannot compile is the failure this step wants.

- [ ] **Step 3: Implement the protocol additions**

In `crates/tipsy-protocol/src/lib.rs`, add `LoginView` beside `ProfileView`, the three `Command`
variants after `TestProvider`, the two `Event` variants after `Live`, and the field on `Welcome` — all
of them exactly as the Interfaces block above writes them, comments included. Nothing else in the file
changes: `PROTOCOL_VERSION` stays `1`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -j 1 -p tipsy-protocol`
Expected: PASS, the whole crate's suite with three new tests.

- [ ] **Step 5: Write the failing section tests**

Append to `crates/tipsy-compadre/src/config/store.rs`'s test module:

```rust
/// The section round-trips through the file, entries in order: file order is the ranking the matcher
/// uses (spec R5), so a save that reordered them would change who is greeted first.
#[test]
fn login_round_trips_and_keeps_its_entry_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut c = Config::default();
    c.login.enabled = true;
    c.login.voice = false;
    c.login.allowed = vec![
        LoginEntry { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] },
        LoginEntry { name: "María".into(), keywords: vec!["maria".into(), "data".into()] },
    ];
    save(&path, &c).unwrap();
    let again = load_or_create(&path).unwrap().config;
    assert_eq!(again.login, c.login);
    assert_eq!(again.login.allowed[0].name, "Andrés", "the first entry is still first");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("[[login.allowed]]"), "the array is written as an array of tables:\n{text}");
}
```

and, beside the crate's existing "a file written before this feature loads" test:

```rust
/// A file from before this feature has no `[login]` section, and loading it must not invent one in
/// the file: the screen never appears (no entries, so `applies` is false), and the file is untouched.
#[test]
fn a_file_without_the_login_section_loads_and_is_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[listening]\nmode = \"auto\"\n").unwrap();
    let c = load_or_create(&path).unwrap().config;
    assert!(c.login.enabled, "the switch defaults on; the entries are what make a screen");
    assert!(c.login.voice);
    assert!(c.login.allowed.is_empty());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("login"), "loading must not rewrite the file, got:\n{text}");
}
```

And append to `crates/tipsy-compadre/src/config/form.rs`'s test module:

```rust
/// The one section the screen does not draw, through the one conversion that would otherwise drop it.
/// This is the test Plan 2C added for `[realtime]`, and it is here for the same reason: a save from
/// the configuration screen must not erase a hand-written allowlist.
#[test]
fn a_save_from_the_screen_keeps_the_login_section() {
    let mut config = Config::default();
    config.login.enabled = false;
    config.login.voice = false;
    config.login.allowed = vec![LoginEntry { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] }];

    let form = form_of(&config);
    let back = config_from(&form).expect("an untouched form saves");
    assert_eq!(back.login, config.login, "the section survives the trip through the form");
}
```

`LoginEntry` joins the store test module's imports, and `form.rs`'s test module needs `LoginEntry`
too (`Config` and `form_of`/`config_from` it already has).

- [ ] **Step 6: Run the tests to verify they fail**

Run: `cargo test -j 1 -p tipsy-compadre config::`
Expected: FAIL to compile — `no field login on type Config`, `cannot find type LoginEntry`, and in
`form.rs` the same two.

- [ ] **Step 7: Implement the section and its passthrough**

Add `LoginConfig` and `LoginEntry` to `crates/tipsy-compadre/src/config/model.rs` exactly as the
Interfaces block writes them, and `pub login: LoginConfig` to `Config` between `realtime` and
`profiles`; `Config::default()` gains `login: LoginConfig::default()`.

Add `LoginForm` and `AllowedPersonForm` to `crates/tipsy-ui/src/config_form.rs` after `RealtimeForm`,
give `ConfigForm` its `pub login: LoginForm` field, and add `login: LoginForm::default()` to the
hand-written `Default`. `crates/tipsy-ui/src/lib.rs:9` re-exports `config_form::*`, so nothing else in
`tipsy-ui` changes.

In `crates/tipsy-compadre/src/config/form.rs`, carry the section in both directions, in the same place
`realtime` is carried and with the same comment's shape:

```rust
    // Carried, not drawn: the same rule as `[realtime]` above, and for a section that is *only* ever
    // written by hand. A save that dropped it would lock the owner out of their own login screen.
    form.login = LoginForm {
        enabled: config.login.enabled,
        voice: config.login.voice,
        allowed: config.login
            .allowed
            .iter()
            .map(|entry| AllowedPersonForm { name: entry.name.clone(), keywords: entry.keywords.clone() })
            .collect(),
    };
```

```rust
        login: LoginConfig {
            enabled: form.login.enabled,
            voice: form.login.voice,
            allowed: form
                .login
                .allowed
                .iter()
                .map(|entry| LoginEntry { name: entry.name.clone(), keywords: entry.keywords.clone() })
                .collect(),
        },
```

with the imports updated in both files (`LoginEntry` into `form.rs`'s `super::model` list,
`AllowedPersonForm` and `LoginForm` into its `tipsy_ui` list).

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -j 1 -p tipsy-compadre config::` and then `cargo test -j 1 -p tipsy-ui`
Expected: PASS — the four new tests plus the existing suites. If `tipsy-ui`'s render tests fail, the
`ConfigForm` literal in a test was not updated: fix the test, never the default.

- [ ] **Step 9: The v1 spec's example file gains the section**

In `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md` §8.1, append to the example
`config.toml`, after the `[realtime]` block:

```toml
# The login screen (spec docs/superpowers/specs/2026-10-11-login-screen-design.md). A ritual, not a
# lock: `server.token` above is the only real requirement. The screen appears only when `enabled` is
# true *and* an entry below has keywords; the first entry whose keywords are all in the answer wins,
# and every keyword is matched as a whole word, case- and accent-insensitively.
[login]
enabled = true
voice = true            # the microphone opens with the screen; typing always works

[[login.allowed]]
name = "Andrés"
keywords = ["andres", "core"]
```

- [ ] **Step 10: Commit**

```bash
git add crates/tipsy-protocol/src/lib.rs crates/tipsy-protocol/tests/roundtrip.rs \
        crates/tipsy-ui/src/config_form.rs \
        crates/tipsy-compadre/src/config/model.rs crates/tipsy-compadre/src/config/form.rs \
        crates/tipsy-compadre/src/config/store.rs \
        docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md
git commit -m "feat(protocol,config): carry the login screen's messages and section

Three commands and two events for the login gate, plus Welcome.login, which is
the only authority on whether a client shows the screen: a native client never
reads [login] itself, so it cannot disagree with the server that judges it. The
section itself is carried through the configuration form exactly as [realtime]
is, because a save from the screen rebuilds the whole file and would otherwise
erase a hand-written allowlist."
```

### Task 2: The matcher `[small]`

**Files:**
- Create: `crates/tipsy-compadre/src/login.rs` — the rules of spec §5 and its tests
- Modify: `crates/tipsy-compadre/src/lib.rs` — the module list gains `pub mod login;` beside the
  other top-level modules

**Interfaces:**
- Consumes:
  - `pub struct LoginConfig { pub enabled: bool, pub voice: bool, pub allowed: Vec<LoginEntry> }` and
    `pub struct LoginEntry { pub name: String, pub keywords: Vec<String> }` — Task 1,
    `crates/tipsy-compadre/src/config/model.rs`
  - The rules of spec §5.1 (R1–R7) and the worked examples of §5.2, which are this task's test table
- Produces:

```rust
// crates/tipsy-compadre/src/login.rs

/// The ruling on one answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub ok: bool,
    /// The matched entry's `name`, or `None` when that entry has none: the screen then greets without
    /// a name (`Pasa.`).
    pub name: Option<String>,
}

impl Verdict {
    /// The ruling when there is no screen to answer (spec §4.1): nothing to check, nobody refused.
    pub fn open() -> Self {
        Self { ok: true, name: None }
    }

    /// The ruling on an answer nobody's keywords matched.
    pub fn refused() -> Self {
        Self { ok: false, name: None }
    }
}

/// True when the login screen applies at all (R1): the section is on **and** at least one entry has a
/// keyword that survives normalization. A mis-edited file can therefore never slam the door.
pub fn applies(login: &LoginConfig) -> bool;

/// The tokens of one answer or keyword (R2): lowercased, Latin diacritics folded, split on everything
/// that is not alphanumeric. `"ANDRÉS, del CORE!!"` → `["andres", "del", "core"]`.
pub fn tokens(s: &str) -> Vec<String>;

/// The first entry, in file order, whose every keyword is in the answer (R3–R6).
pub fn judge(login: &LoginConfig, answer: &str) -> Verdict;
```

- [ ] **Step 1: Write the failing tests**

Create `crates/tipsy-compadre/src/login.rs` with the module's doc comment, the real signatures of the
Interfaces block with `unimplemented!()` bodies, and the test module complete. The tests must compile
before they can fail, which is why the signatures come first and the bodies do not.

**Add `pub mod login;` to `crates/tipsy-compadre/src/lib.rs` in this same step**, before running
anything: an undeclared module is not compiled at all, so `cargo test -p tipsy-compadre login::` would
match zero tests and pass, and Step 2 would have no red run to observe — a TDD cycle that starts with a
green lie is worse than no cycle.

```rust
//! The login gate's rules (spec `docs/superpowers/specs/2026-10-11-login-screen-design.md`, §5).
//!
//! Pure, synchronous and dependency-free on purpose. The capture that produces an answer's text and
//! the screen that draws the verdict live elsewhere; nothing here knows about a frame, a provider or a
//! socket, which is what lets the whole of what the ritual decides be a table of examples.

use crate::config::model::LoginConfig;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::LoginEntry;

    fn entry(name: &str, keywords: &[&str]) -> LoginEntry {
        LoginEntry { name: name.into(), keywords: keywords.iter().map(|k| (*k).into()).collect() }
    }

    fn login(enabled: bool, entries: Vec<LoginEntry>) -> LoginConfig {
        LoginConfig { enabled, voice: true, allowed: entries }
    }

    /// R2, and the reason `char::is_alphanumeric` is the predicate: a naive `is_ascii_alphanumeric`
    /// splits `á` off its word, and `ñ` is a letter in this project's profiles.
    #[test]
    fn tokens_fold_case_accents_and_punctuation() {
        assert_eq!(tokens("ANDRÉS, del CORE!!"), vec!["andres", "del", "core"]);
        assert_eq!(tokens("el niño del core"), vec!["el", "nino", "del", "core"]);
        assert_eq!(tokens("MARÍA-JOSÉ  y   el  core"), vec!["maria", "jose", "y", "el", "core"]);
        assert_eq!(tokens("  "), Vec::<String>::new());
    }

    /// Review Focus 2: an answer that is only punctuation, emoji or spaces tokenizes to nothing, and
    /// an empty keyword must never match it by accident.
    #[test]
    fn an_answer_of_punctuation_or_emoji_alone_is_judged_and_refused() {
        let config = login(true, vec![entry("Andrés", &["andres", "core"])]);
        for answer in ["", "   ", "!!!", "¿?", "🔥", "«»"] {
            assert_eq!(judge(&config, answer), Verdict::refused(), "{answer:?}");
        }
        // An entry whose keywords are all punctuation is not a usable entry either: R1 says the
        // screen does not even appear, and R6 says it never matches.
        let blank = login(true, vec![entry("Nadie", &["", "  ", "!!!"])]);
        assert!(!applies(&blank), "blank keywords are not a usable allowlist");
        assert_eq!(judge(&blank, "!!!"), Verdict::open(), "…and with no screen there is nothing to refuse");
    }

    /// R1: both halves are needed, and the second one is the one that keeps an owner from locking
    /// themselves out of their own application.
    #[test]
    fn applies_needs_the_switch_and_one_usable_keyword() {
        assert!(applies(&login(true, vec![entry("Andrés", &["andres"])])));
        assert!(applies(&login(true, vec![entry("", &["andres"])])), "a name is not required");
        assert!(!applies(&login(false, vec![entry("Andrés", &["andres"])])), "the switch is the switch");
        assert!(!applies(&login(true, Vec::new())), "no entries, no screen");
        assert!(!applies(&login(true, vec![entry("Andrés", &[])])), "no keywords, no screen");
        assert!(!applies(&login(true, vec![entry("Andrés", &["   ", ""])])), "blank is not a keyword");
    }

    /// The spec's own worked examples (§5.2), driven as a table so the wording and the code cannot
    /// drift apart.
    #[test]
    fn the_worked_examples_of_the_spec() {
        let config = login(true, vec![entry("Andrés", &["andres", "core"]), entry("María", &["maria", "data"])]);
        let cases: &[(&str, Verdict)] = &[
            ("yo soy Andrés del equipo del core", Verdict { ok: true, name: Some("Andrés".into()) }),
            ("ANDRÉS, del CORE!!", Verdict { ok: true, name: Some("Andrés".into()) }),
            ("soy andres del score", Verdict::refused()),
            ("soy Andrés", Verdict::refused()),
            ("soy María, conozco a Andrés del core", Verdict { ok: true, name: Some("Andrés".into()) }),
            ("soy Maria de data", Verdict { ok: true, name: Some("María".into()) }),
            ("el niño del core", Verdict::refused()),
        ];
        for (answer, expected) in cases {
            assert_eq!(&judge(&config, answer), expected, "{answer:?}");
        }
        let nino = login(true, vec![entry("El Niño", &["nino", "core"])]);
        assert_eq!(judge(&nino, "el niño del core"), Verdict { ok: true, name: Some("El Niño".into()) });
    }

    /// R3: whole tokens, so a keyword is never a substring of a longer word.
    #[test]
    fn a_keyword_is_never_a_substring() {
        let score = login(true, vec![entry("Andrés", &["andres", "core"])]);
        assert_eq!(judge(&score, "soy andres del score"), Verdict::refused());
        let short = login(true, vec![entry("Andrés", &["andres"])]);
        assert_eq!(judge(&short, "soy andresito del core"), Verdict::refused(), "andresito is not andres");
    }

    /// R3's other half: a multi-word keyword is one run, said together.
    #[test]
    fn a_multi_word_keyword_must_be_said_in_one_run() {
        let config = login(true, vec![entry("Andrés", &["andres del equipo", "core"])]);
        assert_eq!(judge(&config, "soy andres del equipo del core"), Verdict { ok: true, name: Some("Andrés".into()) });
        assert_eq!(judge(&config, "soy andres, y el equipo del core mola"), Verdict::refused());
    }

    /// R5: file order is the ranking, and the first match wins even when a later entry would match too.
    /// The answer has to satisfy **both** entries (R4: every keyword, not any), or the test passes on
    /// the second entry's name and proves nothing about order.
    #[test]
    fn the_first_entry_in_file_order_wins() {
        let config = login(true, vec![entry("Andrés", &["andres", "core"]), entry("María", &["maria", "andres"])]);
        assert_eq!(judge(&config, "soy andres del core y maria"), Verdict { ok: true, name: Some("Andrés".into()) });
    }

    /// R6: a keyword written the way a person writes it behaves like its folded form, and an empty
    /// name is a `None` rather than an empty greeting.
    #[test]
    fn keywords_are_normalized_as_written_and_an_empty_name_is_none() {
        let written = login(true, vec![entry("Andrés", &["Andrés", "CORE"])]);
        assert_eq!(judge(&written, "soy andres del core"), Verdict { ok: true, name: Some("Andrés".into()) });
        let nameless = login(true, vec![entry("  ", &["andres"])]);
        assert_eq!(judge(&nameless, "andres"), Verdict::open(), "an empty name opens without one");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -j 1 -p tipsy-compadre login::`
Expected: FAIL to compile with `cannot find function tokens in this scope` / `applies` / `judge`, and
`no variant open` on `Verdict` until Step 3 exists.

- [ ] **Step 3: Implement the matcher**

```rust
use crate::config::model::LoginConfig;

/// The ruling on one answer: whether the door opens, and who it opened for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub ok: bool,
    /// The matched entry's `name`, or `None` when that entry has none — the screen then greets without
    /// a name (`Pasa.`).
    pub name: Option<String>,
}

impl Verdict {
    /// The ruling when there is no screen to answer (spec §4.1): nothing to check, nobody refused.
    pub fn open() -> Self {
        Self { ok: true, name: None }
    }

    /// The ruling on an answer none of the keywords matched.
    pub fn refused() -> Self {
        Self { ok: false, name: None }
    }
}

/// True when the login screen applies at all (R1): the section is on **and** at least one entry has a
/// keyword that survives normalization. A mis-edited file can therefore never slam the door: no usable
/// keyword, no screen.
pub fn applies(login: &LoginConfig) -> bool {
    login.enabled && login.allowed.iter().any(|entry| !usable_keywords(entry).is_empty())
}

/// The tokens of one answer or keyword (R2): lowercased, Latin diacritics folded, split on everything
/// that is not alphanumeric (Unicode: `ñ` and `á` are letters, `¿` is not).
pub fn tokens(s: &str) -> Vec<String> {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(fold)
        .collect::<String>()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The first entry, in file order, whose every keyword is in the answer (R3–R6).
pub fn judge(login: &LoginConfig, answer: &str) -> Verdict {
    if !applies(login) {
        return Verdict::open();
    }
    let said = tokens(answer);
    login
        .allowed
        .iter()
        .find(|entry| {
            let keywords = usable_keywords(entry);
            !keywords.is_empty() && keywords.iter().all(|keyword| contains_run(&said, keyword))
        })
        .map(|entry| Verdict {
            ok: true,
            name: (!entry.name.trim().is_empty()).then(|| entry.name.clone()),
        })
        .unwrap_or_else(Verdict::refused)
}

/// An entry's keywords, normalized, with the ones that normalize to nothing dropped: an empty keyword
/// is ignored rather than matching everything (R6).
fn usable_keywords(entry: &LoginEntry) -> Vec<Vec<String>> {
    entry.keywords.iter().map(|keyword| tokens(keyword)).filter(|keyword| !keyword.is_empty()).collect()
}

/// True when `keyword` appears as a contiguous run of `said`'s tokens (R3): `score` is not `core`, and
/// a multi-word keyword has to be said in one go.
fn contains_run(said: &[String], keyword: &[String]) -> bool {
    !keyword.is_empty() && keyword.len() <= said.len() && said.windows(keyword.len()).any(|w| w == keyword)
}

/// The Spanish and Galician letters this project's profiles write, folded to their plain form. A
/// character with no entry keeps its own lowercase form, so a profile in another language still works.
fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ý' | 'ÿ' => 'y',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}
```

`LoginEntry` joins the `use` line of the file (it is used by the private helpers below `judge`, even
though the public signatures only name `LoginConfig`).

- [ ] **Step 4: Run the tests**

`crates/tipsy-compadre/src/lib.rs` already declares the module (Step 1), so nothing is added here.

Run: `cargo test -j 1 -p tipsy-compadre login::`
Expected: PASS — the eight new tests.

- [ ] **Step 5: Run the crate's whole suite, and the lints**

Run: `cargo test -j 1 -p tipsy-compadre`
Expected: PASS, nothing else disturbed (the module is new and nothing calls it yet).

Run: `cargo clippy --workspace --all-targets`, and compare its findings with the seven pre-existing ones
the ledger lists (Global Constraints, "Lint and format gates"): the list must be exactly those seven,
none of them new, and none of them in `crates/tipsy-compadre/src/login.rs`. Do **not** run `cargo fmt`:
the baseline is not rustfmt-clean and formatting it would rewrite hundreds of unrelated lines. Keep the
new module in the surrounding hand style.

- [ ] **Step 6: Commit**

```bash
git add crates/tipsy-compadre/src/login.rs crates/tipsy-compadre/src/lib.rs
git commit -m "feat(login): match an answer against the allowlist

The rules of the login spec in one pure module: normalization folds case and
Spanish diacritics, a keyword matches as a whole token so 'score' is never
'core', a multi-word keyword must be said in one run, the first entry in file
order wins, and an entry without usable keywords never matches — which is also
what makes a half-edited config.toml show no screen instead of a locked door."
```

### Task 3: The capture and the verdict `[mid]`

**Files:**
- Modify: `crates/tipsy-compadre/src/vad.rs` — `with_scorer` becomes `pub(crate)` (:95); a forwarding
  `Scorer` impl for `Box<S>` after `Earshot` (:62); the test module's `Scripted` (:262-264) moves into
  a `#[cfg(test)] pub(crate) mod scripted` and changes `Rc<RefCell<Calls>>` for `Arc<Mutex<Calls>>`
- Modify: `crates/tipsy-compadre/src/brain.rs` — `pub async fn transcribe_once` after `run_turn`
  (:125), reusing the private `transcribe` (:321), `Transcribed` (:479), `report` (:492) and `error`
  (:496)
- Modify: `crates/tipsy-compadre/src/session.rs` — the constant and imports (:15-20); the `Session`
  fields after `greeting` (:86) and its literal (:67-86); `welcome` (:216-251); the `Frame::Mic` arm of
  `dispatch` (:271-283); `command` (:337-353); the capture's methods after `listen_stop` (:438); the
  in-file tests (:524 onwards)
- Create: `crates/tipsy-compadre/tests/login.rs` — the typed path, the absent chat, the ignore rule and
  the frames that arrive after the stop

**Interfaces:**
- Consumes:
  - `pub enum VadEvent { SpeechStarted, SpeechEnded, None }`, `pub trait Scorer { fn score(&mut self, frame: &[i16]) -> f32 }`, `pub struct VadState<S = Earshot>` with `pub fn new(silence_ms: u32) -> Self` and `fn with_scorer(scorer: S, silence_ms: u32) -> Self`, and `pub fn push(&mut self, samples: &[i16]) -> VadEvent` — `vad.rs:21-29`, `:37-40`, `:67-100`. The onset needs three voiced windows in a row (`ONSET_FRAMES`, `:18`) and the hangover is `silence_ms.div_ceil(16)` windows (`:100-102`)
  - `async fn transcribe(&self, providers: &Providers, pcm: Pcm, cancel: &CancellationToken) -> Transcribed` and `enum Transcribed { Text(String), Failed(ProviderFailure) }`, `fn report(out, &failure)`, `fn error(out, code, message, recoverable)` — `brain.rs:321`, `:479`, `:492`, `:496`, all private to `brain.rs`
  - `pub fn current(&self) -> Providers` on `ProviderSlot` — `providers/mod.rs:182`; `pub struct Providers { pub chat, pub speech, pub transcribe }` (`:154`) and `ProviderSlot::new(Providers)` (`:170`); `pub trait TranscribeProvider: Send + Sync { async fn transcribe(&self, pcm: Pcm, language: &str) -> Result<String, ProviderError> }` (`:148`)
  - `Session::dispatch(&mut self, ToServer)` (`:268`), `Session::command(&mut self, Command)` (`:337`), the module's private `status(state)`/`send(out, message)` (`:546`, `:551`), `turn_profile(&Config)` (`:99`), and the `Session` literal's field list (`:67-86`)
  - `Brain::new(providers: ProviderSlot, profile: TurnProfile, config: BrainConfig)` and `BrainConfig { chat_timeout, speech_timeout, transcribe_timeout, retry_backoff }` with `Default` — `brain.rs:29-36`, `:59`
  - The fakes of `crates/tipsy-compadre/tests/brain.rs:16-115` — the shape of a `FakeTranscribe` and of `ProviderSlot::new(Providers { chat, speech, transcribe })` this task's own fakes copy
- Produces:

```rust
// crates/tipsy-compadre/src/brain.rs

/// One transcription with the turn pipeline's own timeout and error mapping, and **no turn**: nothing
/// is asked of the chat provider, nothing is spoken, and the history is not touched. The login
/// screen's voice path is the only caller (spec `2026-10-11-login-screen-design.md`, §4.1) — reusing
/// `TurnInput::Utterance` instead would have answered the user's login out loud.
pub async fn transcribe_once(
    &self,
    pcm: Pcm,
    out: &UnboundedSender<ToClient>,
    cancel: &CancellationToken,
) -> Option<String>;
```

```rust
// crates/tipsy-compadre/src/vad.rs

impl<S: Scorer + ?Sized> Scorer for Box<S> {
    fn score(&mut self, frame: &[i16]) -> f32 {
        (**self).score(frame)
    }
}

#[cfg(test)]
pub(crate) mod scripted {
    #[derive(Debug, Default)]
    pub struct Calls { pub scores: Vec<f32>, pub asked: usize }
    pub struct Scripted(/* Arc<Mutex<Calls>> */);
    impl Scripted {
        /// A detector and the handle that says how many windows it was asked about.
        pub fn shared(scores: Vec<f32>) -> (Self, std::sync::Arc<std::sync::Mutex<Calls>>);
        /// A detector with a script and no handle: the login capture's tests only care what it decides.
        pub fn new(scores: Vec<f32>) -> Self;
    }
    impl super::Scorer for Scripted { /* answers the next score, 0.0 when the script runs out */ }
}
```

```rust
// crates/tipsy-compadre/src/session.rs

/// Samples of continuous speech after which a login utterance is closed whether or not the detector
/// has ended it: a login answer is a sentence, and a transcription bill is not a joke (spec §4.1).
const LOGIN_MAX_UTTERANCE_SAMPLES: usize = MIC_SAMPLE_RATE as usize * 15;

// in Session, after `greeting`:
    /// True between `LoginStart` and `LoginStop`: the microphone's frames belong to the login screen
    /// and to nothing else (spec §4.2).
    login_active: bool,
    /// The login capture's own audio, and the detector that decides what in it is speech. Both are
    /// separate from `mic`/`vad` on purpose: a capture and a listening session must never share a
    /// carry buffer, and they can never be open at the same time.
    login_mic: Vec<i16>,
    login_heard: bool,
    /// Boxed rather than `Earshot` so a test can hand it a script: `earshot` cannot be asked to call
    /// synthetic audio speech, and the capture's logic is what needs testing.
    login_vad: VadState<Box<dyn Scorer + Send>>,
    /// The login transcription in flight, if any: a newer utterance replaces it (spec §4.1).
    login_turn: Option<CancellationToken>,

// methods (all private):
    fn login_start(&mut self);
    fn login_stop(&mut self);
    fn login_answer(&mut self, text: String);
    fn login_frame(&mut self, samples: &[i16]);
    fn login_utterance_ended(&mut self);
```

and on the wire, from those methods: `Event::LoginHeard { text }` then
`Event::LoginVerdict { ok, name }`, with `Event::Status { Transcribing }` before them and
`Event::Status { Idle }` after — no `Reply`, no `ReplyDelta`, no `SpeechChunk`, ever.

- [ ] **Step 1: The detector seam, with the existing tests still green**

In `crates/tipsy-compadre/src/vad.rs`:

1. `fn with_scorer` becomes `pub(crate) fn with_scorer`, keeping its body and its doc comment (`:95`).
2. After `Earshot`'s impl (`:62`), add the forwarding impl exactly as the Interfaces block writes it.
3. The test module's `Scripted` and its `Calls` move into a `#[cfg(test)] pub(crate) mod scripted` at
   the bottom of the file, written as the Interfaces block describes, with `Scripted::shared` and
   `Scripted::new`. The existing tests' helper `fn state(calls, scores, silence_ms)` (`:262-264`) is
   rewritten to build the detector through `Scripted::shared` and keeps asserting on `asked`.

Run: `cargo test -j 1 -p tipsy-compadre vad::`
Expected: PASS — the same tests as before the refactor. This step adds no behaviour: if a `vad.rs` test
changes shape, the refactor went too far, and the answer is to fix the test's construction, never its
assertion.

- [ ] **Step 2: Write the failing capture tests**

At the end of `crates/tipsy-compadre/src/session.rs`'s `#[cfg(test)] mod tests`, add the fakes, the two
helpers and three tests:

```rust
    // the tests' own imports, added to the module's existing ones:
    //   crate::vad::{scripted::Scripted, Scorer}
    //   crate::providers::{ChatProvider, ChatRequest, ProviderError, Providers, SpeechProvider,
    //                      SpeechRequest, TextStream, TranscribeProvider}
    //   crate::audio::device::MIC_FRAME_SAMPLES   (MIC_SAMPLE_RATE is already imported at the top)
    //   crate::config::{Config, LoginConfig, LoginEntry}
    //   async_trait::async_trait, std::sync::{Arc, Mutex}, std::sync::atomic::{AtomicUsize, Ordering},
    //   tokio::time::timeout
```

Add only what the module does not already have: `session.rs`'s `use super::*` already brings
`ProviderSlot`, `VadState`, `VadEvent`, `Brain`, `BrainConfig`, `Pcm`, `i16_to_f32`, `UiLanguage`,
`ListeningMode`, `Command`, `Event`, `Frame`, `ToServer`, `ToClient` and `AssistantState` into scope.

```rust
    /// A chat provider that must never be asked anything. It counts the calls instead of panicking
    /// because panicking inside the spawned turn would be reported as a task failure, not as the
    /// honest assertion this test wants.
    struct NeverChat(Arc<AtomicUsize>);

    #[async_trait]
    impl ChatProvider for NeverChat {
        async fn stream(&self, _req: ChatRequest) -> Result<TextStream, ProviderError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(ProviderError::BadResponse("the login path must not ask for a reply".into()))
        }
    }

    /// A speech provider that must never be asked anything either.
    struct NeverSpeech;

    #[async_trait]
    impl SpeechProvider for NeverSpeech {
        async fn synthesize(&self, _req: SpeechRequest) -> Result<Pcm, ProviderError> {
            Err(ProviderError::BadResponse("the login path must not speak".into()))
        }
    }

    struct FakeTranscribe(&'static str);

    #[async_trait]
    impl TranscribeProvider for FakeTranscribe {
        async fn transcribe(&self, _pcm: Pcm, _language: &str) -> Result<String, ProviderError> {
            Ok(self.0.to_string())
        }
    }

    /// A session whose transcriber answers `transcript`, whose chat and speech providers count their
    /// calls, and whose login detector answers `scores` in order. The listening detector is the real
    /// one: these tests never open it.
    fn login_session(
        config: Config,
        transcript: &'static str,
        scores: Vec<f32>,
    ) -> (Session, UnboundedReceiver<ToClient>, Arc<AtomicUsize>) {
        let chat_calls = Arc::new(AtomicUsize::new(0));
        let providers = ProviderSlot::new(Providers {
            chat: Arc::new(NeverChat(Arc::clone(&chat_calls))),
            speech: Arc::new(NeverSpeech),
            transcribe: Arc::new(FakeTranscribe(transcript)),
        });
        let brain_config = BrainConfig::default();
        let brain = Arc::new(Brain::new(providers.clone(), turn_profile(&config), brain_config.clone()));
        let (out, rx) = mpsc::unbounded_channel();
        let silence_ms = config.listening.silence_ms;
        let profile = config.active().clone();
        let session = Session {
            config,
            brain,
            brain_config,
            providers,
            out,
            history: Vec::new(),
            current: None,
            generation: 0,
            listening: false,
            mic: Vec::new(),
            vad: VadState::new(silence_ms),
            heard: false,
            utterance: Vec::new(),
            guard: None,
            profile: profile.view(),
            ui_language: UiLanguage::from_bcp47(&profile.language),
            listening_mode: ListeningMode::Push,
            greeting: profile.greeting.clone(),
            login_active: false,
            login_mic: Vec::new(),
            login_heard: false,
            login_vad: VadState::with_scorer(Box::new(Scripted::new(scores)) as Box<dyn Scorer + Send>, silence_ms),
            login_turn: None,
        };
        (session, rx, chat_calls)
    }

    /// The script of one confirmed utterance: three voiced windows to open it (`ONSET_FRAMES`), then
    /// the silence budget to close it, then zeros for whatever the detector asks after that.
    fn one_utterance(silence_ms: u32) -> Vec<f32> {
        let mut scores = vec![0.9; 3];
        scores.extend(std::iter::repeat(0.0).take(silence_ms.div_ceil(16) as usize + 1));
        scores
    }

    /// Drives `frames` microphone frames of 20 ms into the session, as a client would.
    fn speak(session: &mut Session, frames: usize) {
        for _ in 0..frames {
            session.dispatch(ToServer::Audio(Frame::Mic(vec![0i16; MIC_FRAME_SAMPLES])));
        }
    }

    /// Everything the session sent until the verdict (or until two seconds pass, which fails the test
    /// rather than hanging it).
    async fn events_until_verdict(rx: &mut UnboundedReceiver<ToClient>) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let next = timeout(Duration::from_secs(2), rx.recv()).await.expect("the verdict must arrive").expect("the channel stays open");
            if let ToClient::Event(event) = next {
                let verdict = matches!(event, Event::LoginVerdict { .. });
                seen.push(event);
                if verdict {
                    return seen;
                }
            }
        }
    }

    fn allowed(name: &str, keywords: &[&str]) -> LoginConfig {
        LoginConfig { enabled: true, voice: true, allowed: vec![LoginEntry { name: name.into(), keywords: keywords.iter().map(|k| (*k).into()).collect() }] }
    }

    /// The whole voice path, and the three things that must *not* happen on it: no chat call, no
    /// speech, no turn. `transcribe_once` is what keeps the login out of the conversation.
    #[tokio::test]
    async fn a_login_utterance_becomes_a_verdict_without_a_turn() {
        let mut config = Config::default();
        config.login = allowed("Andrés", &["andres", "core"]);
        let (mut session, mut rx, chat_calls) = login_session(config, "yo soy Andrés del equipo del core", one_utterance(1000));

        session.command(Command::LoginStart);
        assert!(session.login_active, "the capture opens");
        speak(&mut session, 70);

        let events = events_until_verdict(&mut rx).await;
        assert!(events.iter().any(|e| matches!(e, Event::Status { state: AssistantState::Transcribing, .. })), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, Event::LoginHeard { text } if text == "yo soy Andrés del equipo del core")), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, Event::LoginVerdict { ok: true, name: Some(name) } if name == "Andrés")), "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, Event::Reply { .. } | Event::ReplyDelta { .. } | Event::SpeechChunk { .. })), "the login path never replies: {events:?}");
        assert_eq!(chat_calls.load(Ordering::SeqCst), 0, "the chat provider is never asked");
        assert!(session.history.is_empty(), "and the conversation is untouched");
    }

    /// The same capture, refusing: the keywords are not there, so the verdict says no and the capture
    /// stays open for the next attempt (spec §6.2 — the automatic re-arm).
    #[tokio::test]
    async fn a_login_utterance_without_the_keywords_is_refused_and_the_capture_stays_open() {
        let mut config = Config::default();
        config.login = allowed("Andrés", &["andres", "core"]);
        let (mut session, mut rx, _) = login_session(config, "soy Andrés del score", one_utterance(1000));

        session.command(Command::LoginStart);
        speak(&mut session, 70);

        let events = events_until_verdict(&mut rx).await;
        assert!(events.iter().any(|e| matches!(e, Event::LoginVerdict { ok: false, .. })), "{events:?}");
        assert!(session.login_active, "the next utterance is the next attempt");
    }

    /// Silence is not an answer: no transcription request, `NothingHeard`, and no verdict at all — a
    /// cough must never be counted as a wrong answer (spec §4.1).
    #[tokio::test]
    async fn silence_asks_for_no_transcription_and_no_verdict() {
        let mut config = Config::default();
        config.login = allowed("Andrés", &["andres", "core"]);
        let (mut session, mut rx, _) = login_session(config, "unused", vec![0.0; 80]);

        session.command(Command::LoginStart);
        speak(&mut session, 70);

        let next = timeout(Duration::from_millis(200), rx.recv()).await;
        assert!(next.is_err(), "nothing is sent for silence, got {:?}", next.map(|m| m.map(|m| format!("{m:?}"))));
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -j 1 -p tipsy-compadre session::`
Expected: FAIL to compile — `no field login_active`, `no method login_start`, `no function transcribe_once`.

- [ ] **Step 4: Implement `transcribe_once`, the capture and the landing of the commands**

In `crates/tipsy-compadre/src/brain.rs`, after `run_turn`:

```rust
    /// One transcription with the turn pipeline's own timeout and error mapping, and **no turn**:
    /// nothing is asked of the chat provider, nothing is spoken, and the history is not touched. The
    /// login screen's voice path is the only caller (spec `2026-10-11-login-screen-design.md`, §4.1).
    /// Reusing `TurnInput::Utterance` instead would have answered the login out loud and written it
    /// into the conversation.
    pub async fn transcribe_once(
        &self,
        pcm: Pcm,
        out: &UnboundedSender<ToClient>,
        cancel: &CancellationToken,
    ) -> Option<String> {
        let providers = self.providers.current();
        match self.transcribe(&providers, pcm, cancel).await {
            // Empty text is a failure the user can act on ("nothing was heard"), not an answer.
            Transcribed::Text(text) if text.trim().is_empty() => {
                error(out, ErrorCode::NothingHeard, "nothing was heard", true);
                None
            }
            Transcribed::Text(text) => Some(text),
            Transcribed::Failed(failure) => {
                report(out, &failure);
                None
            }
        }
    }
```

In `crates/tipsy-compadre/src/session.rs`:

```rust
    /// `LoginStart`: open the capture. Ignored when there is no screen to serve, and when a capture is
    /// already open — a repeated start must not eat an utterance in flight (spec §4.1).
    fn login_start(&mut self) {
        if !crate::login::applies(&self.config.login) || self.login_active {
            return;
        }
        self.login_active = true;
        self.login_mic.clear();
        self.login_heard = false;
        self.login_vad = VadState::with_scorer(
            Box::new(crate::vad::Earshot::default()) as Box<dyn Scorer + Send>,
            self.config.listening.silence_ms,
        );
    }

    /// `LoginStop`: close the capture, drop what it held, and drop the transcription in flight with
    /// it — a verdict that arrives after the screen is gone has nothing to answer.
    fn login_stop(&mut self) {
        self.login_active = false;
        self.login_mic.clear();
        self.login_heard = false;
        self.login_turn = None;
    }

    /// `LoginAnswer`: a typed answer, judged here and now. Judged whether or not a capture is open, so
    /// a typing-only client never has to send `LoginStart` (spec §4.1). When the screen does not apply
    /// the ruling opens: there is nothing to check, so nobody is refused.
    fn login_answer(&mut self, text: String) {
        let verdict = crate::login::judge(&self.config.login, &text);
        send(&self.out, ToClient::Event(Event::LoginVerdict { ok: verdict.ok, name: verdict.name }));
    }

    /// One microphone frame of an open capture: the same detector discipline `auto` mode uses, with
    /// its own buffer — nothing before a confirmed onset is kept, and only confirmed speech is ever
    /// transcribed.
    fn login_frame(&mut self, samples: &[i16]) {
        match self.login_vad.push(samples) {
            VadEvent::SpeechStarted => {
                self.login_heard = true;
                self.login_mic.clear();
                self.login_mic.extend_from_slice(samples);
            }
            VadEvent::SpeechEnded => {
                self.login_heard = false;
                self.login_mic.extend_from_slice(samples);
                self.login_utterance_ended();
                return;
            }
            VadEvent::None => {
                if self.login_heard {
                    self.login_mic.extend_from_slice(samples);
                }
            }
        }
        // A monologue is not an answer: the cap closes the utterance the detector has not.
        if self.login_heard && self.login_mic.len() >= LOGIN_MAX_UTTERANCE_SAMPLES {
            self.login_heard = false;
            self.login_utterance_ended();
        }
    }

    /// The capture's detector says the answer is over: transcribe it in a task of its own, exactly as a
    /// turn runs, and let a newer utterance replace this one.
    fn login_utterance_ended(&mut self) {
        let samples = std::mem::take(&mut self.login_mic);
        if samples.is_empty() {
            return;
        }
        let cancel = CancellationToken::new();
        self.login_turn = Some(cancel.clone());
        let brain = Arc::clone(&self.brain);
        let out = self.out.clone();
        let login = self.config.login.clone();
        tokio::spawn(async move {
            send(&out, status(AssistantState::Transcribing));
            let pcm = Pcm { samples: i16_to_f32(&samples), sample_rate: MIC_SAMPLE_RATE };
            if let Some(text) = brain.transcribe_once(pcm, &out, &cancel).await {
                send(&out, ToClient::Event(Event::LoginHeard { text: text.clone() }));
                let verdict = crate::login::judge(&login, &text);
                send(&out, ToClient::Event(Event::LoginVerdict { ok: verdict.ok, name: verdict.name }));
            }
            send(&out, status(AssistantState::Idle));
        });
    }
```

and the three landings:

```rust
        // in `dispatch`, at the top of the `Frame::Mic` arm, before the `self.listening` check:
            ToServer::Audio(Frame::Mic(samples)) => {
                if self.login_active {
                    self.login_frame(&samples);
                    return;
                }
```

```rust
        // in `command`, before the match:
        // While the door is shut, the commands that belong to the other side of it are ignored (spec
        // §4.2): no client of ours can send them — the screen is modal and has no other action — and
        // inventing an error for a state no user can reach would be wire surface with no user behind
        // it. `Hello` is unaffected: the handshake happens before the screen exists.
        if self.login_active
            && !matches!(
                command,
                Command::LoginStart | Command::LoginStop | Command::LoginAnswer { .. } | Command::Hello { .. }
            )
        {
            tracing::debug!(?command, "ignored while the login capture is open");
            return;
        }
        match command {
            Command::LoginStart => self.login_start(),
            Command::LoginStop => self.login_stop(),
            Command::LoginAnswer { text } => self.login_answer(text),
            // … the existing arms, unchanged …
        }
```

and `welcome` gains its last field, which is the **only** thing any client reads to decide whether to
show the screen (spec §3):

```rust
            // The one authority on whether a client shows the login screen. A native client never reads
            // `[login]` itself, so it cannot disagree with the server that will judge it. Sent on every
            // `Welcome`; the client shows the screen once per run and ignores the later ones.
            login: crate::login::applies(&self.config.login)
                .then(|| LoginView { voice: self.config.login.voice }),
```

with `LoginView` added to the `tipsy_protocol` import list and `Scorer` to the `crate::vad` list.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -j 1 -p tipsy-compadre session::`
Expected: PASS — the three new tests. If `silence_asks_for_no_transcription_and_no_verdict` fails with
a message the test did not expect, the detector's hangover arithmetic is the first thing to check:
`one_utterance` gives it `silence_ms.div_ceil(16) + 1` windows, which is one more than the budget.

- [ ] **Step 6: Write the failing integration tests**

Create `crates/tipsy-compadre/tests/login.rs`. It builds a real session through
`spawn_session(config, ProviderSlot::new(providers), BrainConfig::default())` and drives it over the
channels a client uses, with the fakes of `tests/brain.rs:16-115` copied (a `FakeChat` that records its
requests, a `FakeSpeech` and a `FakeTranscribe` — copy them; do not import across test binaries). The copy
needs a `requests()` accessor for the assertions below: `tests/brain.rs`'s version has `calls()` and keeps
`requests` private, so add the getter while you copy it and keep both.

```rust
/// Wait for the next event, or fail rather than hang.
async fn next_event(rx: &mut UnboundedReceiver<ToClient>) -> Event {
    let next = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("an event must arrive")
        .expect("the channel stays open");
    match next {
        ToClient::Event(event) => event,
        other => panic!("expected an event, got {other:?}"),
    }
}

/// The `Welcome` every session sends on start carries the verdict on whether a screen applies at all.
#[tokio::test]
async fn welcome_carries_the_login_view_only_when_the_section_applies() {
    let mut config = Config::default();
    let (to_server, mut from_server, _chat) = session_with(config.clone());
    let _ = &to_server;
    match next_event(&mut from_server).await {
        Event::Welcome { login, .. } => assert_eq!(login, None, "no entries, no screen"),
        other => panic!("expected a Welcome, got {other:?}"),
    }

    config.login.allowed = vec![LoginEntry { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] }];
    let (_to_server, mut from_server, _chat) = session_with(config);
    match next_event(&mut from_server).await {
        Event::Welcome { login, .. } => assert_eq!(login, Some(LoginView { voice: true }), "and the voice switch travels with it"),
        other => panic!("expected a Welcome, got {other:?}"),
    }
}

/// A typed answer never touches the chat provider, opens at the right answer and refuses the rest. It
/// is the whole feature's contract in one test, and the reason `LoginAnswer` is judged whether or not
/// a capture is open: this session never sends `LoginStart`.
#[tokio::test]
async fn a_typed_answer_is_judged_without_ever_asking_the_chat_provider() {
    let mut config = Config::default();
    config.login.allowed = vec![LoginEntry { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] }];
    let (to_server, mut from_server, chat) = session_with(config);
    let _ = next_event(&mut from_server).await; // Welcome

    to_server.send(ToServer::Command(Command::LoginAnswer { text: "soy Andrés del score".into() })).unwrap();
    assert_eq!(next_event(&mut from_server).await, Event::LoginVerdict { ok: false, name: None });

    to_server.send(ToServer::Command(Command::LoginAnswer { text: "yo soy Andrés del equipo del core".into() })).unwrap();
    assert_eq!(
        next_event(&mut from_server).await,
        Event::LoginVerdict { ok: true, name: Some("Andrés".into()) }
    );

    assert_eq!(chat.calls(), 0, "a login is not a question");
    assert!(chat.requests().is_empty(), "and it is never asked for a reply");
}

/// With the screen switched off, every answer opens: a stray answer to a screen that is not there must
/// never refuse anybody (spec §4.1).
#[tokio::test]
async fn a_typed_answer_opens_when_the_screen_does_not_apply() {
    let mut config = Config::default();
    config.login.enabled = false;
    config.login.allowed = vec![LoginEntry { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] }];
    let (to_server, mut from_server, _chat) = session_with(config);
    let _ = next_event(&mut from_server).await;

    to_server.send(ToServer::Command(Command::LoginAnswer { text: "quien sea".into() })).unwrap();
    assert_eq!(next_event(&mut from_server).await, Event::LoginVerdict { ok: true, name: None });
}

/// Review Focus 4, the server's half: the commands of the other side of the door are ignored while the
/// capture is open and no reply is ever produced for them, and a microphone frame that arrives after
/// the stop belongs to nobody and is not a panic.
#[tokio::test]
async fn commands_outside_the_login_are_ignored_while_the_capture_is_open() {
    let mut config = Config::default();
    config.login.allowed = vec![LoginEntry { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] }];
    let (to_server, mut from_server, chat) = session_with(config);
    let _ = next_event(&mut from_server).await;

    to_server.send(ToServer::Command(Command::LoginStart)).unwrap();
    to_server.send(ToServer::Command(Command::Ask { text: "¿qué tal?".into() })).unwrap();
    to_server.send(ToServer::Command(Command::Say { text: "hola".into() })).unwrap();

    // Nothing at all comes back for the two of them: no Status, no Reply, no Error.
    let quiet = tokio::time::timeout(Duration::from_millis(200), from_server.recv()).await;
    assert!(quiet.is_err(), "an ignored command answers nothing, got {:?}", quiet.map(|m| m.map(|m| format!("{m:?}"))));
    assert_eq!(chat.calls(), 0);

    to_server.send(ToServer::Command(Command::LoginStop)).unwrap();
    to_server.send(ToServer::Audio(Frame::Mic(vec![0i16; MIC_FRAME_SAMPLES]))).unwrap();
    to_server.send(ToServer::Command(Command::Ask { text: "ahora sí".into() })).unwrap();
    // …and with the capture closed an `Ask` goes through again: the guard is the capture, not the
    // screen. The reply's `Status` is the proof it started.
    let mut saw_status = false;
    for _ in 0..4 {
        if matches!(next_event(&mut from_server).await, Event::Status { state: AssistantState::Thinking, .. }) {
            saw_status = true;
            break;
        }
    }
    assert!(saw_status, "the capture's guard must lift with `LoginStop`");
}
```

with the file's own helper:

```rust
/// A session with one fake provider of each kind, and the handles the tests assert on.
fn session_with(config: Config) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>, Arc<FakeChat>) {
    let chat = FakeChat::new(vec![Ok(vec!["vale"]), Ok(vec!["vale"]), Ok(vec!["vale"])]);
    let providers = ProviderSlot::new(Providers {
        chat: chat.clone(),
        speech: Arc::new(FakeSpeech::default()),
        transcribe: Arc::new(FakeTranscribe { text: "no se usa", languages: Mutex::new(Vec::new()) }),
    });
    let ends = spawn_session(config, providers, BrainConfig::default());
    (ends.to_server, ends.from_server, chat)
}
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -j 1 -p tipsy-compadre --test login`
Expected: PASS — the four tests. The `Ask` that must go through is the one that proves the ignore rule is
scoped to the capture and not to the session's life.

- [ ] **Step 8: The whole crate, and the lints**

Run: `cargo test -j 1 -p tipsy-compadre`, then `cargo clippy --workspace --all-targets` with the same
seven-finding comparison as Task 2 Step 5 (no new finding; the seven may have moved lines, because this
task edits `session.rs`, and `session.rs:233` is one of them).
Expected: PASS. The suite grew by seven tests and nothing in `brain.rs`, `vad.rs` or the existing session
tests changed behaviour.

- [ ] **Step 9: Commit**

```bash
git add crates/tipsy-compadre/src/vad.rs crates/tipsy-compadre/src/brain.rs \
        crates/tipsy-compadre/src/session.rs crates/tipsy-compadre/tests/login.rs
git commit -m "feat(session): judge a login answer without starting a turn

A login capture of its own: a second detector in front of the existing transcriber,
nothing before a confirmed onset kept, only confirmed speech uploaded, a 15 s cap,
and a verdict instead of a reply. Reusing TurnInput::Utterance would have answered
the login out loud and written it into the conversation, so Brain gains
transcribe_once — the turn pipeline's timeout and error mapping with no turn. A
newer utterance replaces the transcription in flight, and the commands of the other
side of the door are ignored for as long as the capture is open."
```

### Amendments after execution (Task 3, commit `3cea34c`)

Six places where the code above did not survive contact with the compiler, the tests or the clippy
baseline. They are recorded here so a reviewer reads one account of the change instead of diffing the
plan against the branch, and so anyone re-running this task does not rediscover them:

1. **No `tracing::debug!`.** No crate in this workspace depends on `tracing` and the Global Constraints
   forbid adding one, so the ignore is documented in the code instead. Spec §4.2 and §9 were amended to
   say so: the logging sink is the v1 spec's, and whichever plan wires it owns the dependency.
2. **`login_start` re-arms the detector instead of rebuilding it** — `self.login_vad.resume()`, which
   clears the counters and the carry buffer (`vad.rs:129-135`) and is exactly a fresh `VadState`'s state.
   Rebuilding it threw away the scorer this task's own test helper installs: a test seam that production
   code silently replaces is not a seam, and two of the task's tests failed because of it.
3. **`cancel_login_turn()` exists.** Replacing or dropping a `CancellationToken` cancels nothing, so
   spec §4.1's "a newer utterance cancels the one in flight" was not implemented at all. The helper is
   used by both a newer utterance and `LoginStop`, and the task gained a test the plan does not have
   (`a_new_utterance_cancels_the_transcription_in_flight`) that fails against the plan's version.
4. **The integration tests use the house helpers** (`next_event_matching`, waiting for the greeting's
   `Done`, then draining — as `tests/session.rs` does): the session's greeting turn shares the client
   channel, so the plan's `next_event` panicked on the first audio frame. Every assertion of the plan's
   four tests was kept.
5. **`spawn_session` returns a tuple**, not the `SessionEnds` struct; the helper destructures.
6. **Warning and lint hygiene**: `.then(|| …)` → `.then_some(…)`, `repeat(…).take(n)` → `repeat_n`,
   `FakeSpeech::default()` on a unit struct, trimmed test imports and fakes, and `Calls` keeps its
   `sizes` field because four existing `vad.rs` assertions read it (assertions were never changed).

One spec sentence was deliberately **not** implemented literally and has been retired from the spec:
§4.1's "nothing collected" branch is unreachable — an utterance exists only because the detector
confirmed an onset, and the frame that opened it was collected — so no code writes it. A guard for an
impossible state, with no test that can reach it, is a lie about the code's shape.

### Task 4: The screen's state `[small]`

**Files:**
- Modify: `crates/tipsy-ui/src/state.rs` — `UiAction` gains `LoginDone` (:83-98); `LOGIN_INPUT_MAX`,
  `LOGIN_AUTO_REARM`, `LoginPhase`, `LoginLine` and `LoginState` go above `UiState` (:100); `UiState`
  gains `pub login` (:125) and `new()` its `login: None` (:158); `show_login` beside `show_config_form`
  (:187); `login_key` beside `config_key` (:242); the early return in `on_key` (:376-385); the two
  placeholder arms in `apply` (:358-362) and the login branch of the `Error` arm (:363); the tests
  (:518 onwards)
- Test: the same file's `#[cfg(test)] mod tests`, whose helpers `type_text` (:536) and `UiState::new`
  this task's tests use

**Interfaces:**
- Consumes:
  - `pub enum UiAction { Send(Command), Quit, Replay(usize), OpenConfig, Config(ConfigAction) }` — `state.rs:83-98`
  - `pub fn apply(&mut self, ev: &Event)` (:274) and `pub fn on_key(&mut self, key: KeyInput) -> Vec<UiAction>` (:376), whose first branch is the configuration screen's (`:377-385`) — the login screen is *outside* that one
  - `Event::LoginHeard { text }` and `Event::LoginVerdict { ok, name }`, and `Event::Error { code, message, recoverable }` — Task 1, `tipsy-protocol/src/lib.rs`
  - `pub fn show_config_form(&mut self, form: ConfigForm)` (:187), `fn toggle_config_form` (:171), `pub fn notify` (:166), `pub fn strings(&self) -> &'static Strings` (:162), `pub fn on_hit(&mut self, hit: &Hit)` (:202) — which already routes a hit with no `action` through `on_key`, so the widget's two controls need nothing here
  - `type_text(s: &mut UiState, text: &str)` and `UiState::new()` — `state.rs:536`, `:135`
- Produces:

```rust
// crates/tipsy-ui/src/state.rs

/// How many characters the login's box takes. A login answer is a sentence, and this is what keeps a
/// pasted novel out of the transcription path (spec §6.2).
pub const LOGIN_INPUT_MAX: usize = 200;

/// How many refusals in a row the screen keeps listening through before it waits for a key (spec L8).
pub const LOGIN_AUTO_REARM: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginPhase {
    /// No capture: the microphone is off or has not been opened yet.
    Waiting,
    /// The capture is open and nothing is in flight.
    Listening,
    /// An answer is with the server: typed or spoken, and a verdict is coming.
    Judging,
    Refused,
    Passed,
}

/// Which sentence the widget's one line shows (spec §6.1), in this precedence: `Passed`, `Refused`,
/// `Heard`, a failure, and the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginLine {
    Prompt,
    Heard(String),
    Refused,
    /// `NothingHeard`, whose wording is the UI's: the server's message for it is English.
    NothingHeard,
    /// Any other `Event::Error`: the provider's own words.
    Failed(String),
    Passed,
}

/// The login screen's own state (spec §6). `open` is the capture **the client reconciles the
/// microphone to**: the reducer cannot return an action from `apply`, so a refusal that stops listening
/// is expressed as a state the client reads, never as a command nobody could send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginState {
    pub input: String,
    /// The `Welcome.login.voice` switch: with it off there is no capture and `Tab` does nothing.
    pub voice: bool,
    pub open: bool,
    pub phase: LoginPhase,
    pub line: LoginLine,
    /// The matched entry's name, once the door opens.
    pub name: Option<String>,
    /// Refusals in a row: the streak `LOGIN_AUTO_REARM` guards.
    pub refusals: u32,
}

impl LoginState {
    /// True once the streak has stopped the automatic re-arm: the border swaps to the escape hint, and
    /// the microphone stays shut until `Tab` — which is why a locked-out owner is never stuck (spec §6).
    pub fn needs_a_key(&self) -> bool;
}

// in UiState:
    /// The login screen while it is up: `None` is a client that has been let in, or one the server
    /// never asked (spec §3 — `Welcome.login` is the only authority).
    pub login: Option<LoginState>,

    /// Shows the login screen, the client's answer to `Welcome.login`.
    pub fn show_login(&mut self, voice: bool);

// in UiAction, after Config:
    /// The door opened and the user pressed `Enter`: the client closes the screen and enters the HUD.
    LoginDone,
```

- [ ] **Step 1: Write the failing tests**

Append to `crates/tipsy-ui/src/state.rs`'s test module, after the last test in it:

```rust
    // ---- the login screen (spec 2026-10-11-login-screen-design.md, §6) ----

    fn login_state(voice: bool) -> UiState {
        let mut s = UiState::new();
        s.show_login(voice);
        s
    }

    fn refused(s: &mut UiState) {
        s.apply(&Event::LoginVerdict { ok: false, name: None });
    }

    /// The screen is the outermost thing on it: while the door is shut there is no second screen, and
    /// `esc` is still the way out (spec §6.2).
    #[test]
    fn the_login_screen_is_asked_first_and_f9_does_nothing() {
        let mut s = login_state(true);
        assert!(s.on_key(KeyInput::F(9)).is_empty(), "F9 is not a door out of the login");
        assert!(s.config_form.is_none(), "and it does not open the configuration screen");
        assert_eq!(s.on_key(KeyInput::Esc), vec![UiAction::Quit]);
    }

    /// The screen has its own buffer, so the HUD's box behind the veil stays empty, and 200 characters
    /// is where it stops (spec §6.2).
    #[test]
    fn typing_goes_to_the_screen_and_is_capped() {
        let mut s = login_state(true);
        type_text(&mut s, "andres");
        assert_eq!(s.login.as_ref().unwrap().input, "andres");
        assert!(s.input.is_empty(), "the HUD's own box is not the screen's");
        type_text(&mut s, &"x".repeat(250));
        assert_eq!(s.login.as_ref().unwrap().input.chars().count(), LOGIN_INPUT_MAX);
        let _ = s.on_key(KeyInput::Backspace);
        assert_eq!(s.login.as_ref().unwrap().input.chars().count(), LOGIN_INPUT_MAX - 1);
    }

    /// A typed answer is a `LoginAnswer` the server judges; the box empties and the screen says it is
    /// waiting (spec §6.2).
    #[test]
    fn enter_sends_the_typed_answer() {
        let mut s = login_state(true);
        type_text(&mut s, "  yo soy andres del core  ");
        assert_eq!(
            s.on_key(KeyInput::Enter),
            vec![UiAction::Send(Command::LoginAnswer { text: "yo soy andres del core".into() })]
        );
        let login = s.login.as_ref().unwrap();
        assert_eq!(login.input, "", "the box empties");
        assert_eq!(login.phase, LoginPhase::Judging);
    }

    /// A refusal is a line and a counter, and the microphone stays open for the next thing said — that
    /// is the automatic re-arm; the third refusal in a row is what stops it (spec §6.2, L8).
    #[test]
    fn the_third_refusal_in_a_row_closes_the_microphone() {
        let mut s = login_state(true);
        let _ = s.on_key(KeyInput::Tab);
        assert!(s.login.as_ref().unwrap().open);
        refused(&mut s);
        assert_eq!(s.login.as_ref().unwrap().line, LoginLine::Refused);
        assert!(s.login.as_ref().unwrap().open, "attempt 1 leaves the capture open");
        refused(&mut s);
        assert!(s.login.as_ref().unwrap().open, "and so does attempt 2");
        refused(&mut s);
        let login = s.login.as_ref().unwrap();
        assert_eq!(login.refusals, 3);
        assert!(!login.open, "the third stops the automatic re-arm");
        assert!(login.needs_a_key());
    }

    /// `Tab` is the microphone: on, off, and on again. An explicit re-arm is consent, so it resets the
    /// streak — the guard's whole job is to stop *automatic* listening (spec §6.2).
    #[test]
    fn tab_toggles_the_capture_and_an_explicit_re_arm_resets_the_streak() {
        let mut s = login_state(true);
        assert_eq!(s.on_key(KeyInput::Tab), vec![UiAction::Send(Command::LoginStart)]);
        assert!(s.login.as_ref().unwrap().open);
        assert_eq!(s.on_key(KeyInput::Tab), vec![UiAction::Send(Command::LoginStop)]);
        assert!(!s.login.as_ref().unwrap().open);
        let _ = s.on_key(KeyInput::Tab);
        for _ in 0..3 {
            refused(&mut s);
        }
        assert!(s.login.as_ref().unwrap().needs_a_key());
        assert_eq!(s.on_key(KeyInput::Tab), vec![UiAction::Send(Command::LoginStart)]);
        assert_eq!(s.login.as_ref().unwrap().refusals, 0, "an explicit re-arm starts the count over");
    }

    /// With the voice switch off there is no capture to toggle and no hint to draw (spec §6).
    #[test]
    fn tab_does_nothing_when_the_voice_is_off() {
        let mut s = login_state(false);
        assert!(s.on_key(KeyInput::Tab).is_empty());
        assert!(!s.login.as_ref().unwrap().open);
    }

    /// The greeting stays until `Enter`, which is the client's cue to close the screen and enter the
    /// HUD (spec §6.2).
    #[test]
    fn a_passed_screen_waits_for_enter_and_then_finishes() {
        let mut s = login_state(true);
        s.apply(&Event::LoginHeard { text: "yo soy Andrés del core".into() });
        assert_eq!(
            s.login.as_ref().unwrap().line,
            LoginLine::Heard("yo soy Andrés del core".into()),
            "a misheard accent is visible rather than mysterious"
        );
        s.apply(&Event::LoginVerdict { ok: true, name: Some("Andrés".into()) });
        let login = s.login.as_ref().unwrap();
        assert_eq!(login.phase, LoginPhase::Passed);
        assert_eq!(login.line, LoginLine::Passed);
        assert_eq!(login.name.as_deref(), Some("Andrés"));
        assert_eq!(s.on_key(KeyInput::Enter), vec![UiAction::LoginDone]);
    }

    /// "Nothing heard" is the UI's wording, not the server's, and it counts no attempt: a cough is not
    /// a wrong answer (spec §4.1, §6.2).
    #[test]
    fn nothing_heard_counts_no_attempt_and_leaves_the_capture_open() {
        let mut s = login_state(true);
        let _ = s.on_key(KeyInput::Tab);
        s.apply(&Event::Error {
            code: ErrorCode::NothingHeard,
            message: "nothing was heard".into(),
            recoverable: true,
        });
        let login = s.login.as_ref().unwrap();
        assert_eq!(login.line, LoginLine::NothingHeard);
        assert_eq!(login.refusals, 0);
        assert!(login.open, "the capture keeps listening");
    }

    /// Any other failure turns the microphone off and shows the provider's own words: retrying the
    /// voice would only repeat it, and typing still works (spec §6.2).
    #[test]
    fn another_error_turns_the_microphone_off_and_shows_its_message() {
        let mut s = login_state(true);
        let _ = s.on_key(KeyInput::Tab);
        s.apply(&Event::Error {
            code: ErrorCode::NoKey,
            message: "no key named `openrouter`".into(),
            recoverable: true,
        });
        let login = s.login.as_ref().unwrap();
        assert_eq!(login.line, LoginLine::Failed("no key named `openrouter`".into()));
        assert!(!login.open);
        assert!(s.log.is_empty(), "a login failure is not a conversation entry");
    }

    /// An empty `Enter` after a refusal re-arms the capture, and does nothing before one: the prompt is
    /// not a button (spec §6.2).
    #[test]
    fn an_empty_enter_re_arms_only_after_a_refusal() {
        let mut s = login_state(true);
        assert!(s.on_key(KeyInput::Enter).is_empty(), "nothing to say, nothing to re-arm");
        let _ = s.on_key(KeyInput::Tab);
        refused(&mut s);
        let _ = s.on_key(KeyInput::Tab);
        assert!(!s.login.as_ref().unwrap().open, "the re-arm has to be the thing that opens it");
        assert_eq!(s.on_key(KeyInput::Enter), vec![UiAction::Send(Command::LoginStart)]);
        assert!(s.login.as_ref().unwrap().open);
        assert_eq!(s.login.as_ref().unwrap().refusals, 0);
    }

    /// With no screen up, a login event is data about nothing and must not touch the HUD (spec §3: the
    /// client shows the screen once per run, from `Welcome`).
    #[test]
    fn login_events_without_a_screen_are_ignored() {
        let mut s = UiState::new();
        s.apply(&Event::LoginHeard { text: "andres".into() });
        s.apply(&Event::LoginVerdict { ok: true, name: Some("Andrés".into()) });
        assert!(s.login.is_none());
        assert!(s.log.is_empty());
        assert!(s.notice.is_none());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -j 1 -p tipsy-ui login`
Expected: FAIL to compile — `no method show_login`, `no field login`, `cannot find type LoginPhase`,
`no variant LoginDone`.

- [ ] **Step 3: Implement the state**

Add the constants, the two enums, `LoginState` and `show_login` exactly as the Interfaces block writes
them, `pub login` to `UiState` and `login: None` to `UiState::new`, `LoginDone` to `UiAction`, and:

```rust
    /// The login screen's keys (spec §6.2). It is the outermost screen, so it is asked before the
    /// configuration screen's: `F9` and the gear do nothing while the door is shut.
    ///
    /// `Tab` is the microphone because every letter is also a possible answer, and `Enter` is the only
    /// other key: no bare-letter shortcut can live here.
    fn login_key(&mut self, key: KeyInput) -> Vec<UiAction> {
        let Some(login) = self.login.as_mut() else {
            return Vec::new();
        };
        match key {
            KeyInput::Char(c) if !c.is_control() && login.input.chars().count() < LOGIN_INPUT_MAX => {
                login.input.push(c);
                Vec::new()
            }
            KeyInput::Char(_) => Vec::new(),
            KeyInput::Backspace => {
                login.input.pop();
                Vec::new()
            }
            KeyInput::Enter => {
                let text = login.input.trim().to_string();
                match (text.is_empty(), login.phase) {
                    (_, LoginPhase::Passed) => vec![UiAction::LoginDone],
                    (false, _) => {
                        login.input.clear();
                        login.phase = LoginPhase::Judging;
                        vec![UiAction::Send(Command::LoginAnswer { text })]
                    }
                    // An empty box after a refusal is the same re-arm `Tab` is.
                    (true, LoginPhase::Refused) => {
                        login.phase = LoginPhase::Listening;
                        login.open = true;
                        login.refusals = 0;
                        login.line = LoginLine::Prompt;
                        vec![UiAction::Send(Command::LoginStart)]
                    }
                    (true, _) => Vec::new(),
                }
            }
            KeyInput::Tab if login.voice => {
                let was_open = login.open;
                login.open = !was_open;
                login.phase = if was_open { LoginPhase::Waiting } else { LoginPhase::Listening };
                login.line = LoginLine::Prompt;
                if !was_open {
                    // An explicit key is consent, and the guard's job is to stop *automatic* listening.
                    login.refusals = 0;
                }
                vec![UiAction::Send(if was_open { Command::LoginStop } else { Command::LoginStart })]
            }
            KeyInput::Esc => vec![UiAction::Quit],
            _ => Vec::new(),
        }
    }
```

`on_key` gains its first branch, above the configuration screen's:

```rust
        if self.login.is_some() {
            // The door is the outermost screen: nothing behind it answers a key while it is up.
            return self.login_key(key);
        }
```

and `apply` replaces its two placeholder arms with the real ones and puts the login branch of the
`Error` arm first:

```rust
            Event::LoginHeard { text } => {
                if let Some(login) = self.login.as_mut() {
                    login.line = LoginLine::Heard(text.clone());
                }
            }
            Event::LoginVerdict { ok, name } => {
                if let Some(login) = self.login.as_mut() {
                    if *ok {
                        login.phase = LoginPhase::Passed;
                        login.line = LoginLine::Passed;
                        login.name = name.clone();
                    } else {
                        login.phase = LoginPhase::Refused;
                        login.line = LoginLine::Refused;
                        login.refusals += 1;
                        if login.needs_a_key() {
                            login.open = false;
                        }
                    }
                }
            }
            Event::Error { code, message, .. } => {
                // While the door is shut, a failure belongs to the screen and not to the log: the
                // wording of "nothing heard" is the UI's, and any other failure stops the voice.
                if let Some(login) = self.login.as_mut() {
                    if *code == ErrorCode::NothingHeard {
                        login.line = LoginLine::NothingHeard;
                    } else {
                        login.line = LoginLine::Failed(message.clone());
                        login.open = false;
                        login.phase = LoginPhase::Waiting;
                    }
                    return;
                }
                // … the existing arm, unchanged …
            }
```

`ErrorCode` joins the `tipsy_protocol` import list at the top of the file.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -j 1 -p tipsy-ui login` and then `cargo test -j 1 -p tipsy-ui`
Expected: PASS — the twelve new tests and the crate's existing 70 + 14 untouched.

- [ ] **Step 5: The crate's suite, the whole workspace, and the lint gate**

Run: `cargo test -j 1 --workspace` and `cargo clippy --workspace --all-targets`
Expected: 336 + 12 tests, 0 failed, and clippy with exactly the seven baseline findings and nothing new
(Global Constraints, "Lint and format gates"). No `cargo fmt`.

- [ ] **Step 6: Commit**

```bash
git add crates/tipsy-ui/src/state.rs
git commit -m "feat(ui): hold the login screen's state

The widget's own state machine: its buffer, its phase, the one line it draws and
the refusal streak. open is the capture the client reconciles the microphone to,
because a reducer cannot return a command: a refusal that stops listening is a
state, not an action nobody could send. LoginDone is the only new action; every
other control is a command the existing Send carries, and Tab and Enter are read
through the same key path a click already uses."
```

---

## Coordinator notes

**The plan's state.** Tasks 1–4 are above and complete, and each carries an "Amendments after execution"
block recording every place where running it proved the plan wrong. **Tasks 5 and 6 are not in this
document**: they were briefed in full before they started, with spec §6, §7 and §8 as their contract. An
earlier draft of these notes said the plan was "written in chunks" and that it was not executable until
the remaining tasks were appended; that text was left behind by a later edit and is no longer true —
every task that ran had its brief first, and where the plan and the spec disagreed the spec won and the
divergence was recorded against the task that caused it.

**Where it stands** — `coordinator-rulings.md` beside this plan in its SDD workspace is the record,
because `progress.md` was rewritten by another session and no longer matches the repository:

- `768e35e` — Task 1, since landed on `master` as `a03f3b6`: the 2C commits had captured this change's
  fallout arms, so master did not compile without its definitions.
- `03250b9` Task 2 · `eccfa86` Task 3 · `a569cb2` the greeting addendum · `8742367` Task 4 ·
  `494de78` the spec and this plan.
- The branch is rebased on `master`, and it was green at **391 tests** when Task 4 landed.

**Standing rulings** (CLAUDE.md, and Plans 1, 2A and 2B both ran this way):

- **One implementer subagent per task, and no per-task reviews.** A single whole-branch review when the
  plan is complete, then one fix wave and one scoped re-review of it.
- A plan's `[tier]` tags are hints about a task's size, not model selections: this harness cannot pick
  a model per subagent.
- Each task's brief goes in the plan's own SDD workspace at
  `.superpowers/sdd/2026-10-11-plan-login-screen/`, beside `global-constraints.md` (this plan's section,
  verbatim), `coordinator-rulings.md` (the coordinator's own record) and the task reports.
- The tests run with `-j 1`, the worktree's own target directory and `TEMP` inside the worktree: a
  `cargo test` that dies with `STATUS_STACK_BUFFER_OVERRUN` at default parallelism is the machine, not
  the code, and a `tempfile` panic naming `G:\dev\tmp` is the shell's broken temporary directory, not a
  test failure.
- The lint gate is not `-D warnings`: this baseline carries seven pre-existing clippy findings and is not
  rustfmt-clean, so `cargo fmt` is never run.
