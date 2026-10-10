# tipsy-compadre v1 — Design

- **Date:** 2026-10-10
- **Status:** awaiting review
- **Companion documents:** [`docs/architecture.md`](../../architecture.md) (diagrams),
  [`docs/research/2026-10-10-providers-and-models.md`](../../research/2026-10-10-providers-and-models.md)
  (providers, prices, local models)
- **Prior art:** the proof of concept in `poc/` (ratatui dashboard + Python worker)

## 1. Purpose

tipsy-compadre is a voice assistant for software developers with a strong, affectionate regional
personality. The default profile, *El Compae*, is a lazy, joking, slightly tipsy Andalusian who
knows Jira, GitHub, GitLab, CI/CD and code review, dodges work with developer excuses and, in the
end, always gives a correct and useful answer.

The project is meant to be shared with the community: a single executable, cloud providers that
anyone can use with their own key, and profiles that anyone can write for their own region.

### Success criteria

1. A user downloads one executable, enters one API key in the configuration screen and talks to
   the assistant from the terminal within five minutes.
2. Time from the end of the user's speech to the first audible sentence: under 1.5 s with
   ElevenLabs and under 3 s with Gemini TTS, on a typical home connection.
3. The same interface works in a terminal, in a browser (WASM) and through the web terminal.
4. A new profile can be added by pasting one `[[profiles]]` block into `config.toml`.
5. Builds and tests pass on Windows, Linux and macOS in CI.

### Non-goals for v1

- Local text-to-speech or speech-to-text (v2, through a Python sidecar or native bindings).
- Reading real Jira tickets, pull requests or pipelines through their APIs (later; the assistant
  must not pretend to have this access, see §8.3).
- Translating the interface into languages other than Spanish and English.
- Deepgram and Anthropic connectors (v1.1).
- Voice cloning of any kind.

## 2. Decisions log

| # | Decision | Alternatives considered |
|---|---|---|
| D1 | v1 is pure Rust and cloud only; local speech arrives in v2 | Local Qwen3-TTS from day one |
| D2 | OpenAI-compatible base connector plus native Gemini and ElevenLabs connectors | Only OpenAI-compatible |
| D3 | `config.toml` in the user config directory; keys in the OS credential store, overridable by environment variables | `.env` file; credential store only |
| D4 | Profiles are a `[[profiles]]` array inside `config.toml`, each with its own language | One file per profile |
| D5 | Client-server: the native executable is the server; clients only draw and move audio | Monolithic TUI |
| D6 | The web client is the same ratatui TUI compiled to WASM (Ratzilla); no separate HTML app | HTML/TypeScript web app |
| D7 | A built-in web terminal (xterm.js + PTY) is also part of v1 | External `ttyd` only |
| D8 | Cargo workspace with four crates, one executable embedding the WASM bundle | Single crate (rejected: the shared UI must compile to WASM) |
| D9 | tokio actors talking through channels | Plain threads |
| D10 | Three listening modes in v1: `push`, `auto`, `hands_free`; hands-free is the last milestone | Push only; hands-free in v1.1 |
| D11 | Code, comments and documentation in English; profiles and interface strings in the profile's language (Spanish or English) | Spanish everywhere |
| D12 | **Terminal first**: every feature must work in the native terminal before it is done | — |

## 3. Architecture

See [`docs/architecture.md`](../../architecture.md) for the diagrams. Summary:

- **Server** (native): `brain`, `providers`, `listener`, `config`, `sysmon` and one `session` per
  connected client, all tokio actors.
- **Clients**: native TUI (crossterm, `cpal`, `rodio`), web client (same TUI in WASM through
  Ratzilla, browser audio) and web terminal (native TUI in a PTY streamed to xterm.js).
- Clients and server speak the protocol in §4, over an in-process channel (native TUI in the same
  process) or a WebSocket.

### 3.1 Workspace

| Crate | Kind | Contents | WASM |
|---|---|---|---|
| `tipsy-protocol` | lib | `Command`, `Event`, audio frame format | yes |
| `tipsy-ui` | lib | HUD state, widgets, configuration screen, es/en strings; backend-agnostic, no audio, no networking | yes |
| `tipsy-web` | cdylib | Ratzilla backend, browser mic (`getUserMedia`) and playback (WebAudio), WebSocket transport | target |
| `tipsy-compadre` | bin | Server, native TUI client, web terminal, embedded web bundle | no |

Plus an `xtask` crate providing `cargo xtask dist` (build the WASM bundle, then the binary that
embeds it).

Dependency rules:

- Only `tipsy-ui` knows ratatui widgets; only `tipsy-compadre::providers` knows HTTP.
- `tipsy-protocol` and `tipsy-ui` must not depend on tokio's multi-threaded runtime, `cpal`,
  `rodio`, `keyring` or `sysinfo`.
- `brain` is tested with fake providers, without network or UI.

### 3.2 Run modes

| Command | What runs |
|---|---|
| `tipsy-compadre` | Server and native TUI in one process (in-process channel) |
| `tipsy-compadre serve` | Headless server; web client at `/`, web terminal at `/terminal` |
| `tipsy-compadre --connect <ws-url> [--token <t>]` | Native TUI only, connected to a remote server |

## 4. Protocol

Transport: an in-process `tokio::sync::mpsc` pair, or one WebSocket per client. Text frames carry
JSON; binary frames carry audio. All types live in `tipsy-protocol` and derive `serde`.

### 4.1 Commands (client → server)

| Command | Fields | Effect |
|---|---|---|
| `Hello` | `token: Option<String>`, `client: Tui \| Web \| WebTerminal`, `protocol_version: u32` | First message; the server replies `Welcome` or closes |
| `Ask` | `text` | Ask the profile; it answers with voice |
| `Say` | `text` | Speak `text` literally, without the LLM |
| `ListenStart` / `ListenStop` | — | Open or close the mic turn (meaning depends on the listening mode, §6) |
| `Cancel` | — | Barge-in: stop the current reply (§5.3) |
| `Reset` | — | Forget this session's conversation |
| `SetProfile` | `id` | Switch profile for all sessions and persist it |
| `SetListeningMode` | `mode` | Switch mode and persist it |
| `GetConfig` | — | Ask for the redacted configuration |
| `UpdateConfig` | `config: ConfigView` | Replace the editable configuration and persist it |
| `SetKey` | `name`, `value` | Store an API key in the credential store |
| `TestProvider` | `capability: Chat \| Speech \| Transcribe` | Make a minimal call and report the result |

### 4.2 Events (server → client)

| Event | Fields | Meaning |
|---|---|---|
| `Welcome` | `session_id`, `profile: ProfileView`, `ui_language`, `listening_mode`, `web_bundle: bool` | Handshake accepted |
| `Status` | `state: Idle \| Listening \| Transcribing \| Thinking \| Speaking`, `detail: Option<String>` | Drives the header |
| `Heard` | `text` | What the transcription understood |
| `ReplyDelta` | `text` | Streamed reply text |
| `Reply` | `text` | Final, cleaned reply text |
| `SpeechChunk` | `seq: u32`, `sample_rate: u32`, `last: bool` | Header for the binary frame that follows |
| `Done` | `first_audio_ms`, `total_ms` | Reply finished |
| `Stats` | `cpu`, `ram`, `net_in`, `net_out` | Once per second |
| `Config` | `config: ConfigView` | Redacted configuration (keys shown as `sk-or-…a1b2`) |
| `ProviderTest` | `capability`, `ok`, `latency_ms`, `message` | Result of `TestProvider` |
| `ProfileChanged` / `ConfigChanged` | — | Broadcast to all sessions |
| `Error` | `code`, `message`, `recoverable` | Message already localized to the profile's language |

### 4.3 Audio frames

- **Mic (client → server):** binary frame `0x01` + PCM 16-bit little-endian, mono, 16 kHz,
  20 ms per frame. Clients resample if their device runs at another rate.
- **Speech (server → client):** binary frame `0x02` + `seq: u32` + PCM 16-bit little-endian,
  mono, at the `sample_rate` announced in the preceding `SpeechChunk`. The server decodes
  whatever the provider returns (MP3, raw PCM…) so clients only ever play PCM.
- Clients compute the audio envelope for the visualisers locally from what they record and play.

`protocol_version` starts at 1. A mismatch closes the connection with a readable error.

## 5. The brain

### 5.1 Flow of one question

1. `listener` produces an `Utterance` (PCM) or the client sends `Ask`.
2. Transcription (`TranscribeProvider`) with the profile's language hint → `Heard`.
3. Chat (`ChatProvider::stream`) with the session history → `ReplyDelta`s.
4. A **sentence splitter** accumulates text and emits a sentence when it ends in `.`, `?`, `!` or
   `…` followed by whitespace and holds at least 25 characters (shorter fragments are merged with
   the next one).
5. Each sentence is **cleaned** (§5.2) and sent to `SpeechProvider`, up to 4 in parallel (1 for
   providers marked sequential).
6. Results are emitted **in order** as `SpeechChunk` + binary frame, even if a later sentence
   finishes first.
7. `Reply` (full cleaned text), then `Done`; the reply is appended to the history.

### 5.2 Text cleaning before speech

- Remove apostrophes glued to words (`pa'` → `pa`, `ná'` → `ná`).
- Remove Markdown markers (`` ` ``, `*`, `_`, `#`) and emojis.
- Collapse whitespace.

### 5.3 Barge-in

If the user starts speaking (VAD) or presses `Tab` while the assistant is talking, the client
sends `Cancel` and drops its audio queue. The brain cancels a `CancellationToken` shared by the
chat stream and all pending synthesis tasks. Only the sentences that were actually sent to the
client are kept in the history, so the LLM never assumes it said something the user did not hear.

### 5.4 Sessions

Each session has its own history. Configuration and active profile are global; changing them
broadcasts `ConfigChanged` or `ProfileChanged`.

## 6. Listening

| Mode | Starts | Stops |
|---|---|---|
| `push` | `ListenStart` (`Tab`) | `ListenStop` (`Tab`) |
| `auto` | `ListenStart` (`Tab`) | `silence_ms` of silence detected by the VAD, or `Tab` |
| `hands_free` | Wake word detected | `silence_ms` of silence |

- VAD and wake word detection run on the server over the mic frames a client streams. In
  `hands_free` mode the client streams continuously; nothing leaves the machine until the wake
  word fires.
- Detection is paused while the session is speaking and for 300 ms afterwards.
- Each profile may define its own `wake_word`.
- Clients show a visible mic indicator in the header whenever the mic is open; in `hands_free`
  it stays red for as long as the mode is active.
- In the browser, audio capture requires a user gesture; the web client shows a "press to start"
  screen until then.

Libraries to evaluate during planning (from prior knowledge, verify): **VAD** — `webrtc-vad`
or a Silero VAD model through `ort`; **wake word** — Rustpotter (pure Rust, trained from a few
recordings) or Porcupine (Rust SDK, proprietary license, requires an access key). Preference:
open-source options.

## 7. Providers

```rust
#[async_trait]
trait ChatProvider {
    async fn stream(&self, req: ChatRequest, cancel: CancellationToken)
        -> Result<BoxStream<'static, Result<String, ProviderError>>, ProviderError>;
}

#[async_trait]
trait SpeechProvider {
    async fn synthesize(&self, req: SpeechRequest, cancel: CancellationToken)
        -> Result<Pcm, ProviderError>;      // decoded, mono
    fn max_parallel(&self) -> usize;
}

#[async_trait]
trait TranscribeProvider {
    async fn transcribe(&self, pcm: Pcm, language: &str, cancel: CancellationToken)
        -> Result<String, ProviderError>;
}
```

`SpeechRequest` carries the text, the profile's `voice_instructions` and the voice resolved from
the profile's `voices` table for the active speech provider.

| Connector | Chat | Speech | Transcribe | Notes |
|---|---|---|---|---|
| `openai_compat` | `/chat/completions` (SSE) | `/audio/speech` | `/audio/transcriptions` | Covers OpenRouter, Ollama, LM Studio, Groq, OpenAI. Model quirks handled by a small rule table (e.g. Gemini via OpenRouter only returns `pcm`; only OpenAI and Gemini accept `instructions`) |
| `gemini` | Gemini API (streaming) | Gemini TTS with delivery instructions | — | Native API |
| `elevenlabs` | — | Streaming synthesis (WebSocket if available, else HTTP) | Scribe | Native API |

`ProviderError` distinguishes `Auth`, `RateLimited`, `Timeout`, `Network`, `BadResponse` and
`Cancelled`.

## 8. Configuration and profiles

### 8.1 File

`config.toml` in the user config directory (`%APPDATA%\tipsy-compadre\`,
`~/.config/tipsy-compadre/`, `~/Library/Application Support/tipsy-compadre/`). Created with
defaults on first run.

```toml
active_profile = "compae"

[server]
bind = "127.0.0.1:7070"
lan = false
token = ""               # server token; empty until the Server tab generates one

[listening]
mode = "auto"            # push | auto | hands_free
silence_ms = 1000
mic_gain = 12.0
# mic_device = "..."     # input device, native client only; omitted means the system default

[providers.chat]
kind = "openai_compat"   # openai_compat | gemini | elevenlabs
base_url = "https://openrouter.ai/api/v1"
model = "google/gemini-2.5-flash"
key = "openrouter"       # key name, never the key itself

[providers.speech]
kind = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
model = "google/gemini-3.8-flash-tts"
key = "openrouter"

[providers.transcribe]
kind = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
model = "openai/whisper-large-v3"
key = "openrouter"

[[profiles]]
id = "compae"
name = "El Compae"
language = "es-ES"
prompt = "…"
voice_instructions = "…"
greeting = "…"
wake_word = "oye compae"
[profiles.voices]
gemini = "Charon"
elevenlabs = "george"
```

The two keys are `active_profile` and `[[profiles]]`. A file written with the previous spelling —
`active_persona` and `[[personas]]` — still loads, silently and unchanged: both are serde aliases on
the same fields, so nobody has to rewrite a `config.toml`, and every save writes the new spelling.

The default transcription model slug must be verified during planning.

### 8.2 Keys

Resolution order for a key named `openrouter`: environment variable `TIPSY_KEY_OPENROUTER`, then
the OS credential store (service `tipsy-compadre`, account `openrouter`). The configuration screen
writes to the credential store and never displays a full key.

### 8.3 Profiles

- `language` (BCP 47) drives the transcription language hint, the interface language (`es-*` →
  Spanish, anything else → English) and the language in which `prompt`, `voice_instructions` and
  `greeting` are written.
- `voices` maps speech connectors to a recommended voice; the provider default is used when there
  is no entry.
- Built-in profiles ship as defaults inside the binary; a profile with the same `id` in the user's
  `config.toml` overrides it.
- Every prompt includes these rules, in the profile's language:
  - the assistant is a software developer's helper and always ends up giving a correct, useful
    answer, even after dodging;
  - it has **no access** to Jira, GitHub, GitLab or the user's code, never invents ticket
    contents, PR states or pipeline errors, and asks the user for them instead;
  - 2–3 short sentences, no Markdown, lists, emojis or stage directions; commands said plainly;
  - an original character: it never claims to be a real person or a character from films, books
    or other media, and never quotes them.

Built-in profiles for v1 (names and traits are a draft, to be refined while writing them):

| id | Name | Language | Flavour |
|---|---|---|---|
| `compae` | El Compae | es-ES | Andalusian: joking, tipsy, lazy, dodges work with developer excuses |
| `meigo` | O Meigo | es-ES | Galician: *retranca*, never gives a straight answer, "depende" |
| `chulapo` | El Chulapo | es-ES | Madrilenian: castizo swagger, has done everything before and better |
| `senyor` | El Senyor | es-ES | Catalan: *seny* and efficiency, counts every token like money |
| `good_ol_boy` | The Good Ol' Boy | en-US | US South: unhurried, folksy sayings, "bless your heart" about other people's code |
| `hustler` | The Hustler | en-US | New York: everything is urgent, knows a shortcut for everything |
| `mate` | The Mate | en-AU | Australian: "no worries", production is always "she'll be right" |
| `bloke` | The Bloke | en-GB | English: understatement, irony, complains about the weather and the CI alike |
| `polite_one` | The Polite One | en-CA | Canadian: apologises even when your code breaks production |

**Profile guideline (also in `CONTRIBUTING.md`):** regional humour is affectionate. Play with the
friendly clichés of a place; never mock or insult people or groups, never use slurs, never imitate
real people.

## 9. Interface

The proof of concept HUD is kept: header, local clock, core rings, voice oscilloscope, system
panel, CHAT/NETWORK tabs (CHAT/RED in Spanish) and question box. Changes:

- Header shows the profile's title, the status, a ⚙ button and the listening mode indicator
  (`push`, `auto`, `hands free`), red while the mic is open.
- **Configuration screen** (⚙ or `F9`), full screen, tabs:
  - *Providers*: connector, base URL, model and key name per capability; "Test" button
    (`TestProvider`) showing latency or the error.
  - *Listening*: mode, silence threshold, mic gain, mic device (native client only).
  - *Profiles*: active profile, multi-line editors for its texts, voices table.
  - *Server*: bind address, LAN toggle, token display and regeneration.
  - Changes apply on save, without restarting.
- Strings in Spanish and English, chosen from the active profile's language. Two static tables in
  `tipsy-ui`, no i18n framework.
- Keyboard and mouse: every action has both. Default shortcuts avoid keys browsers reserve
  (`F5`, `F11`, `Ctrl+W`…).
- Removed from the proof of concept: `F5` "new voice" (it relied on local VoiceDesign; back in v2).

## 10. Web client and web terminal

- **Web client (`/`)**: `tipsy-web` renders `tipsy-ui` with Ratzilla, captures the mic with
  `getUserMedia` (resampled to 16 kHz), plays speech with WebAudio and talks to the server over
  WebSocket. A minimal `index.html` loads the bundle and a monospace font with Braille glyphs.
- **Web terminal (`/terminal`)**: an embedded xterm.js page; per connection the server spawns the
  native TUI in a pseudo-terminal (`portable-pty`, ConPTY on Windows) connected through the
  in-process channel. Keyboard, mouse and resize are forwarded. Audio uses the server machine's
  devices.
- Both bundles are embedded in the executable (`rust-embed`) when built with `cargo xtask dist`.
  Without them, those routes return a page explaining how to build them.

**Risk:** Ratzilla is young. Before the plan is finalised, a spike must confirm Braille canvas
rendering, mouse support and ratatui version compatibility. If it falls short, the web client is
postponed and the web terminal remains the browser option for v1.

## 11. Security

- Binds to `127.0.0.1` by default. LAN exposure (`lan = true`) is opt-in and requires a 32-byte
  random token generated on first run, sent in `Hello` and as a query parameter for `/` and
  `/terminal`.
- The web terminal grants full control of the application; it is never reachable without the token
  outside localhost.
- API keys, configuration and history never leave the server; `ConfigView` redacts keys.
- Mic access from another device requires HTTPS; `serve --tls` generates a self-signed
  certificate on first use.
- Logs never contain keys or audio.

## 12. Errors

No failure crashes the server or is swallowed. Every failure becomes an `Error` event with a code,
a message in the profile's language and a `recoverable` flag.

| Case | Behaviour |
|---|---|
| 401/403 | "The key `<name>` is not valid"; offers to open the Providers tab |
| 429 / overloaded | One retry with exponential backoff, then the provider's message |
| Timeout | 30 s per chat request and per sentence of speech, 60 s per transcription |
| Speech fails mid-reply | Already played sentences stay; the full text stays in the chat log; warning |
| No key configured | Starts anyway and opens the configuration screen |
| No network | Per-request error; no infinite retries; header reflects it |
| Mic unavailable / denied | Listening disabled with a warning; typing still works |
| Empty transcription | "Nothing heard"; in `hands_free`, keeps listening |
| Client disconnects | Its in-flight reply is cancelled; the client reconnects with backoff |
| Corrupt `config.toml` | Renamed to `config.toml.bad-<timestamp>`, defaults loaded, warning |

Logging: `tracing` to a file in the user data directory; `--verbose` also logs to stderr.

## 13. Testing

| Level | Scope | Technique |
|---|---|---|
| Unit | Sentence splitter, text cleaning, key resolution, config load/save (including corrupt files), PCM conversion, envelope | Pure functions |
| Brain | Full flow, ordering under out-of-order synthesis, barge-in history, retry on 429, timeouts, mid-reply speech failure | Fake providers with scripted delays |
| Connectors | Request building and response parsing for `openai_compat`, `gemini`, `elevenlabs` | `wiremock` with recorded responses |
| Listening | VAD end of speech, pause while speaking, the three modes | Test WAV files fed as frames |
| Protocol | Serde round trip; same tests compiled for `wasm32-unknown-unknown` | Unit tests |
| Interface | All panels render, click areas match their labels, es/en strings, configuration screen | ratatui `TestBackend` |
| Server | WebSocket handshake, ask and receive audio, token required on LAN, two sessions with separate histories | Server on a random port with fake providers |
| Live providers | Real latency and audio | `#[ignore]` tests, keys from the environment, never in default CI |

CI (GitHub Actions): `cargo fmt --check`, `cargo clippy -D warnings`, tests on Windows, Linux and
macOS, `tipsy-web` built for `wasm32-unknown-unknown`, release artefacts via `cargo xtask dist`.

Manual check before each release: real mic and speakers in the terminal (first) and in the
browser, wake word in Spanish and English.

## 14. Milestones

1. Workspace, protocol, configuration and keys.
2. Providers (`openai_compat` first, then `gemini`, `elevenlabs`) with tests.
3. Brain with streaming, ordering and barge-in.
4. Native TUI client ported from the proof of concept onto `tipsy-ui`, `push` mode.
5. Configuration screen and profiles (all nine).
6. `auto` mode (VAD).
7. WebSocket server and `--connect`.
8. Web terminal.
9. Web client (WASM), after the Ratzilla spike.
10. `hands_free` mode (wake word).
11. CI, `cargo xtask dist`, README, `CONTRIBUTING.md`, first release.

## 15. Open questions and risks

1. **Ratzilla** maturity (§10): spike required before planning milestone 9.
2. **Wake word** library choice and Spanish accuracy (§6).
3. **Transcription** default model slug and per-minute prices (research doc, pending items).
4. **ElevenLabs** WebSocket streaming availability and latency from Spain.
5. **Free tiers** of Google AI Studio (TTS included?) and ElevenLabs, to document the zero-cost
   setup in the README.
6. **License** of the project itself: to be chosen before the first release (MIT or Apache-2.0
   are the usual choices for Rust crates).
