# tipsy-compadre — The tone floor — Design

- **Date:** 2026-10-12
- **Status:** draft, for review
- **Companion documents:** [`docs/architecture.md`](../../architecture.md) (the actor map and the
  provider table), the v1 spec [`2026-10-10-tipsy-compadre-design.md`](2026-10-10-tipsy-compadre-design.md)
  (§8.3 the profiles, §4 the wire), and the login spec
  [`2026-10-11-login-screen-design.md`](2026-10-11-login-screen-design.md) §8, which is the precedent
  for how this repository treats copy the user approved.
- **Approval:** the requirement and both scope answers are the user's (2026-10-12, quoted in §1). The
  **two sentences in §5 are the user's copy, approved verbatim**; they are reproduced exactly there and
  in the plan, and changing one changes behaviour on both surfaces. Everything else in this document —
  where the floor is composed, what the screen shows, the label — is an engineering decision and is
  this document's to take.
- **Every number and every `file:line` here was measured on 2026-10-12**, first at `5e22919` and then
  **re-measured at `766ead0`** when Phase 3's browser-client merge landed while this document was being
  written. Two files had moved — `crates/tipsy-compadre/tests/session.rs` (+31 lines into its tests)
  and `crates/tipsy-ui/src/strings.rs` (+42 lines of new strings) — and every citation below is the
  re-measured one. Every task in the plan re-reads the file it edits before editing it.

## 1. Purpose

The requirement, in the user's own words (Spanish, 2026-10-12):

> "ahora quiero que como la app se trata un compadre o tipsy en inglés y podemos cambiar la voz
> siempre prevalezca el tono de casi borracho sobre cualquier configuración para que la app nunca
> pierda su gracia"

Rendered: **the application is *tipsy* — that is its name and its joke. A user may change the voice,
and should be able to; but the almost-drunk tone must prevail over any configuration, so the
application never loses its grace.**

The user has already answered the two questions that decide the shape, and both are decisions rather
than open questions:

1. **Scope: the voice *and* the text.** The floor applies to the brief the speech provider receives
   **and** is reinforced in the profile's prompt path, so a profile the user invents cannot lose the
   register in what it writes either.
2. **The copy is approved verbatim, one sentence per language** (§5), and it is the user's text.

So this design adds one thing: **the tone floor** — two approved sentences, one per language, chosen
by the profile's language, composed into every profile's delivery brief and every profile's system
prompt at the moment the request is built. It is **not a setting**. There is no key for it in
`config.toml`, no switch on the configuration screen and no way to turn it off; the only thing the
user can choose is *which voice* says it, which is the requirement's own premise.

### Success criteria

1. **Every profile, whatever its origin.** A built-in, a built-in the user has overridden with the same
   `id` (the rule `config/store.rs`'s `user_override_of_builtin_wins` pins) and a profile the user
   wrote from nothing all reach the speech provider with the floor first in the brief and reach the
   chat model with the floor last in the system prompt.
2. **The floor survives a connector that condenses.** The style the Gemini API receives *is* the floor
   sentence, not the profile's own first sentence — proved through the same `first_sentence` the
   request body runs the brief through (§9, AC1–AC3).
3. **The user's copy is not corrupted and not stored.** The profile's `voice_instructions` and `prompt`
   are byte-identical to what the user wrote, in `Config`, in `config.toml`, on the screen and on the
   wire; the floor exists only in the request (§9, AC5–AC6).
4. **The screen tells the truth about it.** The Profiles tab shows the floor as read-only copy in the
   edited profile's language, and no keystroke or click on that row edits anything (§9, AC8).
5. **No wire change.** `PROTOCOL_VERSION` stays `1`; `ProfileEntryView` keeps carrying the user's raw
   copy (`crates/tipsy-protocol/src/lib.rs:193`).

### Non-goals

- **A floor for the connectors that cannot take one.** `ElevenLabsSpeech::synthesize` sends no
  delivery instructions at all and says so on purpose (`crates/tipsy-compadre/src/providers/elevenlabs.rs:244-245`:
  "*`req.instructions` is deliberately not sent: the note records no general instructions/style field,
  and `voice_settings.style` is V2+-only and raises latency*"), and the OpenAI-compatible connector
  sends none for a model that is neither `google/…` nor `openai/…`
  (`crates/tipsy-compadre/src/providers/openai_compat.rs:122-130`). No composition in this repository
  can put words into a request whose connector drops them (§12 keeps this as a named residual).
- **Local text-to-speech** (v2: Qwen3-TTS or Chatterbox). Whatever local engine arrives will have its
  own notion of a style; the floor reaches it the day it implements `SpeechProvider` with
  instructions, and this design neither promises nor forbids that.
- **The Test button's probes.** `probe_speech` asks whether a row can speak at all, with its own
  comment saying so — "*The connector's own default voice (`None`) and no delivery instructions: a
  Test asks whether this row can speak at all. The profile is not on screen in this tab, and nothing
  here plays what comes back — the samples are counted and dropped*"
  (`crates/tipsy-compadre/src/providers/mod.rs:406-409`) — and `probe_chat` sends no system message at
  all (`:385`). Neither changes.
- **A per-profile or per-session floor.** Two sentences, one rule, no exceptions, no overrides.
- **Fixing the UI-language coupling** of the forms tab's row labels (`config_form.rs:1167` reads the
  Spanish table whatever the interface language is). It is pre-existing, it is not this feature, and
  touching it would put an unrelated refactor in this branch (§7 states how the new row behaves under
  the coupling as it stands).

## 2. Decisions log

Every "measurement" below is a citation into the tree read on 2026-10-12, not a recollection.

| # | Decision | Alternatives rejected | The measurement that rejects them |
|---|---|---|---|
| D1 | **The floor is composed on the way to a request and stored nowhere.** `Config` keeps exactly the copy the user wrote; the composition happens in the two functions that turn a profile into a request. | Store it in `config.toml` (a `[tone]` section, or a brief/`prompt` rewritten at load); rewrite it in the configuration the session holds; put it in the form. | A stored floor is floored again on the next `turn_profile`, so the brief grows by one sentence per save→load→save cycle, and `config/store.rs` saves the whole `Config` it was given. It is also, by construction, a thing a user can then edit or delete — the one outcome the requirement names as forbidden. §9's AC6 is the falsifiable half: one round trip must produce the same brief. |
| D2 | **The copy and the two rules live once, in `crates/tipsy-ui/src/tone.rs`.** | A constant in `tipsy-compadre` plus a second copy in `strings.rs` for the screen; or the sentence as a field of `Strings` in both tables. | Two homes for one sentence is exactly what a reviewer would then have to keep equal. `tipsy-ui` is the crate **both** the server and the screen already compile against (`crates/tipsy-ui/Cargo.toml:9` already depends on `tipsy-protocol` for `UiLanguage`), so one home costs no dependency and no `pub` export. `Strings` is the wrong key even so: its two tables are keyed by `UiLanguage`, an enum of **two** values (`crates/tipsy-protocol/src/lib.rs:37-42`), while the floor is chosen by a *profile's* BCP 47 tag — and a third interface language would then silently acquire a floor sentence the user never approved. The **label** *is* interface copy and does go into `Strings` (§7). |
| D3 | **Prepend for the voice, append for the text.** The brief is `floor + " " + user's brief`; the system prompt is `user's prompt + "\n\n" + floor`. | Append to both; prepend to both. | §3: `GeminiSpeech::request_body` condenses the brief with `first_sentence` (`providers/gemini.rs:428`, `:318-325`), so an **appended** floor is discarded there and the profile's own first sentence is the entire style the API reads. The text half is appended because the persona must be the first thing a model reads and because appending is what makes "not one character of the user's prompt moved" checkable as `composed.starts_with(prompt)` (§6). |
| D4 | **The floor's language is the profile's, by the rule that already exists:** `UiLanguage::from_bcp47` — "Spanish for any `es` / `es-*` tag, English for everything else" (`crates/tipsy-protocol/src/lib.rs:44-50`). | A new language matcher in `tone.rs`; keying the floor off the *interface* language; a third sentence for a third language. | The built-ins declare exactly two primary subtags, `es` and `en` (measured: `es-ES` at `profiles.rs:29,85,133,180`; `en-US` at `:229,276`; `en-AU` at `:322`; `en-GB` at `:367`; `en-CA` at `:412`), so a second matcher would be the same rule spelled twice. A third sentence is copy the user has not approved, and CLAUDE.md's rule is that the profile copy is the user's to approve. |
| D5 | **One approved sentence per language serves both halves.** | A second, writing-specific sentence for the prompt half. | The user said "**one sentence per language**", and a second sentence would be profile copy nobody approved. The sentence is already a register instruction and not only an acoustic one: it names the articulation, the tempo and the humour and ends with the operative clause ("*no suavices ni quites ese tono*" / "*never soften or drop that tone*"), which is what the text half needs. §5 states this as an interpretation, because the requirement did not spell it out. |
| D6 | **Two composition sites, both of them "where the request is built":** `session::turn_profile` (`crates/tipsy-compadre/src/session.rs:256-266`) and `CallSetup::from_config` (`crates/tipsy-compadre/src/realtime/mod.rs:78-103`). | Composing in `Brain` (`brain.rs:217-221` and `:363-369`); composing in each provider; composing in `Config::active()` or in a load-time hook. | `turn_profile` is the **single** funnel from a profile to what a turn needs: it is called when the session is built (`session.rs:207`), on every profile switch (`:810`) and by the login-session test harness (`:1265`), and it is where the greeting's own turn gets its brief (`:465-473`). The brain would leave the realtime path — which builds no `Brain`, `realtime/mod.rs:95` — still unfloored, and would move the floor into `tests/brain.rs:96`, where a hand-built `TurnProfile` is the fixture. Each provider would mean three copies of one rule (`gemini.rs:427-438`, `openai_compat.rs:132-159`, `elevenlabs.rs:239-264`), which is the "fourth place to forget" that D2 exists to avoid. A provider also has no profile and no language. |
| D7 | **The configuration screen shows the floor as one read-only row** in the active profile's language, after its seven scalar rows and before its voices. | Not showing it; showing the *composed* brief in the editable row; a notice line. | `FormRow` already carries `editable: bool` (`config_form.rs:67-75`) and the screen already draws non-editable rows — the profile list rows and the voice connectors (`:1175-1180`, `:1200-1205`). Showing the composed brief in the editable row would make the row lie about the file and would write the floor into `config.toml` on the next save, which is D1's rejection. A notice line is transient and says nothing on the frames where the user is looking at the brief. |
| D8 | **The Test probes stay unfloored** (non-goal above). | Flooring them for consistency. | Their own comments say what they are for and that nothing plays (`providers/mod.rs:385`, `:406-409`); a one-word probe with the floor attached would spend a longer request to answer a question about a connector, not about a profile. |
| D9 | **The row's label is interface copy and lives in `Strings`; the row's value is the approved sentence and is read from `tone`.** | Re-wording the sentence shorter for the row; taking the value from `Strings`. | The value must be the exact bytes the server sends, or the screen teaches the user a floor that is not the floor. The label is a label, like `cfg_voice_instructions` beside it (`strings.rs:51`, `:194`, `:287`). |
| D10 | **No wire change and no change to the field block.** `PROTOCOL_VERSION` stays `1`; `PROFILE_FIELD_COUNT` stays `7` and a new `PROFILE_FLOOR_COUNT = 1` accounts for the new row. | Bumping the version; folding the floor row into `PROFILE_FIELD_COUNT`; sending the floor to a client. | The wire carries the profile's own copy (`ProfileEntryView`, `tipsy-protocol/src/lib.rs:193`) and the client composes nothing: the floor is a server-side rule about a request. `PROFILE_FIELD_COUNT` is the index space `commit_profile` maps rows onto (`config_form.rs:936-946`), so folding the floor into it would renumber a mapping the tests pin field by field — including the one that types into every field in turn (`:1682`). |

## 3. What the tree actually does on the speech path

Measured on 2026-10-12 at `5e22919`. **This section is the design**, because it decides the order of
the two halves.

There are three speech connectors and they treat `SpeechRequest::instructions` in three different
ways. `instructions` is defined as "*Delivery instructions (the profile's `voice_instructions`); ignored
by providers without support*" (`providers/mod.rs:56-61`), and the trait takes the whole request
(`:138-145`).

| Connector | What it does with `instructions` | Evidence |
|---|---|---|
| **Native Gemini** (`ProviderKind::Gemini`) | **Keeps the first sentence only.** `GeminiSpeech::request_body` sets `let style = first_sentence(&req.instructions);` and puts it in a `speech_metadata` annotation; `first_sentence` returns the text up to the first `.`, `!`, `?` or `…`. | `providers/gemini.rs:427-438` (`:428` is the condenser), `:315-325`; pinned by `condenses_the_voice_brief_to_its_first_sentence` (`:548-555`) and `every_builtin_brief_condenses_to_a_single_non_empty_style` (`:565-580`) |
| **OpenAI-compatible**, the shipped default | **Sends the whole string** — but only when the model name starts with `google/` or `openai/`; otherwise it sends nothing at all. `speech_quirks` decides, and the body field is added under `if quirks.instructions && !req.instructions.is_empty()`. | `providers/openai_compat.rs:122-130`, `:144-146`; pinned by `quirks_by_model` and `speech_gemini_sends_pcm_format_and_instructions` (`tests/openai_compat.rs:139-168`) and, for the drop, by `speech_mp3_models_decode_and_skip_instructions` (`:170-191`) |
| **Native ElevenLabs** (`ProviderKind::Elevenlabs`) | **Sends nothing**, deliberately. | `providers/elevenlabs.rs:244-245`, quoted in §1 |

**The shipped default is the OpenAI-compatible connector**, not the native Gemini one:
`ProvidersConfig::default` sets `speech: ProviderConfig::openrouter("google/gemini-3.8-flash-tts")`
(`config/model.rs:160-168`, `:124-131`), so `kind` is `OpenaiCompat` and the model name is what makes
`voice_key()` answer `"gemini"` (`:133-143`) and the `compae` profile's `Charon` the voice
(`config/profiles.rs:74-77`). `speech_quirks("google/gemini-3.8-flash-tts")` is
`SpeechQuirks { format: Pcm24k, instructions: true }`, so **the default path carries the whole brief and
does not condense.**

**The consequence, and the sentence a reviewer should quote:**

> A tone clause **appended** to a brief does not exist for the connector that condenses:
> `GeminiSpeech::request_body` keeps only `first_sentence(&req.instructions)`
> (`providers/gemini.rs:428`), so an appended floor is discarded and the profile's own first sentence
> is the entire style the API reads — the exact failure this requirement exists to prevent.

Two facts sharpen it:

1. The user can reach that connector from the configuration screen alone — it is the Providers tab's
   connector row (`config_form.rs:1096-1102`), one selection away, and the same `SpeechRequest` is
   built for all three (`brain.rs:217-221`, cloned per sentence at `:423`).
2. The existing tests *are* the measurement. `BRIEF` in `providers/gemini.rs:533-536` is a two-sentence
   brief and `STYLE` is its first sentence; `condenses_the_voice_brief_to_its_first_sentence` asserts
   `first_sentence(BRIEF) == STYLE` (`:549-550`). Appending the floor to `BRIEF` yields
   `first_sentence(BRIEF + " " + floor) == STYLE` — the floor never arrives.

And one honest cost, stated rather than hidden: on the condensing connector the floor is what the API
reads, which means **the profile's own first sentence no longer reaches that API**. That is the
requirement's own trade — the tone prevails over the profile's configuration — and it is not a loss of
the voice's identity: the voice identity travels separately, as a *name*, in
`generation_config.speech_config.voice` (`providers/gemini.rs:436`), which `compae` fills with
`Charon`. On the shipped default connector nothing is lost at all: the whole brief is sent, floor
first.

## 4. Where the floor is composed

**Two sites, both where a request is built, and both call the same two functions.** Everything a
profile owns that becomes a request is listed here; the table is the completeness argument.

| Reader of the profile's texts | What it sends | Floored? |
|---|---|---|
| `session::turn_profile` (`session.rs:256-266`) → `Brain` → `SpeechRequest::instructions` (`brain.rs:217-221`) | the delivery brief | **yes, first** |
| the same → `Brain::messages` → `ChatMessage::system` (`brain.rs:363-369`) | the system prompt | **yes, appended** |
| the greeting's own turn (`session.rs:465-473`, a `TurnInput::Say`) | the same brief through the same path | yes, by the same line |
| `CallSetup::from_config` (`realtime/mod.rs:95`) → `session.update.instructions` (`realtime/socket.rs:420-438`) | the system prompt | **yes, appended** |
| `probe_speech` (`providers/mod.rs:409`) | `instructions: String::new()` | no, deliberately (§1, D8) |
| `probe_chat` (`providers/mod.rs:385`) | a user message, no system message | no, same reason |
| `ProfileView` / `ProfileEntryView` (`ProfileEntryView`'s `voice_instructions`, `tipsy-protocol/src/lib.rs:193`) / `ConfigView` | the user's raw copy, to a client | no — the wire carries what the file holds |
| `form_of` / `profile_form` (`config/form.rs:167-193`) | the user's raw copy, to the screen | no — the screen shows the file's copy plus the floor row (§7) |

**Why the composition is not inside `Brain`.** `Brain` receives a `TurnProfile` that `session.rs`
builds; composing there would leave `realtime/mod.rs:95` — which constructs no `Brain` — sending the
raw prompt, so the realtime mode, the one mode whose voice is a free choice from `[realtime] voice`
(`config/model.rs:189`, and the decision recorded at `realtime/mod.rs:79-85`), would be the hole in
the requirement. It would also put the floor
into every hand-built `TurnProfile` fixture (`tests/brain.rs:96`), which is a fact about a *profile*
and not about a turn.

**Why the realtime path is in scope at all.** `CallSetup::from_config` is the only other place in the
tree that turns `config.active().prompt` into a request (grep over `*.rs`: `session.rs:261`,
`realtime/mod.rs:95`, `brain.rs:365`, and the two form conversions that only carry it). Realtime is a
voice mode whose delivery nobody can steer with a brief — its session update has an `instructions`
field and a `voice`, and no style field (`realtime/socket.rs:420-438`) — so the prompt half is the only
surface the floor has there, and the requirement's "we can change the voice" is literally `[realtime]
voice`'s row in the configuration.

**Why not earlier.** Not at load: `store::load_or_create` never rewrites the file and returns a
`Config` a save writes back whole (`config/store.rs`), so a floor applied there is a floor stored (D1).
Not in `Config::active()`: the client reads it for the interface language (`client/settings.rs:154`),
the hub reads it to announce a profile (`server/hub.rs:275`) and `turn_profile` reads it for the voice,
so a getter that composes would make every reader pay for a rule only two of them want.

## 5. The copy, and the language it is chosen by

**The two sentences, exactly as the user approved them, one per language. These are the user's text:
they are reproduced verbatim, punctuation included, and they are edited only by the user.**

```text
Voz de compadre casi borracho: dicción pastosa, tempo lento y guasa — sea cual sea la voz elegida, no suavices ni quites ese tono.

An almost-drunk compadre's voice: slurred articulation, a slow tempo and dry humour — whatever voice was chosen, never soften or drop that tone.
```

Both are **one sentence each**, and that is load-bearing rather than stylistic: the Spanish has no
terminator before its final full stop and the English has none before its own, so a connector that
keeps the first sentence keeps the whole instruction, operative clause included. They are also
complete instructions without the profile's own words, which is the other half of the same decision:
`brief_with_floor` may be the *only* text a condensing connector sees.

**Which sentence a profile gets, measured.** The nine built-ins declare exactly two primary subtags:

| Language | Profiles |
|---|---|
| `es-ES` | `compae` (`profiles.rs:29`), `meigo` (`:85`), `chulapo` (`:133`), `senyor` (`:180`) |
| `en-US` | `good_ol_boy` (`:229`), `hustler` (`:276`) |
| `en-AU` | `mate` (`:322`) |
| `en-GB` | `bloke` (`:367`) |
| `en-CA` | `polite_one` (`:412`) |

The rule is not new and is not re-derived: `UiLanguage::from_bcp47` is "*Spanish for any `es` / `es-*`
tag, English for everything else*" (`tipsy-protocol/src/lib.rs:44-50`), and it is the same function
that selects the interface table on `Welcome` (`session.rs:232`) and the greeting's language. So:

- `es`, `es-ES`, `es-419`, `ES-es` → the Spanish sentence;
- `en-GB`, `en-AU`, `fr-FR`, a language that is empty or whitespace → the English sentence.

**Interpretation taken, and it is judgement.** The requirement did not say what a profile whose
language is neither Spanish nor English gets. This design gives it the English sentence, which is
honest for three reasons rather than one: the approved copy is *two* sentences and inventing a third
would be writing profile copy the user has not approved; English is the language the repository's own
documents and five of the nine built-ins use; and the rule is the app's existing one, so the screen
and the server cannot disagree about which sentence a `fr-FR` profile shows and sends. The alternative
— leaving a third-language profile unfloored — fails the requirement's "cualquier configuración".

## 6. The text half, and why it cannot corrupt a prompt

The composed system prompt is **the user's prompt, a blank line, then the floor** — appended, never
prepended, never interleaved:

```text
{the profile's own prompt}
                                                              ← one blank line
{Voz de compadre casi borracho: … / An almost-drunk compadre's voice: …}
```

Three properties, and each is a test (§9):

1. `composed.starts_with(&profile.prompt)` — **the persona is byte-identical and first.** Not one
   character of the user's prompt moved, which is what "do not corrupt a user's prompt" means
   operationally: the claim is checkable with one `starts_with`, and it is checked.
2. The blank line is deliberate: the model reads two blocks — who it is, then how it must never
   sound — instead of one run-on instruction where the reinforcement could read as part of the
   persona's description.
3. An empty prompt composes to the floor alone. A profile with nothing to say still keeps the
   register; the alternative (an empty system prompt) is the one configuration that would make the
   app lose its grace in writing entirely.

**The two halves are ordered oppositely on purpose**, and the spec should not be read as inconsistent
about it: the voice half is first because a connector that condenses keeps the first sentence, and the
text half is last because a model reads a whole prompt and the persona must not be displaced.

**The realtime mode gets the text half only.** `CallSetup.instructions` becomes
`prompt_with_floor(&profile.language, &profile.prompt)`; its *voice* stays `[realtime] voice`
(`realtime/mod.rs:79-85`), which is a configuration the profile does not map, and its session update
carries no style field to floor.

## 7. What the user sees

**Decision (D7): the Profiles tab shows the floor, read-only, and that is a stated decision rather than
an omission.** The user asked for a floor that cannot be removed; a screen that lets someone edit the
brief and the prompt while saying nothing about the floor teaches them that removing the tone is
possible. So the tab gains exactly one row, after the active profile's seven scalar rows and before
its voice entries:

| Column | Value |
|---|---|
| label | `strings(UiLanguage::Es).cfg_tone_floor` — the form's row labels already read that table whatever the interface language is (`config_form.rs:1167`, `:1185-1195`), and the new row does not introduce a second convention |
| value | `tone::floor(&profile.language)` — **the approved sentence, in the profile's language, byte for byte** |
| `editable` | `false` |

Why the value is not from `Strings` (D9) and why it is not the *composed* brief (D7) are decided above.
The label is interface copy like its neighbours: **Spanish `Tono intocable`, English `Untouchable
tone`** — and the label is not the floor, so an implementer who would phrase the label differently says
so in their report rather than re-wording the sentence.

**What the row does when it is touched, which is nothing, and why no new arm is needed.** It is
focusable by `Tab`/arrows because `next_field`/`prev_field` walk the row list
(`config_form.rs:1053-1062`); `Enter` resolves through `enter_action` to `Toggle` because the row is
not editable (`:793-795`), and the Profiles tab's `toggle` raises nothing (`:891`); a click resolves
through `focus_row` to `activate`, and `activate`'s fallback for a non-Profiles action row raises
nothing (`:843-871`); `BeginEdit` checks `focused_editable` (`:701-705`). So the row is inert **by
construction**, and the plan's test asserts each of those facts rather than trusting the reading.

**The index arithmetic, spelled out because it is where a mistake silently overwrites a field.**
`commit_profile` maps a row to a field by `self.field - list_len` (`:930-961`) and the voices by
`self.field - list_len - PROFILE_FIELD_COUNT` (`:950`). The floor row sits between the two, so:

- `profile_row_count` and `voice_label_row` (`:491-500`) each add `PROFILE_FLOOR_COUNT`;
- `commit_profile` gains an explicit `if self.field < list_len + PROFILE_FIELD_COUNT + PROFILE_FLOOR_COUNT { return false; }`
  **before** the subtraction, because the floor row is exactly at `list_len + PROFILE_FIELD_COUNT` and
  subtracting both constants there underflows a `usize`;
- `PROFILE_FIELD_COUNT` stays `7` and the field → field mapping is untouched, so
  `each_profile_row_commits_its_own_field` (`:1682`) and `active_fields` (`:1378-1389`) keep their
  meaning.

**What the row looks like at the width the screen has, measured.** A `FormRow` is one line and its
value is cut at the column's edge (`render_config.rs:184-193`, `:237`), and the label column is a
third of the body clamped to `[12, 26]` (`:168-170`). The Spanish floor is **130 characters** and the
English is **144**, so at 120 columns — the width the render tests use — the value column is
`120 − 26 = 94` cells and **the sentence is cut in every practical terminal**, with its operative
clause off screen. That is accepted and named rather than discovered: the row's job is to say the
floor exists and cannot be edited, wrapping is not available without breaking the one-row-per-index
contract that every click and every keystroke resolves through, and a shortened value would be a
sentence the user did not approve.

It also has one consequence the implementer must handle rather than trip over:
`every_tab_draws_its_title_its_rows_and_the_help_line` asserts, for **every row of every tab**, that
the drawn text contains the row's whole value (`render_config.rs:491-493`). That assertion cannot hold
for this row and must not be bought by re-wording the sentence; the plan's Task 3 gives the row a
named, label-keyed exception inside that loop, with its own assertion on the sentence's opening, and
says in the comment why.

**Every surface at once.** The native TUI, the web terminal (the same TUI in a PTY) and Phase 3's
browser client all draw this form from `tipsy-ui`, so all three get the row with no client change and
no wire change.

## 8. What would make this green and wrong

Written out because each of these has a test that passes.

1. **A test that only asks whether the floor appears somewhere.** `assert!(request.instructions.contains(TONE_ES))`
   passes for the implementation this requirement exists to forbid — an appended clause — on the
   shipped default connector, because `speech_quirks("google/…").instructions` is `true` and the whole
   string is sent (`openai_compat.rs:122-130`). The only falsifiable form is an equality through the
   condenser: `first_sentence(composed) == floor(language)`, run against the same `first_sentence` the
   request body runs (`gemini.rs:428`). §9's AC1–AC3 are that form, and the plan says in the test's own
   doc comment why a `contains` would not do.
2. **A second condenser in the test that proves the first.** A `tipsy-ui` test may assert structure
   (`composed.starts_with(TONE_ES)`), and must not re-implement "the first sentence" to assert a
   sentence: two condensers is two behaviours, and the one that matters is the provider's.
3. **The floor stored.** A save→load→`turn_profile` cycle that changes the brief means the floor
   reached the file (D1). AC6.
4. **The floor lost from one of the two halves while the other keeps it.** AC4 and AC7 are separate
   assertions on separate surfaces: the brief and the prompt, and the realtime session's instructions.
5. **The floor reaching the user's copy.** Any change to `Config::profiles[i].prompt`,
   `.voice_instructions`, the form's `ProfileForm` or `ProfileEntryView` is a failure, not a
   convenience. AC5.
6. **The floor applied to the Test probes.** A green suite would not notice; D8 and the plan's Review
   Focus do.

## 9. Acceptance criteria

Each is falsifiable, each names the test that decides it, and the tests are given in full in the plan.

- **AC1 — every built-in, through the condenser.** For each of the nine `builtin_profiles()`,
  `first_sentence(turn_profile(&config_with_that_profile_active).voice_instructions) == floor(&profile.language)`.
  *Test:* `providers/gemini.rs`'s `every_brief_the_speech_provider_receives_opens_with_the_tone_floor`
  — in that module because `first_sentence` is private to it (`:318`), which is the point: the test uses
  the real condenser, not a copy.
- **AC2 — a profile written from nothing.** The same equality for a profile whose
  `voice_instructions` is `""`, `"   "` and a contradiction
  (`"Habla como un presentador de telediario, formal y sobrio."`). *Test:* the second half of the same
  test. This is the criterion the requirement names: *an empty brief and a contradicting brief must
  still produce a brief whose first sentence is the tone.*
- **AC3 — the overridden built-in.** A `Config` built from a file that overrides `compae` with the same
  `id` (`config/store.rs:224-233` pins that the override wins) still floors that profile's brief.
  *Test:* the loop of AC1 over `Config::default()` covers the `id`-selection rule; the plan's Task 2
  states why and adds the file-level case only if the implementer finds the code path differs (it does
  not: `turn_profile` reads `config.active()`).
- **AC4 — the brief reaches the provider.** A profile with an empty brief, driven through
  `spawn_with_events` and one greeting turn, records `SpeechRequest`s whose `instructions` start with
  the floor. *Test:* `tests/session.rs`'s
  `a_profile_with_no_brief_still_reaches_the_speech_provider_with_the_floor`.
- **AC5 — the user's copy is untouched.** After `turn_profile(&config)`, `config.active()` still equals
  what was loaded, and the `compae` card still starts with its own words. *Test:*
  `session.rs`'s `the_floor_is_never_written_into_the_configuration`.
- **AC6 — the floor is not stored, expressed as idempotence.** `save` → `load` →
  `turn_profile` produces byte-identical texts to one `turn_profile`. *Test:* the second half of the
  same test.
- **AC7 — the text half, both surfaces.** The chat system message starts with the user's prompt
  byte-for-byte and ends with the floor (`tests/session.rs:701`'s assertion, rewritten), and
  `CallSetup::instructions` is `prompt_with_floor(...)` with the raw prompt as its prefix
  (`realtime/mod.rs:762`, rewritten). *Tests:* `switching_profile_changes_the_prompt_of_the_next_turn`
  and `the_call_asks_for_the_configured_model_voice_and_prompt`.
- **AC8 — the screen.** The Profiles tab draws one row labelled `cfg_tone_floor` whose value is
  `tone::floor(&profile.language)`, is not editable, and edits nothing when focused, entered, toggled
  or clicked. *Tests:* the rewritten
  `the_profiles_tab_shows_the_active_marker_before_the_fields_and_the_voices_last` plus a new
  `the_floor_row_follows_the_profiles_language_and_edits_nothing`, in `config_form.rs`.
- **AC9 — the copy is the user's, verbatim.** `TONE_ES` and `TONE_EN` equal the two sentences above
  byte for byte, and each contains exactly one sentence terminator, at its end. *Test:* `tone.rs`'s
  `the_copy_is_the_two_sentences_the_user_approved`.

## 10. Gates

Every gate below is run with the repository's canonical command shape, which exists because this
machine dies with `STATUS_STACK_BUFFER_OVERRUN` at cargo's default parallelism and because the
machine's temp directory denies directory creation (`.superpowers/sdd/2026-10-11-plan-3-web-tier/global-constraints.md:37-52`):

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP
$env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
cargo test -j 1 -p <crate> <filter>
```

1. **The suite is green**, `cargo test -j 1 --workspace`, with the baseline captured **before Task 1**
   and compared after every task. The plan's Task 1 Step 1 is the capture; the count is not quoted
   here because the tree has moved past the last number anyone wrote down (`5e22919`, and CLAUDE.md's
   own record stops earlier). **Re-measure, do not trust.**
2. **No test is deleted, ignored or weakened.** Two existing tests change an *expected value* on
   purpose (AC7), and one changes an expected count and gains exactly one named, label-keyed exception
   inside a loop (AC8, §7); every other baseline name survives, and the plan's diff of
   `cargo test -- --list` must show only additions plus those edits.
3. **Clippy's gate is the finding set, not the exit code** —
   `cargo clippy --workspace --all-targets` exits `0` and reports **seven** pre-existing findings
   (`.superpowers/sdd/2026-10-11-plan-3-web-tier/global-constraints.md:79-94`), compared **as a set and
   never as a list**, because the lines move. One of them is in `session.rs` (`:484` at `2f25c20`), and
   Task 2 edits `session.rs` above it, so that citation is re-measured and recorded in the plan's own
   SDD workspace.
4. **The wasm gate:** `cargo check -p tipsy-ui -p tipsy-protocol --target wasm32-unknown-unknown` is
   clean, and no `Cargo.toml` gains a dependency. The floor is pure `&str`/`String` work over
   `tipsy-protocol`'s `UiLanguage`, which `tipsy-ui` already depends on.
5. **`PROTOCOL_VERSION` stays `1`**, and `cargo test -p tipsy-protocol` proves the wire is unchanged.
6. **No `cargo fmt`.** The repository is not rustfmt-clean and formatting would rewrite unrelated
   lines.

## 11. What changes, where

| Path | Kind |
|---|---|
| `crates/tipsy-ui/src/tone.rs` | **new**: the two approved sentences, `floor`, `brief_with_floor`, `prompt_with_floor`, and their tests |
| `crates/tipsy-ui/src/lib.rs` | modify: `pub mod tone;` |
| `crates/tipsy-compadre/src/session.rs` | modify: `turn_profile` composes both halves and becomes `pub(crate)`; one `use`; two new tests |
| `crates/tipsy-compadre/src/realtime/mod.rs` | modify: `CallSetup::from_config`'s `instructions`; the existing test's expected value gains the floor |
| `crates/tipsy-compadre/src/providers/gemini.rs` | modify: **tests only** — the condenser's acceptance test |
| `crates/tipsy-compadre/tests/session.rs` | modify: one rewritten assertion, one new integration test |
| `crates/tipsy-ui/src/strings.rs` | modify: one field, both tables |
| `crates/tipsy-ui/src/config_form.rs` | modify: `PROFILE_FLOOR_COUNT`, `voice_label_row`, `profile_row_count`, `commit_profile`, `profile_rows`, `string_keys`, one rewritten test, two new tests |
| `crates/tipsy-ui/src/render_config.rs` | modify: the new label joins the "every label reaches a tab" list (`:503-522`) and the profiles render test asserts the value is drawn |
| `docs/architecture.md` | modify: one line |
| `CLAUDE.md` | modify: the "Current work" paragraph |

Nothing else. In particular `crates/tipsy-compadre/src/config/**` is **not** touched, and neither is
`crates/tipsy-protocol/**`.

## 12. Risks, residuals and what is left open

- **A connector can still drop the floor, and no composition can help.** ElevenLabs sends no
  instructions by decision (`providers/elevenlabs.rs:244-245`) and the OpenAI-compatible connector
  drops them for a model that is neither `google/…` nor `openai/…` (`:122-130`, pinned by
  `tests/openai_compat.rs:170-191`). On those paths the *text* half still holds — the chat model is
  floored — but the delivery cannot be steered. Named rather than papered over; closing it would mean
  changing a connector's contract or refusing a selection, and neither is this feature's.
- **On the condensing connector, the profile's own first sentence no longer reaches the API** (§3).
  This is the requirement's trade and it is recorded here so it is not discovered later as a
  regression.
- **The screen's row is cut at every practical width** — 94 cells at 120 columns against 130 (es) and
  144 (en) characters — so the operative clause is off screen (§7). Accepted; wrapping is not available
  without breaking the one-row-per-index contract, and the general "every row's value is drawn"
  assertion in `render_config.rs:491-493` therefore carries exactly one named exception.
- **`Strings`' two tables are not checked for equality key by key.** The `en` twin of the new label is
  mandatory by the struct literal but is not drawn, because the Profiles tab's labels read the Spanish
  table (`config_form.rs:1167`). Pre-existing and deliberately not touched (§1).
- **Left to the implementer, and expected to be reported rather than fixed silently:** the exact
  wording of the label if a better one reads well in both tables; whether `brief_with_floor`'s single
  separating space should be preserved as two when the user's brief itself starts with whitespace (the
  plan's code preserves the user's bytes and does not trim them).
- **Left to the reviewer:** whether the realtime path belongs in scope at all. It is in it, for the
  reason §4 gives — `[realtime] voice` is the one voice a profile does not map, and
  `CallSetup::from_config` is the second and last place a profile's prompt becomes a request — and the
  cost is one line plus one expected value in an existing test.
