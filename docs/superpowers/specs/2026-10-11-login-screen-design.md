# tipsy-compadre — Login screen — Design

- **Date:** 2026-10-11
- **Status:** awaiting review
- **Companion documents:** [`docs/architecture.md`](../../architecture.md) (diagrams and the
  security model), the v1 spec
  [`2026-10-10-tipsy-compadre-design.md`](2026-10-10-tipsy-compadre-design.md) (§9 the interface,
  §11 security), and Plan 2B
  [`../plans/2026-10-10-plan-2b-config-screen-and-auto-mode.md`](../plans/2026-10-10-plan-2b-config-screen-and-auto-mode.md)
  whose full-screen form this screen deliberately does *not* copy.
- **Approval:** the screen's idea, its shape (only the core and a text box over a pixelated
  dashboard) and the copy in §8 were accepted in conversation on 2026-10-11. The copy is the user's:
  §8 is the record of what was approved, and changing it changes no code path.

## 1. Purpose

The assistant asks who you are before it lets you in. Say the right thing — the words the owner of
the machine listed in `config.toml`, for instance *"yo soy Andrés del equipo del core"* against the
keywords `andres` and `core` — and the door opens. Say anything else and it tells you, in its own
affectionate voice, to go and get your hair combed somewhere else.

**This is a ritual, not authentication.** It is a flavour gate: what it protects is the joke, and
the real door remains the 32-byte `server.token` of the v1 spec (§11). Nothing in this feature may
be described, documented or relied upon as a security boundary, and no other part of the product
changes its behaviour because this screen exists.

### Success criteria

1. With `[login]` configured, a client opens on the login screen and nothing else: the dashboard
   shows through behind a veil, unreadable, and the only crisp thing on screen is the widget in the
   middle with the core in it.
2. Typing "yo soy Andrés del equipo del core" reaches the HUD **after one more key**; typing
   "soy Andrés del score" or "soy Andrés" does not, and answers with the refusal line.
3. Saying the same sentence out loud, with voice on, reaches the same verdict — and the audio that
   leaves the machine is only what the local detector confirmed as speech.
4. Nobody can be locked out of their own application: a denied mic, a missing key, an unreadable
   accent or a forgotten phrase all end in a way that either lets the user type or points them at
   `[login]` in `config.toml`.
5. The three surfaces behave identically: the native TUI today, the web terminal (the same TUI in a
   PTY) for free, and the web client when it exists, which needs no server change because §4 and §6
   are already the whole contract.

### Non-goals

- **Security.** No passwords, no accounts, no lockout, no rate limiting, no per-person tokens, no
  interaction with `server.token`, no attempt to make the phrase a secret.
- **A Login tab** in the configuration screen. The section is hand-edited in `config.toml`, like
  `[[profiles]]`, and the configuration screen must carry it through untouched (§5). **Amended
  2026-10-12**: the screen still has no Login tab — `ConfigTab` has four variants and `Login` is not
  one of them — but the section now **travels in `ConfigView`** (Plan 3's Task 1.1), so a client that is
  entitled to the token *sees the allowlist* and can change it by sending `UpdateConfig`. What is
  unbuilt is the tab, not the wire.
- **Configurable copy** (`[login.copy]`). All interface copy lives in the two static tables of
  `tipsy-ui`; a per-user or per-profile version needs a home on the wire, which `LoginView` (§4) is
  shaped to take later — see §13.
- **Speaking the prompt out loud.** Tempting, and it costs a speech call before the door and needs a
  speech key on a fresh install; §13 keeps it as a follow-up.
- **Re-login on reconnect**, per-session identities, logging attempts, and reading anything in the
  answer other than the keywords (no names, no roles, no mood).

## 2. Decisions log

| # | Decision | Alternatives considered |
|---|---|---|
| L1 | The login is a ritual; `server.token` stays the only real requirement | Replacing the token with the keywords (rejected: a shared phrase is weaker *and* would have to travel in a URL) |
| L2 | The **server** judges the answer; the allowlist leaves it only inside `ConfigView`, to a client entitled to the token | Matching in `tipsy-ui` (rejected: the keywords would have to reach the web client, where anyone could read them off the wire) — **amended 2026-10-12**: Plan 3's Task 1.1 put the section on the wire as `ConfigView.login`, so the allowlist *does* reach a client now (§4.3). What the decision still guarantees is that the **judging** never leaves the server, and that a client which never fetches the configuration never sees the allowlist |
| L3 | Voice is on by default, and only speech the local detector confirms is ever transcribed | Always-on streaming (rejected: cost, and it breaks the "nothing leaves until it is speech" promise); typing only (rejected: the ritual is spoken) |
| L4 | Voice gets its own capture path and its own messages | Reusing `ListenStart`/`ListenStop` (rejected outright: `TurnInput::Utterance` transcribes **and then answers with the LLM**, speaking a reply nobody asked for and writing the login into the conversation history) |
| L5 | The screen is a widget over a veiled HUD, not a full-screen form | A full screen like the configuration screen (rejected: the user asked for the dashboard behind, pixelated) |
| L6 | The veil turns every inked cell into `▒` in the HUD's own colours, darkened by a fixed map; a `Dim` strength is one constant away | A true half-resolution mosaic (rejected: it needs all of `render.rs` to draw into a `Buffer`); `Modifier::DIM` (rejected: browser backends render SGR 2 inconsistently, and an explicit map is testable) |
| L7 | The widget holds the core, one status line and the input box, and nothing else; the mic indicator and the keys ride on its borders | A header, a help bar, an attempt counter (rejected: "solo el núcleo y una caja") |
| L8 | Unlimited attempts; the mic re-arms itself at most 3 times in a row and then waits for `Tab` | Three attempts and out (rejected: locking the owner out of their own app); unlimited auto re-arm (rejected: a room with a television would buy transcriptions all evening) |
| L9 | Only `Tab`, `Enter`, `Backspace` and `Esc` are keys — **no bare letters**, because every letter is also a possible answer | `M` for the mic and `R` to retry (rejected: they collide with typing "soy maria" and "rodrigo") |
| L10 | `PROTOCOL_VERSION` stays 1: the messages are additive | Bumping it (precedent: 2B's `TestProvider` and 2C's `Live` were additive too) |

## 3. Where the screen lives

| Surface | What it does |
|---|---|
| Native TUI (`tipsy-compadre`) | On `Welcome`, if `login` is `Some`, it shows the widget instead of the HUD, starts the microphone if `voice`, and sends the commands of §4 |
| Web terminal (`/terminal`) | Nothing at all: it runs the native TUI in a PTY, so it is the same code path by construction |
| Web client (`tipsy-web`, Plan 3, not built yet) | The same `LoginState` transitions from `tipsy-ui`, its own capture (`getUserMedia` → `Frame::Mic`) and the same three commands. §4 and §6 are the contract: the server needs no change for it |

**`Welcome.login` is the only authority.** The client never reads `[login]` itself, not even the
native one, so a native client connected to a remote server cannot disagree with the server that
will judge it, and `--connect` needs no special case. A change to `enabled` takes effect on the next
connection, never mid-screen.

The screen is shown **once per client run**, when `Welcome` arrives. It is not shown again after a
reconnect, and it is not a screen the user can re-open: `F9` and the gear do nothing while it is up
(§6).

## 4. Protocol

Four additions, all additive: three commands, two events, and one field on `Welcome`. Every message
below is exact and complete; `Command` and `Event` are `#[serde(tag = "type", rename_all = "snake_case")]`,
so these are the wire spellings.

```jsonc
// client → server
{"type":"login_start"}                       // the login screen is up and the microphone is opening
{"type":"login_stop"}                        // stop capturing; discard whatever was buffered
{"type":"login_answer","text":"yo soy andres del core"}   // a typed answer

// server → client
{"type":"login_heard","text":"yo soy Andrés del equipo del core"}   // what the transcriber heard
{"type":"login_verdict","ok":true,"name":"Andrés"}                  // the ruling, and who it was
{"type":"login_verdict","ok":false,"name":null}
```

`Welcome` gains one field:

```rust
Welcome {
    session_id: u64,
    profile: ProfileView,
    ui_language: UiLanguage,
    listening_mode: ListeningMode,
    web_bundle: bool,
    /// `None` — no login screen: the client goes straight to the HUD. `Some` — show the screen, and
    /// `voice` says whether the microphone opens with it. An `Option` rather than two booleans
    /// because the two facts travel together, and because a future per-profile copy has an obvious
    /// home here (§13).
    #[serde(default)]
    login: Option<LoginView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginView {
    pub voice: bool,
}
```

`#[serde(default)]` is what makes the field safe both ways: a client that predates it ignores an
unknown field (serde's default), and a client that knows it parses a `Welcome` from a server that
does not (`login == None`). `PROTOCOL_VERSION` stays 1.

### 4.1 What the server does

| Message | Server behaviour |
|---|---|
| `LoginStart` | If the screen does not apply (`applies(&config.login) == false`) or a capture is already active, nothing happens (the second case matters: a repeated start must not eat an utterance in flight). Otherwise the capture opens: `login_active = true`, a fresh `VadState::new(config.listening.silence_ms)`, an empty buffer |
| `LoginStop` | `login_active = false`, buffer dropped. Idempotent |
| `LoginAnswer { text }` | `judge(&config.login, &text)` → `Event::LoginVerdict`. Judged whether or not a capture is active, so a typing-only session never has to send `LoginStart` at all. When the screen does not apply the verdict is `ok: true, name: None`: there is nothing to check, so nobody is ever refused |
| `Frame::Mic` while `login_active` | Fed to the login's own detector, never to the listening path |

The login capture is a **second `VadState`** (`session.login_vad`), not the session's own: the two
must never share a carry buffer, and the login's is built from `config.listening.silence_ms` exactly
as the session's is (`session.rs:78`). The capture is a small state machine of its own:

- `SpeechStarted` → clear the buffer and start collecting (nothing before the onset is kept — the
  same guarantee `detect()` already gives for `auto` mode).
- `VadEvent::None` while speaking → collect.
- `SpeechEnded`, or 15 s of continuous speech (`LOGIN_MAX_UTTERANCE_MS = 15_000`, pinned because a
  login answer is a sentence and a transcription bill is not a joke) → close the utterance:
  - otherwise `Status { Transcribing }`, then the configured transcribe provider with the brain's
    own 60 s timeout, then, on text: `LoginHeard { text }` and immediately `LoginVerdict`. The one
    `NothingHeard` this path sends is an empty transcription —
    `Event::Error { code: NothingHeard, message: "nothing was heard", recoverable: true }`, no verdict,
    **and no attempt counted** (a cough is not a wrong answer). A "nothing collected" branch is
    deliberately not written and not asked for: an utterance exists only because the detector confirmed
    an onset, and the frame that opened it was already collected, so the buffer cannot be empty at
    `SpeechEnded`. A provider failure is reported with the same
    `ErrorCode` mapping the brain uses (`NoKey`, `Auth`, `RateLimited`, `Timeout`, `Network`,
    `BadResponse`).
- After either ending the buffer is cleared and **the capture stays open**: the next utterance is
  the next attempt, which is what makes the auto re-arm of §6 possible without a round trip.

A new utterance that closes while a login transcription is still in flight **cancels** the one in
flight and replaces it, mirroring how a command that starts a turn replaces whatever was queued
(`session.rs:254`): the last thing said is the thing that counts.

### 4.2 Commands that do not belong to the login

While a capture is active, `Ask`, `Say`, `ListenStart`, `ListenStop`, `Reset`, `Cancel`,
`SetProfile`, `SetListeningMode` and `TestProvider` are **ignored**, and the code says so where it
ignores them. It asked for a `tracing::debug!` line and does not get one: no crate in this workspace
depends on `tracing`, and the v1 spec's logging (§12) owns that dependency — a flavour feature does not
introduce the logging sink. This is deliberate and not a swallowed failure: none of our clients can
send them (the screen is modal and has no other action), and inventing an `ErrorCode` variant for a
state no user can reach would be wire surface with no user behind it. `Hello` is unaffected — it
happens before the screen — and the login's own three commands are the ones that work.

## 5. Configuration

```toml
[login]
# The screen appears only when this is true *and* an entry below has keywords. A fresh install has
# neither, so nobody meets a locked door by surprise.
enabled = true
# The microphone opens with the screen and only speech the detector confirms is transcribed. Typing
# always works and costs nothing, so `false` is a perfectly good way to keep the ritual.
voice = true

[[login.allowed]]
name = "Andrés"              # what the screen greets you with; may be empty
keywords = ["andres", "core"] # all of them must be in the answer; normalized as written below
```

`LoginConfig { enabled: bool, voice: bool, allowed: Vec<LoginEntry> }` and
`LoginEntry { name: String, keywords: Vec<String> }` live in `config/model.rs` beside
`ServerConfig`, `Config` gains `pub login: LoginConfig`, and the whole section is `serde(default)`
like its neighbours. Defaults: `enabled = true`, `voice = true`, `allowed = []` — with no entries
`applies` is false and the screen never appears, so the defaults are safe for a file written before
this feature.

### 5.1 The rules

- **R1 — the screen applies** iff `enabled` **and** at least one entry has at least one keyword that
  is non-empty after normalization. This one function (`login::applies`) feeds three things: the
  server's `Welcome.login`, the server's `LoginStart`/`LoginAnswer` behaviour, and the test that the
  two agree.
- **R2 — normalization.** `tokens(s) -> Vec<String>`: lowercase each character (`char::to_lowercase`,
  which can yield more than one), fold the Latin letters this project's profiles actually use
  (`áàäâãå→a`, `éèëê→e`, `íìïî→i`, `óòöôõ→o`, `úùüû→u`, `ýÿ→y`, `ñ→n`, `ç→c`; anything else keeps its
  own lowercase form), then split on every character that is not alphanumeric (`char::is_alphanumeric`)
  and drop the empty pieces. So `"ANDRÉS, del CORE!!"` → `["andres", "del", "core"]`.
- **R3 — a keyword matches** when its tokens appear as a **contiguous run** in the answer's tokens
  (a one-token keyword is the common case; a multi-word one such as `"andres del equipo"` must be
  said together). Whole tokens only: `"score"` is not `"core"`, and `"andresito"` is not `"andres"`.
- **R4 — an entry matches** iff **every** one of its keywords matches.
- **R5 — the first matching entry in file order wins** and its `name` is the greeting. File order is
  the user's ranking, and it is the reason the array is documented as ordered.
- **R6 — keywords are normalized by `tokens` too**, at judging time: a keyword written `"Andrés"`
  behaves exactly like `"andres"`, and an empty or whitespace-only keyword is ignored rather than
  matching everything.
- **R7 — the answer is compared whole**, trimmed, with no length limit beyond what can be typed
  (§6 caps the input at 200 characters).

### 5.2 Worked examples

| Answer | Entries (in file order) | Verdict |
|---|---|---|
| `yo soy Andrés del equipo del core` | `[andres, core]` | ok — "Andrés" |
| `ANDRÉS, del CORE!!` | `[andres, core]` | ok — case and punctuation are the same answer |
| `soy andres del score` | `[andres, core]` | refused — `score` is one token and it is not `core` |
| `soy Andrés` | `[andres, core]` | refused — `core` is missing |
| `soy María, conozco a Andrés del core` | A `[andres, core]`, then B `[maria, data]` | ok as **A**: the first entry in file order wins |
| `soy Maria de data` | A, then B | ok as **B** |
| `el niño del core` | `[nino, core]` | ok — `ñ` folds to `n` |
| `soy Andrés` | `[andrés del equipo]` | refused — a multi-word keyword must be said in one run |
| anything | `[]` as the only entry | **no screen at all** (R1), and `judge` would never match it (R4, R6) |

The matcher is a pure function in a new `crates/tipsy-compadre/src/login.rs`
(`applies`, `tokens`, `judge -> Verdict { ok, name }`), with no new dependency: the fold is a
`match` over the characters a Spanish or Galician profile needs, and it is unit-tested against the
table above.

### 5.3 The section survives a save

The configuration screen has no Login tab — still true, and `ConfigTab`'s four variants are why — but it
**rewrites the whole file** on save, so `config/form.rs` must carry `[login]` through untouched: a save
that dropped a hand-written allowlist would be data loss, and the store's round-trip test covers it
exactly as it covers `token` today. Since 2026-10-12 the section also **crosses the wire** inside
`ConfigView`, so the round trip is no longer the only thing keeping a hand-written allowlist alive; the
file is still the only place it is *authored*.

## 6. The screen

The widget is the whole interface: the core, one status line, the input box, and the two borders that
carry the microphone indicator and the keys.

```
        ┌─ ◉ tipsy-compadre ─────────────── ● rec ─┐
        │                  ◉                        │   the core, in Braille, pulsing with the mic
        │  ¿Y tú quién eres? Dímelo, que aquí no…   │   one line, four jobs (§6.1)
        │  > andres del equipo del core_            │   the input box
        └─ Tab micro · Enter entrar · esc salir ─────┘
```

- **Geometry.** Centred: `w = min(48, area.width - 4)`, `h = min(13, area.height - 2)`; inside the
  border, the core takes `h - 4` rows when that is at least 3, then one row of status and one row of
  input. At `h = 13` that is 9 rows of core — the same room the HUD's own core panel gets at its
  minimum.
- **The core** is the HUD's, not a copy of it: the canvas inside `draw_rings` (`render.rs:516`,
  `RING_VIEW = 50.0`, `Marker::Braille`, the three arcs and the pulse) moves into a shared helper that
  both callers use. The login draws it **without** the HUD's own "core" panel, because the widget's
  border is its frame, and with the same `state.voice_level()` pulse, which is the only feedback the
  screen needs to say "I am hearing you".
- **The title** on the top border is the application's name (`tipsy-compadre`), not the profile's
  title: the profile speaks from inside, and the badge it wears on the HUD's header is behind the
  veil. No string table entry is needed, and the widget has exactly one title to draw.
- **The microphone indicator** is the widget's title, right-aligned, in red: `● rec` while the
  capture is open, nothing while it is not. The v1 rule — a visible indicator whenever the mic is
  open — is satisfied without a header, and the client sets `state.mic_open` while the login capture
  runs exactly as push-to-talk does.
- **The keys** are the bottom border: `Tab micro · Enter entrar · esc salir`, and from the third
  refusal in a row it becomes `Tab micro · Enter entrar · ¿olvidado? [login] en config.toml`. That
  swap is the whole escape hatch, and it is why a locked-out owner is never stuck.
- **Degradation.** With no room for the core (`h - 4 < 3`) the widget shrinks to `h = 4` — border,
  status, input — and keeps the two lines. On a frame too small for a bordered widget
  (`area.width < 20` or `area.height < 4`) it draws the bare input row on the last line of the frame,
  no border and no core, and the keys still work — the login is never the thing that makes a tiny
  terminal unusable.
- **Hits.** Only what is drawn reports a [`Hit`]: the title area answers `Tab` and the input row
  answers `Enter`. Everything behind the veil reports nothing, so a click on the dashboard cannot
  open the configuration screen, clear the conversation or replay a reply while the door is shut.

### 6.1 One line with four jobs

The status line, in this precedence:

| Phase | Line |
|---|---|
| `Passed` | `Pasa, Andrés.` (or `Pasa.` when the entry's `name` is empty) |
| `Rejected` | `Anda, que te peinen para el lado.` |
| `Judging` after `LoginHeard` | `Te he oído: «yo soy Andrés del equipo del core»` (truncated with `…`) |
| transcribing | the existing `strings.transcribing` |
| otherwise | the prompt |

Typing does not clear a refusal: the input box and the status line are different rows, and seeing
the refusal while typing the next try is the point.

### 6.2 Keys and transitions

| Key | Effect |
|---|---|
| `Char(c)` | append, up to 200 characters |
| `Backspace` | delete the last character |
| `Enter`, box non-empty | `Command::LoginAnswer { text: trimmed }`, box cleared, phase `Judging` |
| `Enter`, box empty, phase `Rejected` | re-arm the microphone: `Command::LoginStart`, refusal cleared, phase `Listening` |
| `Enter`, phase `Passed` | `UiAction::LoginDone` — the client closes the screen and enters the HUD |
| `Tab` | capture on → `LoginStop` (phase `Idle`); capture off → `LoginStart` (phase `Listening`). With `voice = false` the border shows no `Tab` hint and `Tab` does nothing: the switch is an explicit choice, and typing always works, so nothing is lost by honouring it |
| `Esc` | `UiAction::Quit` (on the web client, the same key closes the connection) |
| mouse, title | as `Tab` |
| mouse, input row | as `Enter` |
| anything else, including `F9` and the gear | ignored: while the door is shut there is no second screen |

The state the client and the UI share is `LoginState` in `tipsy-ui`, held as
`UiState.login: Option<LoginState>` beside `config_form`, with `render::draw` gaining a branch of its
own (§7). Transitions the client owns:

- On `Welcome` with `login: Some { voice }` → `UiState::show_login(LoginState::new(lang, voice))`;
  with `voice`, the client opens the input stream (the same one push-to-talk uses) and sends
  `LoginStart` once the first frame is in hand; a mic error sends nothing and leaves the screen
  typing-only with the mic line of §8. With `voice = false` no stream is opened at all.
- On `LoginVerdict { ok: true }` → phase `Passed`, and if a capture is open the client sends
  `LoginStop` and closes the stream. The screen **stays** until `Enter`: the greeting is the reward
  for getting in, and `Enter` is one key.
- On `LoginVerdict { ok: false }` → phase `Rejected`, attempt + 1. Attempts 1 and 2 leave the capture
  open, so the next thing said is the next try with no key at all. On the third, the client sends
  `LoginStop`, closes the stream and swaps the border to the escape hint; `Tab` re-arms the capture
  **and resets the streak**, because an explicit key is consent and the guard's whole job is to stop
  *automatic* listening.
- On `Event::Error` while the screen is up → the status line, never the attempt counter:
  `NothingHeard` becomes `No te he oído. Repite, si eso.` and the capture (if open) keeps listening;
  any other code shows the server's message — the provider's own words — and turns the microphone
  off, because retrying the voice would only repeat the failure. Errors are keyed by **code** for the
  strings the UI owns, not by the server's message, which is English for `NothingHeard` today.
- `UiAction::LoginDone` → `state.login = None`, and the client stops the login's microphone unless
  the listening mode wants it open.

`LoginState` also keeps what is typed, the phase, the attempt streak and the current line; the
transitions above are the whole of its behaviour, and it is unit-tested without a server (§11).

## 7. The veil

While the login screen is up, `render::draw` stops being the same function it was: the HUD is drawn
**whole** — header, clock, log, rings, oscilloscope, system, input, help — and then painted over.

```
draw HUD as today ──▶ veil(frame.buffer_mut(), widget_rect) ──▶ Clear(widget_rect) ──▶ widget
```

- **The veil** walks every cell of the frame's buffer outside the widget's own rectangle and rewrites
  it: a cell that carried a symbol becomes `▒` and keeps its colour, darkened by a fixed map; an empty
  cell becomes `░` in a near-black grey. The result is a dashboard that is visibly there, visibly
  out of focus, and unreadable — and the core inside the widget, drawn afterwards, is the only crisp
  thing on screen, which is the whole effect.
- **The colour map** covers the palette the HUD actually uses, and any colour it does not know is kept
  as it is (`▒` already halves it): `Cyan → Rgb(0,80,80)`, `Red → Rgb(90,30,30)`,
  `Yellow`/`GOLD → Rgb(90,70,30)`, `Green → Rgb(30,80,30)`, `White → Rgb(150,150,150)`,
  `Gray`/`DarkGray → Rgb(90,90,90)`. An explicit map rather than `Modifier::DIM` because SGR 2 is
  uneven across backends and a map is deterministic and testable — the render test asserts the
  mapping and that the widget's own cells were left alone.
- **Two strengths, one constant.** `Veil::Blocks` is the default above. `Veil::Dim` is the other
  extreme, one constant away in `render_login.rs`, for a font or terminal where `▒`/`░` land badly:
  every inked cell keeps its own symbol and takes the darkened colour, and empty cells stay empty.
  Both strengths are tested, so switching is not a rewrite.
- **The hits.** `draw` returns only the widget's hits while the login is up. The HUD's own — the
  gear, the mode indicator, the log rows, the replay — are computed and thrown away, exactly as the
  configuration screen already discards the panes underneath it.

The cost is one pass over the frame's buffer per redraw. If it ever shows on the WASM build, the veil
is static enough to be applied every other frame; §13 keeps that as a note rather than a design.

## 8. Copy

All of it lives in `tipsy-ui/src/strings.rs`, in the two static tables, picked by the profile's
language as everything else is. There are no `[login.copy]` overrides (§1). The `es` column is the
copy accepted on 2026-10-11, and it is the record: the profile's voice is the user's to change, and
changing it changes no code path.

| Key | es | en |
|---|---|---|
| `login_prompt` | `¿Y tú quién eres? Dímelo, que aquí no entra cualquiera.` | `So who are you, then? Tell me — this door isn't for just anyone.` |
| `login_heard` | `Te he oído: «{text}»` | `I heard: "{text}"` |
| `login_refusal` | `Anda, que te peinen para el lado.` | `Off you go, and get your hair combed somewhere else.` |
| `login_pass` | `Pasa, {name}.` | `Come in, {name}.` |
| `login_pass_anon` | `Pasa.` | `Come in.` |
| `login_nothing_heard` | `No te he oído. Repite, si eso.` | `I heard nothing. Try again, if you like.` |
| `login_no_mic` | `Sin micro: escribe, que también vale.` | `No mic: type it, that works too.` |
| `login_rec` | `● rec` | `● rec` |
| `login_keys` | `Tab micro · Enter entrar · esc salir` | `Tab mic · Enter in · esc quit` |
| `login_keys_lost` | `Tab micro · Enter entrar · ¿olvidado? [login] en config.toml` | `Tab mic · Enter in · forgotten it? [login] in config.toml` |

`transcribing` is reused as it stands for the moment between the utterance and the verdict.

## 9. Errors and edge cases

| Case | Behaviour |
|---|---|
| Mic denied or unavailable when the screen opens | No `LoginStart`, no stream: typing only, and the status line says `Sin micro: escribe, que también vale` once. The border **keeps** its keys: the copy's `Tab micro` is frozen and the screen does not reword it, so the line is what tells the user the microphone is out |
| Mic lost mid-capture | The client sends `LoginStop`, closes the stream, and the same line appears; typing carries on |
| No transcribe key configured, or the provider fails | The `Event::Error` message on the status line and the microphone off; the screen keeps working, and the configuration screen is one `Esc` + `F9` away once the user is in — or one `enabled = false` away right now |
| Nothing heard (silence, a cough) | `NothingHeard`, the string above, **no attempt counted**, the capture stays open |
| An answer without the keywords | The refusal line, attempt + 1, the microphone still open for attempts 1 and 2 |
| Three refusals in a row | The client closes the capture and swaps the border to the escape hint; `Tab` re-arms and resets the streak |
| A command that does not belong to the login | Ignored, and the code says so where it ignores them (§4.2) |
| `enabled = true` with no usable keywords | `applies` is false, `Welcome.login` is `None`, the HUD opens directly. A mis-edited file can never slam the door |
| A keyword written with accents or capitals | Normalized by `tokens` (R6), so it matches as written |
| A `Ctrl`-chord on the login screen | Not an answer: the client's key mapping must drop a chord it cannot represent rather than turn it into a character, and `ctrl+c` stays the terminal's interrupt. The plan checks the existing mapping and adds nothing if it is already right |
| A frame too small for the widget | The bare input row, no hits, keys still work (§6) |

## 10. Privacy and cost

- **The microphone opens with the screen** when `voice = true`, and the widget says so: a red `● rec`
  in its own title, which is the v1 rule about a visible indicator kept without a header. `Tab` closes
  it, `Esc` leaves, and `voice = false` means no audio ever leaves the machine.
- **Nothing reaches the provider until the local detector confirms speech — and it is the *server's*
  detector that says so.** The login capture runs `VadState` (`session.rs`'s `login_vad`) in front of
  the transcriber exactly as `auto` mode does: the silence, the room tone and the chair creak before
  the first syllable are dropped, and only what was confirmed as the user's speech reaches the
  provider. That is a stronger guarantee than push-to-talk gives it. **In the browser the wording is
  "reaches the provider", not "leaves the machine"**: `tipsy-web` has no detector at all, so every
  20 ms frame its open capture produces is uploaded to `/ws` — the same frames push-to-talk uploads —
  and the server's own VAD is what keeps the silence out of the transcription call. A client that
  wanted to filter its own uploads would have to bring a detector with it; what the browser does owe,
  and enforces, is that no frame leaves the page while no capture is open, and that a frame the door
  refuses is dropped rather than kept (the phase review's finding 6, which corrected this bullet).
- **One transcription call per finished utterance**, capped at 15 s each, and only the transcribe
  key is spent: no chat call and no speech call happens before the HUD. That is precisely why
  `TurnInput::Utterance` was rejected (L4) — reusing it would have answered the login out loud and
  written it into the conversation.
- **The greeting is not part of the door either.** While a login screen applies, the session withholds
  the profile's greeting and speaks it when a verdict opens: nothing is said behind the widget, and no
  speech is spent on a visitor who is refused. The server speaks it the moment the verdict is on the
  wire. It cannot see the client's HUD and does not wait for it — whether the HUD is on screen by the
  time the greeting's first `SpeechChunk` arrives is the client's own business (§6). Two consequences
  are deliberate: a client that never answers never hears the greeting, and a `LoginStop` taken while the
  greeting is still owed leaves it owed, so a later typed `ok` speaks it. When no screen applies, the
  greeting is exactly what it is today.
- **An idle room costs nothing**: with a 1 s silence budget, a detector that needs 48 ms of confirmed
  voice to open an utterance, and the three-attempt guard, background noise buys no transcriptions.
- **`docs/architecture.md` gets one sentence**: the promise in §"Listening modes" ("nothing is sent to
  any cloud service until the wake word is detected") describes `hands_free`, and the login screen is
  an explicit, visible recording that sends VAD-confirmed speech and nothing else. The amendment is
  part of this feature, not a footnote.

## 11. Testing

| Where | What |
|---|---|
| `crates/tipsy-compadre/src/login.rs` | `tokens` over accents, capitals, punctuation, digits, an empty answer and a non-breaking space; whole-token matching (`score` vs `core`, `andresito` vs `andres`); a multi-word keyword only as a contiguous run; an entry with no keywords never matching; first-match-in-file-order; `applies` for enabled/disabled, no entries, and entries whose keywords are blank; §5.2 as a table-driven test |
| `crates/tipsy-compadre/src/config/store.rs` | `[login]` survives a save/load round trip; a file written before this feature loads with `enabled = true`, `voice = true`, no entries, and is **not** rewritten on load; a save from the configuration screen keeps a hand-written `[[login.allowed]]` intact (§5.3) |
| `crates/tipsy-protocol/tests/roundtrip.rs` | the three commands and the two events round-trip; `Welcome` with `login: Some(LoginView { voice })` round-trips; a `Welcome` JSON without the field parses to `login == None` |
| `crates/tipsy-ui` (state) | the 200-character cap; backspace on an empty box; `Enter` with an empty box re-arming only after a refusal; `Tab` with `voice = false` doing nothing; the streak resetting on an explicit re-arm; `LoginDone` only after `Passed`; `LoginHeard` before `LoginVerdict` in the line; `NothingHeard` never counting an attempt; the phase after each event |
| `crates/tipsy-ui` (render) | the widget's geometry at three frame sizes (full, no room for the core, too small to border); the core missing exactly when `h - 4 < 3`; hits reported only for what is drawn; the veil rewriting the cells outside the widget, darkening by the §7 map, and leaving the widget's own cells untouched; `Veil::Dim` doing the other thing |
| `crates/tipsy-compadre/tests/` (integration) | a session driven by a scripted scorer (the shape `vad.rs`'s own tests already use) and a fake transcription provider (the brain's integration tests already build one): `LoginStart` + frames → `LoginHeard` + `LoginVerdict`, and **no** `Reply`, `ReplyDelta` or `SpeechChunk` in between; a wrong answer → `ok: false`; a typed answer with the login off → `ok: true`; silence → `NothingHeard` and no verdict; a command outside the login during a capture being ignored; a second utterance replacing a transcription in flight |
| `crates/tipsy-compadre/src/client.rs` | `Welcome { login: Some }` showing the screen and `None` going straight to the HUD; `voice = false` opening no stream; the login's `LoginStart`/`LoginStop` on open/close; `Esc` quitting; `mic_open` true only while the capture runs |

## 12. What changes, where

| File | Change |
|---|---|
| `crates/tipsy-protocol/src/lib.rs` | Three `Command`s, two `Event`s, `LoginView`, and `Welcome.login` with `#[serde(default)]` |
| `crates/tipsy-ui/src/state.rs` | `UiState.login`, `LoginState` (input, phase, streak, line), the key handling of §6.2, `UiAction::LoginDone`, `show_login` |
| `crates/tipsy-ui/src/render_login.rs` *(new)* | The widget, the two borders, the geometry and degradation of §6, and `veil` with its two strengths |
| `crates/tipsy-ui/src/render.rs` | `draw` gains the login branch (HUD → veil → widget, and only the widget's hits); the ring canvas moves into a helper `draw_rings` and the login both use |
| `crates/tipsy-ui/src/strings.rs` | The ten strings of §8, es and en |
| `crates/tipsy-ui/src/lib.rs` | The new module and its re-exports |
| `crates/tipsy-compadre/src/config/model.rs` | `LoginConfig`, `LoginEntry`, `Config.login` |
| `crates/tipsy-compadre/src/login.rs` *(new)* | `applies`, `tokens`, `judge`, `Verdict` — the matcher of §5, pure and unit-tested |
| `crates/tipsy-compadre/src/session.rs` | `login_active`, `login_vad`, `login_mic`; the dispatch branch ahead of the listening one; `login_start`/`login_stop`/`login_answer`/login utterance ending; the deferred transcription task and its cancellation; the ignore rule of §4.2 |
| `crates/tipsy-compadre/src/config/form.rs` | Carries `[login]` through a save untouched (§5.3) |
| `crates/tipsy-compadre/src/client.rs` | The `Welcome` branch, the login's mic open/close, the key routing, `mic_open` |
| `docs/architecture.md` | The one-sentence amendment of §10 |

## 13. Risks and follow-ups

- **The veil is an aesthetic bet.** `▒` and `░` are in every font that matters, but the effect is a
  judgement call on a real terminal: `Veil::Dim` is one constant away and both are tested. This is
  the only part of the feature that can be "wrong" without being broken.
- **Veiling the HUD costs a buffer pass per redraw.** Cheap on crossterm; measure it on the WASM
  build before Plan 3 ships, and halve the rate if it shows.
- **`[login.copy]`, and a per-profile login voice**, are the obvious next wants. `LoginView` is
  shaped to carry them (`Option<LoginView>` gains fields without touching `Welcome` again), which is
  why the copy is a table today and a wire field later.
- **A Login tab** in the configuration screen, so nobody has to hand-edit TOML to add a teammate.
- **Speaking the prompt** with `Command::Say`: the ritual would be complete, and it costs a speech
  call before the door and a speech key on a fresh install.
- **Deliberately left out**, and to be said out loud whenever this feature is described: passwords,
  lockouts, rate limiting, per-person tokens, attempt logs, a re-login on reconnect, and any claim
  that this screen protects anything. It does not. The token does.
- **Plan 3 owes this feature two things, and its own plan as written does not say so.** Checked on
  2026-10-11 against `docs/superpowers/plans/2026-10-11-plan-3-web-tier.md` (19 tasks in three phases;
  the login screen appears in it only as "in flight, in its own worktree" and in a comment saying an
  HUD test is about the HUD):
  1. **The browser wiring.** §3 hands the web client the same screen and says the server needs no
     change for it, but the *wiring* is per client: `tipsy-web` must show the screen on
     `Welcome.login`, drive its own capture (`getUserMedia` → `Frame::Mic`) in step with
     `LoginState::open`, send the same three commands and honour the once-per-run rule. None of Plan
     3's Phase 3 tasks (3.2–3.6) mentions it, so it needs a task or an explicit extension of one.

     **Landed 2026-10-12, in Plan 3's Task 3.7.** `tipsy-web`'s `login_door.rs` is the engine around
     the screen `tipsy-ui` already owns, and none of the screen is reimplemented: `Welcome.login` goes
     through `UiState::show_login` — which *is* the once-per-run rule, so the browser keeps no bit of
     its own and a re-sent `Welcome` cannot put the veil back — the capture is reconciled on
     `LoginState::open`'s edges exactly as the native client reconciles its own, and the three
     commands need no browser-specific code at all: `Tab`, the third refusal and a returned-to-empty
     box already raise `UiAction::Send(LoginStart | LoginStop)` and a typed answer
     `UiAction::Send(LoginAnswer { text })` from `UiState::on_key`, down the same socket path as every
     other command. The one thing the browser had to answer for itself is the frame gate, and the
     answer is a single predicate: `Door::sends_frames` reads the screen's own `voice` and `open` and
     is the only thing between the captured frames and `Outbox::send`, so **no captured frame leaves
     the page while the door is shut** — which closes the window §10's first hop used to have, where
     the press-to-start click set the HUD's microphone flag before `Welcome` even arrived and room
     tone travelled to `/ws` from that press on.
  2. **Two sentences of this spec that Plan 3 supersedes.** Its Task 1.1 extends `LoginView` with
     `enabled` and `allowed` (for `ConfigView`, so the configuration screen can show and edit the
     section over the wire). That makes §1's "no Login tab" non-goal and the L2 sentence "the allowlist
     never leaves it" no longer true as written. The *judging* stays server-side and the token stays
     the only door, so the decision itself survives; what has to change is the sentence. Amend both
     when Plan 3 lands, and say in Plan 3's own record that it made the section editable.

   **Landed 2026-10-12, in Plan 3's Task 1.1.** The section is on the wire as `ConfigView.login`, of
   type **`LoginSettingsView`** — not `LoginView`, which is `Welcome`'s login-*screen* payload and
   already existed; the two are different things and the name was the first thing the implementer
   could not compile past. The configuration screen's *tab* is **not** built: the screen carries
   `[login]` through untouched, and a client with the token can read and change the section over the
   socket. The browser wiring of §13's item 1 above is Plan 3's **Task 3.7**, in Phase 3, and the
   once-per-run rule it must honour lives in `UiState::show_login`'s return value rather than in a bit
   the browser keeps for itself.
- **One manual check before release.** With the system microphone disabled, the screen must open
  typing-only and say `Sin micro: escribe, que también vale`. The *decision* is tested — a failed
  `Mic::start` leaves no capture and the line above — but the *device* is not: nothing on this machine
  can make a device fail to open, and push-to-talk's own refusal has never had a test either. Also
  worth a human eye, because no test can see it: the veil's look on a real terminal (`Veil::Dim` is the
  escape hatch if `▒`/`░` land badly), and the core's pulse following the voice while the door is shut.
- **The same check in a browser, and it is Plan 3's Task 3.7's to hand over.** The web client's login
  door is wired by the same state machine, so the *decisions* are tested; the *device*, the *font* and
  the *pulse* are not, and no browser has ever loaded this code. The recipe and the checklist are in
  the README's **Release checklist** ("The login door in the browser"), and the one ordering that
  matters is that the typing-only path is checked **first**, with the microphone really taken away from
  the OS: that is the branch no test can reach. Record the browser and its version, whether the
  typing-only line appeared, whether the veil was readable, and whether the core pulsed — and if
  `Veil::Dim` had to be switched on, say so, because both strengths are tested and the constant is the
  only thing that changes.
- **A change the user has asked for, not yet designed (2026-10-12).** On a **correct** answer the screen
  should come down and the veil should lift **at once**, with no `Enter` in between — the opposite of
  §6.2's "The screen stays until `Enter`". The argument is §7's own: the veil exists to hide the HUD
  from someone who has *not* identified themselves, and once the verdict is `ok` they have, so veiling
  them afterwards protects nothing and reads as a defect. The greeting does not depend on the keypress
  (§10 has the verdict speak it), so nothing is lost by closing the door in the same arm — and
  `LoginPhase::Passed` would then have no state to sit in, which is a simplification worth taking
  deliberately rather than by accident. `render.rs` needs no change: the veil is a function of
  `login.is_some()`. **Nothing has been changed yet.** The code paths and the five tests it touches are
  in this plan's workspace, `.superpowers/sdd/2026-10-11-plan-login-screen/follow-ups.md`, beside the
  one defect the reviews parked.
