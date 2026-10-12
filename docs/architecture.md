# Architecture

> Status: **draft**, under design. This document captures the architecture agreed so far during
> brainstorming. The full design spec will live in `docs/superpowers/specs/`.

tipsy-compadre is a voice assistant for software developers with a strong regional personality
(the default one, *El Compae*, is a lazy, joking, slightly tipsy Andalusian). It is built as a
**client-server** application written entirely in Rust:

- The **server** (the native `tipsy-compadre` executable) does all the real work: speech-to-text,
  the LLM conversation, text-to-speech, voice activity detection, wake word detection,
  configuration, API keys and system stats.
- **Clients** only draw the interface and move audio in and out. They all share the same ratatui
  interface code, and there is no separate HTML/JS web app:
  - the **terminal client** (native, crossterm backend);
  - the **web client**: the very same TUI compiled to WebAssembly and rendered in the browser
    (Ratzilla backend), using the browser's mic and speakers;
  - the **web terminal**: the native terminal client running in a pseudo-terminal on the server
    and streamed byte for byte to an xterm.js page, using the server machine's mic and speakers.

## Guiding principle: terminal first

The **native terminal client is the core of the project** and its first-class citizen. Every
feature must work in the terminal before it is considered done; the web client and the web
terminal are additional ways to reach the same experience, never the other way round. When a
design trade-off arises, the terminal wins.

## Overview

```
                        tipsy-compadre  (server, native)
 ┌────────────────────────────────────────────────────────────────┐
 │ brain (STT → LLM → TTS) · providers · config + keys · sysmon    │
 │ listening: VAD + wake word over whatever audio clients stream  │
 │                  ▲ protocol: Command / Event                   │
 │         ┌────────┴──────────┐                                  │
 │  in-process channel    WebSocket (axum)                        │
 └─────────┬───────────────────┬──────────────────────────────────┘
           │                   │  JSON (events) + binary (audio)
   ┌───────┴───────┐   ┌───────┴──────────────────────────┐
   │ TUI client    │   │ Web client = same TUI in WASM    │
   │ ratatui +     │   │ ratatui + Ratzilla               │
   │ crossterm     │   │ mic: getUserMedia (web-sys)      │
   │ mic: cpal     │   │ audio: WebAudio (web-sys)        │
   │ audio: rodio  │   │ served at /                      │
   └───────┬───────┘   └──────────────────────────────────┘
           │ runs inside a PTY (portable-pty / ConPTY)
   ┌───────┴─────────────────────────────────┐
   │ Web terminal: xterm.js page at /terminal │
   │ raw terminal bytes over WebSocket        │
   │ mic and speakers: the server machine's   │
   └──────────────────────────────────────────┘
```

| | Web client (TUI in WASM) | Web terminal |
|---|---|---|
| What you see | The HUD rendered by the browser | The native TUI, exactly as in a terminal |
| Mic and speakers | The browser's | The server machine's |
| Best for | Talking to the assistant from a phone or another PC | Remote control, browsers without mic permission, fallback if the WASM client falls short |
| Transport | WebSocket carrying the protocol | WebSocket carrying raw PTY bytes |

The web terminal is built into the executable: the server serves an embedded xterm.js page and,
per connection, spawns the native TUI in a pseudo-terminal (`portable-pty`, ConPTY on Windows)
connected to the server through the in-process channel. Keyboard, mouse and resize events are
forwarded. Users who prefer an external tool can also run the binary under `ttyd`.

## Workspace layout

The terminal and web clients share the interface code, and that code must compile to
WebAssembly. Native-only dependencies (multi-threaded tokio, `cpal`, `rodio`, `keyring`,
`sysinfo`) therefore live in their own crate.

| Crate | Kind | Contents | Compiles to WASM |
|---|---|---|---|
| `tipsy-protocol` | library | `Command` / `Event` messages and the audio frame format | yes |
| `tipsy-ui` | library | The front end's shared, wasm-safe half: the ratatui HUD (widgets, state and es/en strings), backend-agnostic, **and the decisions a client makes with no device, socket or clock in reach (`policy`)** | yes |
| `tipsy-web` | `.wasm` bundle | Web client: Ratzilla backend, browser audio, WebSocket transport | yes (target) — **arrives in Phase 3 of `2026-10-11-plan-3-web-tier.md`; it does not exist yet** |
| `tipsy-compadre` | **binary** | The server (brain, providers, config, axum, the WebSocket door) plus the native TUI client | no |
| `xtask` | binary | `cargo xtask dist`: fetch and verify xterm.js, build the bundle, build the binary that embeds it — **arrives in Phase 2 of the same plan** | no |

A client's turn, microphone and playback policy — which command starts a turn or quiets the device,
when the microphone is wanted and where one captured frame goes, which cell a click landed on — lives in
`tipsy_ui::policy`, and **both the terminal client and the browser client call it**, so the two cannot
drift: `crates/tipsy-compadre/src/client.rs` is one caller and Plan 3's web client is the other. It is
the wasm-safe half by construction — nothing there may reach for a device, a socket, the file system,
this process's clock or `crossterm` — which is why `client.rs`'s `key_input` (a crossterm `KeyEvent`) and
`clock_view` (this process's clock) stay per shell instead, and why Plan 3's browser client reads
`js_sys::Date` for its own.

**`client.rs` is the parent module and its four children — `client/mic.rs`, `audio.rs`, `call.rs`,
`settings.rs` — reach its imports, so that file's import list is part of their interface.** Each child
opens with `use super::*` and resolves names it never imports itself (`CallSetup`, `Heard`, `envelope`,
`rms`, `i16_to_f32`, `Capability`, `ClientConfigView`, `NoticeKind`, `config_from`, `form_of`,
`provider_config`, `build_one`) through that glob. **Narrowing `client.rs`'s imports to what the parent
alone names breaks the children**: removing `CallSetup` and `Heard` from `use crate::realtime::{…}`
produces `E0433: cannot find type CallSetup` in `client/call.rs`, which is where both are used.

There is **a single executable**. The `tipsy-web` bundle is embedded into the `tipsy-compadre`
binary at build time and served from memory, so users download one file.

Building the full distribution takes two steps (the WASM bundle first, then the binary that
embeds it), wrapped in a single project command (`cargo xtask dist`). A plain `cargo build`
produces a binary without the web client; both page routes then answer with a page that says the web
UI is not available in this build and names `cargo xtask dist`.

The server's HTTP and WebSocket layer is `axum` 0.8.9 with its `ws` feature, and TLS is
`axum-server` 0.8.0 (`tls-rustls`). One consequence is worth writing down rather than discovering:
`axum` pins `tokio-tungstenite` **0.29** while the native `--connect` client keeps **0.30**, so two
copies of `tungstenite` are compiled into the one executable (spec W1).

**Ratzilla is no longer a risk: the gate was answered `PASS`** on 2026-10-11. The spike verified all
four questions and all four items that had been left `UNVERIFIED` — the Braille canvas (the core
rings and the oscilloscope) renders, mouse input is handled, the ratatui version tracks — and
`WebGl2Backend` is the recommendation, because the DOM backend is ~2.4× slower on a real GPU, panics
at an 18 px font, and `CanvasBackend` mis-maps clicks by a row and a column
(`.superpowers/sdd/plan-3-research/ratzilla-spike-verdict.md`). The web terminal is part of v1
either way.

## Server internals

The server is a set of **actors**: tokio tasks that only talk through message channels.

```
              ┌──────────── Event ───────────────┐
              ▼                                  │
 ┌──────────────────┐  Command   ┌───────────────┴──┐
 │ session          │──────────▶ │ brain            │
 │ (one per client: │            │ STT → LLM → TTS  │
 │ in-process or WS)│◀── Event ──│ history, cancel  │
 └──────────────────┘            └──┬────────────▲──┘
        ▲                  Utterance│            │ Utterance
        │ mic audio frames          │            │
        │                    Speech │ ┌──────────┴─────┐
        │                  (chunks) │ │ listener       │
        │                           ▼ │ VAD, wake word,│
        └────────── audio out ◀─────  │ listening modes│
                                      └────────────────┘
        ┌──────────────────┐
        │ sysmon (sysinfo) │── Stats ──▶ sessions
        └──────────────────┘
```

| Actor / module | Responsibility |
|---|---|
| `config` | Loads and saves `config.toml`, including the `[[profiles]]` array. Resolves API keys: environment variable first, then the OS credential store. |
| `providers` | `ChatProvider`, `SpeechProvider` and `TranscribeProvider` traits. Implementations: `openai_compat` (OpenRouter, Ollama, LM Studio, Groq, OpenAI…), `gemini` and `elevenlabs`. Each capability can use a different provider. |
| `brain` | One question end to end: transcribe, stream the LLM reply, split it into sentences, synthesize them in parallel and emit them in order. Keeps the conversation history. Handles cancel (barge-in) and reset. |
| `listener` | Runs voice activity detection and wake word detection over the mic frames a client streams, according to the listening mode. Paused while the assistant is speaking so it never hears itself. |
| `session` | Bridges one client to the server, either through an in-process channel or a WebSocket. |
| `sysmon` | CPU, RAM and network stats. |

## Protocol

Clients and server exchange:

- **Commands** (client → server), JSON: `Hello`, `Ask`, `Say`, `ListenStart`, `ListenStop`,
  `Cancel`, `Reset`, `SetProfile`, `SetListeningMode`, `GetConfig`, `UpdateConfig`, `SetKey`,
  `TestProvider`.
- **Mic audio** (client → server), binary frames: 16 kHz mono 16-bit PCM.
- **Events** (server → client), JSON: `Welcome`, `Status`, `Heard`, `ReplyDelta`, `Reply`,
  `SpeechChunk`, `Done`, `Stats`, `Config`, `ProviderTest`, `ProfileChanged`, `ConfigChanged`,
  `Error`.
- **Speech audio** (server → client), binary frames: mono 16-bit PCM, played by the client.

The exact message schema is defined in the design spec
(`docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md`, §4).

## Listening modes

| Mode | Starts recording | Stops recording |
|---|---|---|
| `push` | `Tab` | `Tab` |
| `auto` | `Tab` | ~1 s of silence (or `Tab`) |
| `hands_free` | wake word ("Oye, compae") | ~1 s of silence |

Wake word detection always runs on the server, which is the user's own machine. Nothing is sent
to any cloud service until the wake word is detected. Clients must show a clearly visible
indicator whenever the mic is open.

That promise is `hands_free`'s. The login screen is the other kind of recording: an explicit, visible
one, opened by the screen itself and announced by the red `● rec` in its own title, which sends only
the speech the local detector confirms and never the silence or the room before it
(`docs/superpowers/specs/2026-10-11-login-screen-design.md`, §10). It is a ritual, not a security
boundary: `server.token` below remains the only real requirement.

## Security

The server spends the user's API keys and listens to their microphone, so:

- It binds to `127.0.0.1` by default, `serve` refuses to start on any other address without a token,
  and the token is 128 bits of OS entropy as 32 lowercase hex characters. **Nothing generates one on
  the first run yet** — a new `config.toml` carries an empty token and the Server tab's Regenerate
  row is what mints one; a first-run token is on the README's release checklist.
- API keys, configuration and conversation history never leave the server; clients never see a key.
  The one case where a secret travels *towards* the server is setting a new key, and `SetKey` is
  refused with `insecure_transport` unless that connection is loopback or TLS.
- Both web routes (`/` and `/terminal`) require the token when one is configured, and the protocol
  socket takes it in `Hello` and in no other place: `/ws` has no query parameter to put one in. On a
  loopback listener the socket asks for nothing at all — the bound address is what decides exposure
  — so the pages are the half a token gates there. The web terminal grants full control of the
  application, so it is never reachable without it.
- **A loopback listener's `/ws` is reachable by any page in the user's browser.** A WebSocket
  handshake is not subject to the same-origin policy, and the session behind it requires no token
  there, so a page the user merely visits can open `ws://127.0.0.1:<port>/ws`. What it gains is not a
  key: `Command::UpdateConfig` carries the client's own shape, `server.bind` and `lan` included, and
  the server persists it — so the *next* `serve` could bind the LAN. The `/ws` upgrade therefore
  refuses a request whose `Origin` header is present and is not the host the request arrived on
  (`403`), compared against that request's own `Host` header rather than any hard-coded name. A CLI
  client sends no `Origin` and is unaffected, which is what keeps `--connect` working. **This check,
  and not `Hello`'s token, is what keeps a browser out of a loopback listener**; a future
  `/terminal/ws` needs the same one.
- `serve --tls` encrypts the connection: that is what makes the token and a new key safe on a LAN.
  It does **not** by itself unlock `getUserMedia` on another machine — loopback over plain HTTP is
  already a secure context, and a certificate the browser has not been told to trust is not one. The
  microphone from another device needs a certificate that device trusts (`--tls-cert`/`--tls-key`).
- The log is written through a scrubber, on the file and on the `--verbose` copy alike: the token, a
  key value the process resolved and any sample dump of recorded audio are removed before a line is
  written, and the level cap is `DEBUG` so the `trace!` request line a socket crate logs can never be
  reached.

## Run modes

| Command | What runs |
|---|---|
| `tipsy-compadre` | Server + TUI client in the same process, talking through an in-process channel |
| `tipsy-compadre serve` | Server only (headless); the web client is served at `http://127.0.0.1:<port>/` and the web terminal at `/terminal` |
| `tipsy-compadre --connect <url>` | TUI client only, connected to a remote server |

## Roadmap

- **v1:** server, protocol, TUI client, web client (TUI in WASM), web terminal, cloud providers
  (OpenAI-compatible, Gemini, ElevenLabs), the three listening modes, configuration screen and
  regional profiles in Spanish and English.
- **v1.1:** Deepgram and Anthropic connectors.
- **v2:** local text-to-speech (Qwen3-TTS or Chatterbox, through a Python sidecar) and local
  speech-to-text (Whisper).
