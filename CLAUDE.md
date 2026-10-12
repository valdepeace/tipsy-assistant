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

**Three requests from the user, recorded in that workspace's `follow-ups.md` (2026-10-12):** reloading
`config.toml` **in place** — a save from the screen already applies without a restart, but a
hand-edited file is read only at startup — and a panel **beside the HUD's others** showing each
request's state, how long it took and the message it returned. Those two come with the design questions
the plan will have to answer (what counts as one request; where the record comes from; the error text is
a new sink for the redaction rules). The third — **choosing a profile in the screen must apply at once**,
not on `F4` — is queue item 2 below. And a profile switch mid-run **used to leave the greeting behind**,
fixed in `4a6bab9` with the test that pins it.

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
speech the **server's** detector confirms ever reaching a provider — a browser client has no detector of
its own and uploads every frame of an open capture, which is what the server's `login_vad` filters
(Plan 3's fix wave, finding 6); the profile's greeting withheld until the door opens.
**Six tasks** — four with written bodies in the plan and two executed from briefs, whose reports are in
its workspace — one whole-branch review, one fix wave and one scoped re-review that closed every
finding, the worst of them being that the client could forward login audio before the microphone device
had been opened. The merge was a fast-forward. Since then a separate three-commit fix, whole-branch
reviewed and then re-reviewed, found that a *pass* verdict left the login capture open and that `Passed`
was not terminal for the phase — a later `Tab`, a stale refusal or an error could put the user back
outside a door they had walked through. Both are fixed, and that merge closed at **425 tests, 0
failures** — a historical count: `master` moved on with every plan below, and stands at **649 tests,
0 failures** after Plan 3. It
is a **ritual, not authentication**: `server.token` is still the only door, and nothing in its code,
docs or comments claims otherwise. Its record is `.superpowers/sdd/2026-10-11-plan-login-screen/` — read
`coordinator-rulings.md` there, not `progress.md`, which another session overwrote. **Its one open
request from the user (2026-10-12): a correct answer should take the screen down and lift the veil at
once, instead of holding both until `Enter`** — which is what login spec §6.2 specifies today, on
purpose. The code paths, the spec sentences and the five tests it touches are in that workspace's
`follow-ups.md`, beside the one defect the reviews parked.

**Integrated on `master`: Phase 1 of Plan 3, the web tier**
(`docs/superpowers/plans/2026-10-11-plan-3-web-tier.md`, spec
`docs/superpowers/specs/2026-10-11-web-tier-design.md`) — **20 tasks in three phases** (9 server and
remote client, 4 web terminal, 7 web client), whose Ratzilla gate is answered `PASS`. **Tasks 1.1–1.9
are merged** (fast-forward to `2f25c20`, 35 files, +9,369/−300); the branch was reviewed twice
(security and the wire, then correctness and the tests) **and two fix waves closed every blocker**:
the bare run now says `Hello` before the terminal (it used to greet nothing and die on the user's first
keypress), the attach token travels only in `Hello` and no URL that can carry one is printed anywhere,
a handshake deadline bounds a socket that says nothing, a client's `UpdateConfig` is *shaped* so it
cannot move a provider's `base_url`, a provider's `key_name` or `server.token` and is applied only
after admission, the read side has an explicit ceiling and the session's audio buffers are capped,
three vacuous tests were made falsifiable, and `TIPSY_CONFIG_DIR` made the phase's **first live
`serve` + `--connect` + `serve --tls` check possible — which then found that `serve --tls` had never
served at all** (it bound the port twice and died with `WSAEADDRINUSE`). The scoped re-review then found
that removing the route's token check had left **a web page able to reach a loopback listener**, so
`/ws` now refuses with 403 an upgrade whose `Origin` is present and is not the host the request arrived
on — and `--connect` sends none, which is why it is unaffected. **509 tests, 0 failures**,
clippy at the same seven pre-existing findings, wasm clean. Its ledger is
`.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md` and the constraints every task inherits are
beside it in `global-constraints.md`. **Read the review reports in that same directory**
(`review-phase-1-security.md`, `review-phase-1-correctness.md`, then Phase 2's and Phase 3's
`review-once-per-run.md` and `review-fix-wave*.md`) — not
`docs/superpowers/plans/2026-10-11-plan-3-start-here.md`, which the ledger records as superseded and
wrong in three ways.

**Phase 2 of that plan — the web terminal — is merged into `master` at `0ff2e62`** (it ran on branch
`web-tier-phase-2` in its own worktree, which is gone; its ledger and the fix waves' evidence were
synced to `.superpowers/sdd/2026-10-11-plan-3-web-tier/`). **Seven commits** on a branch cut from
`master` at `a548cea`: `f548207` (a
terminal's child in a real pty, `client/pty.rs` over `portable-pty` 0.9.0),
`ba2387c` (the raw-bytes socket and the pump), `59e211d` (the xtask fetch, verify and extract of the
pinned xterm.js), `e93b193` (the page, `/terminal` served end to end, and the embed it lives in) and
`bc1cc00` (the whole-phase review's own fix wave), `28986c8` (the review's three documentation
residues) and `0ff2e62` (the guard in the last paragraph). It
was **reviewed once as a whole phase** — no blockers, five findings and seven nits, all closed in one
fix wave: the page now sends its 256-byte padding frame as a **binary** frame, which is the shape the
route ignores and the shape the test always sent (the string it used to send was a *text* frame, so
the route's binary branch was production-dead and `TerminalClientMessage::Hello` was never sent at
all — it is removed); the ledger and this file no longer say the phase has not started; the plan's
Task 3.6 no longer lists the phantom `web/vendor/xterm.js`, which made its bundle permanently
"incomplete"; `xtask dist` resolves `web/vendor/` from the crate's own location instead of the
shell's, so running it from another directory no longer reports success into a directory nothing
reads; and the fetched files are now checked against the extracted sha256 `web/vendor/README.md`
records, so "already fetched and verified" is true rather than an existence check. It closes at
**544 entries / 542 distinct, 0 failures** (from the phase's own **514/512** baseline), clippy at
exactly the seven, wasm clean. Its record is the same ledger, whose Task 2.1 section now exists and
whose Task 2.4 section carries the live check.

**And the merge found the phase's last defect, which none of the three reviews could see.**
`every_asset_the_page_references_is_embedded` skipped itself when `web/vendor` was not a directory —
but that directory **exists in a fresh clone**, because its `README.md` is committed, so `.is_dir()`
was always true and the skip was **dead code**: the assertions under it ran in a checkout that had
fetched nothing, so the suite was **red on `master` and green in the one worktree where `xtask dist`
had been run** — and that worktree is where every review had run. Fixed by asking about the three
fetched **files** rather than the directory (`0ff2e62`), and verified on both sides: the assertions run
where the files exist, and the test skips with its explanation where they do not.

**Integrated on `master`: Phase 3 of Plan 3 — the web client — and with it the plan is complete**
(merged at `766ead0`, after Phase 1 at `2f25c20` and Phase 2 at `0ff2e62`; `dd02353` then added the tone
documents). **Six commits** on a branch in its own worktree, now gone: `a3b82a6` (`tipsy-web` and its
protocol socket), `1c533d9` (the browser microphone), `cc5c7ea` (playback, the jitter buffer and
barge-in), `2407f5d` (the HUD glue, Ratzilla's input and the host page), `662b2fb`
(`cargo run -p xtask -- dist` and the bundle the binary embeds) and `a8b1dc8` (the login door), then the
**whole-phase review**, one fix wave (`6e589de`) and a scoped re-review that answered *"correct and
sufficient, merge is fine"*. **What it achieved:** `tipsy-web` draws the same `tipsy-ui` HUD through
Ratzilla into a canvas, with the browser's own microphone and playback and the login door, and **the
client's turn, microphone and playback rules are imported from `tipsy_ui::policy`, never cloned** —
which is why the client's files were split before this phase — and the review proved the import by
mutating the shared functions and watching two crates go red. It ends at **649 entries / 647 distinct
names, 0 failures**, clippy at exactly the seven documented findings and the wasm gate clean. **The
review's two blockers are closed:** the browser's push-to-talk was **dead in every configuration** — the
login door's predicate `login.is_some() && voice && open` was the sole guard on the sole send site, and
`state.login` is `None` except during an open login capture, so `Tab` moved the HUD's switch and nothing
followed (the fix is a two-case match, and **the test that pinned the defect had encoded it**); and a
microphone that would not open left the page behind an opaque full-viewport overlay with its explanation
out of sight. The one `AudioContext` is now the press's, so a refused microphone still hears the
greeting and every reply.

**The defects the user's own browser found after Phase 2 merged, and the lesson they carry.** The
terminal page asked for `Consolas` first, which does not carry the HUD's box-drawing, block and
geometric glyphs, so xterm.js fell back **per glyph**, the fallback's advance differed from the cell
width measured from the primary font, and every frame came out misaligned; fixed by putting
`DejaVu Sans Mono` first, with `Cascadia Mono` behind it (`8c2a719`, `407eebd`). And the page **dropped
its own size before the socket opened** — `send` writes only while `OPEN`, and the first `resize` comes
from the `fit()` at load, before the socket exists — so the child kept the size the PTY was born with
until a human dragged the window; fixed by queueing while connecting and re-sending the size on `open`
(`5e22919`). **Both were in a page no test could see**, and the class is now counted: three times in
this plan the tests drove the sequence a script naturally writes while the page did something else (the
padding frame's shape, a guard that asked about a directory, and this dropped resize).

**The client's files are split, and the client's decision policy is shared** — spec
`docs/superpowers/specs/2026-10-12-client-modularity-design.md`. **Merged into `master` at `0de11e0`**
(it ran on branch `client-modularity` in its own worktree, which is now gone; its ledger and the
identity gate's captures were copied to `.superpowers/sdd/2026-10-12-client-modularity/`): `ef8333c`
(the policy → `tipsy_ui::policy`), `726104b` (`client/mic.rs`), `1610ad5` (`client/audio.rs`),
`5b80abc` (`client/call.rs`) and `4237d37` (`client/settings.rs`), then the record. `client.rs` is the
parent module and went from **1176 production lines at
`2f25c20` to 574** (1176 → 1055 → 945 → 905 → 819 → 574, one step per commit); the children hold what
left it — `settings.rs` 271 production lines, `mic.rs` 132, `call.rs` 115, `audio.rs` 58 — and the
client's turn, microphone and playback policy (`starts_turn`, `interrupts_playback`,
`test_interrupts_playback`, `flush_on_interrupt`, `takes_the_floor`, `mic_wanted`, `mic_route`/
`MicRoute`, `hit_at`, and the new `speech_rate`) lives in `tipsy_ui::policy` for both clients to call.
**Reviewed once as a whole branch, answered by one fix wave, scoped-re-reviewed, and that re-review's
own follow-up verified by hand before the merge** — the review proved all 33 original `client.rs` tests
and all 24 moved methods byte-identical and the whole production diff a single equivalent substitution.
**The branch's suite was 513 tests / 0 failures** (511 distinct names: two leaf names are duplicated
across modules, and four were added by Task 1), clippy at the same seven findings, the wasm gate clean,
and the test-name identity gate clean at every one of the five commits; **`master` after the merge is
514 / 0** — the extra one is the profile-switch test `4a6bab9` added — with the identity gate at
514 / 512. **The plan's own "< 450 production
lines" target for `client.rs` was wrong and is not met**: the four moves shed 481 production lines, not
the ~730 its arithmetic assumed, because `client.rs` keeps `frame_loop`, `on_message`, `on_action`
(both settings arms included), `key_input` and `clock_view`. `session.rs` was **not** split — that task
was dropped deliberately (nothing outside the server needs it) — and `key_input` and `clock_view`
deliberately did **not** move, because `tipsy-ui` may never gain `crossterm`. **The `file:line` debt
this left is paid:** every `file:line` the web-tier plan and its tasks carry for `client.rs` is stale —
the line is in `client/settings.rs`, `client/call.rs`, `client/mic.rs` or `client/audio.rs` now — and
Phase 3's tasks re-measured what they touched instead of trusting the number, each recording the drift
with the tree's truth beside it in the plan's ledger, while the plan's own numbers stay as written under
its front-matter warning. (Phase 2 owed the same and paid it the same way.)

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

**Written and waiting to be executed: the tone floor** — `docs/superpowers/plans/2026-10-12-tipsy-tone.md`
(**3 tasks**) and its spec `docs/superpowers/specs/2026-10-12-tipsy-tone-design.md`, committed at
`dd02353`. The user's requirement: the almost-drunk tone must **prevail over any configuration**, in the
voice and in the text, so the app never loses its grace. The copy is the user's and is **approved
verbatim** (it is quoted in both documents, and implementers reproduce it). The plan's central
correction is that the connector which keeps only a brief's **first sentence** is the **native Gemini**
one, not the shipped default — the default OpenAI-compatible route sends the whole brief — so the floor
is **prepended** to the delivery brief (and never stored in the configuration) while the system prompt
gets it last.

**The user's queue, in the order they were told it would run:** (1) the login widget must go **at once**
on a correct answer instead of waiting for `Enter` — the code paths, the spec sentences and the five
tests it touches are in `.superpowers/sdd/2026-10-11-plan-login-screen/follow-ups.md`, item 1;
(2) **choosing a profile in the configuration screen must apply at once** — recorded in
`.superpowers/sdd/2026-10-10-plan-2b-config-screen-and-auto-mode/follow-ups.md`, beside the hot-reload
request; (3) the tone plan above; (4) the **eight residuals** the Phase 3 re-review parked, the first of
them a comment in `mic.rs` that claims something the browser does not do —
`.superpowers/sdd/2026-10-11-plan-3-web-tier/follow-ups.md`; (5) the requests panel (2B's follow-ups)
and Plan 4 `hands_free`, which still needs about ten minutes of the user's own voice. **The documents a
fresh session should open first** are the tone spec, the tone plan, the Phase 3 ledger
(`.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md`) and its `follow-ups.md`.

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
- **A process the tool *starts* can only write inside the project tree.** Not a permission bug and not
  the app's: the confinement applies to spawned processes, so the binary gets `Acceso denegado` when it
  tries to create `%APPDATA%\tipsy-compadre\config.toml` — or a log, or a config **anywhere** outside
  the repository — while the shell's own file operations succeed in the same directory. That is the real
  mechanism behind the `G:`-build trap above, and it is why a hand-editable configuration has to live
  inside the tree: `TIPSY_CONFIG_DIR` is the way, and `.tipsy-config/` at the repository root is ignored
  for exactly this and holds the demo config the user edits.
- **Run the tests with `-j 1`.** At cargo's default job parallelism this machine dies with
  `STATUS_STACK_BUFFER_OVERRUN`, which looks exactly like a code fault and is not one. Plan 2B's
  ledger records the discovery.
- **A running application locks its own binary.** On Windows cargo cannot replace
  `.target-sandbox\debug\tipsy-compadre.exe` while the user's `serve` is running, and the failure reads
  `error: failed to remove file …` followed by `could not compile … (test "login")` — **with no mention
  of a lock**, which is what makes it a trap. Two full verification runs were lost to it. Stop the app
  before rebuilding, or build into a different target directory.
- **A debug build's embedded `web/` is read from disk, but the path is compile-time.** `rust-embed`
  resolves the folder **absolutely** when compiling, so editing `web/terminal/index.html` reaches a
  running **debug** server built in that tree with no rebuild, while a **release** build embeds it and
  needs one; a debug binary copied elsewhere still serves the tree it was built in. The README's
  sentence says "the directory you run the binary from" (`README.md`) and is **wrong** — the practical
  advice stands, the mechanism does not.
- **A mutation harness that restores a file by byte copy must reset its `LastWriteTime`.** `Copy-Item`
  preserves the backup's timestamp, so the restored source can be older than the artifact built from the
  **mutated** source, cargo judges it fresh, and a stale mutated build is reused — this is three
  differently sized `libtipsy_ui-*.rlib` files in `debug/deps`, with `cargo test --workspace` green and
  `cargo test -p tipsy-web` red. `cargo clean -p <crate>` is the cure.
- **`cargo test -p A -p B` stops at the first failing target**; `--no-fail-fast` is needed to see the
  second. And when counting clippy's findings, count the `-->` **source locations**, not `^warning:`
  lines: a dependency's build-script warning (`ring@… ToolExecError`) has no location and inflates a
  naive count.
- **`E:` fills up, and it is the wrong place for a full build of this workspace.** A fresh `tipsy-web`
  build with Ratzilla, web-sys and the wasm target took it to zero. `G:` is where the workspace's
  `.cargo/config.toml` sends builds, and it is the sandboxed shell's `CARGO_TARGET_DIR` override
  (`.target-sandbox`, above) that fills `E:`. Delete `debug/incremental` and the `.pdb` files.
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
