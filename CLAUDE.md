# tipsy-compadre — instructions for Claude

Rust client-server voice assistant for developers with affectionate regional profiles.
Read `docs/architecture.md` and the spec `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md`.

## Current work

**Integrated on `master`:** Plan 1 (`docs/superpowers/plans/2026-10-10-plan-1-terminal-core.md`) and
Plan 2A (`docs/superpowers/plans/2026-10-10-plan-2a-providers-and-personas.md`). Each plan owns a
git-ignored SDD workspace under `.superpowers/sdd/<plan-stem>/` holding its task briefs, the global
constraints and the progress ledger with every ruling taken while executing it. **Resume from the
ledger**: tasks with a `Task <N>: complete` line are done; start at the first one without it.

**Integrated on `master` too: Plan 2B**
(`docs/superpowers/plans/2026-10-10-plan-2b-config-screen-and-auto-mode.md`) — the configuration
screen (`F9` / the header's gear, four tabs, save-and-apply without a restart, the Test button that
makes a real call and reports its measured latency) and the `auto` listening mode, with the detector
in `vad.rs` over `earshot`. All ten tasks, the whole-branch review, one fix wave, one scoped
re-review and the merge; 275 tests, 0 failures. Its ledger records four Minor findings parked as
follow-ups and worth doing before the next release: the screen no longer shows the "unsaved" marker
while it is open (a cost of drawing the header over the full-screen form, and untested), a comment in
`client.rs` that overclaims where a read error is shown, one mangled test line `cargo fmt` would fix,
and the help text that lost the true half of the toggle key (`Enter` still toggles a switch row).

**Integrated on `master` too: Plan 2C**
(`docs/superpowers/plans/2026-10-10-plan-2c-openai-realtime.md`) — the OpenAI Realtime voice mode:
`[realtime]` in the config, the socket over `tokio-tungstenite`, the pure wire-event mapping, the turn
plumbing with one exit for every way a socket dies, honest barge-in truncation, and the live indicator
with `F10`. All six tasks, the whole-branch review, one fix wave and one scoped re-review ("all
findings addressed, no open findings"); **362 tests green** on its own branch, and it is on master
because the login screen's protocol definitions are what made its `Login*` arms compile — the debt that
used to leave master unable to build is paid. Four residual Minors are parked in its ledger, and the
release checklist still owes one live-key spoken exchange.

**Integrated on `master`: the login screen**
(`docs/superpowers/plans/2026-10-11-plan-login-screen.md`, spec
`docs/superpowers/specs/2026-10-11-login-screen-design.md`) — the ritual door: a widget with the core in
the middle over a veiled, unreadable HUD; an allowlist of names and keywords in `[login]` in
`config.toml`, judged on the server so it never travels to a client; typing and voice, with only the
speech the local detector confirms ever uploaded; the profile's greeting withheld until the door opens.
**Six tasks** — four with written bodies in the plan and two executed from briefs, whose reports are in
its workspace — one whole-branch review, one fix wave and one scoped re-review that closed every
finding, the worst of them being that the client could forward login audio before the microphone device
had been opened. The merge was a fast-forward. Since then a separate three-commit fix, whole-branch
reviewed and then re-reviewed, found that a *pass* verdict left the login capture open and that `Passed`
was not terminal for the phase — a later `Tab`, a stale refusal or an error could put the user back
outside a door they had walked through. Both are fixed, and **master is at 425 tests, 0 failures**. It
is a **ritual, not authentication**: `server.token` is still the only door, and nothing in its code,
docs or comments claims otherwise. Its record is `.superpowers/sdd/2026-10-11-plan-login-screen/` — read
`coordinator-rulings.md` there, not `progress.md`, which another session overwrote.

**In flight on its own branch: Phase 1 of Plan 3, the web tier**
(`docs/superpowers/plans/2026-10-11-plan-3-web-tier.md`, spec
`docs/superpowers/specs/2026-10-11-web-tier-design.md`) — **20 tasks in three phases** (9 server and
remote client, 4 web terminal, 7 web client), whose Ratzilla gate is answered `PASS`. **Tasks 1.1–1.9
are executed on branch `plan-3-phase-1` and not merged, and not yet reviewed**, at **490 tests, 0
failures**: the token-guarded WebSocket server (`serve`, the `Hub`, one session per connection, the
bind rule), the embedded bundle with its honest fallback pages, TLS with a certificate generated once
beside `config.toml` and the `insecure_transport` rule, the native `--connect` client, and the
`tracing` bootstrap whose three redaction rules keep the token, every key value this process resolved
and audio out of the log. Its ledger is
`.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md` and the constraints every task inherits are
beside it in `global-constraints.md`; Phases 2 and 3 have not started. Start from
`docs/superpowers/plans/2026-10-11-plan-3-start-here.md`: it names which of its tasks can begin without
touching a file another task owns, the order the compiler imposes, and the environment traps.

**Both throwaway spikes are done, and the second one shapes `hands_free`.** They live in scratch
directories outside the repo and nothing of them is committed. **Ratzilla: PASS on all four gate
questions and on all four items that had been left `UNVERIFIED`**
(`.superpowers/sdd/plan-3-research/ratzilla-spike-verdict.md`) — `WebGl2Backend` is the recommendation,
and the DOM backend is wrong on three counts: ~2.4× slower on a real GPU (24.6 fps against 59.9), it
**panics at an 18 px font** (issue #188, bisected: 16 and 17 px are safe), and `CanvasBackend` mis-maps
clicks by a row and a column. **Rustpotter: yes, with conditions**
(`.superpowers/sdd/plan-4-research/rustpotter.md`) — Spanish costs nothing because it compares acoustics
rather than language, and the repository's own `oye casa` references are a tested asset. But a consumer
must pin **`half = "=2.3.1"`** or the build fails with 20 errors in `candle-core`; a model needs
**recordings of a real voice** (there is no phonetic path — a TTS-built reference is the cheap start,
and one ~10-minute validation with the user's own voice is unavoidable); it needs **its own stateful
detector beside `VadState`**, because it wants 480-sample frames where the client sends 320; and
**Porcupine is no longer an alternative at all** — every version of its Rust crate is yanked and the
free tier was withdrawn. Every profile's wake word stays provisional until it is verified.

## Budget rules (user is on a tight budget)

- One implementer subagent per task, and **no per-task reviews**: a single whole-branch review when
  the plan is complete, then one fix wave and one scoped re-review of it. Plan 1 and Plan 2A both
  ran that way.
- This harness cannot pick a model per subagent — every subagent inherits the session model — so a
  plan's `[tier]`, `[haiku]` or `[sonnet]` tags are hints about a task's size, not model selections.
- Authoring a plan costs more than executing a task. Give task bodies the complete tests and the
  exact wire shapes, quote third-party endpoints from research notes instead of recalling them, and
  state behaviour in prose where the remaining implementation is mechanical.

## Known environment issues

- **The sandbox and where the build goes.** The project tree carries DSH's sandbox grant and the
  build output is configured onto `G:` (`.cargo/config.toml`; the `.cargo/` directory is
  ignored by git). A *sandboxed* shell must therefore build with
  `CARGO_TARGET_DIR="E:\workspace\personal\tipsy-compadre\.target-sandbox"`. Building into the
  configured `G:` directory labels the artifacts low-integrity, and an app started from such a
  binary runs restricted and cannot create `%APPDATA%\tipsy-compadre\config.toml` — that trap cost
  hours once and is recorded in Plan 1's ledger. The permission recovery scripts live in
  `E:\workspace\personal\.dsh-acl-recovery\` (each with its own rollback command).
- **Run the tests with `-j 1`.** At cargo's default job parallelism this machine dies with
  `STATUS_STACK_BUFFER_OVERRUN`, which looks exactly like a code fault and is not one. Plan 2B's
  ledger records the discovery.
- **When DSH's sandbox breaks, the file tools keep working and the shell does not.** Symptom: every
  `pwsh` call fails with `sandbox-local windows-acl temp grant materialization failed and its cleanup
  also failed`, even a bare directory listing, while `read`, `write`, `glob` and `grep` stay fine. An
  interrupted unconfined command can leave the grant half-materialised, and the bundled ACL diagnosis
  reports `NOT_THIS_CLASS` because the project tree already has the permissions DSH needs. Switching
  the session's access level may not clear it; restarting DSH does. Only cargo and git are out of
  reach meanwhile, so nothing committed is at risk.
- `git` prints `fsmonitor_ipc__send_query ... '.git/fsmonitor--daemon.ipc'` on every call: harmless.
- Carried over from the Claude Code era, in case you go back: Haiku subagents fail with
  `400 system.4.type` unless Claude Code starts with `ENABLE_CLAUDEAI_MCP_SERVERS=false`, and in Git
  Bash `/usr/bin` must be removed from `PATH` before `cargo` (Git's `link.exe` shadows MSVC's).

## Conventions

- Code, comments, docs and commit messages in English; profiles in their own language and variety.
- Terminal first: every feature must work in the native terminal before it is done.
- Never clone or imitate the voice of a real person; never use characters or quotes from films,
  books or other media. Profiles are original, affectionate caricatures, and regional humour never
  mocks a group. **The profile copy is the user's to approve**; implementers reproduce it verbatim
  and flag anything they would change instead of changing it.
- The product vocabulary is **profile**, not persona: `Profile`, `Config::profiles`,
  `active_profile`, `[[profiles]]`. A `config.toml` written with the old `active_persona` /
  `[[personas]]` keys still loads through serde aliases and is never rewritten in place, and only
  the alias test and the documents that predate the rename may spell the old word.
- API keys never go into files, logs or fixtures; the config stores key *names* only, and the
  resolution order is `TIPSY_KEY_<NAME>` first, then the OS credential store.
- Plans live in `docs/superpowers/plans/`, the spec in `docs/superpowers/specs/`, and the research
  notes a plan quotes from live in `.superpowers/sdd/<plan-stem>/` beside its ledger.
