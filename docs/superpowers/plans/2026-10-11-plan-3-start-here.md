# Plan 3 — where to start

Handover for whoever begins the **web tier**, whose plan is
[`2026-10-11-plan-3-web-tier.md`](2026-10-11-plan-3-web-tier.md) and whose design is
[`../specs/2026-10-11-web-tier-design.md`](../specs/2026-10-11-web-tier-design.md). Read this before
touching anything.

## What this file was, and what has moved since

It was written by the session finishing the **login screen** while that work was still in its own
worktree at `E:\workspace\personal\tipsy-login`. **That worktree is gone, its branch is merged and
master is the only branch.** So the instruction this file used to carry — "wait for the login merge
before touching these" — **has been carried out**, and the collision table it used to hold against the
login branch is history. What survives is the same table read a different way: those files are where
Plan 3's *own* tasks collide with each other, which is the ordering this plan has to respect.

The state at the time of this rewrite: `master = ab8f769`, holding Plans 1, 2A, 2B, 2C, the login
screen, and three fixes to the login's once-per-run rule — **425 tests, 0 failures**, clippy at its
seven pre-existing findings, and the `wasm32-unknown-unknown` check clean for `tipsy-protocol` and
`tipsy-ui`.

## The rule that matters most

**One writer per working tree.** A second session editing the same checkout is what produced, on
2026-10-11, a `git stash` that threw away an implementer's in-flight work, and three rounds spent
chasing a "vanished symbol" that was really cargo reusing a stale artefact because a copied file kept
an older mtime. So:

- **Work in your own `git worktree`** on its own branch, off `master`. Never write into a tree another
  agent is working in.
- **Never run `git stash`, `git checkout -- .`, `git restore` or `git reset` in a tree you do not own.**
- If you work in the main checkout, work on a branch, not on `master`.

## Start with the work that cannot collide

These are **new files**, so they collide with nothing — not with each other and not with any other
plan:

- **`crates/tipsy-web/`** — the whole crate is new (`Cargo.toml`, `src/lib.rs`, `src/wire.rs`,
  `src/socket.rs`, `src/mic.rs`, `src/playback.rs`, `src/web_audio.rs`, `src/hud.rs`,
  `src/login_door.rs`). Four of those are **pure and host-runnable** — `wire.rs`, `mic.rs`,
  `playback.rs`, `hud.rs` — with their own tests, which is what makes them startable before the server
  exists.
- **`xtask/src/assets.rs`**, **`xtask/src/dist.rs`** and **`web/vendor/README.md`**.
- **`web/terminal/index.html`** and **`crates/tipsy-compadre/src/client/pty.rs`** — new files; their
  *routes* land later, in files that do overlap, so write the module and its tests first and wire it
  when the merge is in.

## The order Plan 3 owes itself

The plan's own phases impose it, and the reason is the compiler rather than taste:

- **Task 1.1 first.** `ConfigView`, the three commands, the three events and five `ErrorCode`
  variants are what every later task names. Nothing that mentions them compiles before it lands.
- **Tasks 1.2 and 1.3 before 1.4, 1.5 and 1.6.** The handshake policy and the `Hub` are what the
  listener, the assets and the connection bridge are built on, and `Hub::spawn` is the only entry
  point a connection uses.
- **Phase 1 before Phases 2 and 3**, which are both clients of the server; and **Task 3.1 is already
  answered** — the Ratzilla spike ran and its verdict is `PASS`, recorded in
  `.superpowers/sdd/plan-3-research/ratzilla-spike-verdict.md` and summarised in the plan's own
  ledger. **Do not redo it.**
- **`client.rs` is not renamed.** Task 1.7 adds `src/client/bridge.rs` and Task 2.1 adds
  `src/client/pty.rs`; `client.rs` stays the parent module and both `pub mod` lines must survive.
  An earlier draft of this file called that a rename, which would have been the worst kind of
  conflict to resolve — it is not one.

## Environment: the traps that cost hours here

```powershell
# The shell already sets CARGO_TARGET_DIR to ANOTHER PROJECT (…\jevalon\target) and that
# overrides .cargo/config.toml, so pin it explicitly on every single command:
$env:CARGO_TARGET_DIR="E:\workspace\personal\tipsy-compadre\.target-sandbox"
# Note the $env: — "$CARGO_TARGET_DIR" is the *shell* variable and is empty, which sets
# TEMP=\tmp → E:\tmp, a denied path that manufactures ~25 failures that look like code faults:
$env:TEMP="$env:CARGO_TARGET_DIR\tmp"; $env:TMP=$env:TEMP
cargo test -j 1 …
```

- **`-j 1` is not optional**: at cargo's default parallelism this machine dies with
  `STATUS_STACK_BUFFER_OVERRUN`, which looks exactly like a code fault and is not one.
- **Never run `cargo fmt`.** The repository is not rustfmt-clean (36 files) and formatting it rewrites
  hundreds of unrelated lines.
- **The lint gate is not `-D warnings`.** Clippy exits `0`; it reports **seven pre-existing findings**
  and the gate is "the finding set is still exactly those seven, none new".
- **wasm is a real gate for `tipsy-protocol` and `tipsy-ui`**:
  `cargo check -p tipsy-ui -p tipsy-protocol --target wasm32-unknown-unknown` (the target is installed;
  the Ratzilla spike put it there).
- **If a symbol you can see in a file "does not exist", it is cargo, not sabotage:** a file copied into
  a worktree keeps its old mtime, so cargo may reuse an artefact built before the change.
  `cargo clean -p <crate>` cures it.

## Suggested shape

Plan 3 is **20 tasks in three phases** — 9 for the server and the remote client, 4 for the web
terminal, 7 for the web client — and **each phase leaves something testable on its own**. Run them as
**three blocks with one whole-branch review each** rather than twenty tasks in one go, which is also
this repository's standing ruling: one implementer per task, no per-task reviews, one review per
branch, then one fix wave and one scoped re-review.
