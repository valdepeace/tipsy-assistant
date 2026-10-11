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

**Complete as a branch, merge gated:** Plan 2C
(`docs/superpowers/plans/2026-10-10-plan-2c-openai-realtime.md`) — the OpenAI Realtime voice mode:
`[realtime]` in the config, the socket over `tokio-tungstenite`, the pure wire-event mapping, the turn
plumbing with one exit for every way a socket dies, honest barge-in truncation, and the live indicator
with `F10`. All six tasks, the whole-branch review, one fix wave and one scoped re-review ("all
findings addressed, no open findings") on branch `plan-2c-openai-realtime`; **362 tests green**. Its
merge waits on the login branch: another agent's `Login*` arms sit inside 2C's commits and their
protocol change is still uncommitted, so 2C does not compile without their work. Four residual Minors
are parked in its ledger, and the release checklist still owes one live-key spoken exchange.

**Not started:** the two throwaway spikes in a scratch directory outside the repo — Ratzilla
(ratatui `Canvas` with Braille + mouse on wasm32) and Rustpotter (a custom Spanish wake word; the
shipped one is "oye compae"). They feed the web-client plan and the `hands_free` plan, and every
profile's wake word stays provisional until Rustpotter verifies it.

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
