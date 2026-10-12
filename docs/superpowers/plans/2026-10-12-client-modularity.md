# Client Modularity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **This plan is a handover.** It was written by the session that wrote
> `docs/superpowers/specs/2026-10-12-client-modularity-design.md`, which does **not** execute it. The
> session that owns Plan 3 executes it and schedules it: read "How to schedule this" below, then take
> the tasks in the order it leaves you.

**Goal:** Split `client.rs` and `session.rs` by reason to change, and move the client's decision policy
into `tipsy_ui::policy`, so Plan 3's browser client imports one policy instead of cloning one — with
zero behaviour change.

**Architecture:** Two moves of different kinds. The mechanical one splits a file into a parent and
children **of the same struct** (`client.rs` stays the parent module and gains `client/{mic,audio,call,settings}.rs`;
`session.rs` becomes `session/{turn,login,config}.rs`), with `pub(super)` on what crosses the new
boundary and nothing widened to `pub`. The load-bearing one lifts the pure decision functions out of a
binary crate and into `tipsy-ui`, which is the crate both clients already compile against and the crate
that must keep compiling to `wasm32-unknown-unknown`. Tests stay inside the file they test and travel
with the code they test.

**Tech Stack:** unchanged. No crate is added, no dependency is added, `PROTOCOL_VERSION` stays `1`.
Rust `edition = "2024"`, pinned `1.96.1`.

**Spec:** [`docs/superpowers/specs/2026-10-12-client-modularity-design.md`](../specs/2026-10-12-client-modularity-design.md)
— read it first. Its §3 names the cut, §4 the membership rule, §5 the coordination, §6 the gates.

## How to schedule this

**Fixed, and not yours to reorder.**

1. **Phase 1 merges first.** This plan's branch is cut from `master` afterwards, in its own
   `git worktree`. Never in the shared checkout: another session owns the files this plan moves.
2. **Task 1 before Tasks 2–5.** Tasks 2–5 stop importing functions that Task 1 is what creates a home
   for. The tree does not compile in the middle of that.
3. **This plan lands before Phase 2 starts.** Phase 2's and Phase 3's bodies cite `file:line` in the
   four files this plan moves, and Task 3.4/3.5 of that plan are rewritten by Task 7 here.
4. **One implementer per task, no per-task reviews.** One whole-branch review when the plan is
   complete, then one fix wave and one scoped re-review of it.

**Yours to decide.** None of these changes a task body.

- Whether **Task 6** is in scope at all. It is droppable: nothing outside the server needs it, and no
  other task depends on it.
- The **order of Tasks 2–5**. Each leaves the tree green on its own; take them in the order that suits
  your merge plans.
- Whether **Task 7** rides with the last code task or lands as its own commit.
- How you interleave this with a fix wave of your own. The only rules are precondition 1 and rule 3.

## Preconditions — do not start Task 1 until all four hold

- [ ] Phase 1 is merged into `master` and `master` is green. (505 tests at `2b328f3`, 0 failures, before
      its own late fix wave; **re-measure, do not trust the number**.)
- [ ] You are in a fresh `git worktree` on its own branch, cut from `master`.
- [ ] `git status --short` in that worktree is clean.
- [ ] The baseline capture of Task 1 Step 1 exists, and the counts are written down.

## Global Constraints

Every task inherits these. They are the spec's §6 and `global-constraints.md`, restated here so this
document can be read alone.

- **English** for code, comments, docs and commit messages. No new user-facing string is added by this
  plan; the two tables in `tipsy-ui/src/strings.rs` are untouched.
- Every commit message ends with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.
- Commits name **only the files the task lists**. This repository has been burned by a `git stash` that
  reverted eleven files of another plan's work; do not run `git stash`, `git restore`,
  `git checkout -- .` or `git reset` anywhere.
- The canonical command shape, from `global-constraints.md`:

  ```powershell
  $env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP
  $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
  cargo test -j 1 -p <crate> <filter>
  ```

  `-j 1` is not optional: at cargo's default parallelism this machine dies with
  `STATUS_STACK_BUFFER_OVERRUN`, which looks exactly like a code fault and is not one.
- **Never run `cargo fmt`.** The repository is not rustfmt-clean (36 files) and formatting rewrites
  hundreds of unrelated lines.
- **Clippy's gate is the finding set, not the exit code.** `cargo clippy --workspace --all-targets`
  reports **seven** pre-existing findings and exits `0`; compare the set with
  `global-constraints.md:32-54` and add nothing new. `session.rs`'s citation moves in this plan, and the
  file records that it has moved at every task.
- **`tipsy-ui` never gains `tokio`, `cpal`, `rodio`, `keyring`, `sysinfo`, `reqwest` or `crossterm`**,
  and `tipsy-ui` and `tipsy-protocol` keep compiling to `wasm32-unknown-unknown`:
  `cargo check -p tipsy-ui -p tipsy-protocol --target wasm32-unknown-unknown`.
- **Unit tests stay inside the file they test** (`#[cfg(test)] mod tests`, `use super::*`). No test file
  is created, and no `#[path]` attribute is introduced.
- **No test is deleted, ignored or renamed.** No `#[ignore]`, no assertion relaxed. A test that cannot
  be made to pass in its new home is a finding for the whole-branch review, not a deletion.
- **`PROTOCOL_VERSION` stays `1`.** This plan changes no wire shape.
- **Terminal first.** The native terminal client is the reference implementation: nothing here may
  change what the terminal does, and the browser is the consumer that gains.

## Review Focus

Five failure modes this plan implies and no single task's happy path exercises. Each is pinned below.

1. **A test lost in the move.** 1119 lines of tests move across six files. Pinned by the identity gate
   in every task (Task 1 Step 8 and the same step in Tasks 2–6).
2. **A visibility widened further than it needs.** `pub(super)` from a child means "visible in the
   parent and its descendants"; `pub` means "visible to the crate and the docs". Pinned by Task 2
   Step 4, and checked again in the branch review with a grep for `pub fn` in the new children.
3. **A behaviour change smuggled into a move.** The one place it can happen is `on_message`, whose order
   — the announced rate first, then the audio — is load-bearing. Pinned by the new `speech_rate` test in
   Task 1 Step 5 and by Task 3's unchanged-body rule.
4. **`crossterm` leaking into `tipsy-ui`.** The tempting move is `key_input`, which takes a crossterm
   `KeyEvent`. Pinned by Task 1 Step 6 (the wasm gate) and by Task 5, which is where the temptation
   arrives.
5. **The other session's work in the shared checkout.** Pinned by the preconditions and by the rule that
   the branch lives in its own worktree.

---

## File structure

| Path | Responsibility after this plan |
|---|---|
| `crates/tipsy-ui/src/policy.rs` | **New.** The decisions a client makes with no device, socket or clock in reach. ~110 production lines. |
| `crates/tipsy-ui/src/lib.rs` | `pub mod policy;` beside the existing modules. |
| `crates/tipsy-compadre/src/client.rs` | The parent: `Client` and its fields, the bootstrap, the frame loop, `on_message`, `draw_frame`, and the two helpers that cannot leave a terminal (`key_input`, `clock_view`). |
| `crates/tipsy-compadre/src/client/mic.rs` | **New.** Capture, the login capture, and the one routing decision's application. |
| `crates/tipsy-compadre/src/client/audio.rs` | **New.** Playback, the flush rule's application, and the reply kept for `Replay`. |
| `crates/tipsy-compadre/src/client/call.rs` | **New.** The live realtime call's lifecycle. |
| `crates/tipsy-compadre/src/client/settings.rs` | **New.** The configuration screen's actions: open, save, test, token, notices. |
| `crates/tipsy-compadre/src/session.rs` | Task 6 only, droppable. Parent of `session/{turn,login,config}.rs`. |
| `docs/architecture.md`, `CLAUDE.md`, the web-tier plan, `global-constraints.md` | Task 7: the record says what landed. |

---

## Task 1: `tipsy_ui::policy` — the client's decisions, where both clients can reach them

**Why first.** Everything else in this plan is a move inside one crate. This one crosses a crate
boundary, it is the reason the plan exists (Plan 3's Phase 3 clones these functions), and Tasks 2–5
remove the last callers of the functions it takes away.

**Files:**
- Create: `crates/tipsy-ui/src/policy.rs`
- Modify: `crates/tipsy-ui/src/lib.rs` (add one line)
- Modify: `crates/tipsy-compadre/src/client.rs` (delete ten functions and `MicRoute`, add one `use`)
- Test: the `#[cfg(test)] mod tests` of `crates/tipsy-ui/src/policy.rs` (new), and the existing test
  module of `client.rs`

**Interfaces:**
- Consumes: `tipsy_protocol::{Capability, Command, Event}`; `crate::Hit` (already exported by
  `tipsy-ui`, from `render.rs`).
- Produces, for Tasks 2–5 and for Plan 3's Tasks 3.3–3.5:

  ```rust
  pub fn starts_turn(command: &Command) -> bool
  pub fn interrupts_playback(command: &Command) -> bool
  pub fn test_interrupts_playback(capability: Capability) -> bool
  pub fn flush_on_interrupt(interrupting: bool, playing: bool) -> bool
  pub fn takes_the_floor(command: &Command) -> bool
  pub fn mic_wanted(mic_open: bool, live: bool, login: bool) -> bool
  pub fn mic_route(device: bool, login: bool, live: bool) -> MicRoute
  pub fn speech_rate(event: &Event) -> Option<u32>
  pub fn hit_at(hits: &[Hit], column: u16, row: u16) -> Option<Hit>

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum MicRoute { Session, Call, Drop }
  ```

- [ ] **Step 1: Capture the baseline before touching anything**

The gate every remaining task repeats. Write down both numbers and keep the file.

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP
$env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
New-Item -ItemType Directory -Force "$PWD\.target-sandbox" | Out-Null
cargo test -j 1 --workspace -- --list 2>&1 |
  Select-String -Pattern ': test$' |
  ForEach-Object { ($_.Line -split ': test$')[0].Split('::')[-1] } |
  Sort-Object | Set-Content "$PWD\.target-sandbox\baseline-tests.txt"
"baseline tests: " + (Get-Content "$PWD\.target-sandbox\baseline-tests.txt").Count
```

The file goes under `.target-sandbox/` because that directory is git-ignored: a baseline written to the
worktree root would show up in `git status` for the rest of the plan. Each step below spells the path
out rather than using a variable, because a fresh shell per command does not carry one.

Expected: a count. At `2b328f3` the workspace reported **505** tests, 0 failures, and the branch has
moved since; the number you measure is the one that matters, not this one.

- [ ] **Step 2: Write the new module**

Create `crates/tipsy-ui/src/policy.rs`. Every function body below is the body that exists today in
`crates/tipsy-compadre/src/client.rs`, character for character apart from the `pub` keyword and the
doc link syntax. Do not reword a comment, do not reorder an arm, and do not "tidy" a match: a
behaviour change disguised as a move is what Review Focus 3 is about.

```rust
//! The decisions a tipsy-compadre client makes, with no device, socket or clock in reach.
//!
//! Every function here was a private free function of `tipsy-compadre`'s `client.rs`. It lives in
//! `tipsy-ui` because that crate is the one both clients compile against — the native terminal client
//! and Plan 3's browser client — and the one that must keep compiling to `wasm32-unknown-unknown`.
//! That second fact is also the membership rule: nothing here may reach for a device, a socket, the
//! file system, the operating system's clock or `crossterm`. See `client.rs`'s `key_input` (a crossterm
//! `KeyEvent`) and `clock_view` (this process's clock) for the two functions the rule keeps out.

use tipsy_protocol::{Capability, Command, Event};

use crate::Hit;

/// Whether a command starts the next turn, which is what makes the audio kept for `Replay` stale.
///
/// The session cancels the running turn and starts another one for `Ask`, `Say` and `ListenStop`
/// (`session.rs:283-287`); `Reset` starts the conversation over (`session.rs:272-281`).
pub fn starts_turn(command: &Command) -> bool {
    match command {
        // The session ignores an empty question and a bare `!` (`session.rs:233-238`, `:242-249`),
        // so those begin nothing and must not silence a reply in progress.
        Command::Ask { text } | Command::Say { text } => !text.trim().is_empty(),
        Command::ListenStop | Command::Reset => true,
        // A Test is the one command the native client answers itself, in `test_provider`: it makes a
        // real call to a provider, but it starts no turn, speaks nothing of its own and writes
        // nothing to the log. This arm is therefore load-bearing for the screen as well —
        // `test_interrupts_playback` reads the decision from it — and may not be flipped to `true`
        // without also deciding that a one-word probe should cut a sentence in half.
        Command::Hello { .. }
        | Command::TestProvider { .. }
        | Command::ListenStart
        | Command::Cancel
        | Command::SetProfile { .. }
        | Command::SetListeningMode { .. } => false,
        // The login screen's three commands belong to the login capture, never to the conversation:
        // they start no turn, and the client routes them to the screen rather than through this
        // decision.
        Command::LoginStart | Command::LoginStop | Command::LoginAnswer { .. } => false,
        // Reading, writing and keying the configuration start no turn, speak nothing and write
        // nothing to the log.
        Command::GetConfig | Command::UpdateConfig { .. } | Command::SetKey { .. } => false,
    }
}

/// Whether the queued audio has to go because the turn it belongs to is over.
///
/// Everything that starts a turn leaves nothing of the previous one to hear; `Cancel`
/// (`session.rs:266-270`) is the one command that ends a turn without starting another.
pub fn interrupts_playback(command: &Command) -> bool {
    starts_turn(command) || matches!(command, Command::Cancel)
}

/// Whether the screen's Test quiets the audio the device is still speaking.
///
/// It does not, and the answer comes from the same rule [`starts_turn`] applies to commands: the
/// probe is a diagnostic — it says nothing out loud, plays nothing (the speech probe counts the
/// samples and drops them) and adds nothing to the log — so a reply in progress keeps playing. This
/// function exists so the screen's path, which never passes through `UiAction::Send`, takes that
/// decision deliberately instead of inheriting it from an arm that only ever existed to make a match
/// exhaustive.
pub fn test_interrupts_playback(capability: Capability) -> bool {
    starts_turn(&Command::TestProvider { capability })
}

/// Whether the audio the device has queued must be dropped right now.
///
/// The two inputs are the whole rule: the user is interrupting (`interrupting`) and there is
/// something left to drop (`playing`). Nothing else flushes. A status change in particular is never
/// a trigger: a reply that finishes normally crosses `Speaking` → `Idle` (`brain.rs:110-115`,
/// `:153-158`) while the device is still speaking it out, so flushing there would cut every reply
/// off mid-sentence — and, because rodio's `clear` pauses the sink, leave the rest of the session
/// silent.
pub fn flush_on_interrupt(interrupting: bool, playing: bool) -> bool {
    interrupting && playing
}

/// Whether a command means the user is taking the floor while a call is live.
///
/// In push mode these verbs are the microphone key and the cancel; in a call the microphone is
/// already open for the whole call, so both directions of that key mean the same thing — "stop
/// talking, I am talking" — and they are answered locally with `response.cancel` rather than sent to
/// a session that is not driving the conversation. The server's own VAD is what reports a barge-in
/// and what knows the number.
pub fn takes_the_floor(command: &Command) -> bool {
    matches!(
        command,
        Command::Ask { .. }
            | Command::Say { .. }
            | Command::ListenStart
            | Command::ListenStop
            | Command::Cancel
    )
}

/// Whether the microphone stream should be open: the user asked for it — with the microphone key, or
/// because the login capture said so — or a call is live and streams it for as long as it lasts.
///
/// The login capture is its own reason and never a reason to write the switch: with the door shut
/// there is exactly one microphone that matters, and it is the screen's, not the HUD's (login spec
/// §6.2, review finding 3).
pub fn mic_wanted(mic_open: bool, live: bool, login: bool) -> bool {
    mic_open || live || login
}

/// Where one captured frame goes, or nowhere at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicRoute {
    /// To the session as a `Frame::Mic`: the listening path, and the login's own detector on the
    /// server.
    Session,
    /// To the live call, which owns the microphone while it lasts.
    Call,
    /// Nowhere: no capture is running, so there is nothing to send and nothing to keep.
    Drop,
}

/// The one routing decision the capture loop makes, as a function of what is open.
///
/// `device` is `Mic::is_some()` and it comes first on purpose: a frame belongs to a *stream*, and
/// "the screen wants a capture" is not the same fact (login spec §9: a denied microphone means no
/// stream, no `LoginStart`). The login beats a live call because it is the only thing on screen and
/// its answer is not the call's.
pub fn mic_route(device: bool, login: bool, live: bool) -> MicRoute {
    match (device, login, live) {
        (false, _, _) => MicRoute::Drop,
        (true, true, _) => MicRoute::Session,
        (true, false, true) => MicRoute::Call,
        (true, false, false) => MicRoute::Session,
    }
}

/// The sample rate an event announces, when it announces one.
///
/// **New in this plan, and the only new behaviour in it.** `on_message` assigns the announced rate to
/// the client's rate *before* it hands the audio to the device, and Plan 3's browser client has to do
/// the same thing in the same order. This function is that rule as a value, so neither client can get
/// the order wrong alone.
pub fn speech_rate(event: &Event) -> Option<u32> {
    match event {
        Event::SpeechChunk { sample_rate, .. } => Some(*sample_rate),
        _ => None,
    }
}

/// The hit whose clickable title contains the click, if any.
///
/// The rectangles are half-open: the cell at `area.right()` and the one at `area.bottom()` are
/// outside. The native client's mouse path and Plan 3's browser path both call this.
pub fn hit_at(hits: &[Hit], column: u16, row: u16) -> Option<Hit> {
    hits.iter().copied().find(|hit| {
        column >= hit.area.x && column < hit.area.right() && row >= hit.area.y && row < hit.area.bottom()
    })
}
```

- [ ] **Step 3: Run the build to see the module does not exist yet where it must**

Run: `cargo check -j 1 -p tipsy-ui`
Expected: FAIL — `error[E0432]`/`E0583` naming `policy`, or an unused-module warning, because
`lib.rs` does not declare it. This is the failing step of a move: nothing compiles until both halves
exist.

- [ ] **Step 4: Declare the module**

In `crates/tipsy-ui/src/lib.rs`, beside the other `pub mod` lines, in alphabetical position:

```rust
pub mod policy;
```

**No glob re-export and no `pub use`.** Callers name the module — `use tipsy_ui::policy::starts_turn;` —
which keeps a future name in `policy` from colliding with `render`'s exports, and keeps the module's
reach visible in every call site.

Run: `cargo check -j 1 -p tipsy-ui`
Expected: PASS for `tipsy-ui` on its own. (`cargo check -p tipsy-compadre` still passes: the old
functions are still there until Step 7.)

- [ ] **Step 5: Write the tests this module owns**

Add to the bottom of `crates/tipsy-ui/src/policy.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_speech_chunk_is_the_only_event_that_announces_a_rate() {
        assert_eq!(speech_rate(&Event::SpeechChunk { seq: 0, sample_rate: 24_000 }), Some(24_000));
        assert_eq!(speech_rate(&Event::Done { cancelled: false }), None);
        assert_eq!(speech_rate(&Event::Status { state: tipsy_protocol::AssistantState::Idle }), None);
    }

    #[test]
    fn only_a_shown_hit_contains_a_click_and_its_edges_are_half_open() {
        use ratatui::layout::Rect;
        let hits = [Hit { area: Rect::new(2, 3, 4, 2), key: crate::KeyInput::F(9), action: None }];
        assert!(hit_at(&hits, 2, 3).is_some(), "the top-left cell is inside");
        assert!(hit_at(&hits, 5, 4).is_some(), "the bottom-right cell is inside");
        assert!(hit_at(&hits, 6, 4).is_none(), "the cell at right() is outside");
        assert!(hit_at(&hits, 5, 5).is_none(), "the cell at bottom() is outside");
        assert!(hit_at(&hits, 1, 3).is_none());
    }

    #[test]
    fn the_capture_needs_a_device_before_anything_else_can_route_it() {
        assert_eq!(mic_route(false, true, true), MicRoute::Drop);
        assert_eq!(mic_route(true, true, true), MicRoute::Session, "the login beats the call");
        assert_eq!(mic_route(true, false, true), MicRoute::Call);
        assert_eq!(mic_route(true, false, false), MicRoute::Session);
    }

    #[test]
    fn a_probe_never_interrupts_and_a_cancel_always_does() {
        assert!(!test_interrupts_playback(Capability::Chat));
        assert!(interrupts_playback(&Command::Cancel));
        assert!(!interrupts_playback(&Command::ListenStart));
        assert!(interrupts_playback(&Command::ListenStop));
    }
}
```

Two things to check against the tree before you trust these four tests: the field names of
`Event::SpeechChunk`, `Event::Done` and `Event::Status`, and whether `Hit`'s fields are exactly
`{ area, key, action }`. If a name differs, fix the test to the tree — the tree wins — and say so in
the commit message. The `hit_at` test is the one that matters: it pins the half-open edges that Plan
3's Task 3.5 is specified to copy.

- [ ] **Step 6: Run the wasm gate, which is the whole point of the module's location**

Run: `cargo check -p tipsy-ui -p tipsy-protocol --target wasm32-unknown-unknown`
Expected: PASS. If it fails, the module reached for something a browser cannot build — most likely
`std::time` or a missing feature on `ratatui`. Fix the module, not the manifest:
`crates/tipsy-ui/Cargo.toml` gains no dependency and no feature in this task.

- [ ] **Step 7: Delete the originals from `client.rs`**

In `crates/tipsy-compadre/src/client.rs`, delete these ten items and nothing else:
`starts_turn`, `interrupts_playback`, `test_interrupts_playback`, `flush_on_interrupt`,
`takes_the_floor`, `mic_wanted`, `mic_route`, `MicRoute`, `hit_at`, and nothing else — `speech_rate`
never existed in this file; the rule it names is inline in `on_message` today, and Task 3 is where that
line changes.

Lines as measured on 2026-10-12 for orientation only — the file was 2296 lines and moving; anchor on
the function names: `flush_on_interrupt` ~998, `call_ended` ~1007 (keep it), `mic_wanted` ~1017,
`MicRoute` ~1022, `mic_route` ~1039, `takes_the_floor` ~1055, `starts_turn` ~1070,
`test_interrupts_playback` ~1105, `interrupts_playback` ~1113, `hit_at` ~1131, `key_input` ~1138
(keep it), `clock_view` ~1165 (keep it).

Then add to the file's imports:

```rust
use tipsy_ui::policy::{MicRoute, hit_at, interrupts_playback, mic_route, mic_wanted, starts_turn, takes_the_floor, test_interrupts_playback};
```

Trim that list to exactly what `client.rs` still uses — the compiler will tell you which names are
unused, and an unused import is a finding this repository's gate does not allow.

Run: `cargo check -j 1 -p tipsy-compadre`
Expected: PASS, or an `unused_imports` warning naming the ones to drop.

- [ ] **Step 8: Move the tests that name only what left, and prove no test was lost**

Read every test in `client.rs`'s test module that calls one of the ten deleted items. The rule:

- A test whose **every** reference is to a moved item **moves** into `policy.rs`'s `mod tests`, with
  its body unmodified and `use super::*` already in place from Step 5.
- A test that builds a `Client`, a `UiState` or a channel **stays** in `client.rs`; it reaches the
  moved functions through the `use` added in Step 7. Do not duplicate it, do not delete it, and do not
  rewrite its assertions.
- Candidates measured on 2026-10-12, to be confirmed against the tree:
  `only_an_interrupt_with_audio_left_to_drop_flushes`, `the_commands_that_end_a_turn_are_the_ones_that_interrupt_it`,
  `the_verbs_that_take_the_floor_are_the_ones_a_call_answers_locally`,
  `the_stream_opens_for_the_switch_the_call_or_the_login_capture`. Some of them build a `Client` and
  therefore stay; that is fine and expected.

Then the gate:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP
$env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
cargo test -j 1 --workspace -- --list 2>&1 |
  Select-String -Pattern ': test$' |
  ForEach-Object { ($_.Line -split ': test$')[0].Split('::')[-1] } |
  Sort-Object | Set-Content "$PWD\.target-sandbox\after-tests.txt"
"before: " + (Get-Content "$PWD\.target-sandbox\baseline-tests.txt").Count +
  " / after: " + (Get-Content "$PWD\.target-sandbox\after-tests.txt").Count
Compare-Object (Get-Content "$PWD\.target-sandbox\baseline-tests.txt") `
               (Get-Content "$PWD\.target-sandbox\after-tests.txt")
```

Later tasks overwrite `after-tests.txt` and compare against the same baseline: the baseline is the tree
**before Task 1**, and it stays the reference until the plan is done.

Expected: the two counts differ by exactly the number of tests Step 5 added (4, unless you had to fix
one to the tree), and `Compare-Object` prints nothing else. **A name that disappears without the count
explaining it stops this task**: find it.

The gate compares test *function* names and not module paths on purpose: a test that moves from
`client::tests::foo` to `policy::tests::foo` is this task working, not a regression.

- [ ] **Step 9: Run both suites and the lint gate**

Run: `cargo test -j 1 -p tipsy-ui -p tipsy-compadre`
Expected: PASS, 0 failures, count = baseline + 4.

Run: `cargo clippy --workspace --all-targets`
Expected: exit `0` with **the same seven findings** `global-constraints.md:39-42` lists, none new. This
task moves no line in `session.rs`, so that file's citation does not move here.

- [ ] **Step 10: Commit**

```bash
git add crates/tipsy-ui/src/policy.rs crates/tipsy-ui/src/lib.rs crates/tipsy-compadre/src/client.rs
git commit -m "refactor(ui): a client's decisions move to tipsy_ui::policy"
```

Body, in the repository's style: why the module exists (both clients compile against `tipsy-ui`), why
it may not reach for `crossterm` or a clock, and the two neighbours it deliberately leaves behind
(`key_input`, `clock_view`).

---

## Task 2: `client/mic.rs` — the capture and the login capture

**Files:**
- Modify: `crates/tipsy-compadre/src/client.rs`
- Create: `crates/tipsy-compadre/src/client/mic.rs`
- Test: the `#[cfg(test)] mod tests` of both, with the capture tests moving to `mic.rs`

**Interfaces:**
- Consumes: `tipsy_ui::policy::{MicRoute, mic_route, mic_wanted}` (Task 1).
- Produces: `impl Client`'s capture block, visible to the parent as `pub(super)`:
  `drain_mic`, `sync_mic`, `login_capture`, `sync_login_mic`, `close_login_mic`.

- [ ] **Step 1: Add the module declaration to the parent**

`client.rs` is the parent module and **is not renamed**. `pub mod bridge;` (Task 1.7 of the web-tier
plan) and, once Task 2.1 of that plan lands, `pub mod pty;` hang off it. Add:

```rust
pub mod mic;
```

and do **not** add a `use` for it: `frame_loop` calls `self.drain_mic(..)`, and inherent methods need
no import.

- [ ] **Step 2: Create the child with the moved methods**

Create `crates/tipsy-compadre/src/client/mic.rs`. It opens with a module doc comment saying what it
owns — the microphone stream, the queue between the device thread and this one, and the login
capture's edges — and then:

```rust
use super::*;
```

Copy these five methods out of `client.rs`, bodies unmodified, and give each the same visibility word:

```rust
impl Client {
    pub(super) fn drain_mic(&mut self, to_server: &UnboundedSender<ToServer>) { .. }
    pub(super) fn sync_mic(&mut self) -> bool { .. }
    pub(super) fn login_capture(&self) -> bool { .. }
    pub(super) fn sync_login_mic(&mut self, to_server: &UnboundedSender<ToServer>) { .. }
    pub(super) fn close_login_mic(&mut self, to_server: &UnboundedSender<ToServer>) { .. }
}
```

**That block is a signature listing, not a stub to fill in.** Each body is the one already in
`client.rs`, copied across character for character. The rule holds for Tasks 3, 4 and 5 too, and it is
the reason this plan is short: **it contains exactly one line of new logic in its whole length** — the
`speech_rate` call of Task 3 Step 2 — and everything else is movement. If you find yourself writing
logic, stop: either you have picked up a task that is not this one, or the plan is wrong and the
whole-branch review needs to hear about it.

**The only edit inside a moved body is the visibility word.** If a body needs a helper that stays in
the parent, call it through `self.` and let the compiler tell you it is private: the fix is
`pub(super)` on that helper, never a copy of it.

- [ ] **Step 3: Move the capture tests**

Move every test in `client.rs`'s test module whose subject is the microphone stream or the login
capture's edges, with its body unmodified. Measured on 2026-10-12, to be confirmed against the tree:

`a_login_capture_with_no_stream_forwards_no_audio_at_all`,
`the_login_capture_is_routed_to_the_session_and_needs_a_device`,
`the_login_screen_leaves_the_huds_microphone_switch_alone`,
`the_voice_switch_off_opens_no_stream_and_sends_nothing`,
`the_capture_opens_and_closes_with_the_login_state`,
`the_door_closes_the_screen_and_the_capture`,
`the_stream_opens_for_the_switch_the_call_or_the_login_capture`,
`a_pass_sends_login_stop_and_ends_the_capture`,
`a_failed_answer_turns_the_capture_off`.

The shared helpers (`mock_client`, `welcome`, `resolver`, `sync_login`, `queue_frames`) **stay in the
parent's `mod tests`** and become `pub(super)`, because a child's tests can see them through
`crate::client::tests::…` but not through `super::super::tests::…`. Import them in the child with:

```rust
use crate::client::tests::{mock_client, sync_login, queue_frames};
```

That import is the one thing in this task that can read badly; if it does, stop and take the fallback
the spec names in §3 — a `client/support.rs` behind `#[cfg(test)] mod support;` — rather than
duplicating a helper. **Do not duplicate a helper**: the web-tier plan's test files duplicate provider
fakes deliberately, but a duplicated `mock_client` is a second definition of the client's own setup.

- [ ] **Step 4: Widening check**

Run: `Select-String -Path crates/tipsy-compadre/src/client/*.rs -Pattern 'pub fn|pub struct|pub enum'`
Expected: **no match** for `client/mic.rs`. Everything this task widens is `pub(super)`. A `pub` here
would export a private method of a binary crate to its own docs and to nothing else, and the branch
review is told to grep for it.

- [ ] **Step 5: The identity gate, the suite, the lint gate**

Repeat Task 1 Step 8's four commands. Expected: the same baseline count as Task 1 left behind (no test
is added or removed by a move), `Compare-Object` prints nothing, the suite is green, and clippy still
reports exactly seven.

- [ ] **Step 6: Commit**

```bash
git add crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/src/client/mic.rs
git commit -m "refactor(client): the microphone and the login capture move to client/mic.rs"
```

---

## Task 3: `client/audio.rs` — playback, the flush, and the reply kept for `Replay`

**Files:**
- Modify: `crates/tipsy-compadre/src/client.rs`
- Create: `crates/tipsy-compadre/src/client/audio.rs`

**Interfaces:**
- Consumes: `tipsy_ui::policy::{flush_on_interrupt, speech_rate}` (Task 1).
- Produces: `impl Client`'s playback block, `pub(super)`: `playing`, `flush_playback`, `on_speech`.

- [ ] **Step 1: Declare and create**

`pub mod audio;` in `client.rs`; `crates/tipsy-compadre/src/client/audio.rs` with `use super::*;` and:

```rust
impl Client {
    pub(super) fn playing(&self) -> bool { .. }
    pub(super) fn flush_playback(&self) { .. }
    pub(super) fn on_speech(&mut self, samples: Vec<i16>) { .. }
}
```

Bodies unmodified.

- [ ] **Step 2: Use `speech_rate` in `on_message`, and change nothing else**

`on_message` stays in the parent — it translates events and touches every child — but its rate
bookkeeping becomes the shared rule:

```rust
// was:
// if let Event::SpeechChunk { sample_rate, .. } = &event {
//     self.rate = *sample_rate;
// }
if let Some(rate) = speech_rate(&event) {
    self.rate = rate;
}
```

**This is a pure substitution and the order is the contract**: the announcement is read *before* the
audio is handed to the device, in this statement and in Plan 3's browser client alike. If a reviewer
cannot see that the two statements are equivalent, that is a finding, not a style note.

- [ ] **Step 3: Move the playback tests**

Measured on 2026-10-12, to be confirmed: `only_an_interrupt_with_audio_left_to_drop_flushes`,
`the_commands_that_end_a_turn_are_the_ones_that_interrupt_it`,
`a_chunk_header_sets_the_rate_the_device_is_told`,
`a_finished_reply_keeps_its_audio_until_a_new_turn_starts`. Tests that build a `Client` stay in the
parent if that is where their helpers are, and reach the moved methods either way; apply Task 1 Step 8's
rule rather than this list.

- [ ] **Step 4: Gates**

Task 1 Step 8's four commands, then `cargo test -j 1 -p tipsy-compadre`, then clippy as before.
Expected: identical test-name set, green suite, exactly seven findings.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/src/client/audio.rs
git commit -m "refactor(client): playback and the replay buffer move to client/audio.rs"
```

---

## Task 4: `client/call.rs` — the live call's lifecycle

**Files:**
- Modify: `crates/tipsy-compadre/src/client.rs`
- Create: `crates/tipsy-compadre/src/client/call.rs`

**Interfaces:**
- Consumes: `tipsy_ui::policy::takes_the_floor` (Task 1); `crate::realtime::{Call, CallSetup, Heard}`,
  which stay native.
- Produces: `impl Client`'s call block, `pub(super)`: `toggle_call`, `start_call`, `sync_call`,
  `end_call`; and `call_ended` as a `pub(super) fn` of the module, because `on_message` stays in the
  parent and calls it.

- [ ] **Step 1: Declare, create, and move**

`pub mod call;`; `crates/tipsy-compadre/src/client/call.rs` with:

```rust
use super::*;

impl Client {
    pub(super) fn toggle_call(&mut self) { .. }
    pub(super) fn start_call(&mut self) -> Result<(), String> { .. }
    pub(super) fn sync_call(&mut self) { .. }
    pub(super) fn end_call(&mut self) { .. }
}

/// Whether a live-state event ends the call here.
pub(super) fn call_ended(event: &Event) -> bool { .. }
```

`call_ended` moves here and **not** to `tipsy_ui::policy`: the browser client implements no realtime
call in v1, so it is not shared behaviour, and Review Focus 4's rule is that only what both clients
need crosses the crate boundary. Keep its doc comment.

- [ ] **Step 2: Fix the parent's call sites**

`on_message` calls `call_ended(&event)`; add to `client.rs`:

```rust
use call::call_ended;
```

- [ ] **Step 3: Move the call tests**

Measured on 2026-10-12, to be confirmed: `the_client_lets_go_of_a_call_when_the_socket_says_it_is_over`,
`a_call_never_starts_while_the_mode_is_disabled`,
`the_verbs_that_take_the_floor_are_the_ones_a_call_answers_locally`,
`tab_with_a_live_call_cancels_the_response_and_leaves_the_call_running`.

- [ ] **Step 4: Gates, exactly as Task 3 Step 4**

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/src/client/call.rs
git commit -m "refactor(client): the live call's lifecycle moves to client/call.rs"
```

---

## Task 5: `client/settings.rs` — the configuration screen's actions

**Files:**
- Modify: `crates/tipsy-compadre/src/client.rs`
- Create: `crates/tipsy-compadre/src/client/settings.rs`

**Interfaces:**
- Consumes: everything it uses today — `crate::config::{Config, store, form}`, `crate::keys`,
  `crate::providers` and `tipsy_protocol::ConfigView`.
- Produces: `impl Client`'s settings block, `pub(super)`: `open_screen`, `open_screen_remote`,
  `on_config_effect`, `test_provider`, `save`, `save_remote`, `regenerate_token`, `config_notice`.

- [ ] **Step 1: Declare, create, move**

`pub mod settings;`, `use super::*;`, and the eight methods with `pub(super)`, bodies unmodified.

- [ ] **Step 2: Leave `key_input` and `clock_view` where they are**

They are the two functions Plan 3's Task 3.5 is specified to copy, and they stay copied:

- `key_input` takes a `ratatui::crossterm::event::KeyEvent`. `tipsy-ui` gets `ratatui` with
  `default-features = false` and no crossterm (`Cargo.toml:14`), and `global-constraints.md:74-75`
  forbids the crate from ever gaining crossterm. Moving it would break the wasm gate.
- `clock_view` reads this process's clock; the browser reads `js_sys::Date`.

If a task step anywhere in this plan tells you to move either one, that step is wrong and this one
wins. Say so in the whole-branch review.

- [ ] **Step 3: Move the settings tests**

Measured on 2026-10-12, to be confirmed: `opening_the_screen_again_reads_the_file_again`,
`a_file_that_cannot_be_read_says_so_on_the_screen_itself`,
`a_refused_save_leaves_the_file_and_the_running_configuration_alone`,
`a_save_whose_providers_cannot_be_built_never_writes_the_file`,
`a_saved_configuration_is_written_and_swapped_into_the_running_stack`,
`regenerating_the_token_fills_the_row_and_writes_nothing`,
`the_test_button_reports_a_real_answer_with_its_latency`,
`a_test_for_a_row_with_no_key_answers_without_calling_anything`,
`a_test_probe_is_not_a_turn_and_leaves_the_playing_audio_alone`, and the helper `press_test`.

These are the ones most likely to need the parent's helpers; `press_test` takes a `&mut Client`, so it
moves with them and becomes `pub(super)` if the parent still uses it.

- [ ] **Step 4: The size gate**

```powershell
(Get-Content crates/tipsy-compadre/src/client.rs |
  Select-String -Pattern '#\[cfg\(test\)\]' | Select-Object -First 1).LineNumber
(Get-Content crates/tipsy-compadre/src/client.rs).Count
```

Expected: production lines (the first number minus one) **at most 574**, and no file under
`crates/tipsy-compadre/src/client/` over 400 production lines. 574 is the delivered measurement and a
ceiling rather than a target — the gate is the spec's §6, item 6, which this plan's front matter points
at. As a comparison, `client.rs` had ~1176 production lines and 2296 total when this plan was written.

- [ ] **Step 5: Gates, exactly as Task 3 Step 4**

- [ ] **Step 6: Commit**

```bash
git add crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/src/client/settings.rs
git commit -m "refactor(client): the configuration screen's actions move to client/settings.rs"
```

---

## Task 6: `session.rs` — the same cut on the server side (droppable)

**This task is optional.** Nothing outside the server needs it, no other task depends on it, and it is
the first thing to drop if the budget bites. If you drop it, record that in the ledger and move to
Task 7.

**Files:**
- Modify: `crates/tipsy-compadre/src/session.rs`
- Create: `crates/tipsy-compadre/src/session/{turn,login,config}.rs`

**Interfaces:** none cross the crate boundary here. This is a split by reason to change, nothing more:
the turn, the login phase, and the configuration commands. `impl Session` was one 723-line block on
2026-10-12 (`session.rs:287-1010`), and `Session`'s fields stay in the parent so the children see them
by Rust's descendant rule.

- [ ] **Step 1: Read the block and classify every method**

No code yet. Write the three lists into the task's commit message draft: which methods are the turn,
which are the login phase, which are configuration, and which are none of the three and stay in the
parent (`run`, `handle`, `dispatch`, `welcome`, the constructor). If more than three methods are "none
of the three", the cut is wrong: report it rather than forcing it.

- [ ] **Step 2: Split, one child at a time, running the suite after each**

`pub mod turn;` (or `login`/`config`) in `session.rs`, `use super::*;` in the child, `pub(super)` on
what the parent calls. Run `cargo test -j 1 -p tipsy-compadre` after each child and commit each one:

```bash
git add crates/tipsy-compadre/src/session.rs crates/tipsy-compadre/src/session/login.rs
git commit -m "refactor(session): the login phase moves to session/login.rs"
```

- [ ] **Step 3: The gates**

The identity gate and clippy, as in every other task. **And the one gate that is only this task's:**
one of clippy's seven findings lives in `session.rs`, and its citation in `global-constraints.md` moves
with it. Update that line in the same commit that moves it, exactly as the file's own note asks.

- [ ] **Step 4: The size gate**

Production lines of `session.rs` under 450 — **this is the dropped task's expectation, not a live
target**: Task 6 is the droppable split (see its heading), the delivered branch did not execute it, and
`session.rs` therefore measures what it measured before. Nothing here expects a move that will not
happen.

---

## Task 7: the record says what landed, and Plan 3's cloners become importers

**Files:**
- Modify: `docs/superpowers/plans/2026-10-11-plan-3-web-tier.md` (Tasks 3.4 and 3.5, and the pending notice)
- Modify: `docs/architecture.md`
- Modify: `CLAUDE.md`
- Modify: `.superpowers/sdd/2026-10-11-plan-3-web-tier/global-constraints.md` (git-ignored, local)

No `tipsy-ui/src/strings.rs` change: this plan adds no user-facing string, and the two es/en tables are
the user's copy to approve.

- [ ] **Step 1: Rewrite the cloning parts of Tasks 3.4 and 3.5**

In the web-tier plan, find the parts of Task 3.4's body that define their own `starts_turn`,
`interrupts_playback` and `flush_on_interrupt`, and of Task 3.5's that define `takes_the_floor`,
`hit_at` and the routing rule. Replace each definition with the import:

```rust
use tipsy_ui::policy::{flush_on_interrupt, hit_at, interrupts_playback, mic_route, starts_turn, takes_the_floor};
```

**Change nothing else in either body.** WebAudio, the jitter buffer, the socket, the page, the manual
checks and the test lists stay exactly as written. Say in each task's `Consumes:` block that the
functions now come from `tipsy_ui::policy` and no longer from `client.rs`.

Task 3.5's key folding and its clock read **stay its own**: see Task 5 Step 2. Add one sentence to
3.5 saying that they are per-shell by design and not duplication, so the next reader does not "fix" it.

- [ ] **Step 2: Replace the pending notice**

Both the front matter of the web-tier plan and the block at the top of `global-constraints.md` carry a
notice added on 2026-10-12 saying the split is pending. Change each to the past tense and to what is
true: the four files are split, the policy lives in `tipsy_ui::policy`, `key_input` and `clock_view`
stayed per shell, and the baseline is the count you actually measured. Keep the instruction to
re-measure citations — the files moved, so every `file:line` the plan carries is now wrong, but it is
wrong against a *stable* tree and can be re-taken once.

- [ ] **Step 3: `architecture.md`**

Two edits, and no more:

- the workspace table's `tipsy-ui` row: its one-sentence description widens from *the ratatui HUD* to
  the front end's shared, wasm-safe half — state, strings, drawing, **and the decisions a client makes
  with no device, socket or clock in reach**.
- the codebase's own map: one line saying that a client's turn, microphone and playback policy lives in
  `tipsy-ui::policy` and is used by both the terminal client and the browser client, so the two cannot
  drift.

- [ ] **Step 4: `CLAUDE.md`**

The "Current work" paragraph this plan added on 2026-10-12 says the split is pending. Turn it into a
record: what landed, the final test count, and the one thing Phase 2 and Phase 3 still owe — that every
`file:line` they carry for the four files is re-measured.

- [ ] **Step 5: The plan's own gate**

Run: `cargo test -j 1 --workspace` and `cargo clippy --workspace --all-targets`, one last time.
Expected: green, and exactly seven findings — with `session.rs`'s citation current if Task 6 ran.

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/plans/2026-10-11-plan-3-web-tier.md docs/architecture.md CLAUDE.md
git commit -m "docs: the client's files are modular, and Plan 3 imports the policy"
```

`global-constraints.md` is git-ignored and cannot be in the commit; edit it in the checkout and say so
in the commit body.

---

## What this plan does not do, deliberately

- **No `Msg`/`Cmd`/`update`.** The Elm architecture is the spec's rejected alternative D1(b): it shares
  the loop rather than the decisions, and it is the step that turns a refactor into a rewrite. The
  residual duplication it would remove is named in the spec's §7 and stays.
- **No crate is added.** `tipsy-client` is the documented ceiling if `tipsy-ui` stops having one reason
  to change; it is a `git mv` plus one `Cargo.toml` when that happens.
- **No test file, no `#[path]`.** Unit tests stay inside the file they test, which is the Rust book's
  convention and this repository's; the spec's D3 records why the two alternatives were dropped.
- **No behaviour change.** The one new function, `speech_rate`, is a rule that was already there as the
  order of two statements, and its test is the proof.

## Self-review record

- **Spec coverage.** §3's cut: Tasks 2–6. §4's membership: Task 1, with Task 5 Step 2 pinning what
  stays and why. §5.1's order: "How to schedule this". §5.2's scoped rewrite: Task 7 Step 1. §5.3's
  citations: Task 7 Steps 2 and 4. §5.4 (`pty.rs` untouched): Task 2 Step 1. §5.6's notice: Task 7
  Step 2, closing the one this session opened. §6's gates: Task 1 Steps 1, 6, 8, 9 and the same steps
  in every later task. §7's risks: Review Focus.
- **The one gap this plan cannot close:** the spec's §6 gate 7 asks for a grep proving the policy names
  nothing from `std::fs`, `std::time`, `tokio` or `crossterm`. Task 1 Step 6 makes the wasm check the
  proof, which is stronger; the grep is Task 1 Step 4's read of the file's imports.
- **Type consistency:** `MicRoute` and the nine function names in Task 1's `Produces` block are the
  names Tasks 2–5 import and the names Task 7 writes into the web-tier plan. There is no second
  spelling of any of them.
