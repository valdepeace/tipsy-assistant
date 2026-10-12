# The tone floor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The application is *tipsy* — that is its name and its joke. A user may change the voice, and
should be able to; but the almost-drunk tone must prevail over any configuration, so the application
never loses its grace. Two approved sentences, one per language, are composed into **every** profile's
delivery brief (first, because a speech connector condenses to the first sentence) and into **every**
profile's system prompt (last, so the persona is untouched and first) — at the moment the request is
built, stored nowhere, and shown read-only on the configuration screen.

**Architecture:** One new module and three call sites. `crates/tipsy-ui/src/tone.rs` holds the two
approved sentences and the two pure functions that compose them; it lives in `tipsy-ui` because that
is the crate **both** the server and the configuration screen already compile against, so the copy has
exactly one home and the screen quotes the bytes the server sends. `session::turn_profile` (the single
funnel from a profile to what a turn needs) composes both halves for the brain, and
`CallSetup::from_config` composes the text half for the realtime session — the only other place in the
tree that turns a profile's prompt into a request. `Config`, `config.toml` and the wire are untouched:
the floor is not a setting and cannot be turned off. The configuration screen gains **one read-only
row** on the Profiles tab, in the profile's language, so a user who edits their brief learns why the
tone cannot be removed.

**Tech Stack:** unchanged. Rust `edition = "2024"`, pinned by `rust-toolchain.toml`. No crate is
added, no dependency is added, `PROTOCOL_VERSION` stays `1`. The floor is `&str`/`String` work over
`tipsy_protocol::UiLanguage`, which `tipsy-ui` already depends on
(`crates/tipsy-ui/Cargo.toml:9`).

**Spec:** [`docs/superpowers/specs/2026-10-12-tipsy-tone-design.md`](../specs/2026-10-12-tipsy-tone-design.md)
— read it first. §3 is the measurement that decides the whole design (the connector that condenses),
§4 the call sites, §5 the copy and its language, §6 the text half, §7 the screen, §8 the ways this can
be green and wrong, §9 the acceptance criteria, §10 the gates.

## The copy, verbatim — the user's text, quoted once here so no task has to hunt for it

**Spanish, 130 characters:**

```text
Voz de compadre casi borracho: dicción pastosa, tempo lento y guasa — sea cual sea la voz elegida, no suavices ni quites ese tono.
```

**English, 144 characters:**

```text
An almost-drunk compadre's voice: slurred articulation, a slow tempo and dry humour — whatever voice was chosen, never soften or drop that tone.
```

Both were approved by the user on 2026-10-12 and are reproduced **character for character**, em dash
(`—`, U+2014) and apostrophe included. Each is exactly one sentence, and each has exactly one
terminator, at its last character (measured: index 129 of 130, index 143 of 144) — that is what makes
the sentence survive a connector that keeps only the first sentence. **An implementer who would phrase
either sentence differently says so in their report; they do not re-word it** (CLAUDE.md: the profile
copy is the user's to approve).

## Inputs: what this plan read in the code

Measured on 2026-10-12: first at `5e22919`, then **re-measured at `766ead0`** after Phase 3's
browser-client merge landed mid-authoring. Two files had moved — `crates/tipsy-compadre/tests/session.rs`
(+31 lines into its tests) and `crates/tipsy-ui/src/strings.rs` (+42 lines of new strings) — and their
citations here are the re-measured ones. **Every task re-reads the file it edits before editing it**:
this repository's plans have been stale in this exact way before, and the tree has moved under at least
one of them.

- `crates/tipsy-compadre/src/providers/gemini.rs:315-325` — `first_sentence(text: &str) -> &str`, the
  condenser: "the first sentence of the profile's `voice_instructions`. The API wants a short style and
  Google's guidance says a long style paragraph is the commonest cause of voice drift". It stops at the
  first `.`, `!`, `?` or `…`. **Private to the module**, which is why the acceptance test lives in it.
- `crates/tipsy-compadre/src/providers/gemini.rs:427-438` — `GeminiSpeech::request_body`:
  `let style = first_sentence(&req.instructions);` (`:428`), the `speech_metadata` annotation (`:431`),
  and the voice as a *name* in `generation_config.speech_config.voice` (`:436`).
- `crates/tipsy-compadre/src/providers/gemini.rs:533-537`, `:548-555`, `:565-580` — the two pins:
  `BRIEF` is two sentences and `STYLE` its first; `first_sentence(BRIEF) == STYLE`; and the loop over
  all nine built-ins asserting each brief condenses to a non-empty style shorter than the brief.
- `crates/tipsy-compadre/src/providers/openai_compat.rs:122-130` — `speech_quirks`: `google/…` →
  `Pcm24k` and **instructions sent**; `openai/…` → `Mp3` and instructions sent; anything else → `Mp3`
  and **instructions dropped** (`:144-146` sends the field only when the quirk allows it).
- `crates/tipsy-compadre/src/providers/elevenlabs.rs:244-245` — "`req.instructions` is deliberately not
  sent: the note records no general instructions/style field".
- `crates/tipsy-compadre/src/providers/mod.rs:56-61` — `SpeechRequest { text, instructions, voice }`;
  `:138-145` the trait; `:385` `probe_chat` (no system message) and `:404-422` `probe_speech`
  (`instructions: String::new()`, `:409`).
- `crates/tipsy-compadre/src/config/model.rs:160-168` — the shipped default speech provider is
  `ProviderConfig::openrouter("google/gemini-3.8-flash-tts")`: kind `OpenaiCompat` (`:124-131`), and
  the model string is what makes `voice_key()` answer `"gemini"` (`:133-143`). `:235-249` `Profile`;
  `:262-270` `Config::active`; `:272-280` `with_builtin_profiles`.
- `crates/tipsy-compadre/src/config/profiles.rs:21-23` — the nine built-ins, with the languages at
  `:29`, `:85`, `:133`, `:180` (`es-ES`), `:229`, `:276` (`en-US`), `:322` (`en-AU`), `:367` (`en-GB`),
  `:412` (`en-CA`). **Two primary subtags, `es` and `en`.**
- `crates/tipsy-compadre/src/config/store.rs:224-233` — `user_override_of_builtin_wins`; `:235-245`
  the other eight survive.
- `crates/tipsy-protocol/src/lib.rs:37-50` — `UiLanguage { Es, En }` and `from_bcp47`: "*Spanish for
  any `es` / `es-*` tag, English for everything else*". `:193` `ProfileEntryView`'s
  `voice_instructions`, the raw copy on the wire.
- `crates/tipsy-compadre/src/session.rs:256-266` — `turn_profile(config) -> TurnProfile`, the single
  funnel; its call sites at `:207` (session construction), `:810` (`reconfigure`, i.e. every profile
  switch) and `:1265` (the login-session test harness). `:465-473` the greeting is an ordinary
  `TurnInput::Say` turn and takes the brief from the same place. Imports at `:7-26`.
- `crates/tipsy-compadre/src/brain.rs:39-44` — `TurnProfile { prompt, voice_instructions, voice,
  language }`; `:217-221` the `SpeechRequest` and `:423` the per-sentence clone; `:363-369` `messages`,
  which pushes `ChatMessage::system(self.profile.prompt.clone())` at `:365`.
- `crates/tipsy-compadre/src/realtime/mod.rs:78-103` — `CallSetup::from_config`, with
  `instructions: config.active().prompt.clone()` at `:95` and the doc comment at `:79-85` saying the
  voice comes from `[realtime] voice`; `:754-773` the test that pins it (`:762`).
- `crates/tipsy-compadre/src/realtime/socket.rs:420-438` — the `session.update` body: `instructions`
  is the only text field it carries.
- `crates/tipsy-compadre/tests/session.rs:67-94` — `FakeSpeech` records every `SpeechRequest`
  (`:83`); `:222-313` the drain helpers; `:679-702`
  `switching_profile_changes_the_prompt_of_the_next_turn`, whose assertion at `:701` compares the
  system message to the raw prompt.
- `crates/tipsy-ui/src/config_form.rs` — `PROFILE_FIELD_COUNT = 7` (`:32-35`); `FormRow` with its
  `editable` flag (`:67-75`); `ProfileForm` (`:147-159`); `voice_label_row` (`:488-493`);
  `profile_row_count` (`:495-500`); `string_keys` (`:533-586`); `enter_action`/`focused_editable`/
  `focus_row`/`activate`/`toggle` (`:790-893`); `commit_profile` (`:926-961`); `profile_rows`
  (`:1166-1215`); the rewritten test (`:1483-1510`) and `each_profile_row_commits_its_own_field`
  (`:1682`); `both_string_tables_carry_every_key_the_screen_uses` (`:1951-1978`).
- `crates/tipsy-ui/src/render_config.rs:164-193` — the row loop, one line per row; `:170` the label
  column (`(body.width / 3).clamp(12, 26)`); `:237` the value cut at the column's edge; `:477-499`
  `every_tab_draws_its_title_its_rows_and_the_help_line`, which asserts at `:491-493` that every row's
  **whole** label and value are drawn at 120 columns.
- `crates/tipsy-ui/src/strings.rs:5-…` (`Strings`), `:51`, `:194`, `:287` (`cfg_voice_instructions` in
  the struct, the Spanish table at `:153` and the English at `:246`), `:341-346` (`strings(lang)`).

## Decisions this plan makes, and why

The spec's §2 holds the full table with the measurement that rejects each alternative. Restated here
so this document can be read alone:

1. **The floor is composed at the two places a request is built and stored nowhere.**
   `session::turn_profile` and `CallSetup::from_config`. `Config`, `config.toml` and `ProfileForm` keep
   the user's copy byte for byte, so a save→load cycle cannot grow the brief by one sentence per cycle
   and nothing can edit the floor away.
2. **Prepend for the voice, append for the text.** `GeminiSpeech::request_body` keeps only
   `first_sentence(&req.instructions)` (`providers/gemini.rs:428`), so an appended floor is discarded
   there and the profile's own first sentence is the entire style the API reads. The text half is
   appended so that `composed.starts_with(prompt)` is true and checkable — the operational meaning of
   "do not corrupt a user's prompt".
3. **The copy has one home, in `tipsy-ui`.** The server and the screen both read `tone::floor`; the
   screen's row shows the bytes the server sends. `Strings` gains only the row's **label**.
4. **The language is the profile's**, by the rule that already exists —
   `UiLanguage::from_bcp47` ("Spanish for any `es`/`es-*` tag, English for everything else"). The nine
   built-ins declare only `es` and `en` (measured above); a third language gets the English sentence,
   because the approved copy is two sentences and inventing a third would be writing profile copy.
5. **One approved sentence per language serves both halves** (D5 in the spec). The requirement said so,
   and a second sentence would be copy nobody approved.
6. **The screen shows it, read-only** (D7), and that is a decision rather than an omission: a screen
   that lets someone edit the brief while saying nothing about the floor teaches that removing the tone
   is possible. It introduces `PROFILE_FLOOR_COUNT = 1`; `PROFILE_FIELD_COUNT` stays `7` so the
   field → field mapping `commit_profile` uses is untouched.
7. **This plan does not touch `tipsy-protocol`, `config/**` or the providers' production code.**
   `providers/gemini.rs` changes in its `mod tests` only.
8. **`pub(crate) fn turn_profile`.** The acceptance criterion is an equality through the *provider's*
   `first_sentence`, which is private to `providers/gemini.rs`; that test therefore drives the real
   composition function, and the widening is one step (`fn` → `pub(crate) fn`), never `pub`.
9. **Commit trailer.** Follow the branch: if `git log -1 --format=%B` on the branch you cut shows a
   `Co-Authored-By:` line, include the same one; if it shows none, do not invent one
   (`2026-10-11-plan-login-screen.md:92-95` records that decision).

## How to schedule this

**Three tasks, three implementers, no per-task reviews.** The budget rule is the repository's
(CLAUDE.md): one implementer subagent per task, **no per-task reviews** — one whole-branch review when
the plan is complete, then **one fix wave** and **one scoped re-review of it**, then the merge. If this
plan is executed in the main checkout, cut a branch first and run the gates the same way.

**Fixed order: 1 → 2 → 3.**

- **Task 1 first, and it is not negotiable.** It creates the module and the functions the other two
  call; the tree does not compile in the middle of it.
- **Tasks 2 and 3 are independent of each other.** Task 2 changes what the server sends; Task 3 changes
  what the screen draws. Either may go first.
- **Task 3's last step writes the record** (`docs/architecture.md`, `CLAUDE.md`), so it must be the last
  commit if you want one commit to own the record. If you would rather land the screen first, move Task
  3 Step 7 into Task 2's commit — that is the one edit this document tolerates, and it changes no code.

**Yours to decide, and none of it changes a task body:** whether the record rides with the last task or
is its own commit; how you interleave this with a fix wave of your own (only the order above is fixed);
and the branch's name.

## Preconditions — do not start Task 1 until all four hold

- [ ] `git status --short` is clean in the checkout you are working in, and you are on your own branch
      (the main checkout is shared; **never** `cargo fmt`, `git stash`, `git checkout -- .`,
      `git restore` or `git reset` anywhere in it).
- [ ] `rustup target list --installed` names `wasm32-unknown-unknown` (Task 1's gate needs it; the
      web-tier plan's Task 3.1 is what installs it).
- [ ] `.target-sandbox/` exists or can be created, and `.superpowers/sdd/2026-10-12-tipsy-tone/` is
      created for the ledger and the constraints copy this plan's executor owns.
- [ ] The baseline test capture of Task 1 Step 1 exists **before** the first edit.

## Global Constraints

Every task inherits these. They are the spec's §10 and the repository's own rules, restated so this
document can be read alone.

- **English** for code, identifiers, comments, docs and commit messages. **The two profile sentences
  are Spanish and English and are reproduced verbatim** — they are the user's copy, not a translation
  exercise.
- **Never run `cargo fmt`.** The repository is not rustfmt-clean and formatting rewrites hundreds of
  unrelated lines.
- **Run the tests with `-j 1`** and with `TEMP`/`TMP` inside the workspace. At cargo's default
  parallelism this machine dies with `STATUS_STACK_BUFFER_OVERRUN`, which looks exactly like a code
  fault and is not one, and the machine's temp directory denies directory creation, which makes every
  `tempfile::tempdir().unwrap()` panic. The canonical shape:

  ```powershell
  $env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP
  $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
  cargo test -j 1 -p <crate> <filter>
  ```

- **Clippy's gate is the finding set, not the exit code.** `cargo clippy --workspace --all-targets`
  exits `0` and reports **seven** pre-existing findings
  (`.superpowers/sdd/2026-10-11-plan-3-web-tier/global-constraints.md:79-94`: `tipsy-protocol/src/lib.rs:473`,
  `crates/tipsy-compadre/src/text.rs:27`, `audio/device.rs:183`, `keys.rs:89`, `keys.rs:93`,
  `providers/sse.rs:43`, `session.rs:484` — measured at `2f25c20`). Compare **the set, never the list**:
  the lines move, and Task 2 moves the `session.rs` one. Re-measure before and after each task and
  record the new citation in your own `.superpowers/sdd/2026-10-12-tipsy-tone/global-constraints.md`.
- **`tipsy-ui` never gains** `tokio`, `cpal`, `rodio`, `keyring`, `sysinfo`, `reqwest` or `crossterm`,
  and `tipsy-ui` and `tipsy-protocol` keep compiling to `wasm32-unknown-unknown`:
  `cargo check -p tipsy-ui -p tipsy-protocol --target wasm32-unknown-unknown`.
- **No dependency is added, to any crate**, and **`PROTOCOL_VERSION` stays `1`**.
- **No test is deleted, ignored or weakened.** Two existing tests change an expected *value* on purpose
  (Task 2 Step 6 and Step 4) and one changes an expected *count* plus gains one named exception inside
  a loop (Task 3 Steps 5 and 6). Each is spelled out below and each is a *strengthening* or a
  *documented* exception, not a relaxation.
- **Unit tests stay inside the file they test** and integration tests live in `crates/tipsy-compadre/tests/`.
- **Terminal first.** The native client is the reference; nothing here changes what the terminal does,
  and the web terminal (the same TUI in a PTY) and Phase 3's browser client get the screen row from
  `tipsy-ui` for free.

## Review Focus

Five failure modes this plan implies and no single task's happy path exercises. The whole-branch review
should be pointed at these.

1. **The floor composed in the wrong order, or composed twice.** The acceptance criterion is an
   equality through `first_sentence` (Task 2 Step 3), not a `contains`, and the "not stored" half is an
   idempotence assertion over a real save→load cycle (Task 2 Step 5). A `contains`-only test would pass
   for an appended clause on the shipped default connector, which sends the whole string.
2. **The user's copy corrupted.** Any diff to `Config::profiles[i].prompt` /
   `.voice_instructions`, to `ProfileForm`, to `ProfileEntryView` or to `config.toml`'s bytes is a
   failure. Pinned by Task 2 Step 5 and Task 3 Step 6's form test.
3. **The floor leaking into the Test probes.** `probe_speech` `instructions` stays `String::new()`
   (`providers/mod.rs:409`) and `probe_chat` keeps sending no system message. A grep for `tone::` must
   name exactly the call sites this plan lists.
4. **The screen's index arithmetic.** `commit_profile` maps a row index to a field; the new read-only
   row sits inside the row range and the voices' offset moves by one. A `usize` underflow there panics;
   an off-by-one writes a voice name into `id`. Pinned by Task 3 Steps 2 and 5.
5. **The exception in `every_tab_draws_its_title_its_rows_and_the_help_line` widening silently.** The
   floor row's value is 130 characters and the value column is 94 at the test's width, so that one
   assertion carries a **label-keyed** exception with its own assertion on the sentence's opening
   (Task 3 Step 6). Any broader skip of that loop is a finding.

---

## File structure

| Path | Responsibility after this plan |
|---|---|
| `crates/tipsy-ui/src/tone.rs` | **New.** The two approved sentences and the two composition rules. ~90 production lines with their tests. |
| `crates/tipsy-ui/src/lib.rs` | `pub mod tone;` beside the other modules. |
| `crates/tipsy-compadre/src/session.rs` | `turn_profile` composes both halves and is `pub(crate)`; one `use`; two new tests. |
| `crates/tipsy-compadre/src/realtime/mod.rs` | `CallSetup::from_config` composes the text half; the existing test's expected value gains the floor. |
| `crates/tipsy-compadre/src/providers/gemini.rs` | **Tests only.** The condenser's acceptance test. |
| `crates/tipsy-compadre/tests/session.rs` | One rewritten assertion, one new integration test. |
| `crates/tipsy-ui/src/strings.rs` | One field, both tables. |
| `crates/tipsy-ui/src/config_form.rs` | `PROFILE_FLOOR_COUNT`, the row, the index arithmetic, `string_keys`, one rewritten test, two new tests. |
| `crates/tipsy-ui/src/render_config.rs` | The new label joins the "every label reaches a tab" list; the row's cut value gets one named exception. |
| `docs/architecture.md`, `CLAUDE.md` | Task 3: the record says what landed. |

---

## Task 1: `tipsy_ui::tone` — the two approved sentences and the two rules

**Why first.** It creates the module Tasks 2 and 3 call. Nothing else compiles without it.

**Files:**
- Create: `crates/tipsy-ui/src/tone.rs`
- Modify: `crates/tipsy-ui/src/lib.rs` (one line)
- Test: the `#[cfg(test)] mod tests` of `crates/tipsy-ui/src/tone.rs`

**Interfaces:**
- Consumes: `tipsy_protocol::UiLanguage` (already a `tipsy-ui` dependency, `crates/tipsy-ui/Cargo.toml:9`).
- Produces, for Task 2, Task 3 and Phase 3's browser client:

  ```rust
  pub const TONE_ES: &str;
  pub const TONE_EN: &str;
  pub fn floor(language: &str) -> &'static str;
  pub fn brief_with_floor(language: &str, brief: &str) -> String;
  pub fn prompt_with_floor(language: &str, prompt: &str) -> String;
  ```

- [ ] **Step 1: Capture the baseline before touching anything**

The gate every remaining task repeats. Write the file under `.target-sandbox/` (git-ignored) so it does
not show up in `git status` for the rest of the plan.

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

Run: the block above. Expected: a count. **No number is quoted here**: the tree is at `5e22919` and
CLAUDE.md's last recorded count stops earlier. The number you measure is the one that matters.

- [ ] **Step 2: Create `crates/tipsy-ui/src/tone.rs`**

```rust
//! The tone floor: the one thing a profile may never lose.
//!
//! The application is *tipsy*, and that is its joke. A user may change the voice — the profile's
//! `voices` table, `[realtime] voice`, whatever they write in the delivery brief — but the
//! almost-drunk tone prevails over every one of those, so the application never loses its grace.
//!
//! The floor is two sentences, one per language, approved by the user on 2026-10-12 and reproduced
//! here verbatim. It is **not a setting**: there is no key for it in `config.toml`, no switch on the
//! configuration screen and no way to turn it off. It is composed on the way to a request, by
//! [`brief_with_floor`] (the voice half) and [`prompt_with_floor`] (the writing half), and it is
//! stored nowhere: `Config` keeps exactly the copy the user wrote.
//!
//! **Why it goes first in the brief.** One speech connector keeps only the first sentence of the
//! delivery instructions: `GeminiSpeech::request_body` calls `first_sentence(&req.instructions)`
//! (`crates/tipsy-compadre/src/providers/gemini.rs:428`, condensed at `:318-325`, pinned by the tests
//! at `:548` and `:565`). A clause appended to a brief is therefore discarded there, and the profile's
//! own first sentence is what the API reads instead. The clause is prepended, and it is a complete
//! instruction on its own, so a connector that keeps only the first sentence still gets all of it.
//!
//! The language rule is not re-derived here: `UiLanguage::from_bcp47` is "Spanish for any `es` /
//! `es-*` tag, English for everything else" (`crates/tipsy-protocol/src/lib.rs:44-50`), and it is the
//! same rule that selects the interface table and the greeting's language.

use tipsy_protocol::UiLanguage;

/// The approved Spanish floor, verbatim (user, 2026-10-12). Its first sentence is the whole sentence:
/// it is what a connector that condenses sends as the style.
pub const TONE_ES: &str =
    "Voz de compadre casi borracho: dicción pastosa, tempo lento y guasa — sea cual sea la voz elegida, no suavices ni quites ese tono.";

/// The approved English floor, verbatim (user, 2026-10-12), for every profile whose language is not
/// Spanish.
pub const TONE_EN: &str =
    "An almost-drunk compadre's voice: slurred articulation, a slow tempo and dry humour — whatever voice was chosen, never soften or drop that tone.";

/// The floor sentence for a profile that declares `language`.
///
/// Spanish for any `es` / `es-*` tag and English for everything else — the rule is
/// `UiLanguage::from_bcp47`'s and is not re-derived here, so the screen and the server cannot disagree
/// about which sentence a profile gets. That is also what an invented profile with `fr-FR`, or one
/// with no language at all, gets: the approved copy is two sentences, and inventing a third is not
/// this module's to do.
pub fn floor(language: &str) -> &'static str {
    match UiLanguage::from_bcp47(language) {
        UiLanguage::Es => TONE_ES,
        UiLanguage::En => TONE_EN,
    }
}

/// The delivery brief a speech connector receives: the floor first, the profile's own brief after it.
///
/// The floor is a whole sentence with its own full stop, so a connector that condenses to the first
/// sentence keeps exactly the floor. A brief with nothing in it — the profile a user wrote from
/// nothing, or one they cleared — composes to the floor alone.
///
/// **No de-duplication.** The brief goes on the wire as the user wrote it, byte for byte, after the
/// floor. There is nowhere for a floor to already be (it is composed here and stored nowhere), and a
/// "the brief already starts with the floor" branch would be a second rule no input can reach.
pub fn brief_with_floor(language: &str, brief: &str) -> String {
    let floor = floor(language);
    if brief.trim().is_empty() { floor.to_string() } else { format!("{floor} {brief}") }
}

/// The system prompt a model receives: the profile's own prompt, then the floor.
///
/// **Appended, never prepended, and this is the "do not corrupt a user's prompt" rule.** The composed
/// prompt starts with the user's own prompt byte for byte, so the persona is the first thing the model
/// reads and not one character of it moved. A prompt with nothing in it composes to the floor alone: a
/// profile with nothing to say still keeps the register.
///
/// The two halves are ordered oppositely on purpose. The voice half is first because a speech
/// connector that condenses a paragraph keeps its *first* sentence; the text half is last because a
/// model reads a whole system prompt, where the persona must come first and the reinforcement must not
/// displace it.
pub fn prompt_with_floor(language: &str, prompt: &str) -> String {
    let floor = floor(language);
    if prompt.trim().is_empty() { floor.to_string() } else { format!("{prompt}\n\n{floor}") }
}
```

- [ ] **Step 3: Declare the module**

In `crates/tipsy-ui/src/lib.rs`, after `pub mod strings;`:

```rust
pub mod tone;
```

**No glob re-export and no `pub use`.** Callers name the module — `tone::brief_with_floor(…)` — which
keeps the reach of the floor visible at every call site, which is the whole point of Review Focus 3.

Run: `cargo check -j 1 -p tipsy-ui`
Expected: PASS. (If it fails naming `tone`, the module declaration is missing; if it fails naming
`UiLanguage`, `tipsy-ui`'s `Cargo.toml` was edited, which this task forbids.)

- [ ] **Step 4: Write the tests this module owns**

Add to the bottom of `crates/tipsy-ui/src/tone.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The copy is the user's, verbatim, and a later edit to it is a decision — not a tidy-up. The
    /// "exactly one terminator, at the end" assertion is what a connector that condenses depends on:
    /// it is the reason the rest of a brief can never become the style. The equality against the *real*
    /// condenser is asserted in `crates/tipsy-compadre/src/providers/gemini.rs`, which owns it; this
    /// module may not grow a second one.
    #[test]
    fn the_copy_is_the_two_sentences_the_user_approved() {
        assert_eq!(
            TONE_ES,
            "Voz de compadre casi borracho: dicción pastosa, tempo lento y guasa — sea cual sea la voz elegida, no suavices ni quites ese tono."
        );
        assert_eq!(
            TONE_EN,
            "An almost-drunk compadre's voice: slurred articulation, a slow tempo and dry humour — whatever voice was chosen, never soften or drop that tone."
        );
        assert_ne!(TONE_ES, TONE_EN);
        for sentence in [TONE_ES, TONE_EN] {
            assert!(sentence.ends_with('.'), "a style has to be a whole sentence: {sentence}");
            assert_eq!(sentence.trim(), sentence, "no stray whitespace reaches an annotation");
            assert_eq!(
                sentence.matches(|c: char| matches!(c, '.' | '!' | '?' | '…')).count(),
                1,
                "one terminator, at the end, or a condensing connector reads past the floor: {sentence}"
            );
        }
    }

    #[test]
    fn the_floor_is_chosen_by_the_profiles_language() {
        for spanish in ["es", "es-ES", "es-419", "ES-es"] {
            assert_eq!(floor(spanish), TONE_ES, "{spanish}");
        }
        // Everything that is not Spanish gets the English sentence: the same rule the interface table
        // and the greeting's language already apply.
        for other in ["en", "en-GB", "en-AU", "fr-FR", "", "   "] {
            assert_eq!(floor(other), TONE_EN, "{other:?}");
        }
    }

    #[test]
    fn the_floor_opens_the_brief_and_the_brief_survives_it() {
        let brief = "Voz masculina adulta, medio-grave y cálida.";
        let composed = brief_with_floor("es-ES", brief);
        assert!(composed.starts_with(TONE_ES), "{composed}");
        assert_eq!(&composed[TONE_ES.len()..], format!(" {brief}"), "the user's brief, byte for byte");
    }

    #[test]
    fn a_brief_with_nothing_in_it_is_the_floor_alone() {
        for empty in ["", "   ", "\n"] {
            assert_eq!(brief_with_floor("es-ES", empty), TONE_ES, "{empty:?}");
            assert_eq!(brief_with_floor("en-GB", empty), TONE_EN, "{empty:?}");
        }
    }

    #[test]
    fn a_brief_that_contradicts_the_floor_still_has_the_floor_first() {
        let mutiny = "Habla como un presentador de telediario, formal y sobrio.";
        let composed = brief_with_floor("es-ES", mutiny);
        assert!(composed.starts_with(TONE_ES), "the floor cannot be argued with: {composed}");
        assert!(composed.ends_with(mutiny), "and the user's brief is still there, after it");
        assert!(composed.find(TONE_ES).unwrap() < composed.find(mutiny).unwrap());
    }

    #[test]
    fn a_prompt_is_appended_to_and_never_corrupted() {
        let prompt = "Eres «el Compae», un asistente andaluz.";
        let composed = prompt_with_floor("es-ES", prompt);
        assert!(composed.starts_with(prompt), "the persona is first, byte for byte: {composed}");
        assert_eq!(&composed[prompt.len()..], format!("\n\n{TONE_ES}"));
        for empty in ["", "  "] {
            assert_eq!(prompt_with_floor("es-ES", empty), TONE_ES);
        }
        assert!(
            prompt_with_floor("en-GB", prompt).ends_with(TONE_EN),
            "the profile's language decides the sentence on this surface too"
        );
    }
}
```

- [ ] **Step 5: The tests, and the wasm gate**

Run: `cargo test -j 1 -p tipsy-ui tone`
Expected: PASS, **6** tests, 0 failures — the filter matches the module path, so exactly these six names
are listed: `the_copy_is_the_two_sentences_the_user_approved`,
`the_floor_is_chosen_by_the_profiles_language`, `the_floor_opens_the_brief_and_the_brief_survives_it`,
`a_brief_with_nothing_in_it_is_the_floor_alone`,
`a_brief_that_contradicts_the_floor_still_has_the_floor_first`,
`a_prompt_is_appended_to_and_never_corrupted`. If a name is missing, the filter is wrong, not the test.

Run: `cargo check -p tipsy-ui -p tipsy-protocol --target wasm32-unknown-unknown`
Expected: PASS. If it fails, the module reached for something a browser cannot build — most likely a
clock or a filesystem call it must not have. Fix the module, not the manifest:
`crates/tipsy-ui/Cargo.toml` gains no dependency in this task.

- [ ] **Step 6: The identity gate and the suite**

Run the Task 1 Step 1 commands again with `after-tests.txt`, then:

```powershell
"before: " + (Get-Content "$PWD\.target-sandbox\baseline-tests.txt").Count +
  " / after: " + (Get-Content "$PWD\.target-sandbox\after-tests.txt").Count
Compare-Object (Get-Content "$PWD\.target-sandbox\baseline-tests.txt") `
               (Get-Content "$PWD\.target-sandbox\after-tests.txt")
```

Expected: the count grows by exactly **6**, and `Compare-Object` prints only those six additions
(`a_brief_that_contradicts_the_floor_still_has_the_floor_first`,
`a_brief_with_nothing_in_it_is_the_floor_alone`, `a_prompt_is_appended_to_and_never_corrupted`,
`the_copy_is_the_two_sentences_the_user_approved`, `the_floor_is_chosen_by_the_profiles_language`,
`the_floor_opens_the_brief_and_the_brief_survives_it`) and nothing else. **A baseline name that
disappears stops this task**: find it.

Run: `cargo test -j 1 -p tipsy-ui`
Expected: PASS, 0 failures.

Run: `cargo clippy --workspace --all-targets`
Expected: exit `0` with **the same seven findings** and none new, compared as a set. This task moves no
line in `session.rs`, so that citation does not move here.

- [ ] **Step 7: Commit**

```bash
git add crates/tipsy-ui/src/tone.rs crates/tipsy-ui/src/lib.rs
git commit -m "feat(ui): the tone floor, and the two rules that compose it"
```

Body, in the repository's style: why the module is in `tipsy-ui` (both the server and the screen
compile against it, so the copy has one home), why the clause is prepended for the voice and appended
for the text (the condensing connector), and that the copy is the user's, verbatim.

---

## Task 2: the server composes the floor into every profile's brief and prompt

**Why second.** It is the requirement's substance: every profile, every mode, every turn. It changes no
wire shape and touches no configuration type.

**Files:**
- Modify: `crates/tipsy-compadre/src/session.rs` (`turn_profile`, one `use`, two tests)
- Modify: `crates/tipsy-compadre/src/realtime/mod.rs` (`from_config`, one `use`, one existing test)
- Modify: `crates/tipsy-compadre/src/providers/gemini.rs` (its `mod tests` only)
- Modify: `crates/tipsy-compadre/tests/session.rs` (one rewritten assertion, one new test)
- Test: all four

**Interfaces:**
- Consumes: `tipsy_ui::tone::{brief_with_floor, prompt_with_floor, floor, TONE_ES, TONE_EN}` (Task 1);
  `crate::config::{Config, Profile}`; `crate::brain::TurnProfile`.
- Produces: `pub(crate) fn turn_profile(config: &Config) -> TurnProfile` whose `prompt` and
  `voice_instructions` carry the floor; `CallSetup::instructions` likewise. **No wire change:
  `PROTOCOL_VERSION` stays `1`, `ProfileEntryView` still carries the user's raw copy.**

- [ ] **Step 1: Compose both halves in `turn_profile`**

In `crates/tipsy-compadre/src/session.rs`, replace the body of `turn_profile` (`:256-266`) — its
existing doc comment stays and gains the three paragraphs below:

```rust
/// What the brain needs from the configuration: the active profile's texts and the voice the speech
/// connector recommends for it. Built when the session starts and again on every `SetProfile`.
///
/// **The tone floor is composed here and nowhere else on this path** (`tipsy_ui::tone`): the delivery
/// brief gets it first, because a connector that condenses keeps the first sentence
/// (`providers/gemini.rs:428`), and the system prompt gets it last, so the persona is unchanged and
/// first. `Config` keeps the user's own copy — this function takes `&Config`, and the file is written
/// from `Config` alone — so a floor written into the file could not happen here and a save→load cycle
/// cannot grow the texts by one sentence a round trip.
///
/// `pub(crate)` because the condenser's own test drives this function: `first_sentence` is private to
/// `providers/gemini.rs`, and the acceptance criterion is an equality through *that* function and not
/// through a second one written for the test.
pub(crate) fn turn_profile(config: &Config) -> TurnProfile {
    let profile = config.active();
    TurnProfile {
        prompt: tone::prompt_with_floor(&profile.language, &profile.prompt),
        voice_instructions: tone::brief_with_floor(&profile.language, &profile.voice_instructions),
        voice: profile.voice_for(&config.providers.speech),
        language: profile.language.clone(),
    }
}
```

Then add the import, with the other external crates (after `use tokio_util::sync::CancellationToken;`
at `:18`, before the `use crate::…` block):

```rust
use tipsy_ui::tone;
```

Run: `cargo check -j 1 -p tipsy-compadre`
Expected: PASS.

- [ ] **Step 2: Compose the text half for the realtime session**

In `crates/tipsy-compadre/src/realtime/mod.rs`, in `CallSetup::from_config` (`:86-102`), take the
profile once and use it for both fields it feeds:

```rust
    pub fn from_config(config: &Config, resolver: Arc<KeyResolver>, key: String) -> CallSetup {
        let realtime = &config.realtime;
        let profile = config.active();
        // The resolved value is dropped, deliberately not stored. See the doc comment above.
        drop(key);
        CallSetup {
            url: url(&realtime.model),
            options: CallOptions::default(),
            config: realtime.clone(),
            model: realtime.model.clone(),
            // `instructions` is the only text field the session update carries (`socket.rs:420-438`),
            // so it is where the tone floor reaches this mode: the floor is appended to the profile's
            // prompt, which is otherwise exactly what the brain sends as its system message.
            // `[realtime] voice` is the one voice a profile does not map — the row a user changes —
            // and the floor's own words are about that choice ("sea cual sea la voz elegida").
            instructions: tone::prompt_with_floor(&profile.language, &profile.prompt),
            voice: realtime.voice.clone(),
            silence_ms: config.listening.silence_ms,
            language: profile.language.clone(),
            limit: Duration::from_secs(u64::from(realtime.max_minutes) * 60),
            resolver,
        }
    }
```

Add the import with the other external crates (after `use tokio_util::sync::CancellationToken;` at
`:19`):

```rust
use tipsy_ui::tone;
```

Run: `cargo check -j 1 -p tipsy-compadre`
Expected: PASS. (`config.active()` was called twice before and is now called once; `language` is
`profile.language.clone()` rather than `config.active().language.clone()`, which is the same value.)

- [ ] **Step 3: The acceptance test — through the real condenser**

In `crates/tipsy-compadre/src/providers/gemini.rs`'s `mod tests`, add the two imports it needs and the
test below. **This is the task's falsifiable half and the reason `turn_profile` is `pub(crate)`.**

```rust
    use crate::config::{Config, Profile};
```

```rust
    /// **The falsifiable half of the tone floor, and the reason this test lives in this module.**
    ///
    /// `request_body` condenses the delivery brief with `first_sentence` — the same function this test
    /// calls. A test that asked whether the floor appears *somewhere* in the brief would pass for an
    /// implementation that appends it, and an appended clause never reaches the Gemini API at all:
    /// `first_sentence(BRIEF + " " + floor)` is `first_sentence(BRIEF)`. What is asserted here is the
    /// equality the API depends on — the style *is* the floor — and it is asserted through the real
    /// condenser and the real composition function, not through copies of either.
    ///
    /// Both halves of the requirement's own example are here: a profile written from nothing, and a
    /// profile whose brief contradicts the floor.
    #[test]
    fn every_brief_the_speech_provider_receives_opens_with_the_tone_floor() {
        for profile in crate::config::profiles::builtin_profiles() {
            let config = Config { active_profile: profile.id.clone(), ..Config::default() };
            let composed = crate::session::turn_profile(&config);
            assert_eq!(
                first_sentence(&composed.voice_instructions),
                tipsy_ui::tone::floor(&profile.language),
                "{}: the API must read the floor as the style",
                profile.id
            );
        }

        // The profile the requirement is about: one the user wrote from nothing, whose brief is empty,
        // blank, or says the opposite of the floor.
        for brief in ["", "   ", "Habla como un presentador de telediario, formal y sobrio."] {
            let mut config = Config::default();
            config.active_profile = "inventado".into();
            config.profiles.push(Profile {
                id: "inventado".into(),
                language: "es-ES".into(),
                voice_instructions: brief.into(),
                ..Profile::default()
            });
            let composed = crate::session::turn_profile(&config);
            assert_eq!(
                first_sentence(&composed.voice_instructions),
                tipsy_ui::tone::TONE_ES,
                "a brief of {brief:?} must still open with the floor"
            );
        }
    }
```

Two things to check against the tree before you trust it: that `Profile` derives `Default` (it does,
`config/model.rs:235`) and that the test module can reach `first_sentence` through `use super::*` (it
already does, in `condenses_the_voice_brief_to_its_first_sentence`). If a name differs, fix the test to
the tree — the tree wins — and say so in the commit message.

Run: `cargo test -j 1 -p tipsy-compadre every_brief_the_speech_provider_receives_opens_with_the_tone_floor`
Expected: PASS, 1 test. Then **prove it is falsifiable**: change `brief_with_floor`'s `format!("{floor} {brief}")`
to `format!("{brief} {floor}")` temporarily and run it again.
Expected: **FAIL** on `compae`, with the assertion naming the first sentence it got instead. Put the
line back. (That failure is the measurement this whole design rests on; a test that cannot fail this way
is Review Focus 1's failure mode.)

- [ ] **Step 4: Prove the floor is not stored, and the copy is not touched**

In `crates/tipsy-compadre/src/session.rs`'s `mod tests` (beside `reconfiguring_to_another_profile_moves_its_greeting`,
`:1232`), add:

```rust
    /// **The floor is composed on the way to the request and stored nowhere.**
    ///
    /// The first half is the "do not corrupt the user's copy" rule: `turn_profile` takes `&Config`, so
    /// after it has run the configuration still holds the card the user wrote. The second half is what
    /// makes a stored floor impossible to miss — a floor written into the file would be floored again
    /// by the next `turn_profile`, so the texts would grow by one sentence per save→load cycle. One
    /// round trip must not change a character.
    #[test]
    fn the_floor_is_never_written_into_the_configuration() {
        let config = Config { active_profile: "compae".into(), ..Config::default() };
        let card = config.active().clone();
        let composed = turn_profile(&config);

        assert_eq!(config.active(), &card, "the configuration is the user's copy, untouched");
        assert!(
            config.active().voice_instructions.starts_with("Voz masculina adulta"),
            "the card still holds the brief the built-in profile ships"
        );
        assert!(
            composed.voice_instructions.starts_with(tipsy_ui::tone::TONE_ES),
            "and the request carries the floor first"
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        crate::config::store::save(&path, &config).unwrap();
        let reloaded = crate::config::store::load_or_create(&path).unwrap().config;
        let again = turn_profile(&reloaded);

        assert_eq!(
            again.voice_instructions, composed.voice_instructions,
            "one save→load cycle must not grow the brief"
        );
        assert_eq!(again.prompt, composed.prompt, "nor the prompt");
    }
```

Run: `cargo test -j 1 -p tipsy-compadre the_floor_is_never_written_into_the_configuration`
Expected: PASS, 1 test.

- [ ] **Step 5: Rewrite the two assertions that pin the raw prompt**

Both of these currently assert an equality with the **raw** profile text, so both fail until they are
updated — and updating them is the proof the composition reached both surfaces, not a relaxation.

In `crates/tipsy-compadre/tests/session.rs`, in `switching_profile_changes_the_prompt_of_the_next_turn`
(the assertion at `:701`), replace the last two statements:

```rust
    let sent = chat.requests.lock().unwrap()[0].messages.clone();
    assert_eq!(sent[0].role, Role::System);
    let senyor = config().profiles.into_iter().find(|p| p.id == "senyor").unwrap();
    assert_eq!(senyor.language, "es-ES", "this assertion's floor sentence depends on it");
    assert_eq!(
        sent[0].content,
        format!("{}\n\n{}", senyor.prompt, tipsy_ui::tone::TONE_ES),
        "the next turn must speak as the profile just chosen, with the floor appended: {sent:#?}"
    );
```

In `crates/tipsy-compadre/src/realtime/mod.rs`, in `the_call_asks_for_the_configured_model_voice_and_prompt`
(`:760-762`), replace the comment and the assertion:

```rust
        // The profile's own prompt is the session's `instructions` — as the brain sends it as the
        // system message — with the tone floor appended: the prompt is byte for byte first, the floor
        // last, and the profile is still the only thing that says who this is.
        assert_eq!(config.active().language, "es-ES", "this assertion's floor sentence depends on it");
        assert_eq!(
            wanted.instructions,
            format!("{}\n\n{}", config.active().prompt, tipsy_ui::tone::TONE_ES),
            "the floor is appended, and the prompt above it is unchanged"
        );
```

Run: `cargo test -j 1 -p tipsy-compadre switching_profile_changes_the_prompt_of_the_next_turn`
Expected: PASS, 1 test.

Run: `cargo test -j 1 -p tipsy-compadre the_call_asks_for_the_configured_model_voice_and_prompt`
Expected: PASS, 1 test. (`cargo test` takes one filter, hence two commands.) If either still passes
**without** the floor in its expectation, the composition did not reach that surface.

- [ ] **Step 6: The integration proof — the floor reaches `SpeechProvider::synthesize`**

In `crates/tipsy-compadre/tests/session.rs`, beside `ask_produces_reply_and_done` (`:494`), add:

```rust
/// **The floor is not a property of `turn_profile` alone: it is what the provider receives.** A
/// profile whose delivery brief is empty — the case the requirement exists for, a card written from
/// nothing — is driven through the real session and the real brain, and every request the speech
/// provider recorded opens with the floor.
///
/// This is a `starts_with` and not the sentence proof: `first_sentence` is private to
/// `providers/gemini.rs`, and the equality that matters is asserted there against the real condenser.
/// What this test adds is the link the other one cannot see.
#[tokio::test]
async fn a_profile_with_no_brief_still_reaches_the_speech_provider_with_the_floor() {
    let chat = FakeChat::new(vec![Ok(vec!["Una respuesta suficientemente larga."])]);
    let speech = FakeSpeech::new();
    let transcribe = Arc::new(FakeTranscribe {
        text: "",
        languages: Mutex::new(Vec::new()),
        pcms: Mutex::new(Vec::new()),
    });
    let providers = ProviderSlot::new(Providers { chat, speech: speech.clone(), transcribe });

    // `compae` with its delivery brief cleared: the card the requirement is about.
    let mut config = config();
    config
        .profiles
        .iter_mut()
        .find(|p| p.id == "compae")
        .expect("the built-in compae profile")
        .voice_instructions = String::new();

    let hub = detached_hub(config.clone(), providers.clone());
    let ends = spawn_with_events(config, providers, BrainConfig::default(), SessionPolicy::local(), hub);
    let (tx, mut rx) = (ends.to_server, ends.from_server);
    send(&tx, hello(None, PROTOCOL_VERSION));
    next_matching(&mut rx, is_done, 500).await; // the greeting, which is a turn like any other

    let requests = speech.requests.lock().unwrap().clone();
    assert!(!requests.is_empty(), "the greeting synthesises the profile's own words");
    for request in &requests {
        assert!(
            request.instructions.starts_with(tipsy_ui::tone::TONE_ES),
            "the floor is first in the brief the provider gets: {request:?}"
        );
    }
}
```

Run: `cargo test -j 1 -p tipsy-compadre a_profile_with_no_brief_still_reaches_the_speech_provider_with_the_floor`
Expected: PASS, 1 test.

- [ ] **Step 7: The gates — identity, suite, clippy, and the moved citation**

Run the Task 1 Step 1 capture into `after-tests.txt` and the two-line comparison.
Expected: additions only — the six names of Task 1 plus this task's **three**
(`every_brief_the_speech_provider_receives_opens_with_the_tone_floor`,
`the_floor_is_never_written_into_the_configuration`,
`a_profile_with_no_brief_still_reaches_the_speech_provider_with_the_floor`) — and **no baseline name
lost**. The two rewritten tests keep their names on purpose: they assert new values, not a new subject.

Run: `cargo test -j 1 --workspace`
Expected: PASS, 0 failures.

Run: `cargo clippy --workspace --all-targets`
Expected: exit `0`, **seven** findings as a set, none new. **Re-measure the `session.rs` line**: this
task inserts lines inside `turn_profile`'s doc comment and body (`:256-266`) and inside
`CallSetup::from_config`, so the finding that was at `session.rs:484` moves down. Write the measured
line into `.superpowers/sdd/2026-10-12-tipsy-tone/global-constraints.md` in the same commit — that
file's own rule is that the citation moves and the record follows it.

Run: `cargo test -j 1 -p tipsy-protocol`
Expected: PASS. This task changes no wire shape.

- [ ] **Step 8: Commit**

```bash
git add crates/tipsy-compadre/src/session.rs crates/tipsy-compadre/src/realtime/mod.rs \
        crates/tipsy-compadre/src/providers/gemini.rs crates/tipsy-compadre/tests/session.rs
git commit -m "feat(server): every profile's brief and prompt carry the tone floor"
```

Body: the two call sites and why they are the two, the prepend/append asymmetry and the condensing
connector that decides it, that `Config` and the wire are untouched, and that the two rewritten
assertions are the proof the change reached both surfaces. Step 7's file,
`.superpowers/sdd/2026-10-12-tipsy-tone/global-constraints.md`, is **git-ignored and cannot be in the
commit**: edit it in the checkout and say so in the commit body.

---

## Task 3: the configuration screen shows the floor, and the record says what landed

**Why third.** It is the half the user sees, and it is the last commit, so the record can describe a
branch in which both halves landed.

**Files:**
- Modify: `crates/tipsy-ui/src/strings.rs` (one field, both tables)
- Modify: `crates/tipsy-ui/src/config_form.rs` (the row and the index arithmetic; one rewritten test and two new ones)
- Modify: `crates/tipsy-ui/src/render_config.rs` (one label in a list; one named exception and its own assertion)
- Modify: `docs/architecture.md`, `CLAUDE.md` (the record)
- Test: `config_form.rs`'s and `render_config.rs`'s `mod tests`

**Interfaces:**
- Consumes: `crate::tone::floor` (Task 1); `Strings`' two tables.
- Produces: `pub const PROFILE_FLOOR_COUNT: usize = 1;`, the floor row in `profile_rows`, and the
  matching arithmetic in `voice_label_row`, `profile_row_count` and `commit_profile`. Nothing else
  consumes it: the row is drawn from `ConfigForm::rows`, the renderer already handles `editable: false`,
  and **no new arm is needed in `toggle`, `activate`, `enter_action`, `focus_row` or `apply`.**

- [ ] **Step 1: The label, in both tables**

In `crates/tipsy-ui/src/strings.rs`, after `pub cfg_voice_instructions: &'static str,` (`:51`):

```rust
    /// The Profiles tab's one read-only row: the tone floor, which is composed into every request and
    /// can be neither edited nor removed. Its *value* is not a table entry — it is the approved
    /// sentence, read from `crate::tone` in the profile's own language.
    pub cfg_tone_floor: &'static str,
```

In the Spanish table (`pub static ES: Strings`, `:153`), after
`cfg_voice_instructions: "Instrucciones de voz",` (`:194`):

```rust
    cfg_tone_floor: "Tono intocable",
```

In the English table (`pub static EN: Strings`, `:246`), after
`cfg_voice_instructions: "Voice instructions",` (`:287`):

```rust
    cfg_tone_floor: "Untouchable tone",
```

The **label is interface copy**, like its neighbours, and it is not the floor: an implementer who would
phrase it differently says so in their report and does not re-word the sentence.

- [ ] **Step 2: The row, and the arithmetic that keeps it out of the fields**

In `crates/tipsy-ui/src/config_form.rs`:

**a. The import**, with the other `use crate::…` lines (after `use crate::strings::{strings, Strings};`
at `:14`):

```rust
use crate::tone;
```

**b. The constant**, after `PROFILE_FIELD_COUNT` (`:35`):

```rust
/// The one row of the active profile that is not a field: the tone floor, which is composed into every
/// request (`crate::tone`) and is not a `ProfileForm` field because it is stored nowhere. It sits
/// between the field rows and the voice entries, so the field → field mapping `commit_profile` uses is
/// untouched and only the voices' offset moves by one.
pub const PROFILE_FLOOR_COUNT: usize = 1;
```

**c. `voice_label_row`** (`:491-493`) — the doc comment gains "and the floor row", the body becomes:

```rust
    pub fn voice_label_row(&self) -> usize {
        self.profiles.len() + PROFILE_FIELD_COUNT + PROFILE_FLOOR_COUNT
    }
```

**d. `profile_row_count`** (`:498-500`):

```rust
    pub fn profile_row_count(form: &ConfigForm) -> usize {
        form.profiles.len() + PROFILE_FIELD_COUNT + PROFILE_FLOOR_COUNT + 2 * active_voices(form).len()
    }
```

A form with **no** profiles already disagrees with this count — `rows()` is empty there because every
row lives inside `if let Some(profile) = self.active()` (`:1185-1213`), while the count still adds the
field rows. That is pre-existing, it is recorded in Plan 2B's notes, and it is **not this task's to
fix**: leave it exactly as it is.

**e. `profile_rows`** (`:1185-1213`) — between the field loop and the voices loop, inside
`if let Some(profile) = self.active() { … }`:

```rust
            // The floor: the one row here that cannot be edited, and the sentence itself is the reason
            // why. It is not a `ProfileForm` field — the floor is composed on the way to a request and
            // stored nowhere — so it is read from `tone` in the profile's own language, which is what
            // decides which of the two approved sentences the server will send.
            rows.push(FormRow {
                label: es.cfg_tone_floor,
                value: tone::floor(&profile.language).to_string(),
                editable: false,
                is_active: false,
            });
```

**f. `commit_profile`** (`:926-961`) — the doc comment's row order gains "then the floor row", and the
body gains the guard and loses one constant from the offset:

```rust
        if self.field < list_len + PROFILE_FIELD_COUNT {
            return match self.field - list_len {
                0 => self.fill(|p| Some(&mut p.name), buffer),
                1 => self.fill(|p| Some(&mut p.language), buffer),
                2 => self.fill(|p| Some(&mut p.prompt), buffer),
                3 => self.fill(|p| Some(&mut p.voice_instructions), buffer),
                4 => self.fill(|p| Some(&mut p.greeting), buffer),
                5 => self.fill(|p| Some(&mut p.wake_word), buffer),
                6 => self.write_active_id(buffer),
                _ => false,
            };
        }
        // The floor row is the one row between the fields and the voices, and it holds no text. The
        // guard is not a formality: the subtraction below takes **both** constants off, and on this
        // row that is a `usize` underflow.
        if self.field < list_len + PROFILE_FIELD_COUNT + PROFILE_FLOOR_COUNT {
            return false;
        }
        // Two rows per voice entry: the connector (a list, not text) and the name. Only the name
        // row commits, so an even offset is the connector and writes nothing.
        let offset = self.field - list_len - PROFILE_FIELD_COUNT - PROFILE_FLOOR_COUNT;
```

**g. `string_keys`** (`:539-585`) — add the new key after `es.cfg_voice_instructions,`:

```rust
            es.cfg_tone_floor,
```

**No new arm is added anywhere else**, and that is a claim this task tests rather than assumes: the row
is not editable, so `enter_action` resolves it to `Toggle` (`:793-795`) and the Profiles tab's `toggle`
raises nothing (`:891`); a click resolves it through `focus_row` (`:824-836`) to `activate`, whose
fallback for a non-action row raises nothing (`:867-869`); `BeginEdit` checks `focused_editable`
(`:701-705`). If any of those three needs an edit, stop and report it — one of them is not what this
plan read.

Run: `cargo check -j 1 -p tipsy-ui`
Expected: PASS.

- [ ] **Step 3: Rewrite the row-count test**

In `crates/tipsy-ui/src/config_form.rs`'s `mod tests`, in
`the_profiles_tab_shows_the_active_marker_before_the_fields_and_the_voices_last` (`:1483-1510`), the
fixture has two profiles, seven field rows, the floor row and one voice entry's two rows:

```rust
        // Two profile-list rows, the active profile's seven field rows, the floor row and its one
        // voice's two rows.
        let count = ConfigForm::profile_row_count(&f);
        assert_eq!(count, 12);
        assert_eq!(f.rows().len(), count);
        let rows = f.rows();
        assert_eq!(rows[0].label, ES.cfg_profile);
        assert_eq!(rows[0].value, "El Compae", "the list shows the names, one row per profile");
        assert!(!rows[0].is_active);
        assert!(rows[1].value.contains("O Meigo"));
        assert!(rows[1].value.contains("◉"), "the active profile is marked in the row itself");
        assert!(rows[1].is_active);
        assert_eq!(rows[2].label, ES.cfg_name, "the fields are the same for every profile");
        assert_eq!(rows[2].value, "O Meigo");
        let voices_at = f.voice_label_row();
        assert_eq!(voices_at, 10, "two list rows, seven field rows and the floor row come first");
        assert_eq!(rows[voices_at - 1].label, ES.cfg_tone_floor, "the floor is the row before them");
        assert_eq!(rows[voices_at - 1].value, crate::tone::TONE_ES, "in the profile's language");
        assert!(!rows[voices_at - 1].editable, "and it is not text: nothing can be typed into it");
        assert_eq!(rows[voices_at].label, ES.cfg_voices, "the voice rows are last");
        assert_eq!(rows[voices_at].value, "gemini");
        assert_eq!(rows[rows.len() - 1].value, "Puck", "the voices shown are the active profile's");
```

The two changed numbers are the new truth (`11 → 12`, `9 → 10`) and the three added assertions are what
makes the new row's *position* pinned rather than counted. The rest of the test is unchanged.

- [ ] **Step 4: Two new tests — the row edits nothing, and it follows the language**

In the same `mod tests`:

```rust
    /// **The floor row is inert by construction, and this is where that is checked rather than
    /// assumed.** It is focusable (the keyboard walks the row list), and every way of touching it —
    /// `Enter`, `Toggle`, a click — must leave the form, the buffer and the profile exactly as they
    /// were. A screen that could be talked into editing the tone floor would be the requirement's
    /// opposite.
    #[test]
    fn the_floor_row_follows_the_profiles_language_and_edits_nothing() {
        let mut f = profiles();
        let before = f.profiles.clone();
        focus(&mut f, ES.cfg_tone_floor);

        assert!(!f.focused_editable(), "a read-only row opens no buffer");
        assert!(f.apply(ConfigAction::BeginEdit).is_empty());
        assert!(f.editing.is_none(), "and `Enter` left no buffer behind");
        assert!(f.apply(ConfigAction::Toggle).is_empty(), "and raises no effect");
        let at = f.field;
        assert!(f.apply(ConfigAction::FocusRow(at)).is_empty(), "a click raises no effect either");
        assert!(!f.dirty, "and none of them marked the form changed");
        assert_eq!(f.profiles, before, "nor wrote anything into a profile");
        assert_eq!(f.active_profile, "meigo", "nor selected another one");

        // The value is the sentence for the profile's language, and the language row is the only thing
        // that decides which one the server will send.
        let floor_of = |f: &ConfigForm| {
            f.rows().into_iter().find(|r| r.label == ES.cfg_tone_floor).expect("the floor row").value
        };
        assert_eq!(floor_of(&f), crate::tone::TONE_ES);
        f.profiles.iter_mut().find(|p| p.id == "meigo").unwrap().language = "en-GB".into();
        assert_eq!(floor_of(&f), crate::tone::TONE_EN, "the profile's language decides");
        f.profiles.iter_mut().find(|p| p.id == "meigo").unwrap().language = String::new();
        assert_eq!(floor_of(&f), crate::tone::TONE_EN, "and an unknown language gets the English one");
    }

    /// The floor is not part of the form, so a save cannot carry it into `config.toml`.
    ///
    /// There is no test that the floor is absent from `ProfileForm`: the type has no such field
    /// (`:147-159`), so there is nothing to build and nothing to assert. What *is* asserted here is the
    /// consequence a reader cares about — the value round-trips through the client's own conversions
    /// unchanged, because those conversions only ever see `ProfileForm`.
    #[test]
    fn the_floor_is_not_part_of_the_form_and_cannot_reach_a_save() {
        let mut f = profiles();
        focus(&mut f, ES.cfg_voice_instructions);
        assert!(f.apply(ConfigAction::BeginEdit).is_empty(), "an editable row opens its buffer");
        type_text(&mut f, "una voz distinta");
        f.apply(ConfigAction::CommitEdit);

        let meigo = f.profiles.iter().find(|p| p.id == "meigo").unwrap();
        assert_eq!(meigo.voice_instructions, "una voz distinta");
        assert_eq!(meigo.prompt, "p", "the prompt is untouched by the floor row's presence");
        assert!(
            !meigo.voice_instructions.contains("Voz de compadre"),
            "the floor is composed on the way to a request, never stored: {}",
            meigo.voice_instructions
        );
    }
```

Two things to check against the tree before you trust them: that `ConfigForm::profiles` is `Vec<ProfileForm>`
with `PartialEq` (it is, `:147-159`, `:176`) and that the test module's `focus` and `type_text` helpers
exist with those signatures (`:1362-1373`, `focus` panics when the label is absent). If a name differs,
fix the test to the tree and say so in the commit message.

Run: `cargo test -j 1 -p tipsy-ui the_floor_row_follows_the_profiles_language_and_edits_nothing the_floor_is_not_part_of_the_form_and_cannot_reach_a_save`
Expected: PASS, 2 tests.

- [ ] **Step 5: The gates mid-task**

Run: `cargo test -j 1 -p tipsy-ui`
Expected: PASS, 0 failures. `both_string_tables_carry_every_key_the_screen_uses` (`:1952`) passes because
`string_keys` returns the new label and both tables carry it — if it fails naming `cfg_tone_floor`, one
of the two table literals was missed.

Run: `cargo clippy --workspace --all-targets`
Expected: exit `0`, the same seven findings. This task adds no `session.rs` line, so the citation Task 2
recorded stands.

- [ ] **Step 6: The renderer: the label reaches a tab, and the value is cut — named, not skipped**

In `crates/tipsy-ui/src/render_config.rs`:

**a.** In `every_tab_draws_its_title_its_rows_and_the_help_line` (`:477-499`), the per-row loop asserts
that the drawn text contains **every row's whole value**, which cannot hold for this row: the Spanish
floor is 130 characters and the English 144 (measured), and the value column is `120 − 26 = 94` cells at
the width the test uses (`:170`, `:237`). Give that one row a label-keyed exception with its own
assertion, and say why in the comment:

```rust
                for row in f.rows() {
                    assert!(text.contains(row.label), "the label {:?} is not drawn:\n{text}", row.label);
                    if row.label == strings(UiLanguage::Es).cfg_tone_floor {
                        // The floor is 130 characters of the user's approved copy and the value column
                        // is 94 at this width, so it is drawn cut like any long value. It is asserted
                        // by its opening here and in full by the Profiles tab's own row test in
                        // `config_form.rs`. Shortening the sentence to fit would be re-wording copy
                        // the user approved; a wider skip of this loop would hide every other row.
                        //
                        // The comparison is against the **Spanish** table on purpose: the Profiles
                        // tab's row labels always read it (`config_form.rs:1167`), whatever the
                        // interface language this pass is drawing, so `s.cfg_tone_floor` would not
                        // match on the English pass and this row would fall into the branch below.
                        assert!(text.contains("Voz de compadre casi borracho"), "the floor is not drawn:\n{text}");
                    } else {
                        assert!(text.contains(&row.value), "the value {:?} is not drawn:\n{text}", row.value);
                    }
                    if !all.contains(&row.label) {
                        all.push(row.label);
                    }
                }
```

**b.** In the same test's trailing list of every label a tab must draw (`:503-522`), add the new label
after `es.cfg_voice_instructions,`:

```rust
            es.cfg_tone_floor,
```

That list is the check that no label in a table is unreachable from a tab, so the floor's label belongs
in it.

Run: `cargo test -j 1 -p tipsy-ui every_tab_draws_its_title_its_rows_and_the_help_line`
Expected: PASS, 1 test. Then **prove the exception is narrow**: change the exception's assertion to
`text.contains("no existe")` and run it again. Expected: **FAIL**, which proves the assertion is doing
work rather than being skipped. Put it back.

- [ ] **Step 7: The record**

**a. `docs/architecture.md`** — the workspace table's `tipsy-ui` row (`docs/architecture.md:93`). After
"…and the decisions a client makes with no device, socket or clock in reach (`policy`)", add:

```markdown
  It also holds the tone floor (`tone`): the two approved sentences every profile's delivery brief is
  prefixed with and every profile's system prompt is suffixed with, so no profile — built-in,
  overridden or written from nothing — can lose the almost-drunk register. The floor is composed when a
  request is built (`session::turn_profile`, `CallSetup::from_config`) and stored nowhere, so it can be
  neither edited nor removed.
```

**b. `CLAUDE.md`** — a new paragraph in "Current work", after the client-modularity paragraph:

```markdown
**The tone floor is on `master`** (`docs/superpowers/plans/2026-10-12-tipsy-tone.md`, spec
`docs/superpowers/specs/2026-10-12-tipsy-tone-design.md`) — the app is *tipsy*, and that is its joke,
so the almost-drunk tone prevails over the chosen voice: two sentences the user approved, one per
language, composed into **every** profile's delivery brief (first) and system prompt (last) in
`session::turn_profile` and `CallSetup::from_config`, and stored nowhere. **Three tasks**, one
implementer each, one whole-branch review, one fix wave, one scoped re-review, then the merge. The
sentence is **prepended to the brief because `GeminiSpeech::request_body` keeps only
`first_sentence(&req.instructions)`** — appended, it is discarded there and the profile's own first
sentence becomes the whole style the API reads, while the shipped default connector
(`google/gemini-3.8-flash-tts` through the OpenAI-compatible one) sends the whole string and would have
hidden the bug. Two residual facts worth carrying: a connector that drops delivery instructions
(native ElevenLabs, and any model that is neither `google/…` nor `openai/…` through the compatible
one) cannot be made to obey the voice half, and the Profiles tab's floor row shows the sentence cut
(94 cells against 130/144 characters), which is the one named exception in
`every_tab_draws_its_title_its_rows_and_the_help_line`.
```

Replace the test counts and the commit list with the ones you actually have, and **re-measure before you
write a number** — the paragraph above deliberately carries none.

- [ ] **Step 8: The full gates, and the plan's own last check**

Run: `cargo test -j 1 --workspace`
Expected: PASS, 0 failures, count = baseline + the tests this plan added (**6** in Task 1, **3** in
Task 2, **2** here = **11**; a rewritten test adds none).

Run: `cargo test -j 1 -p tipsy-ui -p tipsy-compadre -p tipsy-protocol`
Expected: PASS.

Run: `cargo check -p tipsy-ui -p tipsy-protocol --target wasm32-unknown-unknown`
Expected: PASS.

Run: `cargo clippy --workspace --all-targets`
Expected: exit `0`, the same seven findings as a set, the `session.rs` citation current in
`.superpowers/sdd/2026-10-12-tipsy-tone/global-constraints.md`.

Run: `Select-String -Path crates/tipsy-compadre/src, crates/tipsy-ui/src -Pattern 'tone::' -Recurse`
Expected: the call sites this plan lists and no more — `session.rs`'s two composers, `realtime/mod.rs`'s
one, `tone.rs` itself, and Task 3's `config_form.rs` row. **`providers/mod.rs` must not appear**: if it
does, the Test probes were floored and Review Focus 3 is open.

- [ ] **Step 9: Commit**

```bash
git add crates/tipsy-ui/src/strings.rs crates/tipsy-ui/src/config_form.rs crates/tipsy-ui/src/render_config.rs \
        docs/architecture.md CLAUDE.md
git commit -m "feat(ui): the Profiles tab shows the tone floor, and the record says what landed"
```

Body: the row and its position (why the end of the field block keeps the field mapping intact), the
`usize` underflow the guard prevents, the one named exception in the render test and why the sentence
cannot be shortened, and the residual connectors.

---

## What this plan does not do, deliberately

- **No `[tone]` section, no switch, no per-profile override.** The requirement's own words: no key,
  nothing that turns it off. D1 and D2 of the spec carry the measurements.
- **No wire change and no client change.** `PROTOCOL_VERSION` stays `1`; a client composes nothing, and
  Phase 3's browser client — now on `master` — gets the screen row from `tipsy-ui` for free.
- **No change to `Config`, `config/store.rs`, `config/form.rs`, `ProfileEntryView` or the providers'
  production code.** The user's copy is what the file, the form and the wire carry.
- **No second condenser.** The sentence claim is asserted through `providers/gemini.rs`'s own
  `first_sentence`; the `tipsy-ui` tests assert structure only.
- **No fix for `Strings`' tables being incomplete for most keys' `en` twins, and none for the Profiles
  tab's labels reading the Spanish table whatever the interface language is.** Both are pre-existing,
  both are named in the spec's §2 D2 and §7, and neither is this feature.

## Self-review record

- **Spec coverage.** §3's measurement is Task 2 Step 3 (the condenser test) and Task 2 Step 3's
  falsifiability check. §4's two sites: Task 2 Steps 1 and 2. §5's copy and language: Task 1 Steps 2 and
  4, and the copy is quoted verbatim above and in Task 1's constants. §6's text half: Task 2 Steps 1, 2
  and 5, whose two rewritten assertions are the no-corruption proof. §7's screen: Task 3 Steps 1, 2, 3,
  4 and 6. §8's "green and wrong": Review Focus 1–5 and the two falsifiability checks (Task 2 Step 3,
  Task 3 Step 6). §9's AC1–AC9: AC1–AC3 Task 2 Step 3; AC4 Task 2 Step 6; AC5 and AC6 Task 2 Step 4;
  AC7 Task 2 Step 5; AC8 Task 3 Steps 3 and 4; AC9 Task 1 Step 4. §10's gates: every task's last step,
  plus Task 3 Step 8. §11's file list: the File structure table above and each task's `Files:`.
- **The one gap this plan cannot close:** AC3 (the overridden built-in) has no test of its own. It does
  not need one and the plan says why at Task 2 Step 3: `turn_profile` reads `config.active()`, and the
  `id`-selection rule that makes an override win is `config/store.rs:224-233`'s subject, already pinned
  there. Adding a second test of the same rule would test the loader, not the floor.
- **Type consistency:** the five names in Task 1's `Produces` block are the names Task 2 and Task 3 call
  (`brief_with_floor`, `prompt_with_floor`, `floor`, `TONE_ES`, `TONE_EN`), and `PROFILE_FLOOR_COUNT` is
  spelled the same in Task 3's constant, its two callers and its test. There is no second spelling of
  any of them.
- **Numbers in this document are measurements with a date** — `5e22919`, 2026-10-12 — except the
  sentence lengths (130 and 144 characters, and one terminator each at index 129 and 143) and the test
  count, which every task re-measures rather than trusts.
