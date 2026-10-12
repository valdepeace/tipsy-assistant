# tipsy-compadre — Client modularity and the shared front-end core — Design

> Status: **draft, for review.** Written on branch `plan-3-phase-1` on 2026-10-12, while another
> session was editing the same checkout (§7). Every number below is a measurement with a date, not a
> fact to trust: the tree moved from `client.rs` 2196 to 2296 lines *during* the session that wrote
> this document. §6 makes re-measuring them part of the work.

## 1. Purpose

Two problems, one cause.

**The files are unreadable by reason of change, not of size.** Measured 2026-10-12:

| File | Total | Production | Unit tests | One block that mixes everything |
|---|---|---|---|---|
| `crates/tipsy-compadre/src/client.rs` | 2296 | ~1176 | ~1119 (from `:1177`) | `impl Client`, `:166-988` — **823 lines, 30 methods** |
| `crates/tipsy-compadre/src/session.rs` | 1313 | ~1030 | ~282 (from `:1031`) | `impl Session`, `:287-1010` — **723 lines** |
| `crates/tipsy-ui/src/config_form.rs` | 1984 | ~1219 | ~764 (from `:1220`) | one struct and its methods |
| `crates/tipsy-ui/src/state.rs` | 1694 | ~805 | ~888 (from `:806`) | `UiState` and its methods |
| `crates/tipsy-compadre/src/realtime/mod.rs` | 917 | ~691 | ~226 (from `:692`) | one session type |
| `crates/tipsy-ui/src/render.rs` | 857 | ~735 | ~121 (from `:736`) | `draw` and its widgets |

`impl Client` carries at least six reasons to change: bootstrap and wiring; the frame loop and event
translation; audio playback; microphone capture and its routing; the live realtime call; and the
configuration screen's actions. `impl Session` carries four: the turn, the login phase, the realtime
delegation and the configuration commands.

**And the decision policy is private to a binary crate, so the browser client is specified to clone
it.** Plan 3's Task 3.4 reimplements `starts_turn`, `interrupts_playback` and `flush_on_interrupt`
"verbatim in behaviour", citing `client.rs:619-640`, `:658-660` and `:578-580`; Task 3.5 clones the
hit test, the key folding and the clock. Two clients, two copies of one policy, kept honest by
comments that say "the native client is the authority".

### Success criteria

1. Every file that carries one reason to change is small enough to read in one go; measured as
   production lines, not total lines, with the targets in §6.
2. The client's turn, microphone and playback policy is written **once**, and both the native client
   and the browser client call it (§4).
3. **Zero behaviour change.** Same test names, same number of tests, same results (§6).
4. `tipsy-ui` and `tipsy-protocol` still compile for `wasm32-unknown-unknown`, and `tipsy-ui` gains no
   dependency that a browser cannot build.

## 2. Decisions log

| # | Decision | Alternative rejected, and why |
|---|---|---|
| D1 | **The cut plus a shared decision core** — files split by reason to change, and the policy moved into the wasm-safe crate. | (a) *Split only*: leaves the duplication with Plan 3, which was the whole argument for acting before Phase 2. (b) *Full Elm (`Msg`/`Cmd`/`update`)*: shares the loop rather than the decisions, costs a rewrite of all 30 methods, and is the step that turns a refactor into a rewrite. It stays the natural continuation if the cut proves the loop can be shared. |
| D2 | **The shared core lives in `tipsy-ui`.** | A new `tipsy-client` crate is 3 files and a clean boundary, and is the upgrade path. It is not paid for now: `tipsy-ui` is already the wasm-safe crate both clients share, and already owns `Hit`, `KeyInput` and `UiState::on_key`/`on_hit`. Its one-sentence description widens from *the ratatui HUD* to *the front end, minus devices and socket*, and `docs/architecture.md` gains that line. |
| D3 | **Unit tests stay inside the file they test** (`#[cfg(test)] mod tests`, `use super::*`), as the Rust book's Test Organization chapter prescribes, and as the 45 `#[cfg(test)]` blocks already in this repository do. No test file is created, and no `#[path]` attribute is introduced. | Moving them to a sibling file was considered twice and dropped: the book's reason for in-file is that a child `tests` module can reach private items, and `crates/tipsy-compadre/tests/` is already the integration half, which is the other half of the same convention. |
| D4 | **Tests travel with the code they test.** When a method moves to a child module, the tests that exercise it move into that child's inline `mod tests`. | Leaving the whole test module on the parent would leave `client.rs` at ~2095 lines and deliver nothing. |
| D5 | **The `wasm32-unknown-unknown` gate decides what may enter `tipsy-ui`.** A function that needs a backend crate or the operating system's clock stays in the shell. | Stated as a rule because it is not obvious and it removed two functions this design first meant to move (§4). |
| D6 | **The refactor lands after Phase 1 is reviewed and merged, and before Phase 2**, in its own `git worktree`. | Doing it on top of an unreviewed `plan-3-phase-1` would put a mixed diff in front of the branch review. |
| D7 | **Only the cloning parts of Plan 3's Tasks 3.4 and 3.5 are rewritten** to consume the shared core. | Rewriting both task bodies wholesale is re-authoring the largest phase before knowing the core works; not touching them at all leaves the duplication this design exists to remove. |
| D8 | **Every line citation in this document and in the plan it spawns carries a measurement date and an instruction to re-take it.** | The repository already lost time to stale citations; `client.rs` moved under this very session. |

## 3. The cut

**`client.rs` stays the parent module and is not renamed.** Plan 3's Task 1.7 established that
(`client/bridge.rs` hangs off it) and Task 2.1 repeats it (`client/pty.rs`). The children are added
beside those.

| File | Holds |
|---|---|
| `client.rs` (parent) | `Client` and its fields, `run`, `new`, `remote`, `terminal`, `open_player`, `open_in_process`, `hello`, `Attachment` and its `impl`, `frame_loop`, `on_message`, `draw_frame`, and the two helpers that cannot leave a terminal: `key_input` and `clock_view` |
| `client/mic.rs` | `drain_mic`, `sync_mic`, `login_capture`, `sync_login_mic`, `close_login_mic` and the microphone queue |
| `client/audio.rs` | `playing`, `flush_playback`, `on_speech`, and the reply kept for `Replay` |
| `client/call.rs` | `toggle_call`, `start_call`, `sync_call`, `end_call`, `call_ended` |
| `client/settings.rs` | `open_screen`, `open_screen_remote`, `on_config_effect`, `test_provider`, `save`, `save_remote`, `regenerate_token`, `config_notice` — **eight methods, not "the configuration half of `on_action`"**: that half is two arms *inside* `on_action`'s body (`UiAction::OpenConfig` and `UiAction::Config`), they stay in the parent because moving them would rewrite a body this refactor may not touch, and they call these methods |

The free functions at the bottom of `client.rs` do not stay where they are: `starts_turn`,
`interrupts_playback`, `test_interrupts_playback`, `flush_on_interrupt`, `takes_the_floor`,
`mic_wanted`, `mic_route` and `MicRoute` move to `tipsy_ui::policy` (§4), and `hit_at` with them.
`call_ended` stays, because the browser client has no call. Nothing in this file is renamed: the
parent imports what left it.

**`session.rs`** receives the same operation as a **separate, droppable task**: `session/turn.rs`,
`session/login.rs`, `session/config.rs`. Nothing outside the server needs it, so it is the first thing
to leave this spec if the budget bites.

### Visibility rules

Rust's privacy rule does most of the work: a private item is visible in its module **and its
descendants**, so a child module already sees every private field of `Client` declared in the parent.
Only two rules are needed.

1. **A method that moves to a child and is called from the parent becomes `pub(super)`.** Private
   methods of `client::mic` are not visible in `client`, and `frame_loop` calls them.
2. **Nothing becomes `pub`.** `pub(super)` from `client::mic` means "visible in `client` and its
   descendants", which includes `crate::client::tests`.

Shared test helpers (`mock_client`, `welcome`, `welcome_mode`, `resolver`, `queue_frames`,
`sync_login`, `press_test`) stay in the parent's `mod tests`, marked `pub(super)` so a child's tests can
reach them. If `use crate::client::tests::*;` from six child modules reads worse than it should, the
fallback is `client/support.rs` behind `#[cfg(test)] mod support;` — same content, one more file, and
it is *not* a test file: it holds helpers with no `#[test]` in it.

**Two of those helpers did not stay, and the rule that moved them is D4, not this sentence.**
`queue_frames` went with its one moving test to `client/mic.rs` (Task 2) and `press_test` went with its
one moving test to `client/settings.rs`: a helper the parent's own tests no longer call is dead code
there, and this crate's lint gate does not allow dead code.
Measured on the finished tree, the parent's `mod tests` keeps `resolver`, `mock_client`, `welcome` and
`sync_login`, and the parent names `press_test` and `queue_frames` nowhere.

## 4. The shared core, and the gate that decides membership

**The rule is D5, and it is checkable: if the function needs `crossterm`, a device, a socket, the file
system or the operating system's clock, it stays in the shell.**

This is not a stylistic preference here, it is both a build fact and a rule already written down.
`tipsy-ui` depends on `ratatui` through the workspace entry, which is `default-features = false,
features = ["std", "layout-cache", "all-widgets"]` (`Cargo.toml:14`) — **no crossterm**.
`tipsy-compadre` declares its own `ratatui = { version = "0.30" }`
(`crates/tipsy-compadre/Cargo.toml:14`) and so gets the default features, which is why
`ratatui::crossterm::event::KeyEvent` exists in `client.rs` and not in `tipsy-ui`. And it is the
constraints file's own rule: `global-constraints.md:74-75` names **`crossterm`** in the list of crates
`tipsy-ui` may never gain. Moving `key_input` would break both.

| Moves into `tipsy-ui` | Why it is shared | Reaches for |
|---|---|---|
| `starts_turn(&Command) -> bool` | Task 3.4 reimplements it verbatim | `tipsy-protocol` |
| `interrupts_playback(&Command) -> bool` | Task 3.4 | `tipsy-protocol` |
| `test_interrupts_playback(Capability) -> bool` | Task 3.4/3.5 | `tipsy-protocol` |
| `flush_on_interrupt(interrupting, playing) -> bool` | Task 3.4 keeps it deliberately | nothing |
| `takes_the_floor(&Command) -> bool` | Task 3.5's barge-in path | `tipsy-protocol` |
| `mic_wanted(mic_open, live, login) -> bool` | Task 3.3's microphone gate | nothing |
| `mic_route(device, login, live) -> MicRoute` and `MicRoute` | Task 3.3/3.4's routing | nothing |
| `hit_at(&[Hit], column, row) -> Option<Hit>` | Task 3.5 clones it | `ratatui::Rect`, `Hit` (both wasm-safe) |
| `speech_rate(&Event) -> Option<u32>` — **new** | the "rate first, then audio" rule of `on_message`, which today exists only as the order of two statements | `tipsy-protocol` |

| Stays in the shell | Why |
|---|---|
| `key_input(&KeyEvent) -> Option<KeyInput>` | takes a crossterm `KeyEvent`; D5, and Ratzilla supplies its own events. The *folding decision* it encodes is small and each shell keeps its own translation — this is the one duplication this design accepts and names. |
| `clock_view() -> ClockView` | reads this process's clock; the browser reads `js_sys::Date` and its own weekday numbering. `ClockView` itself is already shared. |
| `call_ended(&Event) -> bool` | the browser client implements no realtime call in v1: Plan 3 has no realtime task, and keys never leave the server. |
| `draw_frame`, `Attachment`, the frame loop | terminal, terminal and channels. |

The shared module is `tipsy_ui::policy`. It depends on `tipsy-protocol` and `ratatui` and on nothing
else — no `tokio`, no `cpal`, no `rodio`, no `keyring`, no `sysinfo`.

## 5. Coordination with Plan 3

1. **Order.** Phase 1 is already reviewed — twice, security and correctness — and its fix wave is
   closed: **505 tests, 0 failures**, clippy at the same seven findings, wasm clean, at `2b328f3`. Only
   its **merge** to `master` is outstanding. The refactor is then its own branch off `master`, executed
   in its own `git worktree`; Phases 2 and 3 are executed against the new layout.
2. **The scoped rewrite.** In `docs/superpowers/plans/2026-10-11-plan-3-web-tier.md`, the parts of
   Task 3.4 and Task 3.5 that define their own `starts_turn`, `interrupts_playback`,
   `flush_on_interrupt`, `takes_the_floor`, `hit_at` and the routing rule are replaced by imports from
   `tipsy_ui::policy`. **Nothing else in those two bodies changes**: WebAudio, the jitter buffer, the
   socket, the page and the manual checks stay exactly as written.
3. **Citations.** Every `file:line` the plan carries for `client.rs`, `session.rs`, `state.rs` and
   `config_form.rs` is re-measured before it is used. The plan's own front matter already asks for
   this; this refactor is now the reason it is not optional. Note that Task 2.1's own recon recorded
   `client.rs` as 1253 lines, which no longer matches anything.
4. **`client/pty.rs` is not this work.** Task 2.1 creates it and adds `pub mod pty;` to `client.rs`;
   the refactor must not rename, move or create that file, and must not touch Task 2.1's test files.
5. **The plan this spec spawns is the only place the tasks live.** They are not added to the web-tier
   plan, which stays a web-tier document.
6. **The other session is told, in the files a new session actually reads.** There is no channel from
   this session to another top-level one — `list_agents` is empty and `send_message` reaches only this
   session's own children — so the notice has to be an artifact, and it needs **three** homes because
   the obvious one does not survive: `.superpowers/sdd/` is **git-ignored**, so it is present in this
   checkout and absent from a fresh `git worktree`.
   - **`CLAUDE.md`'s "Current work"** — a short paragraph, because that is the first thing any new
     session reads, and the next Phase 2 session is exactly that.
   - **the web-tier plan's front matter** — one committed line, read by whoever opens the plan.
   - **`global-constraints.md`** — the detailed, dated block, in the file that opens by saying every
     task inherits it.
   What it says: the four files above will be split; every `file:line` for them in Phases 2 and 3 is
   stale until re-measured; `starts_turn`, `interrupts_playback`, `test_interrupts_playback`,
   `flush_on_interrupt`, `takes_the_floor`, `mic_wanted`, `mic_route`/`MicRoute` and `hit_at` are
   imported from `tipsy_ui::policy`; and `key_input` and `clock_view` deliberately stay per shell.

## 6. Gates

Every one of these is run with `-j 1` (this machine dies with `STATUS_STACK_BUFFER_OVERRUN`
otherwise), with `CARGO_TARGET_DIR` and `TEMP`/`TMP` pinned as `global-constraints.md` requires —
**not** as `plan-3-start-here.md` says, which the ledger records as superseded and wrong in three
ways. `cargo fmt` is never run.

1. **Test-name identity.** Before the first edit and after the last one:
   `cargo test -j 1 --workspace -- --list` into a file, then compare. The gate is the **set of test
   function names — the last `::` segment of each listed path — plus the total count**, and it must be
   identical. It is deliberately not the full module path: a test that moves from `client::tests::foo`
   to `policy::tests::foo` is the work, not a regression. This is the gate that catches a test lost
   inside the 823-line split.
2. **The suite is green**, `0` failures, with the same count the baseline recorded. The baseline is
   **505 at `2b328f3`** as this document is written; it is re-measured, not trusted.
3. **No test is deleted, ignored, or weakened.** No `#[ignore]`, no assertion relaxed, no test renamed.
   A test that cannot be made to pass in its new home is a finding for the review, not a deletion.
4. **The wasm gate:** `cargo check -p tipsy-ui -p tipsy-protocol --target wasm32-unknown-unknown` is
   clean, and `crates/tipsy-ui/Cargo.toml` gains no dependency.
5. **Clippy:** the finding set is **exactly the seven pre-existing findings** and nothing new, compared
   **as a set and never as a list** — that is `global-constraints.md`'s own rule, and it pins the seven
   at `2b328f3` with `session.rs:484` among them. That writer's citation has already moved three times
   in this plan (233 → 298 → 416 → 484) and this work moves it again; the updated citation goes back
   into `global-constraints.md` in the same commit that moves it.
6. **Size targets**, measured as production lines — everything above the file's first `#[cfg(test)]`
   line, and a file with no test module counts whole:
   - **`client.rs` (parent) under 574 production lines, and no child under `client/` over 400** — 574 is
     the delivered measurement, not a slack target. The 450 this line first carried was unreachable by
     construction: §3 puts `frame_loop`, `on_message`, `on_action` (both configuration arms included),
     `key_input` and `clock_view` in the parent, and the four moves shed **481** production lines
     (`1176` at `2f25c20` → `1055` → `945` → `905` → `819` → **574**), not the ~730 the arithmetic of the
     time assumed. The four children measure 271, 132, 115 and 58 production lines, which the 400 does
     bind;
   - `session.rs` under 450 if the droppable task is done;
   - `tipsy-ui`'s `policy` module under 250 including its documents.
7. **The shared core is reachable from a browser build**: `tipsy-web` does not exist yet, so the proof
   is the wasm check plus a grep proving `tipsy_ui::policy` names nothing from `std::fs`, `std::time`,
   `tokio` or `crossterm`.

## 7. Risks and residuals

- **A second session owns this checkout.** `client.rs` grew from 2196 to 2296 lines while this document
  was being written; that writer has since committed (`2b328f3` and its two neighbours) and the tree is
  clean apart from this document, but the branch is `plan-3-phase-1` and its owner can return. The
  refactor therefore runs in its own `git worktree`, never here, and only after Phase 1 has merged. The
  numbers in §1 are that tree's and must be re-measured.
- **The 823-line split is where a test can vanish.** Mitigated by gate 1 and by moving tests as intact
  blocks.
- **Residual duplication, accepted and named:** `key_input`'s folding rule and `clock_view`'s clock
  read stay per shell. Both are small; the first is forced by the wasm gate, the second by there being
  two clocks.
- **`tipsy-ui`'s remit widens.** It gains client policy. The ceiling is known and written down: if the
  crate stops having one reason to change, `tipsy-client` is a `git mv` plus one `Cargo.toml`.
- **The Elm architecture is not adopted.** If the cut shows the loop itself can be shared, that is its
  own spec, and it is the only thing that would remove the residual in the row above.
- **The notice in §5.6 is not optional and is not a formality.** If Phase 2 starts before the refactor
  lands and without that notice, its implementer will follow citations into files that have moved, which
  is the failure this whole coordination section exists to prevent.

## 8. What changes, where

| Path | Kind |
|---|---|
| `crates/tipsy-compadre/src/client.rs` | modify: the parent keeps the loop, the wiring and the HUD input; loses four groups of methods and their tests |
| `crates/tipsy-compadre/src/client/{mic,audio,call,settings}.rs` | new: the moved methods and their tests |
| `crates/tipsy-compadre/src/session.rs` and `session/{turn,login,config}.rs` | modify and new, only if the droppable task is in scope |
| `crates/tipsy-ui/src/policy.rs` | new: the shared core of §4 |
| `crates/tipsy-ui/src/lib.rs` | modify: `pub mod policy;` |
| `docs/architecture.md` | modify: one line for `tipsy-ui`'s widened remit, and one for where client policy lives |
| `docs/superpowers/plans/2026-10-11-plan-3-web-tier.md` | modify: Tasks 3.4 and 3.5, only the cloning parts; and one line in the front matter pointing at the notice |
| `.superpowers/sdd/2026-10-11-plan-3-web-tier/global-constraints.md` | modify: the dated notice of §5.6, and the moved `session.rs` clippy citation |
| `CLAUDE.md` | modify: a short paragraph in "Current work" carrying the notice of §5.6 — the first thing a new session reads. The test placement rule of D3 is not new; it is the existing convention, stated in §2 for the next reader |
