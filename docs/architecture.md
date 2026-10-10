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
| `tipsy-ui` | library | The ratatui HUD: widgets, state and es/en strings, backend-agnostic | yes |
| `tipsy-web` | `.wasm` bundle | Web client: Ratzilla backend, browser audio, WebSocket transport | yes (target) |
| `tipsy-compadre` | **binary** | The server (brain, providers, config, axum) plus the native TUI client | no |

There is **a single executable**. The `tipsy-web` bundle is embedded into the `tipsy-compadre`
binary at build time and served from memory, so users download one file.

Building the full distribution takes two steps (the WASM bundle first, then the binary that
embeds it), wrapped in a single project command (`cargo xtask dist`). A plain `cargo build`
produces a binary without the web client; the server then reports that the web UI is not
available.

**Risk:** Ratzilla is young. Before planning, verify that it renders Braille canvas (the core
rings and the oscilloscope), handles mouse input and tracks a compatible ratatui version. The
fallback is the web terminal, which is part of v1 anyway.

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

## Security

The server spends the user's API keys and listens to their microphone, so:

- It binds to `127.0.0.1` by default.
- Exposing it to the local network is opt-in and requires a token generated on first run.
- API keys, configuration and conversation history never leave the server; clients never see a
  key.
- Both web routes (`/` and `/terminal`) require the token when the server is exposed to the LAN.
  The web terminal grants full control of the application, so it is never reachable without it.
- Browsers only allow mic access on `localhost` or over HTTPS, so remote web clients (e.g. a phone
  on the LAN) need HTTPS with a self-signed certificate.

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
