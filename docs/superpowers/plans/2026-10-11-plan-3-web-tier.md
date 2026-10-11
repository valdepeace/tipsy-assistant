# Plan 3 — Web tier Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the assistant reachable from outside the one terminal that owns it: a token-guarded
WebSocket server (`tipsy-compadre serve`), a native TUI that attaches to a remote one
(`--connect`), a full-screen TUI in the browser through a real pseudo-terminal (`/terminal`), and the
same HUD rendered from WebAssembly with the browser's own microphone and speakers (`/`).

**Architecture:** There is no `Transport` abstraction and none is added. `spawn_with_events`
(`session.rs:56`) already hands back `SessionEnds { to_server, from_server, events }`
(`session.rs:46-52`) over `mpsc` channels, and **the in-process channel already is the protocol**, so
a socket peer is a *bridge*: a server-side bridge owns a connection and drives a session's channels,
and a client-side bridge produces a `SessionEnds` of identical shape from a socket, leaving
`client.rs`'s poll loop untouched. `axum` serves the HTTP routes and the two WebSocket endpoints;
`tipsy-ui` is reused verbatim in the browser through Ratzilla, because `draw_web` hands over a
`&mut ratatui::Frame`.

**Tech Stack:** the workspace as Plan 2C and the login plan leave it — Rust `edition = "2024"`,
pinned `1.96.1`, `tokio` actors — plus, **all new and all named here**: `axum` 0.8.9 (`ws`),
`axum-server` 0.8.0 (`tls-rustls`), `rust-embed` 8.13.0, `mime_guess` 2.0.5, `rcgen` 0.14.10,
`portable-pty` 0.9.0, `ratzilla` 0.3.1, `wasm-bindgen` 0.2.129 + `web-sys` + `js-sys` +
`wasm-bindgen-futures` in the new `tipsy-web` crate, and the `xtask` binary. `tokio-tungstenite` 0.30
stays for the client side, and `@xterm/xterm` 6.0.0 with `@xterm/addon-fit` 0.11.0 is what Task 2.3
fetches.

Two additions follow a **provenance rule rather than a version rule**, and the lock settles both:
`tracing` is already in `Cargo.lock` at `0.1.44`, so Task 1.4 — the first task to call
`tracing::error!` — declares `tracing = "0.1"`, which reuses the locked entry and adds no new crate;
`tracing-subscriber` is **absent from the lock** and no primary source was read for its series, so
Task 1.9 declares it with `cargo add tracing-subscriber` at execution and records the resolved version
in its commit message rather than trusting a series number written here. **A task adds the manifest
entry the first time it *uses* the crate; nothing may be deferred to "whoever owns the manifest"**,
because that owner does not exist and the deferral is what conflict 11 was. `rustls-pemfile` is the
same rule with one more condition: `axum-server`'s `RustlsConfig::from_pem_file` reads a supplied
certificate pair itself, so Task 1.8 tries that path first and stops to ask before adding a crate at
all.

Every other version above was
read off crates.io or docs.rs by
[`web-transport-and-terminal.md`](../../../.superpowers/sdd/plan-3-research/web-transport-and-terminal.md)
and [`ratzilla-and-wasm-audio.md`](../../../.superpowers/sdd/plan-3-research/ratzilla-and-wasm-audio.md);
no task may invent one.

**Spec:** [`docs/superpowers/specs/2026-10-11-web-tier-design.md`](../specs/2026-10-11-web-tier-design.md).
The plan argues from the spec and every task cites its sections — §4 is the wire, §5 the server, §6
the remote client, §7 the web terminal, §8 the web client, §9 security, §10 errors, §11 testing, §12
the file map. The v1 spec
([`2026-10-10-tipsy-compadre-design.md`](../specs/2026-10-10-tipsy-compadre-design.md)) is read for
§4 (protocol), §9 (interface), §10, §11 (security), §12 (errors) and §14 (milestones 7–9); the login
spec for its §4 wire.

## The three phases, and what each one ships

Each phase leaves software that works and is tested on its own. **Phase 1 is a complete server;**
Phases 2 and 3 are clients of it and can be executed in either order once Phase 1 is merged.

| Phase | Tasks | What a user can do at the end |
|---|---|---|
| **1 — The server and the remote client** | 1.1 – 1.9 | Run `serve` headless and attach a native TUI to it from another machine with `--connect`, with the token rules of §5.1 and §9 in force, and edit the configuration from the remote screen |
| **2 — The web terminal** | 2.1 – 2.4 | Open `http://<server>:7070/terminal` and drive the real TUI in a browser |
| **3 — The web client** | 3.1 – 3.7 | Open `http://127.0.0.1:7070/`, pass the login door and talk to the assistant with the browser's microphone. **3.1's gate has already been cleared — the spike ran on 2026-10-11 and its verdict is PASS on all four questions** (`.superpowers/sdd/plan-3-research/ratzilla-spike-verdict.md`), so this phase stands |

**One whole-branch review at the end, then one fix wave and one scoped re-review of it** — the
standing budget ruling of Plans 1, 2A, 2B and 2C. There are no per-task reviews.

## Global Constraints

The spec's project-wide requirements, one line each. Every task's requirements implicitly include
this section.

- **Every `path:line` citation in this plan is as of the commit it was written against, and the tree
  has moved since.** The plan was authored while the login screen was still in its own worktree; the
  merge to `master` shifted the files it cites by between two and a hundred and forty-five lines —
  measured on 2026-10-11 against `28c2fb8`: `tipsy-ui/src/state.rs`'s `Welcome` arm moved from `:291`
  to `:436`, `session.rs`'s `web_bundle` line from `:249` to `:314`, `session.rs`'s `SESSION_ID` from
  `:23` to `:26`, `lib.rs`'s `Command` enum from `:107` to `:109`, and `config/model.rs`'s
  `ServerConfig` from `:44-54` to `:46`. **Every task re-reads the file it is about to edit before it
  edits it, and the file wins over the citation.** Read a line number here as "in this region"; treat
  a claim that a symbol is at a line as expired, and a claim that it *exists* as something to confirm
  against the file before building on it.
- Toolchain pinned by `rust-toolchain.toml` to `1.96.1`; every crate uses `edition = "2024"`.
- All code, identifiers, comments, docs and commit messages are in **English**. Interface strings
  keep the two static es/en tables in `tipsy-ui/src/strings.rs`.
- `PROTOCOL_VERSION` **stays `1`** (`tipsy-protocol/src/lib.rs:7`). The three new `Command`s and the
  three new `Event`s are additive; every existing variant keeps its shape byte for byte, and no
  existing test's JSON changes.
- `tipsy-protocol` and `tipsy-ui` must NOT depend on `tokio`, `cpal`, `rodio`, `keyring`, `sysinfo`,
  `reqwest` or `crossterm`, and must keep compiling to `wasm32-unknown-unknown`. Anything a browser
  needs from the system lives in `tipsy-compadre` or `tipsy-web`.
- **No new dependency beyond the Tech Stack list**, and every new one is added with an exact version
  quoted from the research notes. A task that wants another one stops and asks instead.
- API keys never go into files, logs, events or fixtures. The configuration stores key *names*; the
  credential store holds the values and is read only by the connectors, in the server process. **No
  field of `ConfigView` may hold a secret** — spec §4.3.
- `SetKey` is **refused** with `Error { code: "insecure_transport", recoverable: true }` unless the
  connection is loopback or TLS — spec §9.4. The plan's task 1.8 owns the test.
- The token never appears in a log line, a `Referer`, or a cached page: the two HTML routes answer
  with `Referrer-Policy: no-referrer` and `Cache-Control: no-store` — spec §5.3, §9.2.
- **The two audio directions are not the same shape, and confusing them is a plan defect this line
  used to carry.** Upward, `0x01` (`Frame::Mic`) is mono 16 kHz PCM16 in 320-sample (20 ms) frames, as
  `audio/device.rs:18-20` does, and the browser must resample to exactly that. Downward, `0x02`
  (`Frame::Speech`) is **one whole spoken sentence** — `lib.rs:178` says so in as many words, "Mono PCM
  of one spoken sentence, at the rate announced by `Event::SpeechChunk`" — sent as one
  `SpeechChunk` + one frame pair per sentence (`brain.rs:454-455`), with `seq` counting **sentences**
  (`brain.rs:392-393`) and resetting per turn (`brain.rs:198`). So the browser's playback must not pad
  a frame to 20 ms arithmetic: a sentence's length is whatever its synthesis returned, which is why
  Task 3.4 anchors a late sentence at `max(cursor, now + CUSHION_S)` rather than computing a position
  from a frame count.
- Every commit message ends with the trailer line
  `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.
- Do not run `cargo fmt`: the repository's hand style is not rustfmt 1.9.0's, and running it rewrites
  files other features own. The lint gate is clippy compared against the pinned baseline in
  [`global-constraints.md`](../../../.superpowers/sdd/2026-10-11-plan-3-web-tier/global-constraints.md).

## Review Focus

The failure modes the spec implies and no single task's happy path exercises. Each line names the
task whose tests own it.

1. **A reply in flight when the client vanishes.** The socket dies mid-sentence; the server must
   cancel that session's turn (`cancel_running`, `session.rs:494`), drop it from the `Hub`, and keep
   serving everyone else — not leak a task, not panic, not stop the listener. Task 1.6.
2. **A peer that stops reading.** A WebSocket has no backpressure (MDN, quoted in the research note)
   and the session's channels are `unbounded`. A browser tab that stops draining must slow the
   session down instead of growing the server's memory without limit. Tasks 1.6 and 2.2.
3. **The token and a key on a plaintext LAN.** Three separate leaks: a token in a URL that ends up
   in a log line, a token in a `Referer`, and a key crossing a plaintext socket via `SetKey`. Tasks
   1.4, 1.8 and 1.9.
4. **A PTY that outlives its socket.** ConPTY children do not always die of a closed stdin; a
   connection that goes away must kill the child, drop the writer and the master, and leave nothing
   running — with a test that proves the process is gone. Task 2.1.
5. **A `bind` value that is not an `IpAddr`.** `0.0.0.0` is unspecified rather than loopback (so it
   is caught), but a hostname can resolve off this machine, and `TcpListener::bind` accepts it.
   Parsing as `SocketAddr` before the bind is what makes §5.1's rule mean anything. Task 1.4.

## File Structure

New files, and the one responsibility each has. Existing files are modified only where a task names
them.

```
crates/tipsy-protocol/src/lib.rs          MODIFY (1.1): ConfigView + section views, 3 Commands, 3 Events, 5 ErrorCode variants
crates/tipsy-protocol/tests/roundtrip.rs  MODIFY (1.1): the new round trips and the redaction test
crates/tipsy-compadre/src/
├── main.rs                               MODIFY (1.7, 1.8): serve, --connect, --token, --tls*, --verbose
├── logging.rs                            NEW (1.9): the tracing bootstrap and the redaction rules
├── server/
│   ├── mod.rs                            NEW (1.3) — `pub mod hub;` only — MODIFY (1.4): boot, listener, router, bind rule; (1.8): the bind_rustls branch; (2.2): the /terminal/ws route
│   ├── hub.rs                            NEW (1.3): the session registry, the id counter and the broadcasts
│   ├── conn.rs                           NEW (1.6): the per-connection bridge — MODIFY (1.8): the §9.4 guard; (2.2): the PTY pump
│   ├── assets.rs                         NEW (1.5): the embedded bundle and its two fallback pages
│   └── tls.rs                            NEW (1.8): the certificate — supplied, or generated once
├── client/
│   ├── bridge.rs                         NEW (1.7): the client-side socket bridge (returns SessionEnds)
│   └── pty.rs                            NEW (2.1): the PTY owner for a web terminal's child
├── config/{model,store,form}.rs          MODIFY (1.1, 1.3, 1.7): the views, the 32-byte first-run token, the conversions
└── tests/connect.rs                      NEW (1.7): `--connect` end to end against a real server
crates/tipsy-ui/src/{state,strings,config_form}.rs   MODIFY (1.1, 1.5, 1.7): the new events, web_bundle, the conversions
crates/tipsy-web/
├── Cargo.toml                            NEW (3.2): cdylib + rlib, the wasm-only dependency block
├── src/lib.rs                            NEW (3.2): the wasm entry, the socket queue, the rAF loop
├── src/wire.rs                           NEW (3.2): the pure Command/Event/Frame mapping (host-tested)
├── src/socket.rs                         NEW (3.2): the web-sys WebSocket, Hello first, backoff
├── src/mic.rs                            NEW (3.3): the pure permission/frame-assembly half (host-tested)
├── src/playback.rs                       NEW (3.4): the pure timeline, envelope and barge-in rule (host-tested)
├── src/web_audio.rs                      NEW (3.4): the AudioContext half, wasm-only
└── src/hud.rs                            NEW (3.5): the pure key/click translation (host-tested)
xtask/
├── Cargo.toml, src/main.rs               NEW (2.3, 3.6): the `dist` dispatch
├── src/assets.rs                         NEW (2.3): the xterm.js fetch, the pinned sha256 and the verifier
└── src/dist.rs                           NEW (3.6): the four steps, the version pin, the asset check
web/
├── client/index.html                     NEW (3.2) a minimal host page — finished by (3.5)
├── client/mic-worklet.js                 NEW (3.3): the AudioWorkletProcessor, which must be a real file
├── terminal/index.html                   NEW (2.4): the xterm.js page
├── vendor/README.md                      NEW (2.3): what the build fetches, from where, under what licence
└── dist/                                 GENERATED (3.6): wasm-bindgen's `--out-dir`, git-ignored, what rust-embed reads
```

**Two files have more than one owner and the plan says so rather than leaving it to be discovered.**
`server/mod.rs` is **created by 1.3**, because that is the task that first needs a `server` module to
declare `hub` in, and **modified** by 1.4, 1.8 and 2.2 for the router, the TLS branch and the terminal
route; a task that "creates" it twice would fail on the second run. `web/client/index.html` is created
minimal by 3.2 and finished by 3.5, for the reason 3.2 gives: only the embedded bundle serves the page
and `/ws` on one origin, so the page cannot be exercised until 3.6 has built the bundle, and 3.5 is
where its content is owned.

## Shared interface contract

Every task's `Interfaces` block repeats what it needs from here. **This section is the anti-drift
device: a name that appears in two tasks appears here once, spelled the same way.** Names marked
*(existing)* are already in the tree and must be read again before use; names marked *(new)* are
introduced by the task that owns them and no other task may invent a variant of them.

**Existing, and load-bearing** (all from
[`codebase-recon.md`](../../../.superpowers/sdd/plan-3-research/codebase-recon.md) §2–§6):

- `spawn_session(config, providers, brain_config) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>)` — `session.rs:36`. **Task 1.2 adds a fourth parameter, `policy: SessionPolicy`**, and Task 1.3 a fifth, `hub: Hub`, to both constructors. The task bodies carry the final signatures; this line is the current state on disk
- `spawn_with_events(config: Config, providers: ProviderSlot, brain_config: BrainConfig) -> SessionEnds` — `session.rs:56`
- `pub struct SessionEnds { pub to_server: UnboundedSender<ToServer>, pub from_server: UnboundedReceiver<ToClient>, pub events: UnboundedSender<ToClient> }` — `session.rs:46-52`. **Task 1.2 adds `pub session_id: u64`**
- `ToServer` / `ToClient` — the channel payloads; `Frame` — the audio frame type
- `Command` — 13 variants, `#[serde(tag = "type", rename_all = "snake_case")]`, `lib.rs:108`
- `Event` — 13 variants, tagged the same way, `lib.rs:133`
- `ClientKind { Tui, Web, WebTerminal }` — `lib.rs:11`; `PROTOCOL_VERSION = 1` — `lib.rs:7`
- `ProfileView { id, name, language }` — `lib.rs:92-97`
- `ServerConfig { bind: "127.0.0.1:7070", lan: false, token: "" }` — `config/model.rs:46-68`; `ServerConfig::has_token(&self) -> bool` — `model.rs:59-61`
- `store::new_token` — `config/store.rs:28-48`; `store::load_or_create` / `store::save` — `config/store.rs:20-30`
- `form_of` / `config_from` — `config/form.rs:17-102`
- `ProviderSlot`, `build_providers`, `build_one` — `providers/mod.rs:167`, `:251`, `:286`; `KeyResolver` — `keys.rs:79-97`; `keys::SERVICE` — `keys.rs:6`
- `Brain::run_turn`, `BrainConfig`, `TurnProfile`, `TurnInput` — `brain.rs:20-69`
- `tipsy_ui::draw`, `UiState`, `UiAction`, `KeyInput`, `UiLanguage`, `RING_VIEW` — `tipsy-ui/src/render.rs`, `state.rs`
- `ConfigForm`, `ServerForm`, `RealtimeForm` — `tipsy-ui/src/config_form.rs`

**New, and owned by exactly one task each:**

| Name | Owner | Shape, as the owning task must pin it |
|---|---|---|
| `ConfigView` + one section view per section of `ConfigForm` | Task 1.1 | `tipsy-protocol`. **The fields are `tipsy_ui::ConfigForm`'s, moved verbatim**, with each section type renamed `…Form` → `…View` — **with one exception: the form's `ProfileForm` becomes `ProfileEntryView`, because `ProfileView { id, name, language }` already exists at `lib.rs:92-97` and is `Welcome`'s payload.** `ConfigForm` itself stays in `tipsy-ui` and gains `From<&ConfigForm> for ConfigView` and `From<ConfigView> for ConfigForm` (the orphan rule puts both impls there); `config/form.rs` gains `view_of(&Config) -> ConfigView` and `config_of(ConfigView) -> Result<Config, String>`. `ConfigView::redacted(self) -> Self` clears `server.token`. No field may be a secret (spec §4.3) |
| `Command::{GetConfig, UpdateConfig, SetKey}` | Task 1.1 | §4.2's JSON exactly |
| `Event::{Config, ProfileChanged, ConfigChanged}` | Task 1.1 | §4.4's JSON exactly |
| `SessionPolicy` | Task 1.2 | `{ require_token: bool, token: String, loopback: bool }` with `local()` / `exposed(token)` constructors. **`Task 1.4` builds it from the address the listener actually bound, after §5.1's post-bind `local_addr()` check — never from `config.server.lan`**, because a `bind` of `0.0.0.0` with `lan = false` is still exposed and the bound address is the only fact that settles it; the listener carries the policy in its router state and the `/ws` handler passes it to `Hub::spawn`. Handshake refusals are `Event::Error { code: ErrorCode::ProtocolVersion \| TokenRequired \| TokenInvalid, message: <human, in the profile's language>, recoverable: false }`, judged as a **prelude before `welcome()`** so "refused, and no `Welcome`" is testable without racing the greeting turn |
| `ErrorCode`'s new variants | Task 1.1 | `lib.rs:51-64` has ten variants and **none of them can spell this tier's refusals**. Add `ProtocolVersion`, `TokenRequired`, `TokenInvalid`, `BadMessage` and `InsecureTransport` (serde `snake_case` → `protocol_version`, `token_required`, `token_invalid`, `bad_message`, `insecure_transport`). **The `message` field stays the human half** — the v1 spec §4.2 defines it as "already localized to the profile's language", so a client may not branch on it, which is exactly why the codes exist. Adding variants breaks every exhaustive match on `ErrorCode`: the owning task greps for them and adds behaviour-free arms, as it does for `Event` |
| `Hub` | Task 1.3 | `tipsy-compadre::server::hub`, `#[derive(Clone)]`. Holds the configuration, its path, the `KeyResolver` (behind an `Arc`; it is not `Clone`) **and the one shared `ProviderSlot`**. `Hub::new(config, config_path, keys, providers)`. The slot is built by the caller — `main.rs`'s `serve` arm, through `Settings.providers` — and **`Hub::spawn(policy: SessionPolicy) -> SessionEnds` clones it per session** (no `config`, no `brain_config`, no `providers`: the hub holds the first, defaults the second because nothing in this plan configures a `BrainConfig`, and owns the third, so the only per-connection fact a caller passes is the policy), so every `Brain` in the process holds the same slot. **`spawn` is the only entry point: there is no `spawn_with`.** The difference between one caller and another is the slot handed to `Hub::new`, never an argument to `spawn` — a second entry point taking providers again would reopen the ownership question this row just closed. The rest of the surface, none of which a sibling task invents: `path`, `config`, `providers`, `config_view(include_token)`, `attach(session_id, to_server, events)`, `detach`, `broadcast(ToClient) -> usize`, `session_ids`, `resend_welcome`, `reconfigure_all() -> bool` (rebuild the connectors, `replace` them in the shared slot, then hand `Command::UpdateConfig` to every session so each re-derives its own `Brain` — the two halves of a change), `apply_config(Config) -> bool` (persist, remember, re-derive; the one place a change becomes durable), `update_config`, `update_config_from_view`, `set_profile`, `set_listening_mode`, `set_key` **One slot for the whole server, not one per connection**, because that is what the code already does (`client.rs` builds one and `ProviderSlot::replace` exists for exactly this) and because it makes a configuration change atomic: `reconfigure_all` rebuilds the connectors from the new configuration, `replace`s them in the slot, and tells each session to re-derive its `Brain` (the profile's prompt and voice are per session), then `ConfigChanged` goes out. **`Hub::spawn(...)` is the only way a connection starts a session**: it attaches the session to the hub *before* `Welcome`, which is what stops a broadcast from missing a session or a session from missing a broadcast. `Hub::config_view(include_token: bool) -> ConfigView` is §4.3's rule expressed as one `bool`. Also `attach(&self, session_id: u64, to_server: UnboundedSender<ToServer>, events: UnboundedSender<ToClient>)` (**two** senders: `reconfigure_all` talks to the session, not to its client), `detach`, `broadcast(ToClient) -> usize`, `resend_welcome`, `reconfigure_all`. Task 1.6 uses `Hub::spawn` and must not hand-roll attach ordering |
| `server::run(settings, bound) -> Result<(), ServerError>` | Task 1.4 | `tipsy-compadre::server`. The listener, the router and the bind rule; called by `main.rs`. **The final `Settings` shape, which Tasks 1.7 and 1.8 both extend in place:** `Settings { bind: String, lan: bool, token: String, config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot, tls: Option<TlsFiles> }`. **`tls` is a three-state field, not an `Option` around one thing**, because `--tls` alone and `--tls` with a user-supplied pair are different promises (§5.5) and one `Option<(PathBuf, PathBuf)>` cannot tell them apart: `None` is TLS off; `Some(TlsFiles::Generated)` means *`--tls` with no paths* — generate the pair in the config directory on first use and reuse it afterwards; `Some(TlsFiles::Supplied { cert, key })` means *the user named these files* — read them, and **refuse to start naming the path** when one is absent or unparseable (§10). A single `Option<(PathBuf, PathBuf)>` would silently generate over a path the user pointed at, which is the one outcome §10 forbids. `TlsFiles` is **Task 1.4's type, defined in `server/mod.rs` beside `Settings`** — not Task 1.8's, and the ordering is why: Tasks 1.7 and 1.8 must both name the field's type, and 1.7 builds a `Settings` literal (`tls: None`) **before** `1.8`'s `server/tls.rs` exists, so a type introduced by 1.8 would leave 1.4 and 1.7 uncompilable in the interval. Task 1.8 *consumes* it and owns the loading, in `server/tls.rs`, called from **inside `server::run`** so a bad path becomes a `ServerError` carrying the path (§10) rather than a second error type in `main.rs` — which is also what keeps `Settings`' `Clone` and `Debug` derives honest, since `RustlsConfig` is neither. **`config_path` and `keys` are in the struct because `server::run` builds the `Hub` from the `Settings` it is handed** — `Hub::new(config, config_path, keys, providers)` — rather than taking a pre-built hub from `main.rs`; a task that builds a `Settings` literal must supply all eight fields or it does not compile. `bound` is a oneshot sender the listener uses after **both** halves of §5.1's rule pass, so a test on port 0 learns the port and a test that ends instead has proved a refusal |
| `assets::{PAGE_CLIENT, PAGE_TERMINAL, resolve(path), fallback(), asset(path), web_bundle()}` | Task 1.5 | `tipsy-compadre::server::assets`. Returns bytes plus a content type, or `None` when the bundle is absent. **`resolve` and `fallback` are pinned by Task 1.4**, which had to create this file as a placeholder because its own router could not compile without it; Task 1.5 fills their bodies and adds the rest |
| `server::conn::serve(socket, session_ends, hub)` | Task 1.6 | The per-connection bridge |
| `client::bridge::connect(url, token) -> Result<SessionEnds, BridgeError>` | Task 1.7 | `tipsy-compadre::client::bridge`. Returns a `SessionEnds` **indistinguishable in shape** from `spawn_with_events`' |
| `server::tls::{load_or_generate(paths) -> Result<Certificate, TlsError>}` | Task 1.8 | `tipsy-compadre::server::tls`. **Ratified: `sha2 = "0.10"` is added as a direct dependency** for the certificate fingerprint, because the lock already carries `sha2` at **both `0.10.9` and `0.11.0`** (verified) — declaring `"0.10"` reuses the existing entry instead of introducing a third. The fingerprint is hashed from the DER at runtime and its test asserts its shape, never a hard-coded value. Zero-dependency fallback if a reviewer refuses the edge: print the pair's paths and SANs and drop `Certificate::fingerprint` |
| `client::pty::PtySession` | Task 2.1 | `tipsy-compadre::client::pty`. `spawn(command, cols, rows)`, `resize`, `take_reader`, `writer`, `kill` |
| `tipsy_web::start()` | Task 3.2 | The wasm entry point (`#[wasm_bindgen(start)]`) |
| the xtask's asset fetch + hash verify | Task 2.3 and Task 3.6 | `xtask`. The pinned version and its sha256 are **obtained and recorded** by the task, never invented |

**The ordering rule.** A task may use only what it creates itself or what an **earlier** task has
already created or declared. A symbol introduced by a later task may not appear in an earlier task's
code. Three consequences, each of which is a defect this plan was actually caught making rather than
a style preference:

- **A shared type belongs to the earliest task that must spell it**, not to the task whose subject it
  is. `TlsFiles` is Task 1.4's even though TLS is Task 1.8's subject, because `Settings` cannot be
  written without it and Tasks 1.4, 1.7 and 1.8 all name it. A first draft gave the type to 1.8 and
  left 1.4 and 1.7 uncompilable until 1.8 landed.
- **A manifest entry belongs to the first task that *uses* the crate.** Nothing is deferred to
  "whoever owns the manifest", because that task does not exist. A first draft had Task 1.4 calling
  `axum`, `rust-embed`, `mime_guess` and `tracing::error!` while no task added any of them until
  Task 1.9 — the server would not have compiled until the last task of the phase.
- **A file is `Create:`d by exactly one task** — the earliest that needs it — and every later task
  says `Modify:`. `server/mod.rs` is Task 1.3's creation; Tasks 1.4, 1.8 and 2.2 modify it.
- **An extension to a shared type belongs to the task that needs it, and may not be spelled earlier.**
  `ServerError` leaves Task 1.4 with four variants; **Task 1.8 adds `Tls(TlsError)`** — it cannot come
  from 1.4, because `TlsError` does not exist until 1.8's `server/tls.rs` does — and **Task 2.1 adds
  `Message(String)`**, because 2.1 is the first task that needs it. A first draft had both tasks
  mapping into variants nobody had declared, which is a compile error in both.

- **The hub is the only writer of the configuration, and the connection layer is what routes to it.**
  A `Command::{UpdateConfig, SetKey, SetProfile, SetListeningMode}` that arrives **from a socket** is the
  bridge's to put through `Hub::update_config_from_view`, `set_key`, `set_profile` or
  `set_listening_mode` — never the session's. The session's own arms are the *hub's fan-out half*: they
  exist so that each session can apply a change the hub has already written and announced, and a session
  that writes as well is a second writer — which is exactly what made Task 1.3's first implementation
  spin for ever (one `SetListeningMode`, fanned out, every session calling the hub back, for ever, at
  full CPU). **Task 1.3 ships this contract half-done, and Task 1.6 is what completes it**: its
  `SetListeningMode` and `SetProfile` arms do route, with the local field moved first so the echo stops
  there, but its `UpdateConfig` arm does **not**, because routing from there *was* the loop — so a
  client's `UpdateConfig` today reaches its own session and nothing else, while spec §4.2 requires the
  server to persist it and broadcast `ConfigChanged`. That arm's comment already asserts the invariant
  ("by the time this message exists the hub has already written the file"); **Task 1.6's bridge is what
  makes that sentence true**, and the whole-branch review must check that it does.

Both of the first two were found by a reconciliation pass over eleven cross-task conflicts, and
neither was catchable by checking names: they were two *schedules* of one symbol. Whoever executes
this plan should ask the same question of every symbol it touches — **who runs first?** — and whoever
writes the next plan should copy this paragraph into its contract before the first task is written.

## Task index

Bodies follow this index in order. Each body carries its own `Files` and `Interfaces` block.

- **Phase 1 — the server and the remote client**
  - Task 1.1: The wire — `ConfigView`, three commands, three events `[mid]`
  - Task 1.2: The handshake becomes real `[small]`
  - Task 1.3: The `Hub`, unique sessions and the broadcasts `[mid]`
  - Task 1.4: The listener, the router and the bind rule `[mid]`
  - Task 1.5: The embedded bundle and the honest fallback pages `[small]`
  - Task 1.6: The connection bridge, its bounds and its endings `[mid]`
  - Task 1.7: `--connect` and the `serve` subcommand `[mid]`
  - Task 1.8: TLS, and the one rule that keeps a key off the wire `[mid]`
  - Task 1.9: Logging, redaction and the three documents `[small]`
- **Phase 2 — the web terminal**
  - Task 2.1: The PTY, its child and its teardown `[mid]`
  - Task 2.2: The raw-bytes socket and the terminal pump `[mid]`
  - Task 2.3: xterm.js, fetched and verified at build time `[small]`
  - Task 2.4: The terminal page, end to end `[small]`
- **Phase 3 — the web client**
  - Task 3.1: **The spike** — the gate the spec wrote `[mid]`
  - Task 3.2: `tipsy-web` and its socket `[mid]`
  - Task 3.3: The browser microphone `[mid]`
  - Task 3.4: Playback, the jitter buffer and barge-in `[mid]`
  - Task 3.5: The HUD glue, Ratzilla's input and the host page `[mid]`
  - Task 3.6: `cargo xtask dist` and the flag that turns true `[small]`
  - Task 3.7: The login door in the browser — the engine, not the screen `[mid]` *(added after the login plan landed on `master`: the login spec's §13 names it as one of the two things Plan 3 owes that feature, and no earlier task covers it)*

---

### Task 1.1: The wire — `ConfigView`, three commands, three events `[mid]`

**Files:**
- Modify: `crates/tipsy-protocol/src/lib.rs` — `ConfigView` and its seven section views (inserted after `LoginView`, `lib.rs:105`), five `ErrorCode` variants (appended inside the enum at `lib.rs:51-64`), three `Command` variants (inside the enum at `lib.rs:107-130`), three `Event` variants (inside the enum at `lib.rs:132-172`)
- Modify: `crates/tipsy-protocol/tests/roundtrip.rs` — the new variants' round trips and JSON shape, appended to the existing file
- Modify: `crates/tipsy-ui/src/config_form.rs` — `impl From<&ConfigForm> for ConfigView` and `impl From<ConfigView> for ConfigForm`, immediately after `impl Default for ConfigForm` (`config_form.rs:222-270`)
- Modify: `crates/tipsy-compadre/src/session.rs:343-363` — three behaviour-free `Command` arms
- Modify: `crates/tipsy-compadre/src/client.rs:619-640` — three behaviour-free `Command` arms in `starts_turn`
- Modify: `crates/tipsy-ui/src/state.rs:289-387` — three behaviour-free `Event` arms in `UiState::apply`
- Test: `crates/tipsy-protocol/tests/roundtrip.rs` (the existing free-standing test file, helpers at `:3` and `:9`); `crates/tipsy-compadre/src/config/form.rs` (its own `#[cfg(test)] mod tests`, which starts at `form.rs:207`)

**A planning ruling this body records.** `ConfigForm` **stays where it is** (`crates/tipsy-ui/src/config_form.rs`) and gains the conversions, rather than being deleted and moved into `tipsy-protocol`. The spec's §4.3 intent — one shape, no drift — is preserved by a test that round-trips the two. Deleting `ConfigForm` would instead have rewritten three files the spec does not give this task: `tipsy-ui/src/render_config.rs` draws and tests it (`:1`, `:49`, `:358-452`, `:545-546`), `tipsy-ui/src/state.rs` holds it (`:125`, `:177`, `:187`, `:941-957`), and `tipsy-compadre/src/client.rs` saves from it (`:518`, `:539-553`). Where §4.3's wording says "`tipsy-ui`'s form becomes a view of it", read it as the two conversions below plus the round-trip test, not as a move.

**Interfaces:**
- Consumes: `Capability` (`tipsy-protocol/src/lib.rs:69-75`), `ListeningMode` (`:17-24`), `ProfileView { id, name, language }` (`:92-97`) — all already in the crate this task edits; `roundtrip_command` (`crates/tipsy-protocol/tests/roundtrip.rs:3`) and `roundtrip_event` (`:9`).
- Produces:
  - `pub struct ConfigView { pub active_profile: String, pub providers: Vec<ProviderView>, pub listening: ListeningView, pub realtime: RealtimeView, pub login: LoginSettingsView, pub profiles: Vec<ProfileEntryView>, pub server: ServerView }`, deriving `Debug, Clone, PartialEq, Serialize, Deserialize`.
  - `pub struct ProviderView { pub capability: Capability, pub kind: String, pub base_url: String, pub model: String, pub key_name: String }`
  - `pub struct ListeningView { pub mode: ListeningMode, pub silence_ms: String, pub mic_gain: String, pub mic_device: Option<String> }`
  - `pub struct ServerView { pub bind: String, pub lan: bool, pub token: String }`
  - `pub struct RealtimeView { pub enabled: bool, pub model: String, pub key_name: String, pub voice: String, pub max_minutes: u32 }`
  - `pub struct LoginSettingsView { pub enabled: bool, pub voice: bool, pub allowed: Vec<AllowedPersonView> }` — **not `LoginView`**: see the ruling below.
  - `pub struct AllowedPersonView { pub name: String, pub keywords: Vec<String> }`
  - `pub struct ProfileEntryView { pub id: String, pub name: String, pub language: String, pub prompt: String, pub voice_instructions: String, pub greeting: String, pub wake_word: String, pub voices: Vec<(String, String)> }`
  - `impl ConfigView { pub fn redacted(self) -> Self }` — clears `self.server.token`. Task 1.3's answer to `GetConfig` for a client that may not see the token calls exactly this.
  - `impl From<&ConfigForm> for ConfigView` and `impl From<ConfigView> for ConfigForm`, both in `tipsy-ui`.
  - `ErrorCode::{ProtocolVersion, TokenRequired, TokenInvalid, BadMessage, InsecureTransport}` — five **new** variants on the enum at `tipsy-protocol/src/lib.rs:51-64`, which already carries `#[serde(rename_all = "snake_case")]` (`:52`), so the wire spellings are `protocol_version`, `token_required`, `token_invalid`, `bad_message`, `insecure_transport`. **`ErrorCode` is a third enum this task extends, and it is the one whose match set had to be measured rather than assumed** — see Step 3c-bis and Step 3f. `Event::Error.message` stays human text localized to the profile's language (v1 spec §4.2); the variant is what a client branches on.
  - `Command::{GetConfig, UpdateConfig { config: Box<ConfigView> }, SetKey { name: String, value: String }}` — **boxed, and this is not cosmetic**: `ConfigView` carries seven section views, and unboxed it pushes `Command`, `Event`, `ToServer`, `ToClient` and `tipsy-ui`'s `UiAction` over `clippy::large_enum_variant` — five findings against a lint gate that allows none. Serde sees through a `Box`, so the wire JSON is byte-identical and `roundtrip.rs`'s nested-tag test pins that.
  - `Event::{Config { config: Box<ConfigView> }, ProfileChanged { profile: ProfileView }, ConfigChanged}` — boxed for the same reason.

**The complete field list, read from the tree.** `ConfigView` is `ConfigForm`'s configuration-bearing fields, moved verbatim, with each section type renamed `…Form` → `…View`. Every line below was read in this checkout:

| `ConfigForm` field | Type it is, verbatim | Lines |
|---|---|---|
| `active_profile: String` | `String` | `config_form.rs:166` |
| `providers: Vec<ProviderForm>` | `pub struct ProviderForm { capability: Capability, kind: String, base_url: String, model: String, key_name: String }` | `:167`, struct `:74-81` |
| `listening: ListeningForm` | `pub struct ListeningForm { mode: ListeningMode, silence_ms: String, mic_gain: String, mic_device: Option<String> }` | `:168`, struct `:83-92` |
| `realtime: RealtimeForm` | `pub struct RealtimeForm { enabled: bool, model: String, key_name: String, voice: String, max_minutes: u32 }` | `:170`, struct `:109-117` |
| `login: LoginForm` | `pub struct LoginForm { enabled: bool, voice: bool, allowed: Vec<AllowedPersonForm> }` | `:172`, struct `:122-127` |
| `profiles: Vec<ProfileForm>` | `pub struct ProfileForm { id: String, name: String, language: String, prompt: String, voice_instructions: String, greeting: String, wake_word: String, voices: Vec<(String, String)> }` | `:173`, struct `:144-156` |
| `server: ServerForm` | `pub struct ServerForm { bind: String, lan: bool, token: String }` | `:174`, struct `:94-99` |
| — | `pub struct AllowedPersonForm { name: String, keywords: Vec<String> }` | struct `:130-134` |

The six `ConfigForm` fields **left out** are its own view state and none of them is configuration: `tab: ConfigTab` (`:160`), `field: usize` (`:162`), `editing: Option<String>` (`:165`), `notice: Option<(NoticeKind, String)>` (`:175`), `dirty: bool` (`:176`) and the `pub(crate) filled: bool` (`:181`). `filled` in particular **must not** cross the wire: it is what makes an unanswered form refuse to save (`config_form.rs:321-333`), and a `ConfigView` that arrived over a socket must never mark a form filled on the strength of a message rather than of a file that was read.

**One name this body rules on, because the front matter says "renamed `…Form` → `…View`" and the crate already has a `ProfileView`.** `ProfileView { id, name, language }` exists at `lib.rs:92-97`; it is `Welcome`'s payload (`lib.rs:137`) and the type spec §4.4 requires `Event::ProfileChanged` to carry. It is not a profile *entry* — it has no prompt, greeting or voices. The form's `ProfileForm` therefore becomes **`ProfileEntryView`**, not `ProfileView`. Keeping one meaning per word matters more here than the mechanical rename: `ProfileView` is what a client is told, `ProfileEntryView` is what an editor edits.

**And a second name, which this body's first implementer found the only way it could be found — by not compiling.** The rename rule also applies to the form's `LoginForm`, and `LoginView` **already exists** (`lib.rs:99-105`, `{ voice: bool }`): it is `Welcome.login`, the login *screen*'s payload, read in twenty-one places across `session.rs`, `client.rs` and `tests/login.rs`, and it is the type Task 3.7's browser engine takes. The `[login]` *section* is a different thing — it is what the configuration screen edits, and it carries `enabled` and `allowed` as well. So the section view is **`LoginSettingsView`**, and no type this plan adds may be called `LoginView`. This paragraph is the only place that ruling is written down; Task 3.7's uses of `LoginView` are the existing type and keep the name.

**The `Config` ⇄ form job stays where it is.** `form_of` (`crates/tipsy-compadre/src/config/form.rs:21-75`) and `config_from` (`:81-130`) keep their signatures and their job; the wire shape hangs off the form beside them. `form_of`'s own comment (`form.rs:22-25`) explains why: `ConfigForm`'s `filled` is private to `tipsy-ui`, so only `tipsy-ui` may build one with literal syntax — which is also why the two `From` impls live there and not in `tipsy-compadre`.

- [ ] **Step 1: Write the failing test**

Append to `crates/tipsy-protocol/tests/roundtrip.rs`, after `a_welcome_without_the_login_field_parses_as_no_login_screen` (`:185-200`), and add this `use` line at the top of that file. It names only the seven section views: `ConfigView`'s neighbours `Capability`, `ProfileView`, `ListeningMode` and `UiLanguage` come from the glob `use tipsy_protocol::*;` that is already at `:1`, and naming them twice would be an unused-import warning or a duplicate, depending on how rustc resolves the two:

```rust
use tipsy_protocol::{
    AllowedPersonView, ConfigView, ListeningView, LoginSettingsView, ProfileEntryView, ProviderView, RealtimeView,
    ServerView,
};
```

```rust
/// The whole editable configuration as the screen edits it, populated so that nothing round-trips by
/// accident: three connectors of two kinds, both `Option` shapes, a carried token, both carried
/// sections and a profile with a voices table.
fn config_view() -> ConfigView {
    ConfigView {
        active_profile: "compae".into(),
        providers: vec![
            ProviderView {
                capability: Capability::Chat,
                kind: "openai_compat".into(),
                base_url: "https://openrouter.ai/api/v1".into(),
                model: "google/gemini-2.5-flash".into(),
                key_name: "openrouter".into(),
            },
            ProviderView {
                capability: Capability::Speech,
                kind: "elevenlabs".into(),
                base_url: "https://api.elevenlabs.io".into(),
                model: "eleven_flash_v2_5".into(),
                key_name: "elevenlabs".into(),
            },
            ProviderView {
                capability: Capability::Transcribe,
                kind: "openai_compat".into(),
                base_url: "http://localhost:11434/v1".into(),
                model: "openai/whisper-large-v3".into(),
                key_name: String::new(),
            },
        ],
        listening: ListeningView {
            mode: ListeningMode::Auto,
            silence_ms: "1000".into(),
            mic_gain: "12".into(),
            mic_device: Some("Rode NT-USB".into()),
        },
        realtime: RealtimeView {
            enabled: true,
            model: "gpt-realtime-2.1-mini".into(),
            key_name: "openai-realtime".into(),
            voice: "cedar".into(),
            max_minutes: 25,
        },
        login: LoginSettingsView {
            enabled: true,
            voice: false,
            allowed: vec![
                AllowedPersonView { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] },
                AllowedPersonView { name: "María".into(), keywords: vec!["maria".into()] },
            ],
        },
        profiles: vec![ProfileEntryView {
            id: "compae".into(),
            name: "El Compae".into(),
            language: "es-ES".into(),
            prompt: "Habla como un compae.".into(),
            voice_instructions: "andaluz, tranquilo".into(),
            greeting: "¿Qué pasa, quillo?".into(),
            wake_word: "oye compae".into(),
            voices: vec![
                ("elevenlabs".to_string(), "george".to_string()),
                ("openai_compat".to_string(), "alloy".to_string()),
            ],
        }],
        server: ServerView {
            bind: "127.0.0.1:7070".into(),
            lan: false,
            token: "0123456789abcdef0123456789abcdef".into(),
        },
    }
}

/// The three commands this tier adds, each in both directions, with a fully populated `ConfigView`.
#[test]
fn the_configuration_commands_roundtrip() {
    roundtrip_command(Command::GetConfig);
    roundtrip_command(Command::UpdateConfig { config: config_view() });
    roundtrip_command(Command::SetKey { name: "openrouter".into(), value: "sk-or-v1-…a1b2".into() });
}

/// The three events, each in both directions. `ProfileChanged` carries the same `ProfileView`
/// `Welcome` carries, which is why it is built from the type that already exists.
#[test]
fn the_configuration_events_roundtrip() {
    roundtrip_event(Event::Config { config: config_view() });
    roundtrip_event(Event::ProfileChanged {
        profile: ProfileView { id: "senyor".into(), name: "O Senyor".into(), language: "gl-ES".into() },
    });
    roundtrip_event(Event::ConfigChanged);
}

/// §4.2 and §4.4's JSON exactly: the tag is the variant's snake_case name, `UpdateConfig` nests the
/// whole view under `config`, and `ConfigChanged` deliberately carries no payload (spec §4.4).
#[test]
fn the_configuration_messages_spell_their_tags_as_the_spec_says() {
    assert_eq!(serde_json::to_string(&Command::GetConfig).unwrap(), r#"{"type":"get_config"}"#);
    assert_eq!(serde_json::from_str::<Command>(r#"{"type":"get_config"}"#).unwrap(), Command::GetConfig);

    assert_eq!(serde_json::to_string(&Event::ConfigChanged).unwrap(), r#"{"type":"config_changed"}"#);
    assert_eq!(serde_json::from_str::<Event>(r#"{"type":"config_changed"}"#).unwrap(), Event::ConfigChanged);

    let json = serde_json::to_string(&Command::SetKey { name: "openrouter".into(), value: "v".into() }).unwrap();
    assert_eq!(json, r#"{"type":"set_key","name":"openrouter","value":"v"}"#);

    let json = serde_json::to_string(&Event::ProfileChanged {
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
    })
    .unwrap();
    assert_eq!(
        json,
        r#"{"type":"profile_changed","profile":{"id":"compae","name":"El Compae","language":"es-ES"}}"#
    );

    // `UpdateConfig` is the one shape whose whole payload is nested, so it is read apart in full:
    // the field name is `config`, and the view's own field names are the form's, unchanged.
    let json = serde_json::to_string(&Command::UpdateConfig { config: config_view() }).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["type"], "update_config");
    assert_eq!(value["config"]["active_profile"], "compae");
    assert_eq!(value["config"]["listening"]["mode"], "auto");
    assert_eq!(value["config"]["providers"][0]["capability"], "chat");
    assert_eq!(value["config"]["providers"][0]["key_name"], "openrouter");
    assert_eq!(value["config"]["server"]["bind"], "127.0.0.1:7070");
    assert_eq!(value["config"]["realtime"]["key_name"], "openai-realtime");
    assert_eq!(value["config"]["login"]["allowed"][0]["name"], "Andrés");
    assert_eq!(value["config"]["profiles"][0]["voices"][0][0], "elevenlabs");

    let json = serde_json::to_string(&Event::Config { config: config_view() }).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["type"], "config");
    assert_eq!(value["config"]["server"]["token"], "0123456789abcdef0123456789abcdef");
}

/// The five `ErrorCode` variants this tier adds. The spelling is the point and the reason they exist
/// at all: a client branches on `code`, and v1 spec §4.2 makes `Event::Error.message` human text
/// localized to the profile's language, so a code parked in the message would be unreadable to it.
#[test]
fn the_new_error_codes_spell_their_tags_as_the_spec_says() {
    for (code, spelling) in [
        (ErrorCode::ProtocolVersion, "protocol_version"),
        (ErrorCode::TokenRequired, "token_required"),
        (ErrorCode::TokenInvalid, "token_invalid"),
        (ErrorCode::BadMessage, "bad_message"),
        (ErrorCode::InsecureTransport, "insecure_transport"),
    ] {
        assert_eq!(serde_json::to_value(code).unwrap(), serde_json::json!(spelling));
        assert_eq!(serde_json::from_value::<ErrorCode>(serde_json::json!(spelling)).unwrap(), code);
    }

    // The variants are additive: the ten that were there keep their spelling byte for byte.
    for (code, spelling) in [
        (ErrorCode::Auth, "auth"),
        (ErrorCode::RateLimited, "rate_limited"),
        (ErrorCode::Timeout, "timeout"),
        (ErrorCode::Network, "network"),
        (ErrorCode::BadResponse, "bad_response"),
        (ErrorCode::NoKey, "no_key"),
        (ErrorCode::Mic, "mic"),
        (ErrorCode::Config, "config"),
        (ErrorCode::NothingHeard, "nothing_heard"),
        (ErrorCode::Internal, "internal"),
    ] {
        assert_eq!(serde_json::to_value(code).unwrap(), serde_json::json!(spelling));
    }

    // …and they travel inside `Event::Error` unchanged, which is where a client reads them.
    roundtrip_event(Event::Error {
        code: ErrorCode::TokenRequired,
        message: "Este servidor pide un token: pásalo con --token.".into(),
        recoverable: false,
    });
    let json = serde_json::to_string(&Event::Error {
        code: ErrorCode::InsecureTransport,
        message: "Esa conexión no va cifrada.".into(),
        recoverable: true,
    })
    .unwrap();
    assert_eq!(
        json,
        r#"{"type":"error","code":"insecure_transport","message":"Esa conexión no va cifrada.","recoverable":true}"#
    );
}
```

Append to the `mod tests` of `crates/tipsy-compadre/src/config/form.rs`, after `connector_names_are_the_serialised_ones` (`:399-405`). The module already has `config()` at `:218-293` and `form_of`/`config_from` in scope through `use super::*;` at `:211`, which brings in the new `view_of` and `config_of` too:

```rust
    /// §4.3's security rule, as a test over bytes: a `ConfigView` carries the key *names* and nothing
    /// else. The fixture's key values are the ones a store really would hold, so one leak is visible.
    #[test]
    fn a_config_view_carries_no_key_value() {
        let mut config = config();
        // Names the screen really shows. The secret values live only here, in this test, and are not
        // — and must never become — a real key: the rule is that no field may hold one.
        config.providers.chat.key = "openrouter".into();
        config.providers.speech.key = "elevenlabs".into();
        config.realtime.key = "openai-realtime".into();

        let view = view_of(&config);
        let json = serde_json::to_string(&view).unwrap();

        for secret in ["sk-or-v1-9e2f", "el-secret-value", "rt-secret-value"] {
            assert!(!json.contains(secret), "no key value may cross: {secret} in {json}");
        }
        // …and the names are exactly what did cross, so the loop above cannot pass because the field
        // is missing altogether.
        assert!(json.contains(r#""key_name":"openrouter""#), "{json}");
        assert!(json.contains(r#""key_name":"elevenlabs""#), "{json}");
        assert!(json.contains(r#""key_name":"openai-realtime""#), "{json}");
    }

    /// §4.3's second half, and the shape Task 1.3 needs: the token is the one thing the view carries
    /// that some clients may not see, so it lives in exactly one field and there is exactly one way
    /// to take it away.
    #[test]
    fn the_token_lives_in_one_field_and_redacting_empties_it() {
        let json = serde_json::to_string(&view_of(&config())).unwrap();
        assert!(
            json.contains(r#""token":"0123456789abcdef0123456789abcdef""#),
            "the token crosses as itself, in `server.token`: {json}"
        );
        // One occurrence and one only: a second field carrying the token would be the leak §4.3 rules
        // out, and this is the assertion that notices it.
        assert_eq!(json.matches("0123456789abcdef0123456789abcdef").count(), 1, "{json}");

        let redacted = serde_json::to_string(&view_of(&config()).redacted()).unwrap();
        assert!(!redacted.contains("0123456789abcdef0123456789abcdef"), "{redacted}");
        assert!(redacted.contains(r#""bind":"0.0.0.0:7070""#), "only the token goes: {redacted}");
    }

    /// The ruling: one shape, two representations, and the trip through the wire shape is what keeps
    /// them from drifting. What the screen opens on, carried as a `ConfigView` and brought back, is
    /// the configuration that produced it — value for value, order for order.
    #[test]
    fn a_config_view_round_trips_the_whole_configuration() {
        for config in [Config::default(), config()] {
            let form = form_of(&config);
            let view = ConfigView::from(&form);

            let json = serde_json::to_string(&view).unwrap();
            let back: ConfigView = serde_json::from_str(&json).unwrap();
            assert_eq!(back, view, "the wire shape must survive its own JSON: {json}");

            let again = ConfigForm::from(back);
            assert_eq!(config_from(&again).unwrap(), config, "a view must rebuild the same file");
            assert_eq!(config_of(view.clone()).unwrap(), config, "and so must the one-step path");
        }
    }

    /// `ConfigForm`'s own view state belongs to the screen, not to the wire. A form built from a
    /// `ConfigView` is **not** marked filled, because a message is not a file that was read: `Ctrl+S`
    /// on such a form must still refuse (Plan 2B's whole point for the flag).
    #[test]
    fn a_form_built_from_a_view_is_not_filled_and_keeps_the_screens_own_state() {
        let mut opened = form_of(&Config::default());
        opened.dirty = true;

        let view = ConfigView::from(&opened);
        let rebuilt = ConfigForm::from(view);

        assert!(!rebuilt.filled(), "only a file the client read may mark a form filled");
        assert!(!rebuilt.dirty, "the dirty flag is the screen's own, and starts clear");
        assert_eq!(ConfigTab::Providers, rebuilt.tab, "the screen opens on its first tab");
        assert_eq!(config_from(&rebuilt).unwrap(), Config::default());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-protocol the_configuration
```

Expected: FAIL — the crate does not compile. The first error is

```
error[E0433]: failed to resolve: use of undeclared type `ConfigView`
 --> crates\tipsy-protocol\tests\roundtrip.rs
```

with `error[E0599]: no variant or associated item named `GetConfig` found for enum `Command`` behind it, and the same for `UpdateConfig`, `SetKey`, `Config`, `ProfileChanged`, `ConfigChanged`, `ProtocolVersion`, `TokenRequired`, `TokenInvalid`, `BadMessage` and `InsecureTransport`. That is one half of the failure; the other crate is next.

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre config::form
```

Expected: FAIL — the same shape in the other crate:

```
error[E0425]: cannot find function `view_of` in this scope
 --> crates\tipsy-compadre\src\config\form.rs
```

- [ ] **Step 3: Implement the wire shape, the conversions, and the arms the workspace cannot compile without**

**3a.** In `crates/tipsy-protocol/src/lib.rs`, insert after `LoginView` (`:99-105`) and before `Command` (`:107`). The doc comment on `ConfigView` is the one place this rule is written down for a reader of the protocol crate:

```rust
/// `[server]`, as the configuration screen edits it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerView {
    pub bind: String,
    pub lan: bool,
    /// §4.3: the one field of this whole type that may hold something a client is not entitled to
    /// see. A connection that presented no token receives [`ConfigView::redacted`]'s empty string.
    pub token: String,
}

/// One row of the Providers tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderView {
    pub capability: Capability,
    pub kind: String,
    pub base_url: String,
    pub model: String,
    /// The key's *name*, never the key: the same rule the configuration file follows.
    pub key_name: String,
}

/// `[listening]`, as the screen edits it. `None` is the system default device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListeningView {
    pub mode: ListeningMode,
    /// The row is a text field, so the value travels as the text it holds and the save parses it.
    pub silence_ms: String,
    pub mic_gain: String,
    pub mic_device: Option<String>,
}

/// `[realtime]`, carried and never drawn (Plan 2C Task 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RealtimeView {
    pub enabled: bool,
    pub model: String,
    /// The key's *name*, never the key.
    pub key_name: String,
    pub voice: String,
    pub max_minutes: u32,
}

/// One `[[login.allowed]]` entry (the login-screen plan's wire).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllowedPersonView {
    pub name: String,
    pub keywords: Vec<String>,
}

/// `[login]`, carried and never drawn. **Named `LoginSettingsView`, not `LoginView`**: that name
/// belongs to `Welcome`'s login-*screen* payload (`lib.rs:99-105`), which is a different thing — the
/// ruling above the field list says why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginSettingsView {
    pub enabled: bool,
    pub voice: bool,
    pub allowed: Vec<AllowedPersonView>,
}

/// One `[[profiles]]` entry. Deliberately **not** `ProfileView`: that type is the three fields a
/// client is *told* about its active profile; this is the whole entry an editor edits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileEntryView {
    pub id: String,
    pub name: String,
    pub language: String,
    pub prompt: String,
    pub voice_instructions: String,
    pub greeting: String,
    pub wake_word: String,
    /// Sorted by connector name: a map's order is not a screen order, and the row index names back.
    pub voices: Vec<(String, String)>,
}

/// The editable configuration, as the configuration screen edits it (spec §4.3).
///
/// The fields are `tipsy_ui::ConfigForm`'s, field for field, with each section type renamed
/// `…Form` → `…View`. The screen's own view state — the focused tab and row, the text being typed,
/// the notice line, the dirty flag and the "filled" flag — stays on the form and never crosses.
///
/// **No field of this type may hold a secret.** Exactly one field may hold something a client is not
/// entitled to see: `server.token`, which is what [`ServerView`]'s own doc comment says, and
/// [`ConfigView::redacted`] is the one way to take it away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigView {
    pub active_profile: String,
    pub providers: Vec<ProviderView>,
    pub listening: ListeningView,
    pub realtime: RealtimeView,
    pub login: LoginSettingsView,
    pub profiles: Vec<ProfileEntryView>,
    pub server: ServerView,
}

impl ConfigView {
    /// The same view with no token in it: what `Event::Config` answers a client that is neither a
    /// loopback connection nor one that presented the server's token (spec §4.3). It clears the field
    /// rather than removing it, so the answering side has one code path and the other end has no
    /// `Option` to interpret.
    pub fn redacted(mut self) -> Self {
        self.server.token.clear();
        self
    }
}
```

**3b.** Three `Command` variants, appended after `LoginAnswer` (`lib.rs:129`) so that no existing variant's position, shape or JSON changes:

```rust
    /// The configuration screen opening: the session answers `Event::Config`.
    GetConfig,
    /// The configuration screen saving. The server replaces the editable configuration, persists it,
    /// re-derives every session's brain and broadcasts `Event::ConfigChanged` (spec §4.2).
    UpdateConfig { config: ConfigView },
    /// A new API key for the *server's* credential store. Refused unless the connection is loopback
    /// or TLS (spec §9.4), and answered with `Event::ConfigChanged` when it is accepted.
    SetKey { name: String, value: String },
```

**3c.** Three `Event` variants, appended after `LoginVerdict` (`lib.rs:171`), for the same reason:

```rust
    /// The redacted configuration, answering `Command::GetConfig` (spec §4.4).
    Config { config: ConfigView },
    /// The active profile changed. Carries the same three fields `Welcome` carries.
    ProfileChanged { profile: ProfileView },
    /// The configuration changed. Deliberately no payload: a client that is showing it asks again
    /// with `GetConfig`, and sending the whole configuration on every save would put the token on the
    /// wire more often than it needs to be (spec §4.4).
    ConfigChanged,
```

**3c-bis.** Five `ErrorCode` variants, appended after `Internal` (`lib.rs:63`) so that the wire spelling of every existing variant is untouched. They are what makes spec §4.1's and §10's refusals and §9.4's refusal *branchable* on the wire: `Event::Error.message` is human text — v1 spec §4.2 says it is "already localized to the profile's language" — so a client may not read it to decide anything, and a code string parked in `message` would be invisible to the branch a client actually has to make.

```rust
    /// The client's `protocol_version` is not this server's (spec §4.1).
    ProtocolVersion,
    /// The server is exposed and `Hello` carried no token (spec §4.1).
    TokenRequired,
    /// The server is exposed and `Hello` carried a token that is not its own (spec §4.1).
    TokenInvalid,
    /// A text frame that is not a well-formed `Command`, or a binary frame that is not an audio
    /// `Frame`. Recoverable: the connection stays open (spec §10).
    BadMessage,
    /// `SetKey` on a connection that is neither loopback nor TLS (spec §9.4). The variant lands here
    /// because the wire needs the spelling; **Task 1.8 owns the refusal that emits it**, and this
    /// task emits nothing with it.
    InsecureTransport,
```

**3d.** The two conversions in `crates/tipsy-ui/src/config_form.rs`, immediately after `impl Default for ConfigForm` (`:222-270`). Replace the import at `:8` with:

```rust
use tipsy_protocol::{
    AllowedPersonView, Capability, ConfigView, ListeningMode, ListeningView, LoginSettingsView, ProfileEntryView,
    ProviderView, RealtimeView, ServerView, UiLanguage,
};
```

and add:

```rust
/// The form as the wire's own shape. Same fields, so this is a rename and a move of plain values;
/// the round-trip test in `tipsy-compadre` is what keeps it one shape.
impl From<&ConfigForm> for ConfigView {
    fn from(form: &ConfigForm) -> Self {
        Self {
            active_profile: form.active_profile.clone(),
            providers: form
                .providers
                .iter()
                .map(|row| ProviderView {
                    capability: row.capability,
                    kind: row.kind.clone(),
                    base_url: row.base_url.clone(),
                    model: row.model.clone(),
                    key_name: row.key_name.clone(),
                })
                .collect(),
            listening: ListeningView {
                mode: form.listening.mode,
                silence_ms: form.listening.silence_ms.clone(),
                mic_gain: form.listening.mic_gain.clone(),
                mic_device: form.listening.mic_device.clone(),
            },
            realtime: RealtimeView {
                enabled: form.realtime.enabled,
                model: form.realtime.model.clone(),
                key_name: form.realtime.key_name.clone(),
                voice: form.realtime.voice.clone(),
                max_minutes: form.realtime.max_minutes,
            },
            login: LoginSettingsView {
                enabled: form.login.enabled,
                voice: form.login.voice,
                allowed: form
                    .login
                    .allowed
                    .iter()
                    .map(|entry| AllowedPersonView {
                        name: entry.name.clone(),
                        keywords: entry.keywords.clone(),
                    })
                    .collect(),
            },
            profiles: form
                .profiles
                .iter()
                .map(|profile| ProfileEntryView {
                    id: profile.id.clone(),
                    name: profile.name.clone(),
                    language: profile.language.clone(),
                    prompt: profile.prompt.clone(),
                    voice_instructions: profile.voice_instructions.clone(),
                    greeting: profile.greeting.clone(),
                    wake_word: profile.wake_word.clone(),
                    voices: profile.voices.clone(),
                })
                .collect(),
            server: ServerView {
                bind: form.server.bind.clone(),
                lan: form.server.lan,
                token: form.server.token.clone(),
            },
        }
    }
}

impl From<ConfigView> for ConfigForm {
    /// A form that is **not** marked filled: `filled` says "the client read a file and this form
    /// carries it", and a form rebuilt from a message has read nothing yet. Everything else — the tab,
    /// the focused row, the edit buffer, the notice and the dirty flag — is the screen's own state and
    /// starts where `ConfigForm::default` starts it.
    ///
    /// Written as assignments onto `ConfigForm::default()` rather than as one literal, because
    /// `filled` is private to this crate and struct-literal syntax for a `ConfigForm` outside it is
    /// exactly what `form_of`'s comment forbids. The lint that prefers a literal is allowed here for
    /// that reason and no other.
    #[allow(clippy::field_reassign_with_default)]
    fn from(view: ConfigView) -> Self {
        let mut form = ConfigForm::default();
        form.active_profile = view.active_profile;
        form.providers = view
            .providers
            .into_iter()
            .map(|row| ProviderForm {
                capability: row.capability,
                kind: row.kind,
                base_url: row.base_url,
                model: row.model,
                key_name: row.key_name,
            })
            .collect();
        form.listening = ListeningForm {
            mode: view.listening.mode,
            silence_ms: view.listening.silence_ms,
            mic_gain: view.listening.mic_gain,
            mic_device: view.listening.mic_device,
        };
        form.realtime = RealtimeForm {
            enabled: view.realtime.enabled,
            model: view.realtime.model,
            key_name: view.realtime.key_name,
            voice: view.realtime.voice,
            max_minutes: view.realtime.max_minutes,
        };
        form.login = LoginForm {
            enabled: view.login.enabled,
            voice: view.login.voice,
            allowed: view
                .login
                .allowed
                .into_iter()
                .map(|entry| AllowedPersonForm { name: entry.name, keywords: entry.keywords })
                .collect(),
        };
        form.profiles = view
            .profiles
            .into_iter()
            .map(|profile| ProfileForm {
                id: profile.id,
                name: profile.name,
                language: profile.language,
                prompt: profile.prompt,
                voice_instructions: profile.voice_instructions,
                greeting: profile.greeting,
                wake_word: profile.wake_word,
                voices: profile.voices,
            })
            .collect();
        form.server = ServerForm { bind: view.server.bind, lan: view.server.lan, token: view.server.token };
        form
    }
}
```

**3e.** The two one-step helpers, in `crates/tipsy-compadre/src/config/form.rs`. Replace the import at `:9` (`use tipsy_protocol::Capability;`) with `use tipsy_protocol::{Capability, ConfigView};`, then add after `config_from` (`:130`):

```rust
/// The wire's shape for the configuration a file holds: `form_of` and then the form's own
/// conversion. One path, so a `GetConfig` answer can never describe a file differently from the
/// screen that edits it.
pub fn view_of(config: &Config) -> ConfigView {
    ConfigView::from(&form_of(config))
}

/// The configuration a `ConfigView` describes, refusing what `config_from` refuses and by the same
/// names, so a configuration that arrives over the wire is saved exactly as strictly as one typed
/// into the screen.
pub fn config_of(view: ConfigView) -> Result<Config, String> {
    config_from(&ConfigForm::from(view))
}
```

**3f.** The arms the workspace cannot compile without. **Adding three variants to `Command` breaks every exhaustive match over `Command`; adding three to `Event` breaks every exhaustive match over `Event`; adding five to `ErrorCode` breaks every exhaustive match over `ErrorCode`.** Measured by grep over `crates/**/*.rs`, these are all of them:

| Match | Location | What breaks |
|---|---|---|
| `Session::command` | `crates/tipsy-compadre/src/session.rs:343-363` | three `Command` variants |
| `starts_turn` | `crates/tipsy-compadre/src/client.rs:619-640` | three `Command` variants |
| `UiState::apply` | `crates/tipsy-ui/src/state.rs:289-387` | three `Event` variants |
| — | nothing | **no exhaustive match over `ErrorCode` exists anywhere in the workspace** |

That last row is a measurement, not a guess, and it is the row the implementer must re-take rather than trust: a grep for `ErrorCode::` over `crates/**/*.rs` returns `46` hits and **every one of them is a construction or a comparison**, never a `match` arm. The three functions that *produce* an `ErrorCode` — `ProviderError::code` (`crates/tipsy-compadre/src/providers/mod.rs:108-116`), `RealtimeError::code` (`crates/tipsy-compadre/src/realtime/socket.rs:158-167`) and `RealtimeEventError::code` (`crates/tipsy-compadre/src/realtime/events.rs:85-87`) — match over *their own* error enums and return an `ErrorCode`; adding a variant to `ErrorCode` cannot make any of them non-exhaustive and does not change what they return. `tipsy-ui` names `ErrorCode` only in four `Event::Error` fixtures (`state.rs:704`, `:796`, `:851`, `:1035`) and never matches on it. **So this third enum needs no behaviour-free arm** — which is the opposite of the answer for `Event` and `Command`, and is exactly why it had to be measured. Re-take the measurement with the same two greps before believing the row:

```powershell
rg -n "ErrorCode::" crates
rg -n "match .*\{" crates/tipsy-ui/src crates/tipsy-compadre/src
```

The first must come back with constructions and comparisons only; the second must show no `match` whose arms are `ErrorCode` variants. If a match over `ErrorCode` has appeared in the meantime, it gets the same treatment as the three below: a behaviour-free arm, with the same one-line note naming Task 1.2 or 1.8 as the owner of the behaviour.

Nothing else in the workspace breaks either, and this too is worth checking rather than assuming: the other matches the grep turns up are not exhaustive. `takes_the_floor` (`crates/tipsy-compadre/src/client.rs:604-613`) uses `matches!`; the `Event` arms of `crates/tipsy-compadre/tests/realtime.rs` (`:593`, `:603`, `:657`, `:713`, `:726`) all carry a `_ => None`; and `events_of` (`:574-581`) matches `ToClient`, whose variants this task does not touch. This is the trap Plan 2C's Task 1 hit for `Event::Live`, with one correction to what that task recorded: it added an `Event` variant only, so `UiState::apply` was the single match it had to touch. This task adds `Command` variants as well, and that is why two more matches appear here.

In `crates/tipsy-compadre/src/session.rs`, inside `fn command` (the match at `:344`), append after the `LoginStart | LoginStop | LoginAnswer` arm (`:361`):

```rust
            // Task 1.3 owns these three: the configuration over the wire is answered by a session
            // only once there is a server to persist through. Accepted and ignored here so the match
            // stays exhaustive and this task owns no behaviour.
            Command::GetConfig | Command::UpdateConfig { .. } | Command::SetKey { .. } => {}
```

In `crates/tipsy-compadre/src/client.rs`, inside `fn starts_turn` (the match at `:620`), extend the existing "no turn" arm:

```rust
        Command::Hello { .. }
        | Command::TestProvider { .. }
        | Command::ListenStart
        | Command::Cancel
        | Command::SetProfile { .. }
        | Command::SetListeningMode { .. }
        // Task 1.1's three: reading, writing and keying the configuration start no turn, speak
        // nothing and write nothing to the log. Task 1.3 gives them their behaviour; this task only
        // keeps the decision exhaustive.
        | Command::GetConfig
        | Command::UpdateConfig { .. }
        | Command::SetKey { .. } => false,
```

In `crates/tipsy-ui/src/state.rs`, inside `UiState::apply` (the match at `:290`), immediately before the `Event::Error` arm (`:376`):

```rust
            // The configuration's own three events. Where they are *shown* is a later task's: the
            // screen is `ConfigForm`'s to draw and the HUD is `UiState`'s, and neither has a home for
            // them here. They are here to keep the match exhaustive, and this task behaviour-free.
            Event::Config { .. } => {}
            Event::ProfileChanged { .. } => {}
            Event::ConfigChanged => {}
```

**3g.** The crate's own module doc (`tipsy-protocol/src/lib.rs:1-3`) now understates what it carries:

```rust
//! Messages exchanged between the tipsy-compadre server and its clients.
//!
//! Text messages are JSON (`Command`, `Event`, and the configuration they carry as `ConfigView`);
//! audio travels as binary `Frame`s.
```

- [ ] **Step 4: Run the test to verify it passes**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-protocol roundtrip
```

Expected: PASS — `13` tests before this task's `4`, `17` after, `0` failed.

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre config::form
```

Expected: PASS — `7` tests before this task's `4`, `11` after, `0` failed.

Then the two other crates this task touched, because a stale arm anywhere else is the failure most likely to be left behind here:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-ui
```

Expected: PASS — `123` tests (`tipsy-ui` lib `95` + `tests/render.rs` `28`), `0` failed. **An earlier draft of this line said `85` (`71` + `14`) and that was wrong** — Task 1.1's implementer measured the real split and it is 95 + 28, so any task that "fixes" a count of 123 back to 85 is following a stale document rather than the tree. If the only failure is `error[E0004]: non-exhaustive patterns` naming a file this task's `Files` block does not list, Step 3f's grep was run against a tree that has since moved: find the arm, add the behaviour-free one, and name the file in the commit body.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-protocol/src/lib.rs crates/tipsy-protocol/tests/roundtrip.rs crates/tipsy-ui/src/config_form.rs crates/tipsy-ui/src/state.rs crates/tipsy-compadre/src/config/form.rs crates/tipsy-compadre/src/session.rs crates/tipsy-compadre/src/client.rs
git commit -m "feat(protocol): carry the configuration and its changes on the wire

ConfigView and its seven section views are tipsy_ui::ConfigForm's fields,
moved verbatim, with ConfigForm keeping its home and gaining the two
conversions rather than being deleted and moved: the round-trip test is what
holds the one shape together. Adds Command::{GetConfig, UpdateConfig, SetKey}
and Event::{Config, ProfileChanged, ConfigChanged}, all additive, with
PROTOCOL_VERSION staying 1, plus the behaviour-free arms the two enums' new
variants force in session.rs, client.rs and state.rs — a workspace that does
not compile is not a deliverable. Task 1.3 owns the real handling.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 1.2: The handshake becomes real `[small]`

**Files:**
- Modify: `crates/tipsy-compadre/src/session.rs` — `SessionPolicy`, the session-id counter, `SessionEnds.session_id`, the handshake prelude before `welcome()`, the `Command::Hello` arm, and the `ControlFlow` the refusal paths need (`spawn_session` `:36-43`, `spawn_with_events` `:56-95`, `run` `:150-204`, `handle` `:263-272`, `dispatch` `:274-293`, `command` `:343-363`)
- Modify: `crates/tipsy-compadre/src/client.rs:119` — the in-process client's `spawn_with_events` call gains the policy
- Test: `crates/tipsy-compadre/tests/session.rs` (the existing harness: fakes `:27-105`, `config()` `:114-117`, `session()` `:120-133`, `send` `:135-137`, `next_matching` `:144-167`, `drain` `:182-195`)

**Interfaces:**
- Consumes: `Command::Hello { token: Option<String>, client: ClientKind, protocol_version: u32 }` (`crates/tipsy-protocol/src/lib.rs:110`); `PROTOCOL_VERSION` (`:7`); `ClientKind` (`:9-15`); `Event::Welcome` (`:135-145`); `Event::Error { code: ErrorCode, message: String, recoverable: bool }` (`:154`); `ErrorCode::ProtocolVersion`, `::TokenRequired`, `::TokenInvalid` — Task 1.1's three new variants (`lib.rs:51-64` region).
- Produces — **these are the signatures the plan's other tasks must be told about.** Task 1.3 adds one more parameter to both constructors; Task 1.6 and Task 1.7 call them and read `SessionEnds`:
  - `pub struct SessionPolicy { pub require_token: bool, pub token: String }`, deriving `Debug, Clone, PartialEq, Eq`, with `pub fn local() -> Self` (`require_token: false`, `token: String::new()`) and `pub fn exposed(token: impl Into<String>) -> Self` (`require_token: true`).
  - `pub fn spawn_session(config: Config, providers: ProviderSlot, brain_config: BrainConfig, policy: SessionPolicy) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>)`
  - `pub fn spawn_with_events(config: Config, providers: ProviderSlot, brain_config: BrainConfig, policy: SessionPolicy) -> SessionEnds`
  - `pub struct SessionEnds { pub session_id: u64, pub to_server: UnboundedSender<ToServer>, pub from_server: UnboundedReceiver<ToClient>, pub events: UnboundedSender<ToClient> }` — one **new** field, `session_id`, which the front matter's contract does not have; Task 1.3 keys the `Hub` by it.
  - `Command::Hello` is answered with `Event::Welcome` or with exactly one `Event::Error { code: ErrorCode::ProtocolVersion | ErrorCode::TokenRequired | ErrorCode::TokenInvalid, message: <a sentence>, recoverable: false }`, and in the three refusal cases the session ends with **no** `Welcome` sent. **The `code` field is the branch a client makes; `message` is a sentence the HUD shows as-is** — v1 spec §4.2 says `Event::Error.message` is "already localized to the profile's language", so it may not carry a machine-readable code and no client may parse it.
- **Why the policy is a parameter and not read from `config.server.lan`.** `lan` is the *user's intent*, and the listener is the only thing that knows whether that intent survived the bind: a file with `bind = "0.0.0.0:7070"` and `lan = false` — hand-edited, or written before the Server tab existed — is exposed, and a session that asked `config.server.lan` would hand that listener's whole conversation over with no token. The fact originates in Task 1.4's `server::run` and travels here as an explicit argument: `SessionPolicy::local()` for the in-process client (`crates/tipsy-compadre/src/client.rs:119`), `SessionPolicy::exposed(config.server.token.clone())` for a listener that is not loopback. Task 1.4 is where the two are told apart; **this task must not decide it from a config value**, and a reviewer should read the missing `config.server.lan` as the point rather than as an omission.

- [ ] **Step 1: Write the failing test**

In `crates/tipsy-compadre/tests/session.rs`, change the two imports (`:19`, `:21`) and replace the `session()` helper (`:119-133`) with the policy-carrying pair below, so that all `14` existing tests keep working through `session()`:

```rust
use tipsy_compadre::session::{SessionPolicy, spawn_with_events};
```

```rust
use tipsy_protocol::{
    AssistantState, ClientKind, Command, ErrorCode, Event, Frame, ListeningMode, PROTOCOL_VERSION, ToClient,
    ToServer, UiLanguage,
};
```

```rust
/// A session on fake providers with the handshake policy the test needs, handing back its ends, its
/// session id and the transcribe fake.
///
/// `spawn_with_events` and not `spawn_session`: the tests that follow need the id, and
/// `spawn_session`'s return is the two channels and nothing else.
fn session_with_policy(
    chat: Arc<FakeChat>,
    speech: Arc<FakeSpeech>,
    transcript: &'static str,
    policy: SessionPolicy,
) -> (mpsc::UnboundedSender<ToServer>, mpsc::UnboundedReceiver<ToClient>, Arc<FakeTranscribe>, u64) {
    let transcribe = Arc::new(FakeTranscribe {
        text: transcript,
        languages: Mutex::new(Vec::new()),
        pcms: Mutex::new(Vec::new()),
    });
    let providers = ProviderSlot::new(Providers { chat, speech, transcribe: transcribe.clone() });
    let ends = spawn_with_events(config(), providers, BrainConfig::default(), policy);
    (ends.to_server, ends.from_server, transcribe, ends.session_id)
}

/// The same session over the in-process policy: the handshake path covers both transports, and the
/// in-process one needs no token (spec §4.1).
fn session(
    chat: Arc<FakeChat>,
    speech: Arc<FakeSpeech>,
    transcript: &'static str,
) -> (mpsc::UnboundedSender<ToServer>, mpsc::UnboundedReceiver<ToClient>, Arc<FakeTranscribe>) {
    let (tx, rx, transcribe, _) = session_with_policy(chat, speech, transcript, SessionPolicy::local());
    (tx, rx, transcribe)
}

/// The only message that may precede any other, and the one the whole handshake is judged from.
fn hello(token: Option<&str>, protocol_version: u32) -> Command {
    Command::Hello {
        token: token.map(str::to_string),
        client: ClientKind::Tui,
        protocol_version,
    }
}

/// Everything the session sends until it closes its channel, or `wait_ms` passes with it still open.
async fn until_closed(rx: &mut mpsc::UnboundedReceiver<ToClient>, wait_ms: u64) -> Vec<ToClient> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return out;
        }
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(message)) => out.push(message),
            // `Ok(None)`: the session dropped its sender, so it is over. `Err`: the deadline.
            _ => return out,
        }
    }
}
```

`spawn_session`'s return type is a two-element tuple and does not carry the id, so the helper above reads it from `SessionEnds` instead — which means this test file uses `spawn_with_events`:

```rust
fn session_with_policy(
    chat: Arc<FakeChat>,
    speech: Arc<FakeSpeech>,
    transcript: &'static str,
    policy: SessionPolicy,
) -> (mpsc::UnboundedSender<ToServer>, mpsc::UnboundedReceiver<ToClient>, Arc<FakeTranscribe>, u64) {
    let transcribe = Arc::new(FakeTranscribe {
        text: transcript,
        languages: Mutex::new(Vec::new()),
        pcms: Mutex::new(Vec::new()),
    });
    let providers = ProviderSlot::new(Providers { chat, speech, transcribe: transcribe.clone() });
    let ends = spawn_with_events(config(), providers, BrainConfig::default(), policy);
    (ends.to_server, ends.from_server, transcribe, ends.session_id)
}
```

with `use tipsy_compadre::session::{SessionPolicy, spawn_with_events};` on the import line above.

Then the four tests over the in-process channel (no socket), which are spec §4.1's table plus the acceptance path:

```rust
/// The next `Event::Error` as `(code, message, recoverable)`, panicking with everything received if
/// the session closes or the timeout expires first. The test asserts on `code` because `message` is
/// interface copy, already localized (v1 spec §4.2), and therefore not a thing a client may branch on.
async fn next_error(rx: &mut mpsc::UnboundedReceiver<ToClient>, wait_ms: u64) -> (ErrorCode, String, bool) {
    match next_matching(rx, |m| matches!(m, ToClient::Event(Event::Error { .. })), wait_ms).await {
        ToClient::Event(Event::Error { code, message, recoverable }) => (code, message, recoverable),
        other => panic!("expected an error event, got {other:?}"),
    }
}

/// The whole refusal clause of §4.1: the error event, no `Welcome` before it and none after, and a
/// session that has ended by the time the last message has been read.
#[tokio::test]
async fn a_hello_with_the_wrong_protocol_version_is_refused_before_the_welcome() {
    let (tx, mut rx, _, _) =
        session_with_policy(FakeChat::new(vec![]), FakeSpeech::new(), "", SessionPolicy::local());
    send(&tx, hello(None, PROTOCOL_VERSION + 1));

    let (code, message, recoverable) = next_error(&mut rx, 1000).await;
    assert_eq!((code, recoverable), (ErrorCode::ProtocolVersion, false));
    assert!(
        !message.contains("protocol_version"),
        "the code belongs in `code`, not spelled into the message: {message}"
    );
    assert!(message.split_whitespace().count() > 1, "the message is a sentence: {message}");

    // …and nothing was welcomed, before the error or after it.
    let seen = until_closed(&mut rx, 1000).await;
    assert!(
        !seen.iter().any(|m| matches!(m, ToClient::Event(Event::Welcome { .. }))),
        "a refused handshake is never welcomed: {seen:#?}"
    );
    assert!(tx.send(ToServer::Command(hello(None, PROTOCOL_VERSION))).is_err(), "the session closed");
}

#[tokio::test]
async fn a_token_the_server_requires_and_does_not_receive_is_refused() {
    let (tx, mut rx, _, _) =
        session_with_policy(FakeChat::new(vec![]), FakeSpeech::new(), "", SessionPolicy::exposed("s3cret"));
    send(&tx, hello(None, PROTOCOL_VERSION));

    let (code, message, recoverable) = next_error(&mut rx, 1000).await;
    assert_eq!((code, recoverable), (ErrorCode::TokenRequired, false));
    assert!(
        !message.contains("token_required") && !message.contains("s3cret"),
        "neither the code nor the token may appear in the message: {message}"
    );

    let seen = until_closed(&mut rx, 1000).await;
    assert!(!seen.iter().any(|m| matches!(m, ToClient::Event(Event::Welcome { .. }))), "{seen:#?}");
}

#[tokio::test]
async fn a_wrong_token_is_refused() {
    let (tx, mut rx, _, _) =
        session_with_policy(FakeChat::new(vec![]), FakeSpeech::new(), "", SessionPolicy::exposed("s3cret"));
    send(&tx, hello(Some("not-the-token"), PROTOCOL_VERSION));

    let (code, message, recoverable) = next_error(&mut rx, 1000).await;
    assert_eq!((code, recoverable), (ErrorCode::TokenInvalid, false));
    assert!(
        !message.contains("token_invalid") && !message.contains("not-the-token"),
        "the refused token is not echoed back to the client that sent it: {message}"
    );

    let seen = until_closed(&mut rx, 1000).await;
    assert!(!seen.iter().any(|m| matches!(m, ToClient::Event(Event::Welcome { .. }))), "{seen:#?}");
}

/// The acceptance path, and the row of §4.1's table that must not be over-refused: the right version
/// and the right token are served exactly as the in-process client is.
#[tokio::test]
async fn the_right_version_and_token_are_welcomed() {
    let (tx, mut rx, _, _) =
        session_with_policy(FakeChat::new(vec![]), FakeSpeech::new(), "", SessionPolicy::exposed("s3cret"));
    send(&tx, hello(Some("s3cret"), PROTOCOL_VERSION));

    let first = next_matching(&mut rx, |_: &ToClient| true, 500).await;
    assert!(
        matches!(first, ToClient::Event(Event::Welcome { .. })),
        "the accepted handshake is served: {first:?}"
    );
    // The greeting turn then runs, which is the proof that nothing about a valid handshake changed.
    next_matching(&mut rx, is_done, 500).await;
}
```

- [ ] **Step 2: Run the test to verify it fails**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre --test session
```

Expected: FAIL — the file does not compile, and the first error is

```
error[E0432]: unresolved import `tipsy_compadre::session::SessionPolicy`
  --> crates\tipsy-compadre\tests\session.rs
   |
   | use tipsy_compadre::session::{SessionPolicy, spawn_with_events};
   |                               ^^^^^^^^^^^^^ no `SessionPolicy` in `session`
```

with `error[E0061]: this function takes 3 arguments but 4 arguments were supplied` on the `spawn_with_events` call behind it. Nothing about `Command::Hello` has to be wrong for this to fail: the test cannot even state the policy it is testing with, which is the gap this task closes.

- [ ] **Step 3: Implement the policy, the id, and the handshake**

**3a.** In `crates/tipsy-compadre/src/session.rs`, replace the constant and its comment (`:22-23`) with the counter and the policy, and add `use std::sync::atomic::{AtomicU64, Ordering};` beside `use std::sync::Arc;` (`:7`):

```rust
/// The session ids this process has handed out. One session per connection means the id can no
/// longer be a constant (spec §5.2): two clients connected at once have to be told apart, and
/// `Welcome.session_id` is what tells them apart. The first session is `1`, so a single-client run
/// reports what it always did.
static SESSION_IDS: AtomicU64 = AtomicU64::new(0);

/// The next session id: unique within the process, and never reused.
fn next_session_id() -> u64 {
    SESSION_IDS.fetch_add(1, Ordering::Relaxed) + 1
}

/// What this session requires of a client before it serves one (spec §4.1).
///
/// **The server's exposure is not read from the configuration.** `config.server.lan` is the user's
/// intent, and the only thing that knows whether that intent survived the bind is the listener:
/// `bind = "0.0.0.0:7070"` with `lan = false` is exposed all the same. The listener therefore says
/// so explicitly — `SessionPolicy::local()` for the in-process client, `SessionPolicy::exposed(token)`
/// for anything that is not loopback — and this type carries the answer rather than re-deriving it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPolicy {
    /// True when the server is reachable from somewhere other than this machine's loopback.
    pub require_token: bool,
    /// The token a client must present. Empty when none has been generated; a server that requires a
    /// token and has none refuses every client rather than accepting every one.
    pub token: String,
    /// True when the connection reaches this server only over loopback. `GetConfig`'s token rule
    /// (spec §4.3, Task 1.3) and `SetKey`'s refusal (spec §9.4, Task 1.8) are both decided from it.
    pub loopback: bool,
}

impl SessionPolicy {
    /// A session reached over the in-process channel: no token, none asked for, and the token still
    /// crosses to it, because it is the client's own machine (spec §4.3).
    pub fn local() -> Self {
        Self { require_token: false, token: String::new(), loopback: true }
    }

    /// A session behind a listener that is not loopback.
    pub fn exposed(token: impl Into<String>) -> Self {
        Self { require_token: true, token: token.into(), loopback: false }
    }

    /// The one place "may this client be served?" is decided, so the version check, the missing token
    /// and the wrong token cannot drift apart. `Err` names the refusal with the `ErrorCode` that
    /// branches it on the wire (Task 1.1's three variants); the sentence a client shows is the
    /// session's own, because this type is not where an interface string belongs.
    ///
    /// A `Command` that does not parse is `ProtocolVersion` and not `BadMessage`: `Hello` is the only
    /// message allowed to be first, and a first message that is not `Hello` means the client is not
    /// speaking this protocol. `BadMessage` is for a later malformed frame on an open connection
    /// (spec §10), which is Task 1.6's path, not this one.
    fn admits(&self, hello: &Command) -> Result<(), ErrorCode> {
        let Command::Hello { token, protocol_version, .. } = hello else {
            return Err(ErrorCode::ProtocolVersion);
        };
        if *protocol_version != PROTOCOL_VERSION {
            return Err(ErrorCode::ProtocolVersion);
        }
        if !self.require_token {
            return Ok(());
        }
        match token.as_deref() {
            None => Err(ErrorCode::TokenRequired),
            Some(presented) if presented == self.token => Ok(()),
            Some(_) => Err(ErrorCode::TokenInvalid),
        }
    }
}
```

**3b.** `SessionEnds` (`:45-52`) gains the id; `spawn_session` (`:35-43`) and `spawn_with_events` (`:54-95`) gain the policy:

```rust
/// The four ends the client keeps, exactly as a session hands them over.
pub struct SessionEnds {
    /// The id this session reports in `Welcome.session_id`: unique per session in this process.
    pub session_id: u64,
    pub to_server: UnboundedSender<ToServer>,
    pub from_server: UnboundedReceiver<ToClient>,
    /// A second sender for the same client channel, for producers that live beside the session, such
    /// as the system monitor.
    pub events: UnboundedSender<ToClient>,
}

/// Starts a session on its own task and returns the two ends of its channels.
pub fn spawn_session(
    config: Config,
    providers: ProviderSlot,
    brain_config: BrainConfig,
    policy: SessionPolicy,
) -> (UnboundedSender<ToServer>, UnboundedReceiver<ToClient>) {
    let session = spawn_with_events(config, providers, brain_config, policy);
    (session.to_server, session.from_server)
}

/// Like [`spawn_session`], but also hands back the sender the session reports to, so a caller can put
/// other producers of [`ToClient`] on the same channel, and the session's own id.
pub fn spawn_with_events(
    config: Config,
    providers: ProviderSlot,
    brain_config: BrainConfig,
    policy: SessionPolicy,
) -> SessionEnds {
    let session_id = next_session_id();
    let (to_server_tx, to_server_rx) = mpsc::unbounded_channel();
    let (to_client_tx, to_client_rx) = mpsc::unbounded_channel();
    // Held so the client end only closes once the session task itself has ended.
    let keep_alive = to_client_tx.clone();

    let profile = config.active().clone();
    let listening_mode = config.listening.mode;
    let listening_silence_ms = config.listening.silence_ms;
    let brain = Arc::new(Brain::new(providers.clone(), turn_profile(&config), brain_config.clone()));

    let mut session = Session {
        session_id,
        policy,
        token_presented: false,
        config,
        brain,
        brain_config,
        providers,
        out: to_client_tx.clone(),
        history: Vec::new(),
        current: None,
        generation: 0,
        listening: false,
        mic: Vec::new(),
        vad: VadState::new(listening_silence_ms),
        heard: false,
        utterance: Vec::new(),
        guard: None,
        profile: profile.view(),
        ui_language: UiLanguage::from_bcp47(&profile.language),
        listening_mode,
        greeting: profile.greeting.clone(),
    };

    tokio::spawn(async move {
        session.run(to_server_rx).await;
        // The task ends with the session; the client end closes with it.
        drop(keep_alive);
    });

    SessionEnds { session_id, to_server: to_server_tx, from_server: to_client_rx, events: to_client_tx }
}
```

That literal is every field of the existing one (`:67-86`) with the three new names at the top, in the struct's own declaration order. `Session` (`:119-147`) therefore becomes

```rust
struct Session {
    /// What `Welcome` reports, and what Task 1.3's `Hub` keys this session by.
    session_id: u64,
    /// The handshake this session insists on before it serves anything.
    policy: SessionPolicy,
    /// Whether this client is entitled to the server's token in a `GetConfig` answer (spec §4.3):
    /// true for a loopback connection, and for one that presented the token. Recorded once, where the
    /// handshake is judged, because it is a fact about the handshake and not about the request.
    token_presented: bool,
    /// The configuration the session is running. Task 1.3's `UpdateConfig` replaces it and re-derives
    /// the brain from it, so a change takes effect on the next turn.
    config: Config,
    brain: Arc<Brain>,
    brain_config: BrainConfig,
    /// The live provider stack: read once per turn, swapped in place by a save.
    providers: ProviderSlot,
    out: UnboundedSender<ToClient>,
    history: Vec<ChatMessage>,
    current: Option<Turn>,
    generation: u64,
    /// True between `ListenStart` and `ListenStop`.
    listening: bool,
    /// Mono 16 kHz samples gathered while listening, in `push` mode.
    mic: Vec<i16>,
    /// The `auto` mode detector, paused while the assistant speaks and for 300 ms after.
    vad: VadState,
    /// True from the frame that reports `SpeechStarted` until the one that reports `SpeechEnded`.
    heard: bool,
    /// The audio the detector confirmed: what `SpeechEnded` transcribes.
    utterance: Vec<i16>,
    /// When the 300 ms guard after the assistant's speech expires, if it is running.
    guard: Option<Instant>,
    profile: ProfileView,
    ui_language: UiLanguage,
    listening_mode: ListeningMode,
    greeting: String,
}
```

Task 1.3 adds `hub: Hub` to this same struct.

`welcome()` (`:241-258`) reports the real id — `session_id: self.session_id,` in place of `session_id: SESSION_ID,` (`:245`). Leave `web_bundle: false` (`:249`) exactly as it is: Task 1.5 makes it truthful.

**3c.** The prelude. `run` (`:150-204`) gains the handshake before the first byte it says:

```rust
    async fn run(&mut self, mut rx: UnboundedReceiver<ToServer>) {
        // The handshake is judged before the first byte the session says. `Hello` is sent by every
        // client as its first message (spec §4.1), so "refused, then closed" is exactly this: no
        // `Welcome`, one `Error`, and a channel that closes when this function returns. Judging it
        // here rather than in `command` is what removes the race between a refusal and the greeting
        // turn — a `Welcome` already sent could not be taken back, and the three refusals would be
        // indistinguishable from a session that was served and then died.
        let first = match rx.recv().await {
            Some(message) => message,
            // The client vanished before saying anything: nothing to serve and nobody to tell.
            None => return,
        };
        self.token_presented = self.policy.loopback
            || matches!(&first, ToServer::Command(hello @ Command::Hello { .. }) if self.policy.admits(hello).is_ok());
        if let ToServer::Command(hello @ Command::Hello { .. }) = &first
            && let Err(code) = self.policy.admits(hello)
        {
            self.refuse(code);
            return;
        }

        self.welcome();
        // The greeting is an ordinary turn: while it plays, `Cancel` still works.
        let greeting = self.greeting.clone();
        self.current = Some(self.start_turn(TurnInput::Say(greeting)));

        loop {
            // The guard's deadline is read once per iteration (`:162`), and the three branches are the
            // three that already exist (`:163-197`): the running turn's result, the message from the
            // client — replaced by the branch below, which now also breaks the loop on a refusal — and
            // the guard's deadline. Nothing else about the loop changes.
            let guard = self.guard;
            tokio::select! {
                biased;
                result = async {
                    match self.current.as_mut() {
                        Some(turn) => (&mut turn.done).await,
                        None => std::future::pending().await,
                    }
                } => {
                    let history = result.ok();
                    if let Some(turn) = self.current.take() {
                        self.end_turn(turn, history);
                    }
                }
                message = rx.recv() => {
                    let Some(message) = message else {
                        break;
                    };
                    if self.handle(message, &mut rx).is_break() {
                        break;
                    }
                    // A turn that ended is settled here, where the borrow the `select` held is over. A
                    // turn the command just started has no history yet and is left alone.
                    self.settle_finished_turn().await;
                }
                () = async {
                    match guard {
                        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.guard_elapsed();
                }
            }
        }
        // Nothing may keep speaking after the client hung up.
        self.cancel_running();
    }
```

The three comment blocks that explain the guard's deadline, the biased poll order and the guard branch move with the code unchanged; the only edit inside the `select!` is the message branch.

`SessionPolicy` therefore also needs the one field §4.3 and Task 1.8 both read — a **third** `pub` field, added here and pinned in `Produces` above:

```rust
    /// True when the connection reaches this server only over loopback. `GetConfig`'s token rule
    /// (spec §4.3) and Task 1.8's `SetKey` rule (spec §9.4) are both decided from it.
    pub loopback: bool,
```

with `local()` setting `loopback: true` and `exposed(..)` setting `loopback: false`.

The `self.settle_finished_turn().await;` that used to sit after the `select!` (`:198-200`) is **deleted** in the same edit: the message branch above is the only branch whose command can have started or ended a turn, the turn-result branch already settles its own, and the guard branch has no turn to settle.

**3d.** `handle` (`:263-272`), `dispatch` (`:274-293`) and `command` (`:343-363`) return `ControlFlow<()>`, so a refusal can end the session from anywhere a command is dispatched:

```rust
    /// Handles one message and everything already queued behind it, so a `ListenStop` never leaves the
    /// tail of an utterance waiting behind it. `Break` means the session is over: a `Hello` refused
    /// here was refused at a message that is not the first one.
    fn handle(&mut self, first: ToServer, rx: &mut UnboundedReceiver<ToServer>) -> ControlFlow<()> {
        let mut message = first;
        loop {
            let flow = self.dispatch(message);
            if flow.is_break() {
                return flow;
            }
            match rx.try_recv() {
                Ok(next) => message = next,
                Err(_) => return ControlFlow::Continue(()),
            }
        }
    }

    fn dispatch(&mut self, message: ToServer) -> ControlFlow<()> {
        match message {
            ToServer::Command(command) => self.command(command),
            // This arm's body is `:277-290` verbatim — the `!self.listening` early return, the `Auto`
            // arm calling `self.detect(&samples)` and the `Push | HandsFree` arm extending
            // `self.mic` — with `ControlFlow::Continue(())` as the last expression instead of the
            // implicit `()`.
            ToServer::Audio(Frame::Mic(samples)) => {
                if !self.listening {
                    return ControlFlow::Continue(());
                }
                match self.listening_mode {
                    ListeningMode::Auto => self.detect(&samples),
                    ListeningMode::Push | ListeningMode::HandsFree => self.mic.extend_from_slice(&samples),
                }
                ControlFlow::Continue(())
            }
            ToServer::Audio(Frame::Speech { .. }) => ControlFlow::Continue(()),
        }
    }
```

`command` gives each existing arm the flow it already had and replaces `Command::Hello { .. } => {}` (`:353`) with the real arm:

```rust
            // The first `Hello` was judged in `run`, before the `Welcome`; this arm is both the
            // idempotent second pass `handle` makes for a queued message and the one place a *late*
            // `Hello` — after the handshake, with the channel still open — is refused. A client that
            // got this far was already admitted, so a failure here means a second hello with
            // different credentials: the session ends rather than changing its mind.
            Command::Hello { token, client, protocol_version } => {
                let hello = Command::Hello { token, client, protocol_version };
                match self.policy.admits(&hello) {
                    Ok(()) => ControlFlow::Continue(()),
                    Err(code) => {
                        self.refuse(code);
                        ControlFlow::Break(())
                    }
                }
            }
```

`Session::refuse` is the one place a `Hello` refusal becomes an event, so the prelude and this arm cannot describe the same refusal two ways. `ErrorCode` is the branch; the sentence is interface copy, and interface copy lives in the two static tables:

```rust
    /// Answers a refused handshake: the `code` a client branches on, and a sentence it shows as-is.
    ///
    /// The message is deliberately **not** the code spelled out. v1 spec §4.2 makes
    /// `Event::Error.message` "already localized to the profile's language", so no client may parse
    /// it; the machine-readable half is `code`. Both halves of that rule are load-bearing: a client
    /// that branched on this string would break the day somebody translated it.
    fn refuse(&self, code: ErrorCode) {
        let strings = tipsy_ui::strings(self.ui_language);
        let message = match code {
            ErrorCode::ProtocolVersion => strings.err_protocol_version,
            ErrorCode::TokenRequired => strings.err_token_required,
            ErrorCode::TokenInvalid => strings.err_token_invalid,
            // `refuse` is called from the handshake only, where those three are the only endings; the
            // fallback is what keeps it total rather than pretending a fourth case cannot be reached.
            _ => strings.err_handshake,
        };
        send(
            &self.out,
            ToClient::Event(Event::Error { code, message: message.to_string(), recoverable: false }),
        );
    }
```

That needs three more `&'static str` fields on `Strings` (`crates/tipsy-ui/src/strings.rs:80-98`, the struct the two tables at `:99-135` and `:137-...` populate), and one more for the fallback:

| Field | `ES` (`strings.rs:99-135`) | `EN` (`strings.rs:137+`) |
|---|---|---|
| `err_protocol_version` | `"Esta versión del cliente no habla el protocolo {PROTOCOL_VERSION} del servidor."` | `"This client's version does not speak this server's protocol {PROTOCOL_VERSION}."` |
| `err_token_required` | `"Este servidor está expuesto y pide un token: pásalo con --token."` | `"This server is exposed and asks for a token: pass it with --token."` |
| `err_token_invalid` | `"Ese no es el token de este servidor."` | `"That is not this server's token."` |
| `err_handshake` | `"No se pudo completar el saludo con el servidor."` | `"The handshake with the server could not be completed."` |

A `&'static str` cannot interpolate `PROTOCOL_VERSION`, so write the first one without the number — `"…no habla el protocolo de este servidor."` / `"…does not speak this server's protocol."` — and leave the version itself to the log line, which is Task 1.9's. `tipsy_ui::strings` and `Strings` are both public (`crates/tipsy-ui/src/lib.rs:12`), and `tipsy-compadre` already depends on `tipsy-ui`, so this needs no new dependency. These four strings are the only ones this plan adds to the tables, and they are the user's copy to approve like every other line in them.

Every other arm of that match keeps its body and returns `ControlFlow::Continue(())`. Imports to add at `session.rs:10`: `ClientKind`, `ErrorCode` and `PROTOCOL_VERSION` to the `tipsy_protocol` list, plus `use std::ops::ControlFlow;`.

**3e.** Every caller. There are exactly seven `spawn_session`/`spawn_with_events` call sites in the workspace, and all seven are here:

| Caller | Location | What it gets |
|---|---|---|
| the in-process client | `crates/tipsy-compadre/src/client.rs:119` | `SessionPolicy::local()` |
| the test harness | `crates/tipsy-compadre/tests/session.rs:131` (inside `session()`, `:120-133`) | `SessionPolicy::local()` |
| a rebuilt-provider test | `crates/tipsy-compadre/tests/session.rs:360` | `SessionPolicy::local()` |
| the listening-mode test | `crates/tipsy-compadre/tests/session.rs:391` | `SessionPolicy::local()` |
| the profile-switch test | `crates/tipsy-compadre/tests/session.rs:421` | `SessionPolicy::local()` |
| the `auto`-mode harness | `crates/tipsy-compadre/tests/session.rs:468` (inside `auto_session()`, `:456-475`) | `SessionPolicy::local()` |
| the definition | `crates/tipsy-compadre/src/session.rs:41` | forwarded to `spawn_with_events` |

The four direct calls at `tests/session.rs:360`, `:391`, `:421` and `:468` each need `SessionPolicy::local()` appended to the argument list — mechanical, and their `let (tx, mut rx) = …` destructuring does not change, because `spawn_session` keeps returning its two-element tuple. `client.rs:119` becomes

```rust
        let ends = spawn_with_events(
            self.config.clone(),
            self.providers.clone(),
            BrainConfig::default(),
            SessionPolicy::local(),
        );
```

with `use crate::session::{SessionPolicy, spawn_with_events};` replacing `use crate::session::spawn_with_events;` at `client.rs:27`.

**3f.** Nothing else changes shape. The `Welcome` assertion at `tests/session.rs:208-212` destructures with `..` and keeps compiling; the `session_id` it never reads is now real, and Task 1.3's test is the one that checks two sessions get different ones.

- [ ] **Step 4: Run the test to verify it passes**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre --test session
```

Expected: PASS — `14` tests before this task's `4`, `18` after, `0` failed. The `14` that were already there must be green with the policy threaded through, because the in-process policy admits everything: if one of them fails, the prelude is refusing a client it should serve.

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre
```

Expected: PASS — the whole `tipsy-compadre` suite (`254` tests before this task), `0` failed. The only change that reaches code this task's `Files` block does not name is `SessionEnds`' new `session_id` field, and there is no such code: `SessionEnds` is constructed at `session.rs:94` and read by field at `client.rs:120-125`, both listed above.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/session.rs crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/tests/session.rs
git commit -m "feat(session): judge the handshake before serving a client

Hello becomes real: every client sends it first, and the session validates the
protocol version and the token before its Welcome, answering
Error { code: config, message: protocol_version|token_required|token_invalid,
recoverable: false } and ending without a Welcome. The 'is this server
exposed' fact arrives as an explicit SessionPolicy, because only the listener
knows it: a bind on 0.0.0.0 with lan = false is exposed all the same, so it is
never read from config.server.lan. spawn_session and spawn_with_events gain
the policy parameter, SessionEnds gains the session id, and the SESSION_ID
constant becomes an atomic counter. Task 1.3 keys its Hub by that id.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 1.3: The `Hub`, unique sessions and the broadcasts `[mid]`

**Files:**
- Create: `crates/tipsy-compadre/src/server/mod.rs` and `crates/tipsy-compadre/src/server/hub.rs`
- Modify: `crates/tipsy-compadre/src/lib.rs:19` — `pub mod server;` in the module list
- Modify: `crates/tipsy-compadre/src/session.rs` — the five commands' real handling in `fn command` (`:343-363`), the fifth constructor parameter, `set_profile` (`:371-385`) deleted and replaced by `reconfigure`, `set_listening_mode` (`:389-392`, unchanged), `welcome` (`:241-258`, unchanged)
- Modify: `crates/tipsy-compadre/src/client.rs:119` — the in-process caller builds a `Hub` and calls `Hub::spawn`
- Test: `crates/tipsy-compadre/src/server/hub.rs` (its own `#[cfg(test)] mod tests`); `crates/tipsy-compadre/tests/session.rs` (Task 1.2's harness)

**Interfaces:**
- Consumes: `SessionPolicy`, `SessionEnds { session_id, … }`, `spawn_with_events(config, providers, brain_config, policy)` — Task 1.2; `Command::{GetConfig, UpdateConfig, SetKey}` and `Event::{Config, ProfileChanged, ConfigChanged}` and `ConfigView`/`ConfigView::redacted()` — Task 1.1; `view_of`, `config_of` — Task 1.1 Step 3e; `config_from` (`crates/tipsy-compadre/src/config/form.rs:81`); `store::save` (`config/store.rs:20-26`); `build_providers(&ProvidersConfig, &KeyResolver) -> Result<Providers, BuildError>` (`providers/mod.rs:251`), `ProviderSlot::{new, replace}` (`:170`, `:175`); `KeyResolver` (`keys.rs:79-97`) and its `store()`/`resolve(name)` (`:89`, `:97`); `BrainConfig` (`brain.rs:20-26`).
- Produces — pinned here, and no other task may invent a variant of these:
  - `pub struct Hub` in `crates/tipsy-compadre::server::hub`, re-exported as `tipsy_compadre::server::Hub`. Five handle-backed fields and a `#[derive(Clone)]`, which is what lets a session hold one while the connection that started it holds another. **`ProviderSlot` is one of the five, and it is the server's single stack** — Step 3c says why.
  - `impl Hub { pub fn new(config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot) -> Hub }`
  - `impl Hub { pub fn path(&self) -> &Path }`, `impl Hub { pub fn config(&self) -> Config }` and `impl Hub { pub fn providers(&self) -> ProviderSlot }` — the last is the slot's clone accessor, which the in-process client and the tests both need.
  - `impl Hub { pub fn config_view(&self, include_token: bool) -> ConfigView }` — §4.3's rule is this one `bool`, decided by the caller that knows who is asking.
  - `impl Hub { pub fn attach(&self, session_id: u64, to_server: UnboundedSender<ToServer>, events: UnboundedSender<ToClient>) }` — two senders, not one: a change the *session* has to apply travels on `to_server`, and `events` is the name `SessionEnds` gives the client channel (see Step 3c). `impl Hub { pub fn detach(&self, session_id: u64) }`
  - `impl Hub { pub fn broadcast(&self, message: ToClient) -> usize }` — the count of sessions that took it: what the tests assert on, and what a log line may say.
  - `impl Hub { pub fn session_ids(&self) -> Vec<u64> }` — ascending.
  - `impl Hub { pub fn resend_welcome(&self) }`
  - `impl Hub { pub fn reconfigure_all(&self) -> bool }` — **rebuilds the connectors through `build_providers` and `replace`es them in the shared slot, then hands `Command::UpdateConfig` to every session so each re-derives its own `Brain`**; `false` when the connectors cannot be built, in which case the running stack is left alone. Two halves, because a change has two: the connectors are shared and the `Brain` is per session.
  - `impl Hub { pub fn apply_config(&self, config: Config) -> bool }` — persist, remember, re-derive every session; `true` when the file was written. The one place a change becomes durable. The connector build is `reconfigure_all`'s and its outcome is not folded into this answer.
  - `impl Hub { pub fn update_config(&self, config: Config) -> usize }`, `impl Hub { pub fn update_config_from_view(&self, view: ConfigView) -> usize }`
  - `impl Hub { pub fn set_profile(&self, id: &str) -> usize }`, `impl Hub { pub fn set_listening_mode(&self, mode: ListeningMode) -> usize }`, `impl Hub { pub fn set_key(&self, name: &str, value: &str) -> usize }`
  - `impl Hub { pub fn spawn(&self, policy: SessionPolicy) -> SessionEnds }` — **no `config`, no `brain_config`, no `providers`**: the hub holds all three (the `BrainConfig` is a `Default` it owns, `brain.rs:28-37`), so the policy is the only per-connection fact a caller passes. The session gets a clone of the hub's own slot, which is the point of the ruling.
  - `pub fn spawn_with_events(config: Config, providers: ProviderSlot, brain_config: BrainConfig, policy: SessionPolicy, hub: Hub) -> SessionEnds` — one more parameter than Task 1.2 pinned, and the reason is in Step 3d; `spawn_session` gains the same one and forwards it.
  - `Hub` needs no `Default` and no `Debug`: it is cloned into every session `Hub::spawn` starts (five shared handles), and nothing prints it.

**Where each rule lives, so a reviewer does not read a missing guard as an oversight.** `SetKey`'s loopback-or-TLS refusal is **Task 1.8's** (spec §9.4, the front matter's constraint, and the plan index all say so). What this task builds is the point it plugs into: `SessionPolicy.loopback` (Task 1.2) is the fact, and `Session`'s `Command::SetKey` arm is the one call site — Step 3f writes it in the shape it will have. **Add no refusal here.** `GetConfig`'s token rule *is* this task's, because nothing else can answer "did this client present the token": the session knows, and it says so with one `bool` into `config_view`.

**Where the two halves of "re-derive" live.** The **connectors** are the hub's, because there is one `ProviderSlot` for the server and `replace` reaches every `Brain` at once. The **`Brain`** is each session's, because it is built from the active profile's prompt and voice, and a change is a different configuration per session only if the profile differs — which it does not, but the object is still per session. `Command::UpdateConfig` is therefore still the message, and `Session::reconfigure` is still what applies it; what changes is that the *providers* are no longer that command's business. `Arc<Mutex<Config>>` appears in this plan **once**, inside the `Hub`, and `ProviderSlot`'s own `Arc<RwLock<…>>` is the second shared value.

- [ ] **Step 1: Write the failing test**

Two files, because each technique exists in exactly one of them. The registry's own behaviour goes in `hub.rs`'s `#[cfg(test)] mod tests`; everything that needs a running session goes in `tests/session.rs`, whose harness Task 1.2 built.

In `crates/tipsy-compadre/src/server/hub.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // The three fakes at the end of this module answer nothing, so these are only ever used to write
    // their signatures down. `crate::audio::pcm::Pcm` is the type `SpeechProvider` returns.
    use crate::audio::pcm::Pcm;
    use crate::providers::{
        ChatProvider, ChatRequest, ProviderError, SpeechProvider, SpeechRequest, TextStream, TranscribeProvider,
    };

    /// A hub with no sessions, and a configuration file in a tempdir that nothing has read.
    fn hub() -> Hub {
        let dir = tempfile::tempdir().unwrap();
        Hub::new(
            Config::default(),
            dir.path().join("config.toml"),
            KeyResolver::new(crate::keys::MemoryStore::default(), |_| None),
            silent(),
        )
    }

    /// One session that is not running: both of its ends, and both of their receivers. `reconfigure_all`
    /// talks to the *session*, so a test that watched only the client end would see nothing.
    fn ends() -> (
        UnboundedSender<ToServer>,
        UnboundedSender<ToClient>,
        mpsc::UnboundedReceiver<ToClient>,
        mpsc::UnboundedReceiver<ToServer>,
    ) {
        let (to_server, from_connection) = mpsc::unbounded_channel();
        // The client end is called `events` here for the same reason `Hub::attach` names it that: it is
        // the channel every producer of `ToClient` writes to.
        let (events, from_server) = mpsc::unbounded_channel();
        (to_server, events, from_server, from_connection)
    }

    /// Three connectors that answer nothing, so a broadcast's own delivery is what is under test.
    /// `build_providers` never runs in these tests — the slot is handed to the hub already built —
    /// which is why a fake with no key behind it is enough.
    fn silent() -> ProviderSlot {
        ProviderSlot::new(Providers {
            chat: Arc::new(SilentChat),
            speech: Arc::new(SilentSpeech),
            transcribe: Arc::new(SilentTranscribe),
        })
    }

    #[test]
    fn a_detached_session_is_not_broadcast_to() {
        let hub = hub();
        let (first_to_server, first_to_client, mut first_rx, _) = ends();
        let (second_to_server, second_to_client, mut second_rx, _) = ends();
        hub.attach(1, first_to_server, first_to_client);
        hub.attach(2, second_to_server, second_to_client);

        assert_eq!(hub.broadcast(ToClient::Event(Event::ConfigChanged)), 2);
        assert_eq!(first_rx.try_recv().unwrap(), ToClient::Event(Event::ConfigChanged));
        assert_eq!(second_rx.try_recv().unwrap(), ToClient::Event(Event::ConfigChanged));

        hub.detach(1);

        assert_eq!(hub.broadcast(ToClient::Event(Event::ConfigChanged)), 1);
        assert!(first_rx.try_recv().is_err(), "a detached session must not be written to");
        assert_eq!(second_rx.try_recv().unwrap(), ToClient::Event(Event::ConfigChanged));
        assert_eq!(hub.session_ids(), vec![2]);
    }

    /// A session whose connection died without detaching: `broadcast` counts what it delivered, and a
    /// closed channel is not one of those.
    #[test]
    fn a_session_whose_channel_is_closed_is_not_counted() {
        let hub = hub();
        let (live_to_server, live_to_client, mut live_rx, _) = ends();
        let (dead_to_server, dead_to_client, _dead_rx, _) = ends();
        hub.attach(1, live_to_server, live_to_client);
        hub.attach(2, dead_to_server, dead_to_client);
        // The dead session is the one whose client end nobody holds any more — the shape of a socket
        // that went away before `detach` ran.
        drop(dead_to_client);

        assert_eq!(hub.broadcast(ToClient::Event(Event::ConfigChanged)), 1);
        assert_eq!(live_rx.try_recv().unwrap(), ToClient::Event(Event::ConfigChanged));
    }

    /// Attaching the same id twice replaces the ends: one session per id, which is what the id is for.
    #[test]
    fn attaching_an_id_twice_leaves_one_session() {
        let hub = hub();
        let (first_to_server, first_to_client, mut first_rx, _) = ends();
        let (second_to_server, second_to_client, mut second_rx, _) = ends();
        hub.attach(7, first_to_server, first_to_client);
        hub.attach(7, second_to_server, second_to_client);

        assert_eq!(hub.broadcast(ToClient::Event(Event::ConfigChanged)), 1);
        assert!(first_rx.try_recv().is_err(), "the replaced session is gone");
        assert_eq!(second_rx.try_recv().unwrap(), ToClient::Event(Event::ConfigChanged));
        assert_eq!(hub.session_ids(), vec![7]);
    }

    /// The ids come in order, so a broadcast's iteration order cannot be what makes a test flaky.
    #[test]
    fn session_ids_are_ascending() {
        let hub = hub();
        let (a_to_server, a_to_client, _, _) = ends();
        let (b_to_server, b_to_client, _, _) = ends();
        hub.attach(11, a_to_server, a_to_client);
        hub.attach(2, b_to_server, b_to_client);
        assert_eq!(hub.session_ids(), vec![2, 11]);
    }

    /// `apply_config` writes the file and remembers the value — and hands `Command::UpdateConfig` to
    /// every session, which is the command and not an event: it is the session's own brain that is
    /// re-derived, and the client-facing `ConfigChanged` is a separate decision (spec §5.2).
    #[test]
    fn applying_a_configuration_persists_it_and_commands_every_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let hub = Hub::new(
            Config::default(),
            path.clone(),
            KeyResolver::new(crate::keys::MemoryStore::default(), |_| None),
            silent(),
        );
        // The sessions' own ends, so the test can read what the hub sent them.
        let (first_to_server, first_to_client, _, mut first_commands) = ends();
        let (second_to_server, second_to_client, _, mut second_commands) = ends();
        hub.attach(1, first_to_server, first_to_client);
        hub.attach(2, second_to_server, second_to_client);

        let mut edited = Config::default();
        edited.server.bind = "127.0.0.1:7171".into();
        assert!(hub.apply_config(edited.clone()), "the file must be written");

        for rx in [&mut first_commands, &mut second_commands] {
            match rx.try_recv().unwrap() {
                ToServer::Command(Command::UpdateConfig { config }) => {
                    assert_eq!(config.server.bind, "127.0.0.1:7171");
                    assert_eq!(config.listening.mode, ListeningMode::Push);
                }
                other => panic!("a configuration change is a command to the session, got {other:?}"),
            }
        }
        assert_eq!(store::load_or_create(&path).unwrap().config.server.bind, "127.0.0.1:7171");
        assert_eq!(hub.config().server.bind, "127.0.0.1:7171");
    }

    /// A write that fails must not leave the hub believing a configuration nobody can restart from.
    #[test]
    fn a_configuration_that_cannot_be_written_is_not_remembered() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be: `store::save`'s write fails, and that is the point.
        let path = dir.path().join("config.toml");
        std::fs::create_dir(&path).unwrap();
        let hub = Hub::new(
            Config::default(),
            path,
            KeyResolver::new(crate::keys::MemoryStore::default(), |_| None),
            silent(),
        );

        let mut edited = Config::default();
        edited.server.bind = "127.0.0.1:7171".into();
        assert!(!hub.apply_config(edited), "a write that failed is not a change");
        assert_eq!(hub.config().server.bind, "127.0.0.1:7070", "the old value stands");
    }

    /// §4.3's rule in its smallest form: the one `bool` is the whole of it.
    #[test]
    fn the_token_leaves_the_view_only_when_the_caller_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.server.token = "s3cret".into();
        let hub = Hub::new(
            config,
            dir.path().join("config.toml"),
            KeyResolver::new(crate::keys::MemoryStore::default(), |_| None),
            silent(),
        );

        assert_eq!(hub.config_view(true).server.token, "s3cret");
        assert_eq!(hub.config_view(false).server.token, "");
        assert_eq!(hub.config_view(false).server.bind, "127.0.0.1:7070", "only the token goes");
    }
}
```

`MemoryStore` is `crate::keys::MemoryStore`, the same store `tests/session.rs`'s own `resolver()` builds (`:42-46`); `UnboundedSender`, `UnboundedReceiver`, `mpsc`, `Command`, `ToServer`, `ListeningMode`, `KeyResolver` and `Config::default()` all reach the tests through `use super::*;` and the module imports of Step 3c.

The three fakes `silent()` builds are written out in full at the end of the same `mod tests`, after the last test. They answer nothing because no test in this file talks to a provider: the point of every one of them is who was *told* something. The signatures are the three traits at `providers/mod.rs:132-151`:

```rust
    /// The fake chat connector: it always fails, and no test here reaches the failure.
    struct SilentChat;

    #[async_trait::async_trait]
    impl ChatProvider for SilentChat {
        async fn stream(&self, _req: ChatRequest) -> Result<TextStream, ProviderError> {
            Err(ProviderError::BadResponse("no provider in this test".into()))
        }
    }

    /// The fake speech connector, the same way. `max_parallel` is the trait's own default.
    struct SilentSpeech;

    #[async_trait::async_trait]
    impl SpeechProvider for SilentSpeech {
        async fn synthesize(&self, _req: SpeechRequest) -> Result<Pcm, ProviderError> {
            Err(ProviderError::BadResponse("no provider in this test".into()))
        }
    }

    /// The fake transcriber, the same way.
    struct SilentTranscribe;

    #[async_trait::async_trait]
    impl TranscribeProvider for SilentTranscribe {
        async fn transcribe(&self, _pcm: Pcm, _language: &str) -> Result<String, ProviderError> {
            Err(ProviderError::BadResponse("no provider in this test".into()))
        }
    }
```

`async_trait::async_trait` is written in full rather than imported, because `hub.rs` has no production use for it; `ChatRequest`, `SpeechRequest` and `Pcm` come from `crate::providers` and `crate::audio::pcm`, which the test module imports itself:

In `crates/tipsy-compadre/tests/session.rs`, after Task 1.2's handshake tests:

```rust
/// A hub the tests drive directly: no listener, one configuration file in a tempdir, the server's one
/// provider stack, and the key store the stack's fakes never consult.
fn hub(config: &Config, path: &std::path::Path, providers: ProviderSlot) -> Hub {
    Hub::new(config.clone(), path.to_path_buf(), KeyResolver::new(MemoryStore::default(), |_| None), providers)
}

/// A session on the hub, attached exactly as a connection will be: the ends first, then the registry.
/// The provider stack is the hub's, which is why this takes none.
fn session_attached(
    hub: &Hub,
    policy: SessionPolicy,
) -> (mpsc::UnboundedSender<ToServer>, mpsc::UnboundedReceiver<ToClient>, u64) {
    let ends = hub.spawn(policy);
    hub.attach(ends.session_id, ends.to_server.clone(), ends.events.clone());
    (ends.to_server, ends.from_server, ends.session_id)
}

/// A loaded configuration in a tempdir, the way every test in this file gets one (`:114-117`).
fn config_in(dir: &tempfile::TempDir) -> (Config, std::path::PathBuf) {
    let path = dir.path().join("config.toml");
    let config = store::load_or_create(&path).unwrap().config;
    (config, path)
}

/// A provider stack that answers nothing: these tests are about who is told what.
fn silent() -> ProviderSlot {
    ProviderSlot::new(Providers {
        chat: FakeChat::new(vec![]),
        speech: FakeSpeech::new(),
        transcribe: Arc::new(FakeTranscribe {
            text: "",
            languages: Mutex::new(Vec::new()),
            pcms: Mutex::new(Vec::new()),
        }),
    })
}

/// Spec §5.2 and success criterion 3: two clients connected at once have separate ids and separate
/// histories. The session harness (`:27-105`, `:120-124`) is the precedent.
#[tokio::test]
async fn two_sessions_get_distinct_ids_and_separate_histories() {
    let dir = tempfile::tempdir().unwrap();
    let (config, path) = config_in(&dir);
    let chat = FakeChat::new(vec![Ok(vec![S1]), Ok(vec![S2])]);
    // The hub's own stack, so the chat fake's two recorded requests are the two sessions' own — which
    // is what makes "separate histories" checkable rather than merely plausible.
    let slot = ProviderSlot::new(Providers {
        chat: chat.clone(),
        speech: FakeSpeech::new(),
        transcribe: Arc::new(FakeTranscribe {
            text: "",
            languages: Mutex::new(Vec::new()),
            pcms: Mutex::new(Vec::new()),
        }),
    });
    let hub = hub(&config, &path, slot);

    let (first_tx, mut first_rx, first_id) = session_attached(&hub, SessionPolicy::local());
    let (second_tx, mut second_rx, second_id) = session_attached(&hub, SessionPolicy::local());
    assert_ne!(first_id, second_id, "one session per connection means one id per connection");
    assert_eq!(hub.session_ids(), vec![first_id, second_id]);

    next_matching(&mut first_rx, is_done, 500).await; // the greeting
    next_matching(&mut second_rx, is_done, 500).await; // the greeting

    send(&first_tx, Command::Ask { text: "primera pregunta".into() });
    next_matching(&mut first_rx, is_done, 500).await;

    send(&second_tx, Command::Ask { text: "segunda pregunta".into() });
    next_matching(&mut second_rx, is_done, 500).await;

    let requests = chat.requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "one chat call per session: {requests:#?}");
    let second = requests[1].messages.clone();
    assert!(
        !second.iter().any(|m| m.content.contains("primera pregunta") || m.content == S1),
        "a session's history is its own: {second:#?}"
    );
    assert_eq!(second.last().unwrap(), &ChatMessage::user("segunda pregunta"));
}

/// A profile change reaches **both** sessions (spec §5.2), as `Event::ProfileChanged` carrying the
/// same three fields `Welcome` carries — and the ids the sessions report do not change with it.
#[tokio::test]
async fn a_profile_change_reaches_both_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let (config, path) = config_in(&dir);
    let hub = hub(&config, &path, silent());

    let (_first_tx, mut first_rx, first_id) = session_attached(&hub, SessionPolicy::local());
    let (_second_tx, mut second_rx, second_id) = session_attached(&hub, SessionPolicy::local());
    next_matching(&mut first_rx, is_done, 500).await;
    next_matching(&mut second_rx, is_done, 500).await;

    assert_eq!(hub.set_profile("senyor"), 2, "both sessions take the change");

    for rx in [&mut first_rx, &mut second_rx] {
        match next_matching(rx, |m| matches!(m, ToClient::Event(Event::ProfileChanged { .. })), 500).await {
            ToClient::Event(Event::ProfileChanged { profile }) => {
                assert_eq!(profile.id, "senyor");
                assert_eq!(profile.language, "es-ES");
            }
            other => panic!("a profile change is announced with ProfileChanged, got {other:?}"),
        }
    }
    assert!(first_id < second_id);
    assert_eq!(store::load_or_create(&path).unwrap().config.active_profile, "senyor");
}

/// `UpdateConfig` persists and broadcasts (spec §5.2), and the **next** session to start reads what
/// was written — the `tempfile::tempdir()` pattern of `config/store.rs`'s own tests (`:87-97`).
#[tokio::test]
async fn an_updated_configuration_is_persisted_broadcast_and_read_by_the_next_session() {
    let dir = tempfile::tempdir().unwrap();
    let (config, path) = config_in(&dir);
    let hub = hub(&config, &path, silent());

    let (_tx, mut rx, _id) = session_attached(&hub, SessionPolicy::local());
    next_matching(&mut rx, is_done, 500).await;

    let mut edited = config.clone();
    edited.listening.mode = ListeningMode::Auto;
    edited.server.bind = "127.0.0.1:7171".into();
    assert_eq!(hub.update_config(edited.clone()), 2, "persist, re-derive every session, then signal");

    next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::ConfigChanged)), 500).await;

    // The file, read back the way the next run reads it — a fresh `load_or_create`, not the hub's own
    // copy, which is what makes this a persistence test rather than an echo.
    let back = store::load_or_create(&path).unwrap().config;
    assert_eq!(back.server.bind, "127.0.0.1:7171");
    assert_eq!(back.listening.mode, ListeningMode::Auto);

    // And a hub built from that file, with a session on it, answers `GetConfig` with what was written.
    let restarted = hub(&back, &path, silent());
    let (late_tx, mut late_rx, _late_id) = {
        let ends = restarted.spawn(SessionPolicy::local());
        restarted.attach(ends.session_id, ends.to_server.clone(), ends.events.clone());
        (ends.to_server, ends.from_server, ends.session_id)
    };
    next_matching(&mut late_rx, is_done, 500).await;
    send(&late_tx, Command::GetConfig);
    match next_matching(&mut late_rx, |m| matches!(m, ToClient::Event(Event::Config { .. })), 500).await {
        ToClient::Event(Event::Config { config }) => {
            assert_eq!(config.server.bind, "127.0.0.1:7171");
            assert_eq!(config.listening.mode, ListeningMode::Auto);
        }
        other => panic!("GetConfig must be answered with Event::Config, got {other:?}"),
    }
}

/// §4.3's token rule, over the one fact only the session has: whether *this client* presented the
/// token. Both of these are sessions the handshake admitted, so both are served — with the token, or
/// without it.
#[tokio::test]
async fn get_config_answers_the_token_only_to_a_client_entitled_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut config, path) = config_in(&dir);
    config.server.token = "s3cret".into();
    store::save(&path, &config).unwrap();
    let hub = hub(&config, &path, silent());

    // The client that presented the token.
    let (tx, mut rx, _) = session_attached(&hub, SessionPolicy::exposed("s3cret"));
    send(&tx, Command::Hello {
        token: Some("s3cret".into()),
        client: ClientKind::Tui,
        protocol_version: PROTOCOL_VERSION,
    });
    next_matching(&mut rx, is_done, 500).await;
    send(&tx, Command::GetConfig);
    match next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Config { .. })), 500).await {
        ToClient::Event(Event::Config { config }) => assert_eq!(config.server.token, "s3cret"),
        other => panic!("expected Event::Config, got {other:?}"),
    }

    // A loopback client, which presented nothing because its policy requires nothing.
    let (tx, mut rx, _) = session_attached(&hub, SessionPolicy::local());
    send(&tx, Command::Hello {
        token: None,
        client: ClientKind::Tui,
        protocol_version: PROTOCOL_VERSION,
    });
    next_matching(&mut rx, is_done, 500).await;
    send(&tx, Command::GetConfig);
    match next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Config { .. })), 500).await {
        ToClient::Event(Event::Config { config }) => assert_eq!(config.server.token, "s3cret"),
        other => panic!("expected Event::Config, got {other:?}"),
    }

    // The third shape is a session that is neither loopback nor token-bearing, which Task 1.2's
    // handshake refuses outright — it never reaches `GetConfig`. What remains to pin is that the
    // redacted view is one call away and is what a non-entitled caller would get:
    assert_eq!(hub.config_view(false).server.token, "");
    assert_eq!(hub.config_view(false).active_profile, config.active_profile);
}
```

- [ ] **Step 2: Run the test to verify it fails**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre --lib server::hub
```

Expected: FAIL, with no `Hub` to test at all:

```
error[E0583]: file not found for module `server`
 --> crates\tipsy-compadre\src\lib.rs
```

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre --test session
```

Expected: FAIL —

```
error[E0433]: failed to resolve: could not find `Hub` in `tipsy_compadre`
  --> crates\tipsy-compadre\tests\session.rs
```

- [ ] **Step 3: Implement the `Hub`, the configuration as a command, and the three commands' handling**

**3a.** `crates/tipsy-compadre/src/lib.rs`, after `pub mod session;` (`:19`):

```rust
pub mod server;
```

**3b.** `crates/tipsy-compadre/src/server/mod.rs` — the file Task 1.4 fills in with the listener, created here with only what the `Hub` needs so that Task 1.4 adds to a file rather than making one:

```rust
//! The server a socket reaches: its session registry, and (Task 1.4) the listener, the router and the
//! bind rule.

pub mod hub;

pub use hub::Hub;
```

**3c.** `crates/tipsy-compadre/src/server/hub.rs` — the whole type, in one piece. Every method below is the version to write; there is no earlier draft to reconcile.

```rust
//! The live-session registry: who is connected, and how a change reaches all of them (spec §5.2).
//!
//! The `Hub` is the only thing that outlives a connection. A session's `from_server` end belongs to
//! whoever bridges that connection — the frame loop, a socket — but the senders a session is reached
//! through are the registry's, because a change made by one client has to reach the others.
//!
//! What the hub owns, and what it deliberately does not: it owns the configuration, where that
//! configuration is persisted, the credential store, and **the one `ProviderSlot` the whole server
//! runs on** — `ProviderSlot` is `Arc<RwLock<Providers>>`, so `reconfigure_all` swaps the connectors
//! in place for every session at once. It does **not** own a `Brain`: a `Brain` is built from the
//! active profile's prompt and voice, which is per session, and a change reaches each of them as
//! `Command::UpdateConfig`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tipsy_protocol::{Command, ConfigView, Event, ListeningMode, ToClient};
use tokio::sync::mpsc::UnboundedSender;

use crate::brain::BrainConfig;
use crate::config::form::{config_of, view_of};
use crate::config::{Config, store};
use crate::keys::KeyResolver;
use crate::providers::{ProviderSlot, build_providers};
use crate::session::{SessionEnds, SessionPolicy, ToServer, spawn_with_events};

/// One live session: the two ends the hub needs. `events` is how a broadcast reaches the client — the
/// same name `SessionEnds` gives that sender — and `to_server` is how a change the *session* has to
/// apply, a new configuration and the brain derived from it, is handed to the task that owns it.
struct Live {
    to_server: UnboundedSender<ToServer>,
    events: UnboundedSender<ToClient>,
}
```

`ToServer` and `ToClient` are re-exported by `session.rs` (`:10`), so importing them from there is what every other module in this crate does. The `Hub` itself:

```rust
/// The live sessions, the configuration they are all running, and the two things that make a change
/// stick: where it is written and how a key is stored.
///
/// Cheap to clone — every field is a handle — which is what lets a session hold one while the
/// connection that started it holds another.
#[derive(Clone)]
pub struct Hub {
    /// Keyed by the session id Task 1.2's counter hands out. A `BTreeMap`, so `session_ids` and any
    /// log line are in a stable order: a broadcast's iteration order must not be what makes a test
    /// flaky.
    live: Arc<Mutex<BTreeMap<u64, Live>>>,
    /// The configuration every session is running. Shared rather than copied per session: a save
    /// changes it once, and every session that re-derives anything does so from this one value.
    config: Arc<Mutex<Config>>,
    /// Where a change is persisted. The listener's `--bind` override never reaches this file
    /// (spec §5.1), which is why the path and not the loaded file is what a save writes.
    config_path: PathBuf,
    /// The server's credential store, for `SetKey`. Spec §9.4 rules on *when* it may be used, and
    /// Task 1.8 owns that gate. Behind an `Arc` because `KeyResolver` is not `Clone` (`keys.rs:79-82`
    /// holds two boxed trait objects) and this type has to be.
    keys: Arc<KeyResolver>,
    /// **The one provider stack the whole server runs on.** `ProviderSlot` is `Arc<RwLock<Providers>>`
    /// (`providers/mod.rs:167`) and `ProviderSlot::replace` (`:175`) exists precisely so a
    /// configuration change swaps the connectors in place for everyone at once: every session's
    /// `Brain` holds a clone of this same slot, so a `replace` is seen by all of them with no
    /// per-session work. One slot for the server, not one per connection.
    providers: ProviderSlot,
    /// The turn timeouts and the retry backoff every session's `Brain` is built with. The hub owns it
    /// so a connection carries exactly one fact of its own — its policy — and because nothing in this
    /// plan configures it: `BrainConfig::default` (`brain.rs:28-37`) is 30 s / 30 s / 60 s / 800 ms and
    /// no tab of the configuration screen reaches it.
    brain_config: BrainConfig,
}
```

`Hub::new` therefore takes the slot as its fourth parameter, exactly as the front matter now pins it, and defaults the `BrainConfig` itself:

```rust
impl Hub {
    pub fn new(config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot) -> Hub {
        Hub {
            live: Arc::new(Mutex::new(BTreeMap::new())),
            config: Arc::new(Mutex::new(config)),
            config_path,
            keys: Arc::new(keys),
            providers,
            brain_config: BrainConfig::default(),
        }
    }

    /// The stack the server is running on. Read by the in-process client, which owns the `Player` and
    /// the `Test` probe that both need it, and by the tests.
    pub fn providers(&self) -> ProviderSlot {
        self.providers.clone()
    }

    /// The file a change is written to. Read by the tests that prove persistence and by Task 1.4's
    /// error messages, so it is not private.
    pub fn path(&self) -> &Path {
        &self.config_path
    }

    /// The configuration as it stands. A clone: the caller never holds the lock.
    pub fn config(&self) -> Config {
        self.config.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// The configuration as the wire carries it. `include_token` **is** §4.3's rule, and the caller is
    /// the side that knows who is asking: a loopback connection, or one that presented this exact
    /// token, passes `true`; everyone else passes `false` and gets the emptied field.
    pub fn config_view(&self, include_token: bool) -> ConfigView {
        let view = view_of(&self.config());
        if include_token { view } else { view.redacted() }
    }

    /// Registers a session's two ends. The id is the session's own, from Task 1.2's counter; attaching
    /// an id twice replaces the entry, which is one session per id and nothing else. The second sender
    /// is named `events` because it is the channel every producer of `ToClient` writes to — the same
    /// name `SessionEnds` gives it (`session.rs:51`).
    pub fn attach(
        &self,
        session_id: u64,
        to_server: UnboundedSender<ToServer>,
        events: UnboundedSender<ToClient>,
    ) {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id, Live { to_server, events });
    }

    /// Forgets a session. Called when its connection ends, whatever ended it (spec §10): the hub holds
    /// nothing that outlives the session it belongs to.
    pub fn detach(&self, session_id: u64) {
        self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&session_id);
    }

    /// Writes one message to every live session and reports how many took it. A closed channel is not
    /// an error: that session's connection is on its way out and `detach` is about to run.
    pub fn broadcast(&self, message: ToClient) -> usize {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .filter(|session| session.events.send(message.clone()).is_ok())
            .count()
    }

    /// The ids of the live sessions, ascending.
    pub fn session_ids(&self) -> Vec<u64> {
        self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).keys().copied().collect()
    }

    /// The server half of a change, in two parts, because a change has two parts (spec §5.2).
    ///
    /// **The connectors** are rebuilt from the configuration and swapped into the one shared
    /// `ProviderSlot`, so every session's `Brain` sees them on its next turn with no per-session work
    /// at all: they all hold a clone of this same slot. That is what `ProviderSlot::replace`
    /// (`providers/mod.rs:175`) is for.
    ///
    /// **The `Brain`** is per session and has to be rebuilt in each of them, because the active
    /// profile's prompt and voice are what a `Brain` is built from: `Command::UpdateConfig` is handed
    /// to every session so each one re-derives its own, in its own task. A per-session provider
    /// rebuild would be redundant work on top of the `replace`.
    ///
    /// Whether the *clients* are told `ConfigChanged`, `ProfileChanged`, a fresh `Welcome` or nothing
    /// at all is the caller's decision, which is why this sends no event.
    ///
    /// A configuration whose connectors cannot be built — a key name with no key behind it — leaves
    /// the running stack alone rather than installing nothing: a save must never make a working server
    /// unable to answer. The connectors that were there keep serving, `false` says the stack did not
    /// turn over, and the caller that cares (`client.rs:525-528`) is the one that refuses the save in
    /// the first place. The session half runs either way, because the configuration *is* saved by
    /// then and every `Brain` must at least read the profile it now names.
    pub fn reconfigure_all(&self) -> bool {
        let config = self.config();
        let replaced = match build_providers(&config.providers, &self.keys) {
            Ok(rebuilt) => {
                self.providers.replace(rebuilt);
                true
            }
            Err(_) => false,
        };
        let view = view_of(&config);
        let live = self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for session in live.values() {
            let _ = session.to_server.send(ToServer::Command(Command::UpdateConfig { config: view.clone() }));
        }
        replaced
    }

    /// A fresh `Welcome` to every session. `SetListeningMode` is the one command whose whole effect is
    /// that announcement (`session.rs:389-392`), which is why the hub re-sends it with the mode it has
    /// rather than needing a command of its own (spec §5.2: the mode is not configuration, so it is
    /// not a `ConfigChanged`).
    pub fn resend_welcome(&self) {
        let mode = self.config().listening.mode;
        let live = self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for session in live.values() {
            let _ = session.to_server.send(ToServer::Command(Command::SetListeningMode { mode }));
        }
    }

    /// Starts a session whose events this hub owns, and registers it before returning. The registry
    /// gets the session's two senders before anyone can read from its receiver, so its first `Welcome`
    /// cannot race the `attach` and no broadcast can miss it.
    ///
    /// The session is handed a clone of this same hub — the three commands of Task 1.1 are answered
    /// *through* it — a clone of **the server's one `ProviderSlot`**, so the stack a session's `Brain`
    /// runs on is the stack `reconfigure_all` replaces, and the hub's own `BrainConfig`. The policy is
    /// the only per-connection fact, because the configuration, the stack and the timeouts are all the
    /// hub's.
    pub fn spawn(&self, policy: SessionPolicy) -> SessionEnds {
        let ends = spawn_with_events(
            self.config(),
            self.providers.clone(),
            self.brain_config.clone(),
            policy,
            self.clone(),
        );
        self.attach(ends.session_id, ends.to_server.clone(), ends.events.clone());
        ends
    }

    /// Persist, remember, and re-derive every session from the value. `true` when the file was
    /// written. The order is the point, and it is the order `client.rs:516-543`'s save already uses:
    /// the file goes first and nothing running changes until it has, because a configuration this
    /// process cannot restart from is damage rather than a saved edit. A write that fails leaves the
    /// file, the sessions, their brains and the provider stack exactly as they were.
    ///
    /// What `reconfigure_all` found is not folded into the answer: the providers live in the shared
    /// slot and the callers that can *build* them are the ones that already refuse an unbuildable save
    /// (`client.rs:525-528`). A caller that wants to know whether the stack turned over calls
    /// `reconfigure_all` itself.
    pub fn apply_config(&self, config: Config) -> bool {
        if store::save(&self.config_path, &config).is_err() {
            return false;
        }
        *self.config.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
        let _ = self.reconfigure_all();
        true
    }

    /// `UpdateConfig` as the wire carries it. The refusal is `config_of`'s, by the configuration
    /// screen's own row names, and a refused view writes nothing.
    pub fn update_config_from_view(&self, view: ConfigView) -> usize {
        match config_of(view) {
            Ok(config) => self.update_config(config),
            Err(_) => 0,
        }
    }

    /// The configuration a save produced: persist it, re-derive every session, and tell every client
    /// to ask again. Returns what `broadcast` reported, so a caller can log it and a test can assert
    /// on it.
    pub fn update_config(&self, config: Config) -> usize {
        if !self.apply_config(config) {
            return 0;
        }
        self.broadcast(ToClient::Event(Event::ConfigChanged))
    }

    /// `SetProfile`: persist the active profile, re-derive every session, and tell every client which
    /// profile is now speaking. The clients are told with `ProfileChanged` and not with a
    /// `ConfigChanged`, because a profile is not the configuration (spec §5.2).
    pub fn set_profile(&self, id: &str) -> usize {
        let mut config = self.config();
        if !config.profiles.iter().any(|profile| profile.id == id) {
            // An id that is not in the configuration is a no-op rather than a brain built around
            // nothing — the rule `session.rs:371-385` already applies.
            return 0;
        }
        config.active_profile = id.to_string();
        if !self.apply_config(config) {
            return 0;
        }
        let profile = self.config().active().view();
        self.broadcast(ToClient::Event(Event::ProfileChanged { profile }))
    }

    /// `SetListeningMode`: persist the mode, re-derive every session, and give every client a fresh
    /// `Welcome` — the mode is not configuration, so it is not a `ConfigChanged` (spec §5.2). The
    /// count is the live sessions, which is the number `resend_welcome` just announced to.
    pub fn set_listening_mode(&self, mode: ListeningMode) -> usize {
        let mut config = self.config();
        config.listening.mode = mode;
        if !self.apply_config(config) {
            return 0;
        }
        self.resend_welcome();
        self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).len()
    }

    /// Stores a key in the **server's** credential store (spec §4.2) and tells every client that the
    /// configuration changed. Spec §9.4's loopback-or-TLS gate is Task 1.8's and is deliberately not
    /// here: this is the call site it will guard, and `SessionPolicy.loopback` is the fact it reads.
    pub fn set_key(&self, name: &str, value: &str) -> usize {
        if self.keys.store().set(name, value).is_err() {
            return 0;
        }
        self.broadcast(ToClient::Event(Event::ConfigChanged))
    }
}
```

The imports also need `use crate::brain::BrainConfig;` and
`use crate::session::{SessionEnds, SessionPolicy, spawn_with_events};`. `KeyResolver`'s store API is
`keys.rs:79-97`'s; the implementer reads that region before writing `self.keys.store().set(name, value)`
and uses whatever the store trait's own setter is called there. The name is not invented here: if it
differs, the *call* changes and nothing else in this body does.

Two deliberate choices to leave alone. `Hub::new` takes no `ProviderSlot` — the stack belongs to the
connection, and `spawn` receives it. And `broadcast` takes a `ToClient` rather than the `Event` the
front matter's one-word summary says, because `Event` is not what the channel carries: `Hub::broadcast`
is `broadcast(message: ToClient) -> usize` and the three call sites pass
`ToClient::Event(Event::…)`.

**3d.** `crates/tipsy-compadre/src/session.rs` gains the fifth constructor parameter and the real
handling. `spawn_with_events` (`:56`) becomes

```rust
pub fn spawn_with_events(
    config: Config,
    providers: ProviderSlot,
    brain_config: BrainConfig,
    policy: SessionPolicy,
    hub: Hub,
) -> SessionEnds {
```

with `spawn_session` (`:36-43`) taking and forwarding a `hub: Hub`, and `Session` gaining
`hub: Hub` beside `config` (`:122`). `welcome` (`:241-258`) is unchanged, and `set_listening_mode`
(`:389-392`) stays exactly as it is: it is still the mode announcement, and `Hub::resend_welcome` is
what sends it to every session.

The three arms replace Task 1.1's behaviour-free one:

```rust
            Command::GetConfig => {
                // §4.3: the token goes to a client that already presented it, or to a loopback one.
                // The session is the only thing that knows which it is, so it is the thing that says.
                send(
                    &self.out,
                    ToClient::Event(Event::Config {
                        config: self.hub.config_view(self.token_presented),
                    }),
                );
                ControlFlow::Continue(())
            }
            Command::UpdateConfig { config } => {
                // `config_of` is the same refusal, by the same row names, that the configuration
                // screen's own save makes (`config/form.rs:81-130`): a view that does not describe a
                // configuration is refused here, before anything is written.
                match config_of(config) {
                    Ok(config) => {
                        // Persist and re-derive everyone first, then re-derive this session from the
                        // value the hub kept — one source of truth, so two sessions cannot end up
                        // running two different readings of the same save.
                        let _ = self.hub.update_config(config);
                        self.reconfigure(self.hub.config());
                    }
                    // A view that does not describe a configuration is refused with nothing written
                    // and nothing re-derived. The client's own screen shows the row it could not
                    // parse, exactly as it does for a local save.
                    Err(_) => {}
                }
                ControlFlow::Continue(())
            }
            Command::SetKey { name, value } => {
                // Spec §9.4's refusal — `Error { code: ErrorCode::InsecureTransport, recoverable:
                // true }` unless the connection is loopback or TLS — is Task 1.8's, and this is the
                // point it plugs into: `self.policy.loopback` is the fact it needs, and
                // `self.hub.set_key` is the call it guards. Implementing the refusal here would put
                // the rule where TLS is not visible.
                let _ = self.hub.set_key(&name, &value);
                ControlFlow::Continue(())
            }
```

and `Command::SetProfile` / `Command::SetListeningMode` (`:351-352`) stop mutating this session alone
and become the server's:

```rust
            Command::SetProfile { id } => {
                // The profile is global (spec §5.2), so the command is the hub's to apply: it persists
                // the choice, re-derives every session and announces `ProfileChanged` to every client,
                // this one included. A session that mutated only its own copy would be the bug the
                // spec's §4.2 broadcast exists to prevent.
                let _ = self.hub.set_profile(&id);
                ControlFlow::Continue(())
            }
            Command::SetListeningMode { mode } => {
                // The same, for the mode: persisted, then a fresh `Welcome` to every session — which
                // reaches this one through the hub as a `SetListeningMode` of its own.
                let _ = self.hub.set_listening_mode(mode);
                ControlFlow::Continue(())
            }
```

and `Session::set_profile` (`:371-385`) is **deleted**; its body's re-derivation is now
`Hub::set_profile` + `Command::UpdateConfig` + the `reconfigure` below. `Session::set_listening_mode`
stays.

**3e.** `reconfigure`, next to where `set_profile` was (`:371`):

```rust
    /// Replaces the configuration this session is running and re-derives what comes from it: the
    /// `Brain` is rebuilt, and the profile the header shows, the interface language and the mode follow
    /// the configuration. The provider stack is unchanged — a save that changed it swaps the slot in
    /// place, which `client.rs:530-535` and `providers/mod.rs:175` already do. This is
    /// `UpdateConfig`'s session half (spec §5.2): no restart, and a turn already running keeps the
    /// brain it started with, because the rebuild happens here and the turn holds its own `Arc`.
    fn reconfigure(&mut self, config: Config) {
        self.config = config;
        let profile = self.config.active();
        self.profile = profile.view();
        self.ui_language = UiLanguage::from_bcp47(&profile.language);
        self.listening_mode = self.config.listening.mode;
        self.brain = Arc::new(Brain::new(
            self.providers.clone(),
            turn_profile(&self.config),
            self.brain_config.clone(),
        ));
    }
```

**3f.** `crates/tipsy-compadre/src/client.rs:119`, the one in-process caller:

```rust
        // The in-process client's own hub: no listener, one session, and a save from the screen still
        // reaches it through the same three commands the server uses. The hub takes the provider
        // stack, so this is the one slot the client's `Player`, its `Test` probe and the session all
        // read.
        let hub = Hub::new(
            self.config.clone(),
            self.config_path.clone(),
            self.keys.clone(),
            self.providers.clone(),
        );
        let ends = hub.spawn(SessionPolicy::local());
        // `self.providers` and `hub.providers()` are the same slot; after this line the client reads
        // the stack through the hub, so a `reconfigure_all` reaches the `Test` probe too.
        self.providers = hub.providers();
```

with `use crate::server::Hub;` added to `client.rs`'s imports (`:27` already imports from
`crate::session`). `self.keys` is the client's `KeyResolver`; if `Client` does not currently keep one,
it builds one at startup the way `main.rs:81-83` does and stores it, which is the same resolver the
`Test` probe already resolves keys with (`client.rs:490-500`). Note what did **not** leave this file:
`self.providers` stays the client's field, because the `Player` and the `Test` probe are the client's
own concerns — the hub holds the same slot, not a different one.

**3g.** Two existing tests change their expectation, and this is the task that owns the change:

- `crates/tipsy-compadre/tests/session.rs:409-441` `switching_profile_changes_the_prompt_of_the_next_turn`
  asserts a fresh `Welcome` after `Command::SetProfile` (`:425-432`). Every session is now told with
  `Event::ProfileChanged { profile }` — the same three fields, in a type whose whole purpose is this
  announcement — and the assertion becomes that event. The prompt half of the test is unchanged, and it
  still passes because `Hub::set_profile` re-derives the session's brain through
  `Command::UpdateConfig`.
- `crates/tipsy-compadre/tests/session.rs:380-405` `a_new_listening_mode_is_announced_to_the_client`
  keeps its `Welcome` assertion unchanged, which is exactly why `Hub::resend_welcome` sends
  `SetListeningMode` rather than an event.

**3h.** The whole-file path, so the commit is predictable: `crates/tipsy-compadre/src/server/hub.rs`
holds `Live`, `Hub`, and the `#[cfg(test)] mod tests`; `crates/tipsy-compadre/src/server/mod.rs` holds
`pub mod hub;` and `pub use hub::Hub;`; `crates/tipsy-compadre/src/lib.rs` holds `pub mod server;`.

- [ ] **Step 4: Run the test to verify it passes**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre --lib server::hub
```

Expected: PASS — `7` tests, `0` failed.

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre --test session
```

Expected: PASS — `18` tests before this task's `4`, `22` after, `0` failed.

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre
```

Expected: PASS — the whole `tipsy-compadre` suite, `0` failed. `tests/realtime.rs` calls `Call::open` and never the session, so a failure there is pre-existing and gets reported, not fixed.

Then the lint gate, without ever running `cargo fmt`:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo clippy -j 1 -p tipsy-compadre --all-targets
```

Expected: the finding set `global-constraints.md` pins (`tipsy-protocol/src/lib.rs:202`, `text.rs:27`, `audio/device.rs:167`, `keys.rs:81`, `keys.rs:85`, `providers/sse.rs:43`, `session.rs:233`'s line having moved, and one `derivable_impls` from the login plan) and **nothing new**. A new finding in `server/hub.rs` is this task's to fix before the commit.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/lib.rs crates/tipsy-compadre/src/server/mod.rs crates/tipsy-compadre/src/server/hub.rs crates/tipsy-compadre/src/session.rs crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/tests/session.rs
git commit -m "feat(server): one session per connection, and a hub that reaches all of them

Adds the Hub: the live-session registry keyed by the session id Task 1.2's
counter hands out, with attach, detach, broadcast, resend_welcome,
reconfigure_all and the writes that used to be one session's own business.
The hub owns the server's one ProviderSlot as well as the configuration, the
file it is persisted to and the credential store, so a change has two halves:
the connectors are rebuilt through build_providers and replaced in the shared
slot for every session at once, and each session re-derives its own Brain
because the active profile's prompt and voice are per session. SetProfile
persists and reaches every session as Event::ProfileChanged,
SetListeningMode persists and re-sends a Welcome to everyone, UpdateConfig
persists through store::save, re-derives and broadcasts ConfigChanged, and
SetKey stores through the server's KeyResolver and broadcasts the same signal
— with spec 9.4's loopback-or-TLS refusal left to Task 1.8, whose gate is
SessionPolicy::loopback, threaded in for it. GetConfig answers with
Event::Config and spec 4.3's token rule, decided by the session because it is
the only thing that knows whether the client presented the token.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 1.4: The listener, the router and the bind rule `[mid]`

**Files:**
- Modify: `crates/tipsy-compadre/src/server/mod.rs` — **Task 1.3 creates this file** (it needs the module to declare `hub` in) and leaves it holding `pub mod hub;` and nothing else; this task adds `TlsFiles`, the boot function, the listener, the router and §5.1's bind rule. Task 1.8 and Task 2.2 modify it again, so it is never a `Create:` from here on.
- Modify: `crates/tipsy-compadre/src/lib.rs` (the module list at `:3-23`; `pub mod server;` is already there from Task 1.3 — check before adding it twice)
- Modify: `crates/tipsy-compadre/Cargo.toml` — **the four dependencies this task is the first to use**: `axum = { version = "0.8.9", features = ["ws"] }`, `rust-embed = "8.13.0"`, `mime_guess = "2.0.5"` (each read off crates.io or docs.rs, quoted in `web-transport-and-terminal.md:23-25`) and `tracing = "0.1"` (already in `Cargo.lock` at `0.1.44` transitively, so declaring it adds an edge and no new crate). **`tracing-subscriber` is not this task's** — Task 1.9 owns the bootstrap that needs it. The rule the plan runs on: a task adds the manifest entry the first time it *uses* the crate, and this is that task for these four.

**Interfaces:**
- Consumes: `ServerConfig { bind: String, lan: bool, token: String }` — `config/model.rs:44-54`; `ServerConfig::has_token(&self) -> bool` — `config/model.rs:59-61`; `Config` (Clone) and `Config::active(&self) -> &Profile` — `config/model.rs:8-28`, `:254`; `ProviderSlot` (Clone) — `providers/mod.rs:167`; `KeyResolver` — `keys.rs:79-103`; `hub::Hub` — Task 1.3, whose final contract is `Hub::new(config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot) -> Hub`, `Hub::spawn(policy) -> SessionEnds`, `attach(session_id, to_server, events)`, `detach(session_id)`, `broadcast(ToClient) -> usize`, `resend_welcome()`, `reconfigure_all()`, `config_view(include_token: bool) -> ConfigView`; `crate::server::assets::{page, page_missing, asset}` — Task 1.5; `crate::server::conn::{serve, ConnOptions}` — Task 1.6.
- Produces, pinned here and nowhere else:
  - `pub struct Settings { pub bind: String, pub lan: bool, pub token: String, pub config: Config, pub config_path: std::path::PathBuf, pub keys: KeyResolver, pub providers: ProviderSlot, pub tls: Option<TlsFiles> }`, derives `Clone` + `Debug`. This is the plan's pinned eight-field `Settings`: `run` builds the `Hub` from the `Settings` it is handed, which is why `config_path`, `keys` and `providers` are on it at all. The `providers` slot is the **one shared slot** the whole server runs on, and it is what flows into `Hub::new`.
  - `#[derive(Clone, Debug)] pub enum TlsFiles { Generated, Supplied { cert: std::path::PathBuf, key: std::path::PathBuf } }` — **this task's own type, defined in `server/mod.rs` beside `Settings`**, with `None` meaning "no TLS at all". It lives here and not in `server/tls.rs` because `Settings` cannot be spelled without it, and because Task 1.7 builds a `Settings` literal and Task 1.8 consumes the value — a type introduced by 1.8 would leave both of them uncompilable until 1.8 lands. Task 1.8 adds `server/tls.rs`, whose `load_or_generate(files: TlsFiles) -> Result<Certificate, TlsError>` is called from `run`'s TLS branch.
  - `pub enum ServerError { BadBind { bind: String }, LanWithoutToken { addr: std::net::SocketAddr }, BoundWiderThanAsked { addr: std::net::SocketAddr }, Io(std::io::Error) }` with `impl Display` and `impl std::error::Error`.
  - `pub async fn run(settings: Settings, bound: tokio::sync::oneshot::Sender<std::net::SocketAddr>) -> Result<(), ServerError>`.
  - `pub(crate) fn state(settings: Settings, hub: crate::server::hub::Hub, policy: crate::session::SessionPolicy) -> ServeState` and `#[derive(Clone)] pub(crate) struct ServeState { pub(crate) token: Option<String>, pub(crate) hub: hub::Hub, pub(crate) policy: crate::session::SessionPolicy }`.
  - `pub(crate) fn mount(state: ServeState) -> axum::Router`.

`TlsFiles` is an enum and not `Option<(PathBuf, PathBuf)>` because §5.5 and §10 require **three** states and a path pair can only express two: with `Some((cert, key))` there is no way to say "generate the pair in the config directory on first use", so a `--tls-cert` pointing at a path that does not exist would be answered with a silently generated certificate instead of the refusal §10 demands ("TLS is requested but the certificate cannot be read or parsed" → refuse to start with the path in the message). `Generated` is `--tls` alone, `Supplied { .. }` is `--tls` with `--tls-cert`/`--tls-key`, and `None` is no TLS. The material is loaded **inside `server::run`** rather than in `main.rs`, which is what turns a bad path into a `ServerError` carrying it instead of a second error type in the CLI. `Settings` keeps an honest `Clone` + `Debug`: `TlsFiles` is both, while `axum_server::tls_rustls::RustlsConfig` is neither — which is why this field holds what was *asked for* rather than a built config.

Read the real files before editing them. Two line-number notes for the implementer: the recon cites `ServerConfig` as `config/model.rs:46-68`, and the file puts the struct at `:44-54` with `has_token` at `:59-61` and `impl Default` at `:64-68` — the struct citation is two lines out, everything else agrees, and the file wins. `SESSION_ID` really is `const SESSION_ID: u64 = 1;` at `session.rs:23`, with its "one session per process for now" comment at `:22`.

The bind rule is spec §5.1 and is Review Focus line 5. Three steps, in this order: parse `bind` as a `SocketAddr` and refuse anything else (a hostname must be refused, not resolved, because `TcpListener::bind` takes `ToSocketAddrs` — `web-transport-and-terminal.md:516`); refuse a non-loopback address with no token; and re-run the loopback test on `listener.local_addr()` after the bind. `0.0.0.0` is *unspecified*, not loopback (`web-transport-and-terminal.md:540`), so the second step already catches it; `is_unspecified()` is not needed for the rule, only for wording. `--bind` overrides the file for one run and never writes it back (spec §5.1) — that is `main.rs`'s job in Task 1.7; nothing here persists anything.

- [ ] **Step 1: Write the failing test**

`crates/tipsy-compadre/src/server/mod.rs` — the file Task 1.3 left holding `pub mod hub;` — gains the implementation below and this test module at the bottom. These six tests are this task's; the loopback-with-a-token one carries Review Focus line 3's first two leaks (the pages' token and their two headers).

There are no provider fakes here, and that is a consequence of Task 1.3's final contract rather than an omission: `main.rs` builds the **one shared `ProviderSlot`** the whole server runs on — the same pattern `client.rs:119-125` already uses, and why `ProviderSlot::replace` exists (`providers/mod.rs:175`) — and `Hub::new` takes it, so nothing in this module builds a stack. What the tests must supply is a resolver that answers for the key *names* the default configuration carries — `openrouter` for chat, speech and transcribe (`config/model.rs:113`, `:147-150`) plus the `openai` names a pasted configuration may use — because the slot is filled by `build_providers` when the caller builds it, and on a machine that has never run `key set` a `MissingKey` there would leave the server unable to start under test. The placeholder values never leave the test process and never reach a provider.

```rust
#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::keys::{KeyResolver, MemoryStore};

    /// How long a test lets the server do its work before calling it refused.
    const PATIENCE: Duration = Duration::from_millis(250);

    /// A real configuration file in a tempdir, and its path. The path matters now: `Hub::new` takes
    /// it, because a `SetProfile` or an `UpdateConfig` persists through the same file the server
    /// loaded (spec §5.2). The built-in `compae` profile is what `Config::active()` needs.
    fn config() -> (Config, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = crate::config::store::load_or_create(&path).unwrap().config;
        (config, path)
    }

    /// A resolver with a placeholder for each key name the default configuration asks for.
    ///
    /// **These are not secrets and this test must not depend on one existing.** The shared slot is
    /// filled from the configuration's key *names* (`model.rs:104`, `:113`, `:147-150`) when the
    /// caller builds it; without a value for each name, `build_providers` answers `MissingKey` and
    /// the server cannot start at all on a machine that has never run `key set`. The values never
    /// leave this process, never reach a provider, and never appear in a fixture — the connectors
    /// are built and no request is made.
    fn keys() -> KeyResolver {
        let store = MemoryStore::default();
        for name in ["openrouter", "openai", "elevenlabs"] {
            store.set(name, "test-placeholder-not-a-secret").expect("a memory store never fails");
        }
        KeyResolver::new(Box::new(store), Box::new(|_| None))
    }

    /// A real provider stack for the shared slot. The connectors are constructed and never called:
    /// no page route and no bind test starts a session.
    fn providers(config: &Config) -> ProviderSlot {
        ProviderSlot::new(
            crate::providers::build_providers(&config.providers, &keys())
                .expect("the placeholder resolver must satisfy the default configuration"),
        )
    }

    fn settings(bind: &str, token: &str) -> Settings {
        let (config, config_path) = config();
        Settings {
            bind: bind.into(),
            lan: false,
            token: token.into(),
            providers: providers(&config),
            config,
            config_path,
            keys: keys(),
            tls: None,
        }
    }

    /// Starts `run` on its own task and returns the address it bound, or the error it refused with.
    /// `run` never returns while it is serving, so "an address came back" is the success signal and
    /// "the task ended" is the refusal one.
    async fn start(settings: Settings) -> Result<SocketAddr, ServerError> {
        let (bound_tx, bound_rx) = tokio::sync::oneshot::channel();
        let mut handle = tokio::spawn(run(settings, bound_tx));
        if let Ok(addr) = bound_rx.await {
            return Ok(addr);
        }
        match tokio::time::timeout(PATIENCE, &mut handle).await {
            Ok(Ok(Err(e))) => Err(e),
            Ok(Ok(Ok(()))) => panic!("the server returned without binding and without an error"),
            Ok(Err(join)) => panic!("the server task panicked: {join}"),
            Err(_) => panic!("no bound address and no refusal inside {PATIENCE:?}"),
        }
    }

    /// The status one page answers with, with or without a `?token=`.
    async fn page_status(addr: SocketAddr, path: &str, token: Option<&str>) -> u16 {
        let url = match token {
            Some(t) => format!("http://{addr}{path}?token={t}"),
            None => format!("http://{addr}{path}"),
        };
        reqwest::get(url).await.unwrap().status().as_u16()
    }

    #[tokio::test]
    async fn a_loopback_bind_with_no_token_starts() {
        let addr = start(settings("127.0.0.1:0", "")).await.expect("loopback needs no token");
        assert!(addr.ip().is_loopback(), "the server must be on loopback, got {addr}");
        assert!(addr.port() != 0, "the kernel's port is reported, not the 0 that was asked for");
    }

    #[tokio::test]
    async fn a_wildcard_bind_with_no_token_refuses_and_names_lan() {
        // `0.0.0.0` is unspecified, not loopback (research note `:540`), so the rule catches it.
        let error = start(settings("0.0.0.0:0", "")).await.expect_err("0.0.0.0 is every interface");
        let message = error.to_string();
        assert!(message.contains("0.0.0.0"), "the message names the address it refused: {message}");
        assert!(message.contains("lan = true"), "and the switch that would make it legal: {message}");
        assert!(message.contains("token"), "and the token it needs: {message}");
    }

    #[tokio::test]
    async fn a_hostname_bind_is_refused_before_the_bind() {
        let error = start(settings("localhost:7070", "")).await.expect_err("a hostname may resolve off-box");
        let message = error.to_string();
        assert!(message.contains("localhost:7070"), "the message names the address: {message}");
        assert!(message.contains("IP:port"), "and says what shape it wanted: {message}");
    }

    #[tokio::test]
    async fn a_loopback_bind_with_a_token_starts_and_the_wrong_token_is_refused() {
        let addr = start(settings("127.0.0.1:0", "correct-horse")).await.expect("a token never stops a loopback bind");
        assert_eq!(page_status(addr, "/", Some("correct-horse")).await, 200, "the token it holds opens the page");
        assert_eq!(page_status(addr, "/", Some("wrong-horse")).await, 401, "another token does not");
        assert_eq!(page_status(addr, "/terminal", None).await, 401, "and both pages need it");
    }

    #[tokio::test]
    async fn both_pages_carry_the_two_headers() {
        let addr = start(settings("127.0.0.1:0", "")).await.expect("loopback needs no token");
        for path in ["/", "/terminal"] {
            let response = reqwest::get(format!("http://{addr}{path}")).await.unwrap();
            assert_eq!(response.status(), 200, "{path}");
            assert_eq!(
                response.headers().get("referrer-policy").and_then(|v| v.to_str().ok()),
                Some("no-referrer"),
                "{path} must not put the token in a Referer (spec §5.3, §9.2)"
            );
            assert_eq!(
                response.headers().get("cache-control").and_then(|v| v.to_str().ok()),
                Some("no-store"),
                "{path} must not be cached (spec §9.2, RFC 6750 §2.3)"
            );
        }
    }

    #[tokio::test]
    async fn both_halves_of_the_bind_rule_are_exercised() {
        // Step 1 and step 2, before the bind, called directly so a regression names itself.
        assert!(matches!(guard_pre_bind("0.0.0.0:0", None), Err(ServerError::LanWithoutToken { .. })));
        assert!(matches!(guard_pre_bind("bind.me:7070", None), Err(ServerError::BadBind { .. })));
        assert!(guard_pre_bind("0.0.0.0:0", Some("t")).is_ok(), "a token makes a wide bind legal");
        assert!(guard_pre_bind("127.0.0.1:7070", None).is_ok(), "loopback needs nothing");

        // Step 3, after the bind. The only way to reach a non-loopback `local_addr` here is to ask
        // for one directly — which is exactly what the third check exists for: a listener that came
        // out wider than the address that was requested.
        let wide = tokio::net::TcpListener::bind("0.0.0.0:0").await.expect("0.0.0.0 is bindable");
        assert!(!wide.local_addr().unwrap().ip().is_loopback(), "0.0.0.0 is not loopback");
        assert!(matches!(guard_post_bind(&wide, None), Err(ServerError::BoundWiderThanAsked { .. })));
        assert!(guard_post_bind(&wide, Some("t")).is_ok(), "a token makes it legal after the bind too");
        let tight = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert!(guard_post_bind(&tight, None).is_ok(), "a loopback bind passes with no token");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre server::tests`
Expected: FAIL to compile with `error[E0432]: unresolved import crate::server::hub` (Task 1.3's module, until it lands) and `error[E0425]: cannot find function run in this scope` / `cannot find type Settings`. The module has no `run`, no `Settings`, no `ServerError` and no `guard_pre_bind` until Step 3.

- [ ] **Step 3: Implement the listener, the router and the bind rule**

```rust
//! The headless server: the listener, the router and the two checks that keep a LAN behind a token.

pub mod assets;
pub mod conn;
pub mod hub;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use tipsy_protocol::ClientKind;

use crate::config::Config;
use crate::keys::KeyResolver;
use crate::providers::ProviderSlot;
use crate::server::hub::Hub;
use crate::session::SessionPolicy;

/// Everything `run` needs, as `main.rs` reads it out of the file with `--bind` already applied for
/// this run (spec §5.1). The token is a plain `String`: it is ours, it is compared on the upgrade,
/// and it is never logged (spec §9.6).
///
/// `providers` is the **one shared slot** the whole server runs on, built by the caller from this
/// configuration — the same pattern `client.rs:119-125` already uses for the in-process client, and
/// why `ProviderSlot::replace` exists (`providers/mod.rs:175`). `Hub::new` takes it and clones it
/// into every session it spawns, and `reconfigure_all` is what swaps the connectors in it when the
/// configuration changes (spec §5.2).
#[derive(Clone, Debug)]
pub struct Settings {
    pub bind: String,
    /// The `lan` switch. It is deliberately **not** part of the refusal test — the bound address is,
    /// because `lan = true` on a loopback address is still loopback and needs no token. It rides
    /// here so Task 1.9's log line can say which mode the operator asked for.
    pub lan: bool,
    pub token: String,
    /// The configuration every session spawned from this server starts with. W3 puts it on the wire
    /// and the `Hub` broadcasts its changes; the server still owns the file.
    pub config: Config,
    /// Where that configuration lives, which the `Hub` needs to persist a `SetProfile` or an
    /// `UpdateConfig` (`config/mod.rs:4-6`, `config/store.rs:20-26`).
    pub config_path: PathBuf,
    /// The key resolver the server builds providers with, and the one `SetKey` writes to.
    pub keys: KeyResolver,
    /// The shared provider slot, built from `config` before the server starts.
    pub providers: ProviderSlot,
    /// What `--tls` asked for (spec §5.5, W2), or `None` for no TLS at all. Task 1.8's
    /// `server/tls.rs` reads this; the type is defined here because `Settings` cannot be spelled
    /// without it and Task 1.7 builds a `Settings` literal before 1.8 lands.
    pub tls: Option<TlsFiles>,
}

/// Which certificate the listener uses, when TLS is on at all (spec §5.5, W2).
///
/// Three states, which is why it is not a path pair: `Generated` is `--tls` alone and means "mint a
/// self-signed pair in the config directory on first use, reuse it after that"; `Supplied` is `--tls`
/// with `--tls-cert`/`--tls-key` and means "read exactly these, and refuse to start naming the path
/// when one is missing or unparseable" (spec §10). A pair of paths cannot tell those apart, so a
/// mistyped `--tls-cert` would be answered with a silently generated certificate instead of the
/// refusal. `Settings`'s `Clone` + `Debug` stay honest because this is both.
#[derive(Clone, Debug)]
pub enum TlsFiles {
    Generated,
    Supplied { cert: PathBuf, key: PathBuf },
}

#[derive(Debug)]
pub enum ServerError {
    /// `bind` is not an explicit `IP:port`. A hostname is refused on purpose: `TcpListener::bind`
    /// takes `ToSocketAddrs`, so an unqualified name can resolve to something off this machine.
    BadBind { bind: String },
    /// Not loopback, and no token (spec §5.1 step 2, spec §10).
    LanWithoutToken { addr: SocketAddr },
    /// The bind landed somewhere other than what was asked for, with no token to guard it
    /// (spec §5.1 step 3).
    BoundWiderThanAsked { addr: SocketAddr },
    Io(std::io::Error),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::BadBind { bind } => write!(
                f,
                "bind must be an explicit IP:port and {bind:?} is not one: a hostname can resolve to \
                 something off this machine. Use e.g. 127.0.0.1:7070 or 0.0.0.0:7070."
            ),
            ServerError::LanWithoutToken { addr } => write!(
                f,
                "refusing to bind {addr}: it is not a loopback address and no token is configured. \
                 Set a token, or set `lan = true` in [server] together with a token."
            ),
            ServerError::BoundWiderThanAsked { addr } => write!(
                f,
                "bound to {addr}, which is not a loopback address, and no token is configured"
            ),
            ServerError::Io(e) => write!(f, "cannot listen: {e}"),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServerError::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// What every route shares. Cloned per request by axum. There is no provider stack here: the `Hub`
/// holds it, and a connection gets a session from `Hub::spawn`.
#[derive(Clone)]
pub(crate) struct ServeState {
    pub(crate) token: Option<String>,
    pub(crate) hub: Hub,
    /// What every session this listener starts runs under, decided once from the **bound** address:
    /// loopback needs no token, anything wider does (spec §4.1, §9.1). It is the same `SessionPolicy`
    /// `Hub::spawn` takes.
    pub(crate) policy: SessionPolicy,
}

/// The token as the routes compare it. `has_token` (`config/model.rs:59-61`) trims, so a
/// whitespace-only value in a hand-edited file is no token here either.
fn token_of(token: &str) -> Option<String> {
    (!token.trim().is_empty()).then(|| token.trim().to_string())
}

pub(crate) fn state(settings: Settings, hub: Hub, policy: SessionPolicy) -> ServeState {
    ServeState { token: token_of(&settings.token), hub, policy }
}

/// Spec §5.1 steps 1 and 2, before the bind: the user-facing refusal.
fn guard_pre_bind(bind: &str, token: Option<&str>) -> Result<SocketAddr, ServerError> {
    let addr: SocketAddr = bind.parse().map_err(|_| ServerError::BadBind { bind: bind.to_string() })?;
    // `is_loopback()` is the whole rule: `0.0.0.0` is unspecified rather than loopback, so it fails
    // here too. `is_unspecified()` would only change the wording, and the wording lives in `Display`.
    if !addr.ip().is_loopback() && token.is_none() {
        return Err(ServerError::LanWithoutToken { addr });
    }
    Ok(addr)
}

/// Spec §5.1 step 3, after the bind. The listener is dropped by the caller when this returns `Err`.
fn guard_post_bind(listener: &tokio::net::TcpListener, token: Option<&str>) -> Result<(), ServerError> {
    let addr = listener.local_addr().map_err(ServerError::Io)?;
    if !addr.ip().is_loopback() && token.is_none() {
        return Err(ServerError::BoundWiderThanAsked { addr });
    }
    Ok(())
}

/// Serves until the process ends. `bound` is sent the address once **both** halves of §5.1's rule
/// have passed, and that is also the point a caller may start connecting.
pub async fn run(settings: Settings, bound: tokio::sync::oneshot::Sender<SocketAddr>) -> Result<(), ServerError> {
    let token = token_of(&settings.token);
    let addr = guard_pre_bind(&settings.bind, token.as_deref())?;
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(ServerError::Io)?;
    guard_post_bind(&listener, token.as_deref())?;
    let local = listener.local_addr().map_err(ServerError::Io)?;
    let _ = bound.send(local);
    // The bound address decides the policy every session from this listener runs under (Task 1.2's
    // `SessionPolicy`; spec §4.1, §9.1): loopback needs nothing, anything wider needs the token.
    let policy = SessionPolicy {
        require_token: !local.ip().is_loopback(),
        token: token.clone(),
        loopback: local.ip().is_loopback(),
    };
    // The `Hub` is built once, before the first connection, and outlives every session (spec §5.2).
    // It takes the one shared provider slot: `Hub::spawn` clones it into each session, and
    // `reconfigure_all` is what replaces its contents when the configuration changes.
    let hub = Hub::new(
        settings.config.clone(),
        settings.config_path.clone(),
        settings.keys.clone(),
        settings.providers.clone(),
    );
    // Task 1.8's TLS branch goes here: `tls::load_or_generate(files)` against `settings.tls`, with
    // the listener handed to `axum_server::tls_rustls::bind_rustls` instead of `axum::serve`. Doing
    // the loading inside `run` is what makes an absent or unparseable `--tls-cert` a `ServerError`
    // naming the path (spec §10) rather than a second error type in the CLI. `None` is no TLS, and
    // that is the only branch this task implements.
    let _ = &settings.tls;
    let app = mount(state(settings, hub, policy));
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!(error = %e, "the listener stopped with an error");
    }
    Ok(())
}

pub(crate) fn mount(state: ServeState) -> Router {
    Router::new()
        .route("/", get(page_client))
        .route("/terminal", get(page_terminal))
        .route("/assets/{*path}", get(asset))
        .route("/ws", get(ws))
        // `/terminal/ws` belongs to Task 2.2 and is deliberately not registered here: it is a raw
        // PTY byte socket with its own handshake, and it is a 404 until Phase 2 lands.
        .with_state(state)
}

fn page_of(path: &str) -> Response {
    match assets::page(path) {
        Some((content_type, bytes)) => page_response(content_type, bytes),
        None => {
            let (content_type, html) = assets::page_missing();
            page_response(content_type, html.into_bytes())
        }
    }
}

async fn page_client(State(state): State<ServeState>, query: Query<TokenQuery>) -> Response {
    if !token_ok(&state, &query) {
        return refused();
    }
    page_of("/")
}

async fn page_terminal(State(state): State<ServeState>, query: Query<TokenQuery>) -> Response {
    if !token_ok(&state, &query) {
        return refused();
    }
    page_of("/terminal")
}

async fn asset(State(state): State<ServeState>, query: Query<TokenQuery>, path: Path<String>) -> Response {
    if !token_ok(&state, &query) {
        return refused();
    }
    match assets::asset(&path.0) {
        Some((content_type, bytes)) => {
            let mut response = Response::new(Body::from(bytes));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                content_type.parse().expect("a MIME type is a header value"),
            );
            // A hashed asset name may be cached; the two pages may not (spec §5.3). An asset carries
            // no token, so it needs no `Referrer-Policy`.
            response.headers_mut().insert("cache-control", "no-cache".parse().unwrap());
            response
        }
        // Not in the bundle, and the bundle is not a file server.
        None => (StatusCode::NOT_FOUND, "not in the bundle").into_response(),
    }
}

/// The one place the two page headers are set, so the explanation page carries them too: it is
/// reached with the same `?token=` (spec §5.3, §9.2, RFC 6750 §2.3).
fn page_response(content_type: &'static str, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type.parse().expect("a MIME type is a header value"));
    headers.insert("referrer-policy", "no-referrer".parse().expect("a static policy is a value"));
    headers.insert("cache-control", "no-store".parse().expect("a static directive is a value"));
    response
}

/// The protocol socket. Registered here; everything past the upgrade belongs to Task 1.6.
async fn ws(State(state): State<ServeState>, query: Query<TokenQuery>, upgrade: WebSocketUpgrade) -> Response {
    if !token_ok(&state, &query) {
        return refused();
    }
    let options = conn::ConnOptions::default();
    // The research note reads axum 0.8.9's own defaults: the write buffer is 128 KiB and
    // `max_write_buffer_size` is **unlimited** until it is set (`web-transport-and-terminal.md:1339-1344`).
    // A peer that stops reading must not grow the server's memory, so the ceiling is explicit.
    let upgrade = upgrade
        .write_buffer_size(options.write_buffer_size)
        .max_write_buffer_size(options.max_write_buffer_size);
    // Taken out before the upgrade so the async block owns what it needs: `spawn` needs the hub
    // itself, and `serve` needs another handle to it.
    let hub = state.hub.clone();
    let policy = state.policy.clone();
    upgrade.on_upgrade(move |socket| async move {
        // `Hub::spawn` is the only path a connection uses: the session is attached to the registry
        // **before** `Welcome` goes out, which is what removes the race a spawn-then-attach ordering
        // would leave (Task 1.3; spec §5.2). It clones the shared slot the `Hub` was built with, so
        // this handler passes no providers and builds none.
        let ends = hub.spawn(policy);
        // `ClientKind::Web`: this socket carries the HUD protocol, not raw PTY bytes. `/terminal/ws`
        // (`ClientKind::WebTerminal`) is Task 2.2's.
        conn::serve(socket, ends, hub.clone(), ClientKind::Web).await;
    })
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// Spec §9.1: a server with no token accepts anything, and §5.1 guarantees that server is on
/// loopback. A server with a token needs it on the upgrade for the pages and, until Task 1.2's
/// `Hello` validation answers, for the socket too — the query form is what a browser can send on a
/// navigation, which is the whole reason §9.2 pays for it.
fn token_ok(state: &ServeState, query: &TokenQuery) -> bool {
    match &state.token {
        None => true,
        Some(expected) => query.token.as_deref() == Some(expected.as_str()),
    }
}

/// Never echoes the token, never names it, never logs it (spec §9.6).
fn refused() -> Response {
    (StatusCode::UNAUTHORIZED, "the token is missing or wrong").into_response()
}
```

Two mechanical notes on that code. The two page handlers take `State` and `Query` only — never a `WebSocketUpgrade`, which would make a page route reject a plain browser navigation. The upgrade handler's shape is the research note's **INFERRED** one for axum (`web-transport-and-terminal.md:245-266`): `WebSocketUpgrade` has no `callback` method (`:236-243`), so the pre-upgrade hook is extractor ordering — `Query` next to `WebSocketUpgrade` in the signature, and the handler either returns a non-upgrade `Response` or calls `on_upgrade`. `on_failed_upgrade` cannot refuse a request, because it fires after a failed background upgrade (`:240-243`). The `on_upgrade(|socket| async move { … })` form is the note's own example (`:182-200`).

`axum` is `axum = { version = "0.8.9", features = ["ws"] }` — the version and the `ws` feature read off crates.io in `web-transport-and-terminal.md:23` and `:158`. **This task is the one that adds the manifest entries.** It is the first to use `axum`, `rust-embed`, `mime_guess` and `tracing::error!`, so its `Files` block carries the `Cargo.toml` modification and nothing here calls a macro the manifest cannot resolve. `tracing-subscriber` is Task 1.9's alone, because 1.9 owns the bootstrap that needs it. A first draft of this task deferred all of them to "the task that first adds the dependency" — which was nobody, so the server would not have compiled until the last task of the phase caught up. Registering the module needs `pub mod server;` in `crates/tipsy-compadre/src/lib.rs`'s list (`lib.rs:3-23`).

- [ ] **Step 4: Run the test to verify it passes**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre server::tests`
Expected: PASS, `6 passed; 0 failed`, with no port hard-coded anywhere: every test binds `127.0.0.1:0` and reads back the address `run` reported.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/server/mod.rs crates/tipsy-compadre/src/lib.rs
git commit -m "feat(server): bind, route and refuse a LAN without a token" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

Task 1.4 does not create `server/tls.rs`: 1.8 adds that file with the loading that consumes `TlsFiles` (and it is 1.8's commit that names it).

### Task 1.5: The embedded bundle and the honest fallback pages `[small]`

**Files:**
- Create: `crates/tipsy-compadre/src/server/assets.rs`
- Modify: `crates/tipsy-compadre/src/session.rs:241-258` (the `welcome` method, whose body writes `web_bundle: false` at `:249`)
- Modify: `crates/tipsy-ui/src/state.rs:100-126` (the `UiState` fields), `:135-160` (`UiState::new`), `:289-312` (the `Welcome` arm), `:531-547` (the test helpers `welcome`/`welcome_mode`)
- Modify: `crates/tipsy-ui/src/strings.rs:5-71` (the `Strings` fields), `:134` (the ES table's last entry), `:198` (the EN table's last entry)

**Interfaces:**
- Consumes: `Event::Welcome { .., web_bundle: bool, .. }` — `tipsy-protocol/src/lib.rs:135-145`; `Session::welcome(&self)` — `session.rs:241`; `UiState::apply(&mut self, ev: &Event)` — `state.rs:289`; `UiState::strings(&self) -> &'static Strings` — `state.rs:162`; `UiState::notify(&mut self, text: impl Into<String>)` — `state.rs:166`; `Strings` — `strings.rs:5`.
- Produces: `crate::server::assets::{DIST_COMMAND, PAGE_CLIENT, PAGE_TERMINAL, resolve, fallback, asset, web_bundle}` with the exact shapes below; `pub struct UiState { …, pub web_bundle: bool }`; `Strings::web_unavailable`.

**`resolve` and `fallback` are not the names an earlier draft of this task pinned, and the change is deliberate.** Task 1.4 could not compile without a `server/assets.rs` — its router and its six tests call `assets::resolve(path) -> Option<(&'static str, Vec<u8>)>` and `assets::fallback() -> (&'static str, Vec<u8>)`, and `server/mod.rs`'s `page_of`/`page_response` are the callers — so Task 1.4 created that file as a loudly-commented placeholder with exactly those two signatures and registered `/` and `/terminal` against them. **Fill those two bodies; do not rename them** to the `page`/`page_missing` this document used to say, because a rename breaks Task 1.4's six tests. `asset`, `web_bundle`, `DIST_COMMAND`, `PAGE_CLIENT` and `PAGE_TERMINAL` are new and entirely this task's; `/assets/{*path}` is yours to register, since Task 1.4 deliberately left it out.

```rust
pub const DIST_COMMAND: &str = "cargo xtask dist";
pub const PAGE_CLIENT: &str = "client/index.html";
pub const PAGE_TERMINAL: &str = "terminal/index.html";
pub fn resolve(path: &str) -> Option<(&'static str, Vec<u8>)>;       // bytes + content type, or None
pub fn fallback() -> (&'static str, Vec<u8>);                       // bytes + content type, always
pub fn asset(path: &str) -> Option<(&'static str, Vec<u8>)>;
pub fn web_bundle() -> bool;
```

**What this task can and cannot serve, said plainly so nobody thinks a page is missing.** The bundle's real contents do not exist yet: Phase 2's Task 2.4 fills `web/terminal/index.html` and Phase 3's Task 3.6 fills `web/client/index.html`, and `cargo xtask dist` (spec §8.3) is what puts them in the embed. Until then `web/` is absent, the embed is empty, and **both** page routes answer the explanation page that names `cargo xtask dist` — which is exactly the behaviour spec §10's "The bundle is not embedded" row requires and spec §5.4 describes, not a shortfall. `rust-embed`'s `#[allow_missing = true]` is the whole mechanism; quote of the crate README from the research note: "You can add `#[allow_missing = true]` to the `RustEmbed` struct to allow the embedded folder to be missing. In that case, RustEmbed will be empty." (`web-transport-and-terminal.md:369-372`), and the note's conclusion: "the derivation does **not** need to be conditionally compiled away, and no `build.rs` gate is strictly required" (`:374`). The embed folder path resolves relative to `crates/tipsy-compadre/Cargo.toml` in release and relative to where the binary runs in debug (`:353-358`), which is why the tests below assert what the routes answer rather than pinning the constants.

- [ ] **Step 1: Write the failing test**

`crates/tipsy-compadre/src/server/assets.rs` is created with its test module at the bottom.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Spec §10's "The bundle is not embedded" row, for both routes. What is asserted is the
    /// *property* — an embedded page, or the page naming the command — so the test keeps passing
    /// when Phase 2 and Phase 3 land the real pages, and it fails today only if a route answers
    /// neither.
    #[test]
    fn page_routes_answer_a_page_or_the_explanation_page() {
        for path in ["/", "/terminal"] {
            let (content_type, body) = page(path).expect("a page route always answers something");
            assert_eq!(content_type, "text/html; charset=utf-8", "{path}");
            assert!(!body.is_empty(), "{path} answered an empty page");
            let html = String::from_utf8(body).expect("a page is HTML text");
            if embedding(path).is_none() {
                assert!(html.contains(DIST_COMMAND), "{path} must name the command that builds it");
            } else {
                // Phase 2 or Phase 3 has landed the real page: it is served, and the explanation
                // page is not.
                assert!(!html.contains("not available in this build"), "{path} served the fallback");
            }
        }
        assert!(page("/nope").is_none(), "a route that is not a page has no page");
    }

    /// With the folder absent, `web_bundle()` is `false`, which is what `Welcome.web_bundle`
    /// reports and what the HUD's message is for (spec §5.4). The assertion is the property, so it
    /// holds in a `cargo xtask dist` tree too.
    #[test]
    fn the_bundle_flag_agrees_with_the_embed() {
        assert_eq!(web_bundle(), embedding(PAGE_CLIENT).is_some_and(|bytes| !bytes.is_empty()));
    }

    /// `asset` is `None` for anything the bundle does not hold, and an unknown path is not a licence
    /// to walk out of the folder.
    #[test]
    fn an_unknown_asset_is_nothing() {
        // The real embed is empty in this checkout, so every path misses — including the two that
        // would be dangerous if they did not.
        assert_eq!(asset("nope.js"), None);
        assert_eq!(asset("../config.toml"), None);
        assert_eq!(asset(""), None);
    }

    /// The explanation page says which command builds the bundle and what is missing, and says
    /// nothing about the token that reached it.
    #[test]
    fn the_explanation_page_names_the_command() {
        let (content_type, body) = page_missing();
        assert_eq!(content_type, "text/html; charset=utf-8");
        assert!(body.contains(DIST_COMMAND), "the page must say how to build the bundle");
        assert!(body.contains("web UI"), "and what is not available");
        assert!(!body.to_lowercase().contains("token"), "and must not talk about the token");
    }
}
```

The client-side half lives in `crates/tipsy-ui/src/state.rs`'s existing test module. Change the two helpers (`state.rs:531-547`) so `welcome` reports an available bundle, and add a third that takes the flag:

```rust
    fn welcome(lang: UiLanguage) -> Event {
        welcome_mode(lang, ListeningMode::Push)
    }

    /// The same announcement for any mode: the mode is an argument because what the HUD says about
    /// it is what the tests below are about. The bundle is reported as available here so these tests
    /// stay about the mode; §5.4's message has its own test below.
    fn welcome_mode(lang: UiLanguage, mode: ListeningMode) -> Event {
        welcome_bundle(lang, mode, true)
    }

    /// The same announcement with `Welcome.web_bundle` under the caller's control.
    fn welcome_bundle(lang: UiLanguage, mode: ListeningMode, web_bundle: bool) -> Event {
        Event::Welcome {
            session_id: 1,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: lang,
            listening_mode: mode,
            web_bundle,
            // No login screen: the HUD tests below are about the HUD.
            login: None,
        }
    }

    /// `architecture.md:85-86` promises that a binary built without the web bundle "reports that
    /// the web UI is not available". The flag is the server's truth, so it reaches the state — and
    /// the HUD says so, in the interface's own language, from the same two static tables every
    /// other string comes from.
    #[test]
    fn a_binary_without_the_bundle_says_so_in_both_languages() {
        for (lang, word) in [(UiLanguage::Es, "no está disponible"), (UiLanguage::En, "not available")] {
            let mut s = UiState::new();
            s.apply(&welcome_bundle(lang, ListeningMode::Push, false));
            assert!(!s.web_bundle, "the flag reaches the state: {lang:?}");
            let (notice, left) = s.notice.as_ref().expect("a binary without the bundle must say so");
            assert!(notice.contains(word), "{lang:?}: {notice}");
            assert_eq!(notice.as_str(), s.strings().web_unavailable, "{lang:?}");
            assert!(*left > 0, "and the notice is a live one, not a spent one");
        }
    }

    /// A server that *has* the bundle says nothing: a message about a missing page over a server
    /// that serves it would be its own lie.
    #[test]
    fn a_binary_with_the_bundle_stays_quiet() {
        let mut s = UiState::new();
        s.apply(&welcome_bundle(UiLanguage::En, ListeningMode::Push, true));
        assert!(s.web_bundle, "the flag reaches the state");
        assert!(s.notice.is_none(), "an available bundle has nothing to explain");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre assets::tests`
Expected: FAIL to compile with `error[E0425]: cannot find function page in this scope`, `cannot find function page_missing`, `cannot find function asset`, `cannot find function web_bundle`, `cannot find function embedding`, and `cannot find value DIST_COMMAND`.

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-ui web_
Expected: FAIL to compile with `error[E0609]: no field web_bundle on type UiState` (twice: the field and the read) and `no field web_unavailable on type Strings`.

- [ ] **Step 3: Implement the embed, the lookup and the truthful flag**

```rust
//! The bundle the binary carries, and the page that says so when it carries none.
//!
//! `#[allow_missing = true]` is the whole mechanism: with `web/` absent the embed is empty, every
//! lookup is `None`, and the two page routes fall back to [`page_missing`]. No `build.rs` gate is
//! needed (spec §5.4; research `web-transport-and-terminal.md:365-386`).

use rust_embed::RustEmbed;

/// What `cargo xtask dist` produces. Hashed filenames may be cached; the two pages may not.
#[derive(RustEmbed)]
#[folder = "web/dist/"]
#[allow_missing = true]
struct Bundle;

/// What a user runs to build the bundle this binary does not carry.
pub const DIST_COMMAND: &str = "cargo xtask dist";

/// The web client's host page. Phase 3's Task 3.6 fills it.
pub const PAGE_CLIENT: &str = "client/index.html";
/// The xterm.js host page. Phase 2's Task 2.4 fills it.
pub const PAGE_TERMINAL: &str = "terminal/index.html";

const HTML: &str = "text/html; charset=utf-8";
const OCTET: &str = "application/octet-stream";

/// Whether this binary carries a bundle at all — the truth `Welcome.web_bundle` reports
/// (spec §5.4), so a plain `cargo build` and a `cargo xtask dist` differ.
pub fn web_bundle() -> bool {
    embedding(PAGE_CLIENT).is_some_and(|bytes| !bytes.is_empty())
}

/// The page for a route: the embedded file, or `None` when the route has no page in this binary.
/// Today that is always `None`, because `web/` does not exist until Phase 2 and Phase 3 build it.
/// **Keep the name and the signature**: Task 1.4's `server/mod.rs` calls both, and this file was
/// created by that task as a placeholder with exactly this shape.
pub fn resolve(path: &str) -> Option<(&'static str, Vec<u8>)> {
    embedding(path).map(|bytes| (HTML, bytes))
}

/// The honest page: which command builds the bundle this binary does not have. Hand-written HTML —
/// there is no markdown renderer in the tree and none is added for one page. **Returns `Vec<u8>`, not
/// the `String` an earlier draft of this task specified**, because Task 1.4 pinned the signature when
/// it made this file a placeholder: the `format!` below therefore ends in `.into_bytes()`.
pub fn fallback() -> (&'static str, Vec<u8>) {
    (
        HTML,
        format!(
            "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
             <title>tipsy-compadre — no web UI in this build</title></head>\n<body>\n\
             <h1>The web UI is not available in this build</h1>\n\
             <p>This binary was built without the web bundle. Build it with:</p>\n\
             <pre>{DIST_COMMAND}</pre>\n\
             <p>then run <code>tipsy-compadre serve</code> again.</p>\n\
             </body></html>\n"
        ),
    )
}

/// A file from the embed, with the content type a browser needs for it.
pub fn asset(path: &str) -> Option<(&'static str, Vec<u8>)> {
    // `EmbeddedFile.data` is a `Cow<'static, [u8]>` (research `:336-346`), owned here so the
    // response body is a `Vec<u8>` and nothing borrows the embed.
    let bytes = Bundle::get(path)?.data.into_owned();
    if bytes.is_empty() {
        return None;
    }
    Some((content_type(path), bytes))
}

/// The bytes of a page in the embed, when it holds one. `client/index.html` and
/// `terminal/index.html` are the only two paths a page route may answer with; a path with `..` or
/// no extension is never one.
fn embedding(path: &str) -> Option<Vec<u8>> {
    let name = match path {
        "/" | "/client" | PAGE_CLIENT => PAGE_CLIENT,
        "/terminal" | PAGE_TERMINAL => PAGE_TERMINAL,
        _ => return None,
    };
    let file = Bundle::get(name)?;
    (!file.data.is_empty()).then(|| file.data.into_owned())
}

/// The content type by extension. `mime_guess::from_path(path).first()` is the verified call
/// (`web-transport-and-terminal.md:468-473`); `first_or_octet_stream()` is marked INFERRED on the
/// same page, so the fallback is written out here rather than taken from the crate. The bundle's own
/// extensions are matched first, which is also what the note recommends (`:476-479`).
fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => HTML,
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        _ => mime_guess::from_path(path).first().map_or(OCTET, |mime| match mime.as_ref() {
            "text/javascript" => "text/javascript",
            "text/css" => "text/css",
            "application/wasm" => "application/wasm",
            "application/json" => "application/json",
            "image/png" => "image/png",
            "image/svg+xml" => "image/svg+xml",
            _ => OCTET,
        }),
    }
}

#[cfg(test)]
mod tests { /* the module written in Step 1 */ }
```

`rust-embed = "8.13.0"` and `mime_guess = "2.0.5"` are the versions read off crates.io and docs.rs in `web-transport-and-terminal.md:24-25`; the manifest entries belong to the task that first adds them.

Then `welcome` in `crates/tipsy-compadre/src/session.rs:241-258` stops lying about the bundle:

```rust
    fn welcome(&self) {
        send(
            &self.out,
            ToClient::Event(Event::Welcome {
                session_id: SESSION_ID,
                profile: self.profile.clone(),
                ui_language: self.ui_language,
                listening_mode: self.listening_mode,
                // The truth for the binary that is running, not the constant `false` this used to
                // be: `cargo xtask dist` embeds the bundle and a plain `cargo build` does not
                // (spec §5.4; `architecture.md:83-86`).
                web_bundle: crate::server::assets::web_bundle(),
                login: None,
            }),
        );
    }
```

`SESSION_ID` is still the constant at `session.rs:23` — Task 1.3 replaces it with the counter — and the `login: None` line with its comment at `:250-255` stays exactly as it is.

And `tipsy-ui` learns the flag. In `crates/tipsy-ui/src/state.rs`, the field goes at the end of the struct's fields (`:125`):

```rust
    /// What the running server reported about its own binary (`Welcome.web_bundle`, spec §5.4):
    /// `false` means the web UI is not available in this build.
    pub web_bundle: bool,
```

`UiState::new` (`:136-159`) initialises it `web_bundle: false,` next to `config_form: None`, and the `Welcome` arm (`:291-312`) becomes:

```rust
            Event::Welcome { profile, ui_language, listening_mode, web_bundle } => {
                self.profile = Some(profile.clone());
                self.lang = *ui_language;
                self.listening_mode = *listening_mode;
                self.web_bundle = *web_bundle;
                // `architecture.md:85-86`: a binary without the bundle "reports that the web UI is
                // not available". The notice slot holds one message, so this only ever replaces a
                // notice it left itself — a warning from the session keeps the screen.
                if !self.web_bundle
                    && self
                        .notice
                        .as_ref()
                        .is_none_or(|(text, _)| text == self.strings().web_unavailable)
                {
                    self.notify(self.strings().web_unavailable);
                }
                // The configuration accepts three modes and this plan implements two, so the third
                // one says so instead of going quiet: the wake word arrives in a later plan, and
                // until then a `hands_free` session behaves like `push` — the microphone is opened
                // by hand, which is why this only speaks and never touches `mic_open`.
                let unsupported = self.strings().hands_free_unimplemented;
                match *listening_mode {
                    ListeningMode::HandsFree => self.notify(unsupported),
                    ListeningMode::Push | ListeningMode::Auto => {
                        if self.notice.as_ref().is_some_and(|(text, _)| text == unsupported) {
                            self.notice = None;
                        }
                    }
                }
            }
```

The `..` in the destructuring pattern had to go, because the flag is now read; every other field of `Welcome` is still named. `Option::is_none_or` is stable (Rust 1.82; the toolchain is pinned to 1.96.1 by `rust-toolchain.toml:2`). The equivalent older spelling, if the reader prefers it, is
`!self.notice.as_ref().is_some_and(|(text, _)| text != self.strings().web_unavailable)`.

Finally the two tables. Add the field to `Strings` after `hands_free_unimplemented` (`strings.rs:70`):

```rust
    /// The HUD's notice for a binary whose embed holds no bundle (spec §5.4). It names the
    /// user-visible consequence, not the build command: the command belongs on the page the routes
    /// serve, and it is long.
    pub web_unavailable: &'static str,
```

and one line in each table, after `hands_free_unimplemented` (`strings.rs:134` in ES, `:198` in EN):

```rust
    web_unavailable: "la interfaz web no está disponible en esta compilación",
```

```rust
    web_unavailable: "the web UI is not available in this build",
```

The ES copy is the user's to approve like every other interface string: reproduce it verbatim and flag a disagreement instead of rewriting it.

- [ ] **Step 4: Run the test to verify it passes**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre assets::tests`
Expected: PASS, `4 passed; 0 failed`.

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-ui state::tests`
Expected: PASS, `31 passed; 0 failed` against the recon's count of 29 tests in `crates/tipsy-ui/src/state.rs` (`codebase-recon.md:1023`) plus the two added here. A previously green state test that is red now almost certainly went through `welcome`/`welcome_mode`, and the `web_bundle: true` argument in `welcome_mode` is the first thing to check. Finish the crate with the wider run, since `strings.rs` has no test module of its own and the two tables are only compiled as one: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-ui` — expected PASS with no test that was green before this task now red.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/server/assets.rs crates/tipsy-compadre/src/session.rs crates/tipsy-ui/src/state.rs crates/tipsy-ui/src/strings.rs
git commit -m "feat(server): embed the bundle and tell the truth when it is absent" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 1.6: The connection bridge, its bounds and its endings `[mid]`

**Files:**
- Create: `crates/tipsy-compadre/src/server/conn.rs`

**Interfaces:**
- Consumes: `crate::session::SessionEnds` — `session.rs:46-52`, including Task 1.2's `session_id` field; `crate::session::SessionPolicy` — Task 1.2's `{ require_token, token, loopback }`; `crate::server::hub::Hub` — Task 1.3, whose `Hub::spawn(policy)` is the only path that starts a session (see the contract below); `tipsy_protocol::{Command, ErrorCode, Event, Frame, ToClient, ToServer}` — `tipsy-protocol/src/lib.rs:107`, `:51`, `:132`, `:174`, `:244`, `:251`; `axum::extract::ws::{WebSocket, Message}` — axum 0.8.9's **own** enum, `Text(Utf8Bytes) | Binary(Bytes) | Ping(Bytes) | Pong(Bytes) | Close(Option<CloseFrame>)`, with no `Frame` variant (`web-transport-and-terminal.md:203-219`).
- Produces:
  - `pub async fn serve(socket: axum::extract::ws::WebSocket, ends: crate::session::SessionEnds, hub: crate::server::hub::Hub, client: tipsy_protocol::ClientKind) -> Ending`.
  - `pub async fn serve_with(socket: axum::extract::ws::WebSocket, ends: crate::session::SessionEnds, hub: crate::server::hub::Hub, client: tipsy_protocol::ClientKind, options: ConnOptions) -> Ending` — the same bridge with the bounds chosen, which is the shape the tests drive and the shape a later caller (Task 2.2's PTY pump) can reuse.
  - `#[derive(Clone, Copy, Debug)] pub struct ConnOptions { pub write_timeout: Duration, pub queue: usize, pub write_buffer_size: usize, pub max_write_buffer_size: usize }` with `impl Default` (`write_timeout: 5s`, `queue: WRITE_QUEUE`, `write_buffer_size: 128 * 1024`, `max_write_buffer_size: 1024 * 1024`; the two buffer defaults are axum's own, quoted from `web-transport-and-terminal.md:1339-1344`, with the ceiling deliberately finite).
  - `pub const WRITE_QUEUE: usize = 32;`.
  - `#[derive(Debug, PartialEq, Eq)] pub enum Ending { ClosedByClient, ServerStopped, TransportError, Saturated }`.

**Which `Message` this file means, once, so nobody has to guess.** Every `Message` in `conn.rs` is **`axum::extract::ws::Message`** — the server-side one, whose text payload is `Utf8Bytes` and whose binary payload is `bytes::Bytes`, and which has no `Frame` variant. Its automatic ping answer and close handshake are axum's, so the bridge never writes either. The **client** side (Tasks 1.7 and 1.9) uses `tokio_tungstenite::tungstenite::Message` 0.30, a different type with a `Frame(Frame)` variant; the two never meet, and the test module below says which one it is constructing at each call site. W1 accepts this cost knowingly (`web-transport-and-terminal.md:161-167`).

**The `Hub` contract, which is Task 1.3's and is not re-argued here.** The plan's Shared interface contract now carries the final shape, and this bridge uses exactly these four names:

```rust
// tipsy-compadre::server::hub, #[derive(Clone)]
pub fn new(config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot) -> Hub;
/// **The** session-spawning entry point, and the only one. It attaches the session to the registry
/// **before** `Welcome` goes out, so no client can miss a broadcast that was meant for it, and it
/// clones the hub's shared provider slot into the session it starts.
pub fn spawn(policy: SessionPolicy) -> SessionEnds;
pub fn attach(&self, session_id: u64, to_server: UnboundedSender<ToServer>, events: UnboundedSender<ToClient>);
pub fn detach(&self, session_id: u64);
pub fn broadcast(&self, message: ToClient) -> usize;
pub fn resend_welcome(&self);
pub fn reconfigure_all(&self);
pub fn config_view(&self, include_token: bool) -> ConfigView;
```

There is **one** `ProviderSlot` for the whole server, not one per connection, and **one** entry point: `main.rs` builds the slot, `Hub::new` takes it, `Hub::spawn` clones it into each session, and `reconfigure_all` rebuilds the connectors, `replace`s them in the slot (`providers/mod.rs:175`) and tells every session to re-derive its `Brain`, because the profile's prompt and voice are per session. A test therefore puts its fakes in the slot it hands to `Hub::new` — there is no second way to choose the slot, and no `spawn_with`.

Two consequences for this file, and both are the point of the hub rather than a detail:

- **There is no *public* `next_session_id()` to call, and this bridge never allocates an id.** The counter is private to `session.rs` (`static SESSION_IDS: AtomicU64`, incremented at spawn time) and the id arrives as `SessionEnds.session_id` (Task 1.2 adds the field). The `detach` above is the only place a session id is needed here.
- **This bridge never calls `spawn_with_events` and then `attach`.** `Hub::spawn` is the only path a connection uses, precisely because it attaches *before* `Welcome`; a hand-rolled spawn-then-attach ordering would reintroduce the race the hub exists to remove. The bridge clones the `Hub` out of the router state and calls it.

`Hub::spawn(policy)` is the only path a connection uses — the only entry point there is — and it returns the `SessionEnds` the bridge consumes, with the hub's shared slot cloned into the session and the session already attached. One caller shape, two callers of it:

```rust
// product (Task 1.4's `/ws` handler) and this module's tests alike
let ends = hub.spawn(policy);
```

The only difference between them is which slot the `Hub` was built with: 1.4's tests hand `Hub::new` a real stack built from the default configuration, and this module hands it the fakes. The policy is the connection's own — in 1.4 it is `SessionPolicy { require_token, token, loopback }` built from the bound address and the configured token; in this module the hub it runs under has no token, so the permissive default is what the tests want.

`SessionPolicy` is Task 1.2's `{ require_token, token, loopback }` and is a parameter of `spawn_session`, `spawn_with_events` and `Hub::spawn`. `attach` takes the two senders separately — `to_server` and `events`, the latter being `SessionEnds.events` — because `reconfigure_all` has to reach the *session* with a new configuration, not just its client; **the bridge itself never calls it**, because `Hub::spawn` already did.

`SessionEnds` (`session.rs:46-52`, plus Task 1.2's `session_id` field) is **destructured, never cloned**: `from_server` is an `UnboundedReceiver` and is not `Clone`, which is why the socket and the receiver have to live in the same task below. `spawn_session` and `spawn_with_events` also gained Task 1.2's `SessionPolicy` parameter; nothing in this file constructs a `SessionEnds`, so the new field and the new parameter cost this task only the `session_id` binding it now needs for `detach`.

**Endings — Review Focus line 1.** The spec's five endings converge on one exit, and they collapse onto four `Ending` values because two of them are the same shutdown seen from different sides. This is the shape Plan 2C's Task 4 gave a live call's teardown.

| How it ends | What reaches the exit | `Ending` |
|---|---|---|
| the client closes cleanly | `StreamExt::next` yields `Some(Ok(Message::Close(_)))`, or `None` | `ClosedByClient` |
| the client vanishes mid-reply | `next` yields `Some(Err(_))` | `TransportError` |
| a socket read or write fails | the same arm, or `write_side`'s own error path | `TransportError` |
| the server raises a non-recoverable `Event::Error` | the event is queued for the client, then the loop leaves | `ServerStopped` |
| the server says `stop` — the session ends, or a `Cancel` is what it needs | `from_server.recv()` yields `None`; or the bridge sends `Command::Cancel` on the way out and drops `to_server`, and the session's own exit at `session.rs:203` runs `self.cancel_running()` (`session.rs:494`) | `ServerStopped` |
| a peer that stops reading | `write_side` gives up and closes the queue, or the socket's own write fails | `Saturated` |

The bridge never reaches into the session: `cancel_running` is private (`session.rs:494`) and the session's `run` calls it when its receiver closes. Half-close is not a case: `to_server` is dropped on the way out of `serve`, so the session sees the disconnect whether or not its last command was handled.

**Backpressure — Review Focus line 2.** The session's channels are `mpsc::unbounded` (`session.rs:57-58`) and a WebSocket has no backpressure of its own — the research note quotes MDN saying exactly that (`web-transport-and-terminal.md:1330-1334`). This bridge therefore owns the only bound on the path: a **bounded** `mpsc::channel` between the session and the socket, and a deadline on every socket write. When a peer stops reading, the bounded channel fills, the drain stops pulling from the session, and the socket write fails or times out — the connection ends instead of the server's memory growing. The socket's own second queue is the write buffer, whose ceiling is **unlimited by default** until `max_write_buffer_size` is set (`:1339-1344`); Task 1.4 sets it from `ConnOptions`.

**`sysmon` for a remote peer is the server's (W11).** The client already spawns its own sampler — `sysmon::spawn(ends.events)` at `client.rs:125` — so in `--connect` mode the *client* owns `Stats`. The rule, quoted from the decision table: "whoever owns the process the user is looking at owns `Stats` — the client in-process, the server for a remote or web peer" (spec §2, W11). This bridge therefore attaches **no** `sysmon`: exactly one source per connection, and for a web or remote peer it is the server's. A grep for `sysmon` in `conn.rs` must come back empty, and Task 1.7's tests assert the client side of the same rule.

- [ ] **Step 1: Write the failing test**

`crates/tipsy-compadre/src/server/conn.rs` is created with this test module at the bottom. It is an inline module rather than `tests/`: it drives `ConnOptions` and the test-only queue counter, and a file under `tests/` would force both public for no gain. The provider fakes are copied from `crates/tipsy-compadre/tests/session.rs:27-105` — this file owns its own copies, as `tests/brain.rs` and `tests/session.rs` already do — and they reach the bridge as the shared slot handed to `Hub::new`, so the hub under test is the real one and only the connectors are fakes.

```rust
#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use futures::{SinkExt, StreamExt};
    use tipsy_protocol::{ClientKind, Command, ErrorCode, Event, Frame, PROTOCOL_VERSION};
    use tokio::net::TcpStream;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
    use tokio_tungstenite::tungstenite::{Message as ClientMessage, Utf8Bytes};

    use super::*;
    use crate::audio::pcm::Pcm;
    use crate::keys::{KeyResolver, MemoryStore};
    use crate::providers::{
        ChatProvider, ChatRequest, ProviderError, ProviderSlot, Providers, SpeechProvider, SpeechRequest, TextStream,
        TranscribeProvider,
    };

    /// How long a test lets the server do its work before calling it dead.
    const PATIENCE: Duration = Duration::from_secs(5);

    // ---------- the provider fakes (`crates/tipsy-compadre/tests/session.rs:27-105`) ----------

    /// Chat fake: each call pops the next scripted outcome. `calls` is an `AtomicUsize` and not a
    /// `Mutex<Vec<_>>` because the disconnect test reads it while the session is running.
    struct FakeChat {
        script: Mutex<Vec<Result<Vec<&'static str>, ProviderError>>>,
        calls: AtomicUsize,
        chunk_delay: Duration,
    }

    impl FakeChat {
        fn new(script: Vec<Result<Vec<&'static str>, ProviderError>>) -> Arc<Self> {
            Self::slow(script, Duration::ZERO)
        }

        /// The same fake, sleeping `chunk_delay` before each streamed chunk.
        fn slow(script: Vec<Result<Vec<&'static str>, ProviderError>>, chunk_delay: Duration) -> Arc<Self> {
            Arc::new(Self { script: Mutex::new(script), calls: AtomicUsize::new(0), chunk_delay })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        /// Appends to the script, so a test can arm the *next* connection.
        fn push(&self, outcome: Result<Vec<&'static str>, ProviderError>) {
            self.script.lock().unwrap().push(outcome);
        }
    }

    #[async_trait]
    impl ChatProvider for FakeChat {
        async fn stream(&self, _req: ChatRequest) -> Result<TextStream, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let next = self.script.lock().unwrap().remove(0);
            let chunks = next?;
            let delay = self.chunk_delay;
            Ok(futures::stream::iter(chunks)
                .then(move |c| async move {
                    tokio::time::sleep(delay).await;
                    Ok(c.to_string())
                })
                .boxed())
        }
    }

    struct FakeSpeech;

    #[async_trait]
    impl SpeechProvider for FakeSpeech {
        async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError> {
            Ok(Pcm { samples: vec![0.25; req.text.chars().count()], sample_rate: 24000 })
        }
    }

    struct FakeTranscribe;

    #[async_trait]
    impl TranscribeProvider for FakeTranscribe {
        async fn transcribe(&self, _pcm: Pcm, _language: &str) -> Result<String, ProviderError> {
            Ok("¿qué pasa, compadre?".into())
        }
    }

    // ---------- helpers ----------

    /// A real configuration file in a tempdir, and its path. The path matters now: `Hub::new` takes
    /// it, because a `SetProfile` or an `UpdateConfig` persists through the same file the server
    /// loaded (spec §5.2). The built-in `compae` profile is what the greeting comes from.
    fn config() -> (tipsy_compadre::config::Config, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = tipsy_compadre::config::store::load_or_create(&path).unwrap().config;
        (config, path)
    }

    /// A resolver with a placeholder for each key name the default configuration asks for.
    ///
    /// **These are not secrets and this test must not depend on one existing.** The shared slot is
    /// filled by `build_providers` from the configuration's key *names* (`model.rs:104`, `:113`,
    /// `:147-150`) when the test's caller builds it; without a value for each name the build answers
    /// `MissingKey` and the server cannot start at all on a machine that has never run `key set`.
    /// The values never leave this process and never reach a provider.
    fn keys() -> KeyResolver {
        let store = MemoryStore::default();
        for name in ["openrouter", "openai", "elevenlabs"] {
            store.set(name, "test-placeholder-not-a-secret").expect("a memory store never fails");
        }
        KeyResolver::new(Box::new(store), Box::new(|_| None))
    }

    /// The registry a test's route shares, built on the fakes. The **shared slot** is the fourth
    /// argument of `Hub::new`, so `Hub::spawn` clones the fakes into each session and the only
    /// difference between this and a plain `serve` is which connectors sit in that slot. The policy
    /// is the connection's, and this listener is loopback with no token, so the permissive one is
    /// what the tests pass.
    #[derive(Clone)]
    struct ServeTest {
        hub: Hub,
        options: ConnOptions,
    }

    /// The `/ws` route's own handler. The `WebSocketUpgrade` extractor takes the last slot and
    /// `State` the one before it, which is how Task 1.4's `ws` handler is written too.
    async fn handler_with_state(
        axum::extract::State(state): axum::extract::State<ServeTest>,
        upgrade: axum::extract::ws::WebSocketUpgrade,
    ) -> axum::response::Response {
        let hub = state.hub.clone();
        let options = state.options;
        upgrade
            .write_buffer_size(options.write_buffer_size)
            .max_write_buffer_size(options.max_write_buffer_size)
            .on_upgrade(move |socket| async move {
                // The hub's entry point: it allocates the session id, attaches the session before
                // `Welcome` can be missed, and clones the fakes in its slot into the session. This
                // listener is on loopback and holds no token, so the policy needs neither.
                let policy = SessionPolicy { require_token: false, token: None, loopback: true };
                let ends = hub.spawn(policy);
                serve_with(socket, ends, hub.clone(), ClientKind::Web, options).await;
            })
    }

    /// Starts the listener and returns its address. The listener is bound before this returns, so a
    /// connect from the test cannot race it.
    async fn server(chat: Arc<FakeChat>, options: ConnOptions) -> SocketAddr {
        let (config, config_path) = config();
        let state = ServeTest {
            hub: Hub::new(
                config,
                config_path,
                keys(),
                ProviderSlot::new(Providers {
                    chat,
                    speech: Arc::new(FakeSpeech),
                    transcribe: Arc::new(FakeTranscribe),
                }),
            ),
            options,
        };
        let app = axum::Router::new()
            .route("/ws", axum::routing::get(handler_with_state))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        addr
    }

    /// Connects the real `tokio-tungstenite` 0.30 client and sends `Hello`, which every client sends
    /// first (spec §4.1). `ClientMessage` below is tungstenite's enum; the server side is axum's.
    async fn connect(addr: SocketAddr) -> WebSocketStream<MaybeTlsStream<TcpStream>> {
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.unwrap();
        send_command(
            &mut socket,
            Command::Hello { token: None, client: ClientKind::Web, protocol_version: PROTOCOL_VERSION },
        )
        .await;
        socket
    }

    async fn send_command(socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>, command: Command) {
        let text = Utf8Bytes::from(serde_json::to_string(&command).unwrap());
        socket.send(ClientMessage::Text(text)).await.expect("the socket must stay open");
    }

    /// The first event matching `wanted`, with everything received on the way there handed back in
    /// order. Not a fixed sleep: this is the polling-with-timeout idiom of
    /// `crates/tipsy-compadre/tests/realtime.rs:149-179`.
    async fn events_until(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        wanted: impl Fn(&Event) -> bool,
    ) -> (Vec<Event>, Event) {
        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(!left.is_zero(), "no matching event inside {PATIENCE:?}; received {seen:#?}");
            let message = tokio::time::timeout(left, socket.next())
                .await
                .expect("no matching event inside the patience window")
                .expect("the socket must stay open")
                .expect("the socket must not error");
            if let ClientMessage::Text(text) = message {
                let event: Event = serde_json::from_str(&text).expect("the server sends events as JSON");
                if wanted(&event) {
                    return (seen, event);
                }
                seen.push(event);
            }
        }
    }

    /// The same, for one event, discarding what came before it.
    async fn event_until(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        wanted: impl Fn(&Event) -> bool,
    ) -> Event {
        events_until(socket, wanted).await.1
    }

    fn is_welcome(event: &Event) -> bool {
        matches!(event, Event::Welcome { .. })
    }

    fn is_done(event: &Event) -> bool {
        matches!(event, Event::Done { .. })
    }

    /// The greeting is a turn of its own and it runs before anything else (`session.rs:150-154`), so
    /// every test waits it out before asking for the floor.
    async fn wait_for_the_floor(socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>) {
        event_until(socket, is_welcome).await;
        event_until(socket, is_done).await;
    }

    // ---------- tests ----------

    #[tokio::test]
    async fn ask_in_reply_out_over_a_real_socket() {
        let chat = FakeChat::new(vec![Ok(vec!["Vamos allá, ", "compadre."])]);
        let addr = server(chat.clone(), ConnOptions::default()).await;
        let mut socket = connect(addr).await;
        wait_for_the_floor(&mut socket).await;

        send_command(&mut socket, Command::Ask { text: "¿qué tal?".into() }).await;
        let (seen, reply) = events_until(&mut socket, |e| matches!(e, Event::Reply { .. })).await;
        match reply {
            Event::Reply { text } => assert_eq!(text, "Vamos allá, compadre."),
            other => panic!("`Ask` produces a reply, got {other:?}"),
        }
        let deltas: String = seen
            .iter()
            .filter_map(|e| match e {
                Event::ReplyDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, "Vamos allá, compadre.", "the deltas carry the same text the reply does");
        assert_eq!(chat.calls(), 1, "one question is one chat call");
    }

    #[tokio::test]
    async fn speech_frames_come_down_as_binary_frames() {
        let chat = FakeChat::new(vec![]);
        let addr = server(chat, ConnOptions::default()).await;
        let mut socket = connect(addr).await;
        event_until(&mut socket, is_welcome).await;

        // The greeting is spoken through `Frame::Speech`; two frames prove the binary path and that
        // a frame is not JSON.
        let mut decoded = 0usize;
        let deadline = tokio::time::Instant::now() + PATIENCE;
        while decoded < 2 {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(!left.is_zero(), "the greeting speaks at least two frames");
            let message = tokio::time::timeout(left, socket.next())
                .await
                .expect("a speech frame inside the patience window")
                .expect("the socket must stay open")
                .expect("the socket must not error");
            if let ClientMessage::Binary(bytes) = message {
                let frame = Frame::decode(&bytes).expect("a binary frame is a protocol frame");
                assert!(matches!(frame, Frame::Speech { .. }), "the server sends speech, got {frame:?}");
                decoded += 1;
            }
        }
    }

    #[tokio::test]
    async fn a_malformed_text_frame_is_an_error_event_and_the_connection_survives() {
        let chat = FakeChat::new(vec![Ok(vec!["sigo aquí."])]);
        let addr = server(chat, ConnOptions::default()).await;
        let mut socket = connect(addr).await;
        wait_for_the_floor(&mut socket).await;

        socket.send(ClientMessage::Text(Utf8Bytes::from("{not json"))).await.unwrap();
        match event_until(&mut socket, |e| matches!(e, Event::Error { .. })).await {
            Event::Error { code, message, recoverable } => {
                assert!(message.contains("bad_message"), "spec §10 names it: {message}");
                assert_eq!(code, ErrorCode::Internal, "`ErrorCode` has no `bad_message` variant");
                assert!(recoverable, "a bad message is recoverable: the connection stays open");
            }
            other => panic!("a malformed frame is an error event, got {other:?}"),
        }

        // The same connection still works, which is what "recoverable" means.
        send_command(&mut socket, Command::Ask { text: "¿sigues?".into() }).await;
        event_until(&mut socket, |e| matches!(e, Event::Reply { .. })).await;
    }

    #[tokio::test]
    async fn a_binary_frame_that_is_not_audio_is_an_error_event_too() {
        let chat = FakeChat::new(vec![]);
        let addr = server(chat, ConnOptions::default()).await;
        let mut socket = connect(addr).await;
        event_until(&mut socket, is_welcome).await;

        // 0x7f is neither `KIND_MIC` (0x01) nor `KIND_SPEECH` (0x02), so `Frame::decode` answers
        // `FrameError::UnknownKind` (`tipsy-protocol/src/lib.rs:238`).
        socket.send(ClientMessage::Binary(vec![0x7f, 0x00].into())).await.unwrap();
        match event_until(&mut socket, |e| matches!(e, Event::Error { .. })).await {
            Event::Error { message, recoverable, .. } => {
                assert!(message.contains("bad_message"), "{message}");
                assert!(recoverable, "not a panic, and not a dropped connection (spec §10)");
            }
            other => panic!("an unknown frame kind is an error event, got {other:?}"),
        }
    }

    /// Review Focus line 2, first half: the bridge's own queue is **bounded**. `write_timeout` is
    /// zero here on purpose — it makes `write_side` give up on its first message deterministically,
    /// so the queue's receiver is dropped and nothing can drain it while the test measures. What
    /// the assertion is about is the depth the bridge may hold, not the clock.
    #[tokio::test]
    async fn the_write_side_queue_is_bounded() {
        let options = ConnOptions { queue: 2, write_timeout: Duration::ZERO, ..ConnOptions::default() };
        let chat = FakeChat::slow(
            vec![Ok(vec!["uno ", "dos ", "tres ", "cuatro ", "cinco ", "seis ", "siete ", "ocho ", "nueve ", "diez "])],
            Duration::from_millis(2),
        );
        let addr = server(chat, options).await;
        let mut socket = connect(addr).await;

        // The greeting alone is more than the queue holds, so the bound is engaged before the `Ask`
        // even arrives. The client is never polled after the connect: it has stopped reading.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let queued = queued_on_the_write_side();
        assert!(
            queued <= options.queue + 1,
            "the write side must hold at most its capacity plus the message in flight, held {queued}"
        );

        // And the connection really did end, rather than the bound being satisfied by a server that
        // simply stopped: the client sees the close.
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(!left.is_zero(), "the writer must have given up and closed the connection");
            match tokio::time::timeout(left, socket.next()).await {
                Ok(Some(Ok(_))) => continue,
                Ok(Some(Err(_))) | Ok(None) => break,
                Err(_) => panic!("the connection never closed after the writer gave up"),
            }
        }
    }

    /// Review Focus line 2, second half: the socket's own ceiling. The greeting is far more than the
    /// 4 KiB `max_write_buffer_size` set here and the client never reads, so this must be a closed
    /// connection and not a server that queues for ever. `max_write_buffer_size` is axum's
    /// **unlimited** default until it is set (`web-transport-and-terminal.md:1339-1344`); this is
    /// the test that shows the ceiling has an effect.
    #[tokio::test]
    async fn a_peer_that_stops_reading_ends_the_connection_instead_of_growing() {
        let options = ConnOptions {
            queue: 1,
            write_timeout: Duration::from_millis(200),
            write_buffer_size: 1024,
            max_write_buffer_size: 4096,
        };
        let chat = FakeChat::new(vec![Ok(vec!["una frase bastante larga para llenar el búfer."])]);
        let addr = server(chat, options).await;
        let mut socket = connect(addr).await;
        event_until(&mut socket, is_welcome).await;

        // The client never reads again. What the assertions are after is the end, not the traffic.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(!left.is_zero(), "a peer that stops reading must not keep the connection alive for ever");
            match tokio::time::timeout(left, socket.next()).await {
                Ok(Some(Ok(_))) => continue,
                Ok(Some(Err(_))) | Ok(None) => break,
                Err(_) => panic!("the write never gave up; the connection is still open"),
            }
        }
    }

    /// Review Focus line 1: a reply in flight when the client vanishes. The turn is cancelled — the
    /// effect of `cancel_running` (`session.rs:494`) reached through the session's own exit
    /// (`session.rs:203`) — and the listener goes on serving.
    #[tokio::test]
    async fn a_disconnect_cancels_the_turn_and_the_listener_answers_the_next_connection() {
        // Two chunks 150 ms apart: a turn cancelled after the first never produces the second, and a
        // turn left running would make a second chat call.
        let chat = FakeChat::slow(vec![Ok(vec!["primera ", "segunda"])], Duration::from_millis(150));
        let addr = server(chat.clone(), ConnOptions::default()).await;
        let mut socket = connect(addr).await;
        wait_for_the_floor(&mut socket).await;
        chat.push(Ok(vec!["aquí sigo."]));

        send_command(&mut socket, Command::Ask { text: "¿vienes?".into() }).await;
        event_until(&mut socket, |e| matches!(e, Event::ReplyDelta { .. })).await;
        let calls_before = chat.calls();
        assert_eq!(calls_before, 1, "the first connection made exactly one chat call");

        // The client vanishes mid-sentence.
        drop(socket);
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(chat.calls(), calls_before, "a cancelled turn starts no further chat call");

        // And the listener still answers the next connection, which is the half of Review Focus
        // line 1 that a leaked or panicking session would fail.
        let mut second = connect(addr).await;
        wait_for_the_floor(&mut second).await;
        send_command(&mut second, Command::Ask { text: "¿estás?".into() }).await;
        event_until(&mut second, |e| matches!(e, Event::Reply { .. })).await;
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre conn::tests`
Expected: FAIL to compile with `error[E0425]: cannot find function serve in this scope`, `cannot find function serve_with`, `cannot find function queued_on_the_write_side`, `cannot find type ConnOptions`, and `error[E0599]: no method named spawn found for struct Hub` (until Task 1.3's `spawn` and Task 1.2's `session_id` land). The bridge does not exist yet.

- [ ] **Step 3: Implement the bridge, one exit and two bounds**

```rust
//! One connection: the socket on one side, a session's channels on the other, and no way out that
//! skips the teardown.
//!
//! Every `Message` here is **axum's**: `Text(Utf8Bytes) | Binary(Bytes) | Ping(Bytes) | Pong(Bytes)
//! | Close(Option<CloseFrame>)`, with no `Frame` variant and with ping and the close handshake
//! handled by axum itself (`web-transport-and-terminal.md:203-228`). It is not
//! `tokio_tungstenite::tungstenite::Message`, which the client side uses and which has a `Frame`
//! variant: axum 0.8.9 pins `tokio-tungstenite ^0.29` while the native client keeps 0.30, so both
//! enums are compiled into this binary (W1, `web-transport-and-terminal.md:161-167`).

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures::stream::StreamExt;
use tipsy_protocol::{Command, ErrorCode, Event, Frame, ToClient, ToServer};
use tokio::sync::mpsc;

use crate::server::hub::Hub;
use crate::session::SessionEnds;

/// How many messages may sit between the session's channel and the socket. Past this the drain stops
/// pulling from the session and blocks, which is what carries the pressure back to the producer.
pub const WRITE_QUEUE: usize = 32;

/// The two bounds, in one place: how long a socket write may take, and how much the bridge and the
/// socket may hold.
#[derive(Clone, Copy, Debug)]
pub struct ConnOptions {
    /// How long one socket write may take before the connection is over. A peer that has stopped
    /// reading is what this catches.
    pub write_timeout: Duration,
    /// The depth of the bridge's own queue; [`WRITE_QUEUE`] by default.
    pub queue: usize,
    /// The socket's write buffer, and its ceiling. axum's defaults are 128 KiB and **unlimited**
    /// (`web-transport-and-terminal.md:1339-1344`); the ceiling here is finite because a LAN tool
    /// may not grow with a peer that stopped reading.
    pub write_buffer_size: usize,
    pub max_write_buffer_size: usize,
}

impl Default for ConnOptions {
    fn default() -> Self {
        Self {
            write_timeout: Duration::from_secs(5),
            queue: WRITE_QUEUE,
            write_buffer_size: 128 * 1024,
            max_write_buffer_size: 1024 * 1024,
        }
    }
}

/// Why a connection ended. Four ways in, one way out.
#[derive(Debug, PartialEq, Eq)]
pub enum Ending {
    /// The client sent a close frame, or the socket closed under it.
    ClosedByClient,
    /// The session stopped producing, or raised a non-recoverable `Error`.
    ServerStopped,
    /// A socket read or write failed.
    TransportError,
    /// The peer stopped reading long enough that the queue filled and a write gave up.
    Saturated,
}

/// What one inbound frame came to.
enum Inbound {
    /// Handled: a ping, a pong, a close, or a malformed frame the bridge already answered.
    Nothing,
    /// Something for the client. It goes through the same bounded queue a reply does, so it cannot
    /// overtake one.
    Send(ToClient),
    /// The session's end is gone: the loop must leave.
    Gone,
}

/// Serves one connection to its end, with the default bounds. Task 1.4's `/ws` handler calls this.
pub async fn serve(socket: WebSocket, ends: SessionEnds, hub: Hub, client: tipsy_protocol::ClientKind) -> Ending {
    serve_with(socket, ends, hub, client, ConnOptions::default()).await
}

/// The same bridge with the bounds chosen — the shape the tests drive.
pub async fn serve_with(
    socket: WebSocket,
    ends: SessionEnds,
    hub: Hub,
    client: tipsy_protocol::ClientKind,
    options: ConnOptions,
) -> Ending {
    // `session_id` is the id the hub allocated and `Welcome` carries. `Hub::spawn` already attached
    // this session, before anything was sent to the client, so nothing here attaches it again — the
    // id is bound here only so the one exit can hand it back to `detach`.
    let SessionEnds { to_server, mut from_server, session_id, .. } = ends;
    tracing::debug!(?client, "a client connected to /ws");

    let (queue_tx, queue_rx) = mpsc::channel::<Message>(options.queue.max(1));
    let (sink, mut stream) = socket.split();
    let mut writer = tokio::spawn(write_side(sink, queue_rx, options.write_timeout));

    let mut ending = None;
    while ending.is_none() {
        tokio::select! {
            // Biased: a socket that has already failed is noticed before the next server message is
            // pulled, so a dead peer cannot make the bridge drain one more message.
            biased;
            incoming = stream.next() => match incoming {
                None => ending = Some(Ending::ClosedByClient),
                Some(Err(e)) => {
                    tracing::debug!(error = %e, "the socket read failed");
                    ending = Some(Ending::TransportError);
                }
                Some(Ok(Message::Close(_))) => ending = Some(Ending::ClosedByClient),
                // axum answers ping and the close handshake itself (`:221-228`).
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                Some(Ok(message)) => match inbound(message, &to_server) {
                    Inbound::Nothing => {}
                    Inbound::Gone => ending = Some(Ending::Saturated),
                    Inbound::Send(message) => {
                        if queue_tx.send(encode(message)).await.is_err() {
                            ending = Some(Ending::Saturated);
                        }
                    }
                },
            },
            message = from_server.recv() => match message {
                None => ending = Some(Ending::ServerStopped),
                Some(ToClient::Event(event)) => {
                    // A non-recoverable error is the server asking for the connection: the event is
                    // queued for the client first, and then the loop leaves.
                    if matches!(&event, Event::Error { recoverable: false, .. }) {
                        ending = Some(Ending::ServerStopped);
                    }
                    if queue_tx.send(encode(ToClient::Event(event))).await.is_err() {
                        ending = Some(Ending::Saturated);
                    }
                }
                Some(ToClient::Audio(frame)) => {
                    if queue_tx.send(encode(ToClient::Audio(frame))).await.is_err() {
                        ending = Some(Ending::Saturated);
                    }
                }
            },
            // The writer gave up on a peer that stopped reading.
            finished = &mut writer => {
                let _ = finished;
                ending = Some(Ending::Saturated);
            }
        }
    }

    // **One exit**, in this order: ask the session to cancel its in-flight turn and then let it end
    // by dropping the last sender, forget it in the registry, close the socket and let the writer
    // drain. `Session::run` calls `self.cancel_running()` (`session.rs:203`, `:494`) when its
    // receiver closes, and the writer owns the only other end of the queue.
    let _ = to_server.send(ToServer::Command(Command::Cancel));
    drop(to_server);
    hub.detach(session_id);
    let _ = queue_tx.send(Message::Close(None)).await;
    drop(queue_tx);
    let _ = writer.await;
    drop(stream);
    ending.unwrap_or(Ending::ClosedByClient)
}

/// Decodes one inbound frame and hands it to the session. A malformed command or audio frame is an
/// `Error` event on the wire and **not** a panic and **not** a dropped connection (spec §10).
async fn inbound(message: Message, to_server: &mpsc::UnboundedSender<ToServer>) -> Inbound {
    match message {
        // axum answers ping, pong and the close handshake itself; the close is handled by the loop.
        Message::Ping(_) | Message::Pong(_) | Message::Close(_) => Inbound::Nothing,
        Message::Text(text) => match serde_json::from_str::<Command>(&text) {
            Ok(command) => {
                if to_server.send(ToServer::Command(command)).is_err() {
                    Inbound::Gone
                } else {
                    Inbound::Nothing
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "a text frame that is not a command");
                Inbound::Send(ToClient::Event(bad_message("the frame is not a command")))
            }
        },
        Message::Binary(bytes) => match Frame::decode(&bytes) {
            Ok(frame) => {
                if to_server.send(ToServer::Audio(frame)).is_err() {
                    Inbound::Gone
                } else {
                    Inbound::Nothing
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "a binary frame that is not audio");
                Inbound::Send(ToClient::Event(bad_message("the frame is not an audio frame")))
            }
        },
    }
}

/// Spec §10's malformed-message row: `code: "bad_message", recoverable: true`. `ErrorCode` has no
/// `bad_message` variant (`tipsy-protocol/src/lib.rs:51-64`), so the code travels in `message` and
/// the enum carries the honest class.
fn bad_message(what: &'static str) -> Event {
    Event::Error { code: ErrorCode::Internal, message: format!("bad_message: {what}"), recoverable: true }
}

/// The one place a `ToClient` becomes a socket frame: events are JSON text, audio is binary
/// (`tipsy-protocol/src/lib.rs:1-3`).
fn encode(message: ToClient) -> Message {
    match message {
        ToClient::Event(event) => {
            Message::Text(serde_json::to_string(&event).expect("an event serializes").into())
        }
        ToClient::Audio(frame) => Message::Binary(frame.encode().into()),
    }
}

/// The writer half: one socket write at a time, each with a deadline. A send that does not finish in
/// `write_timeout` ends this task; that drops the queue's receiver, which is what stops the drain on
/// the other side and so carries the pressure back to the session.
async fn write_side<S>(mut sink: S, mut queue: mpsc::Receiver<Message>, write_timeout: Duration)
where
    S: futures::Sink<Message> + Unpin,
{
    while let Some(message) = queue.recv().await {
        // Held across the write: the count is the queue's depth plus the message in flight, and
        // never more than the queue's capacity plus one.
        let _in_flight = InFlight::take();
        match tokio::time::timeout(write_timeout, futures::SinkExt::send(&mut sink, message)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "the socket write failed");
                return;
            }
            Err(_) => {
                tracing::debug!(?write_timeout, "the peer stopped reading; closing the connection");
                return;
            }
        }
    }
}

/// One message the writer has taken off the queue and not finished sending.
struct InFlight(());

impl InFlight {
    fn take() -> Self {
        #[cfg(test)]
        QUEUED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(())
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        #[cfg(test)]
        QUEUED.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The messages the write side is holding right now, for the test that proves the bound engages. The
/// queue is bounded, so this number is bounded; `the_write_side_queue_is_bounded` asserts exactly
/// that.
#[cfg(test)]
static QUEUED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn queued_on_the_write_side() -> usize {
    QUEUED.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(test)]
mod tests { /* the module written in Step 1 */ }
```

Three mechanical notes. `socket.split()` needs `futures::stream::StreamExt` in scope and axum's `WebSocket` is a `Stream + Sink`; `futures::SinkExt::send` is called by its fully-qualified path in `write_side` so the `StreamExt` import alone is enough. `axum::extract::ws::WebSocketUpgrade` — not `WebSocket` — is what Task 1.4's handler takes, and `serve` is what it calls. And the session id is read out of `SessionEnds` — `let SessionEnds { to_server, mut from_server, session_id, .. } = ends;` — and handed back to `hub.detach(session_id)` on the one exit; nothing in this file allocates one, and nothing calls `attach`.

`tracing::debug!` is called above because W12 brings `tracing` and `tracing-subscriber` in with the server rather than with milestone 11 (`spec §2`, `spec §9.6`); the manifest entries and the subscriber bootstrap belong to the logging task, and if the macros do not resolve yet, that task adds the dependency rather than these calls being deleted.

- [ ] **Step 4: Run the test to verify it passes**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre conn::tests`
Expected: PASS, `7 passed; 0 failed`. If `a_peer_that_stops_reading_ends_the_connection_instead_of_growing` is flaky, this machine's loopback buffers are larger than the test's 4 KiB ceiling: raise the amount of greeting text rather than the test's patience, because the point is the ceiling and not the clock.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/server/conn.rs
git commit -m "feat(server): bridge a socket to a session, bounded, with one exit" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 1.7: `--connect` and the `serve` subcommand `[mid]`

Spec §3.2, §6, §4.2, W3, W6, W9, W11. This is the task that makes the tier usable from a terminal:
`serve` starts the server Task 1.4 built, `--connect` reaches it from another machine, and the
configuration screen stops being a native-only feature.

**Files:**
- Create: `crates/tipsy-compadre/src/client/bridge.rs`
- Create: `crates/tipsy-compadre/tests/connect.rs`
- Modify: `crates/tipsy-compadre/src/main.rs:16-21` (the `Cli` struct), `:23-35` (`Cmd`), `:53-59`
  (`main`), `:61-88` (`run_client`), `:90-98` (`run_terminal`)
- Modify: `crates/tipsy-compadre/src/client.rs:26-31` (the imports), `:37-59` (`run`), `:61-107` (the
  `Client` struct and `new`), `:117-125` (the head of `frame_loop`), `:196-225` (`on_message`),
  `:333-409` (`on_action`), `:411-434` (`open_screen`), `:436-452` (`on_config_effect`),
  `:454-503` (`test_provider`), `:505-539` (`save`)
- Modify: `crates/tipsy-compadre/src/providers/mod.rs:251` (`build_providers`) and `:286`
  (`build_one`) — one `#[cfg(test)]` counter line each, plus its two items near them
- Test: `crates/tipsy-compadre/tests/connect.rs` (its own file: the socket side)
- Test: `crates/tipsy-compadre/src/client/bridge.rs` (its own `#[cfg(test)] mod tests`: the backoff)
- Test: `crates/tipsy-compadre/src/client.rs` (its existing `#[cfg(test)] mod tests` at `:724`: the
  provider audit, added to it)
- Test: `crates/tipsy-compadre/src/main.rs` (a new `#[cfg(test)] mod tests`: the CLI)

**Interfaces:**
- Consumes: `spawn_with_events(config: Config, providers: ProviderSlot, brain_config: BrainConfig) -> SessionEnds` and `SessionEnds { to_server: UnboundedSender<ToServer>, from_server: UnboundedReceiver<ToClient>, events: UnboundedSender<ToClient> }` (`session.rs:56`, `:46-52`); `Command`/`Event`/`Frame`/`ToServer`/`ToClient`, `ClientKind::Tui`, `PROTOCOL_VERSION` (`tipsy-protocol/src/lib.rs:7`, `:11`, `:108`, `:133`, `:174`, `:244`, `:251`); `Command::GetConfig`, `Command::UpdateConfig { config: ConfigView }`, `Command::TestProvider { capability }`, `Event::Config { config: ConfigView }`, `Event::ConfigChanged` (Task 1.1); `tipsy_ui::{ConfigForm, ConfigEffect, ConfigAction, UiAction, NoticeKind}` and `form_of`/`config_from` (`config/form.rs:21`, `:81`); **`ConfigForm::from(ConfigView)` and `ConfigView::from(&ConfigForm)`** — the screen's form and the wire's view are two shapes with one conversion each way (§4.3), so both directions of this seam are explicit; `ConfigView::redacted(self) -> Self` (Task 1.1), which is the *server's* rule for what a client receives and which this task never calls; `server::run(settings: Settings) -> Result<(), ServerError>` and the pinned `Settings` (Task 1.4)
- Produces:
  - `client::bridge::connect(url: &str, token: Option<&str>) -> Result<SessionEnds, BridgeError>` — `pub async fn`. It connects once, sends `Hello` as the first frame, waits for `Welcome`, and only then returns; every failure before that is an `Err`, so a refused attach leaves no partial connection (success criterion 2). The returned `SessionEnds` is the same three fields `spawn_with_events` returns, fed from a socket
  - `client::bridge::BridgeError` — `pub enum`, `thiserror`: `Unreachable { url: String, reason: String }`, `Refused { code: String, message: String }` (`code` is the wire spelling of `Event::Error.code`, read back through serde so this variant needs no list of codes of its own; the variants themselves are Task 1.1's), `NoWelcome`
  - the reconnection, pinned: after `connect` returns, one tokio task owns the socket, `select!`s between it and `to_server`, and on a close or transport error injects `ToClient::Event(Event::Error { code: ErrorCode::Network, message: "<url>: connection lost; reconnecting", recoverable: true })` into the view channel **once per loss**, waits `backoff(n)`, reconnects, re-sends `Hello`, waits for `Welcome`, and then resumes. `backoff(n) = 100 ms << min(n, 5)`, capped at 3.2 s (`n` counts consecutive failures at 0; the schedule is `100, 200, 400, 800, 1600, 3200, 3200, …`). A refusal is **never** retried (spec §9.1). Between the loss and the next `Welcome` the task keeps draining `to_server` and drops what it drains: the peer that comes back is a new session with a new `session_id` (W9), and an `Ask` replayed into it would answer a question the server never heard, while a `Frame::Mic` belongs to a session that is gone
  - `client::run_remote(url: String, token: Option<String>) -> Result<(), String>` — `pub async fn`, the `--connect` entry point `main.rs` awaits
  - `Attachment` — `enum`, **private to `client.rs`**: `InProcess` or `Remote { url: String }` (the token is not stored: it went into `Hello` and nothing else needs it, and a field nothing reads is a `dead_code` warning)
  - `Client::remote(config: Config, config_path: PathBuf, url: String) -> Self` — private; `Client::new` keeps its four parameters and its behaviour
  - `providers::builds_on_this_thread() -> usize` — `#[cfg(test)] pub fn`, the audit instrument (see Step 1)
  - CLI: `Cli { connect: Option<String>, token: Option<String>, verbose: bool, command: Option<Cmd> }` and `Cmd::Serve { bind: Option<String> }`

- [ ] **Step 1: Write the failing test**

```rust
// ---------------------------------------------------------------- crates/tipsy-compadre/tests/connect.rs
//! `--connect`: the client-side bridge, against a real socket server on `127.0.0.1:0`.
//!
//! The socket side is the technique `tests/realtime.rs:85-248` established — a `TcpListener` plus
//! `accept_hdr_async` — because `wiremock` speaks HTTP and this is a duplex WebSocket. The server
//! half of the protocol is Tasks 1.4 and 1.6; what is under test here is the client.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tipsy_compadre::client::bridge;
use tipsy_protocol::{
    ClientKind, Command, ErrorCode, Event, Frame, ListeningMode, PROTOCOL_VERSION, ProfileView, ToClient, ToServer,
    UiLanguage,
};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

/// What the fake server does on one accepted connection, after the client's `Hello`.
#[derive(Clone)]
enum Script {
    /// Answer `Hello` with `Welcome`, then send one `Event::Reply` and one binary speech frame, then
    /// keep reading and record every client frame.
    Serve,
    /// Answer `Hello` with `Welcome`, then drop the TCP connection with no close frame.
    Drop,
    /// Answer `Hello` with `Error { code, recoverable: false }` and close, as a refused attach ends.
    Refuse { code: ErrorCode, message: &'static str },
}

/// One accepted connection, as the test sees it: the request target, the `Hello`, and everything the
/// client sent afterwards.
#[derive(Default)]
struct Seen {
    target: String,
    hello: Option<String>,
    texts: Vec<String>,
    binaries: Vec<Vec<u8>>,
}

/// The `accept_hdr_async` hook: it records the request line so a test can prove the token is not in
/// it (spec §9.2). A type rather than a closure only so the `allow` sits on the exact function
/// clippy complains about — tungstenite's callback returns the whole `ErrorResponse` by value, as
/// `tests/realtime.rs:95-109` already records.
struct RecordTarget(Arc<Mutex<Seen>>);

impl tokio_tungstenite::tungstenite::handshake::server::Callback for RecordTarget {
    #[allow(clippy::result_large_err)]
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        self.0.lock().unwrap().target = request.uri().to_string();
        Ok(response)
    }
}

struct FakeServer {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Arc<Mutex<Seen>>>>>,
}

impl FakeServer {
    /// Binds a real listener on a random loopback port. The `n`-th accepted connection runs
    /// `scripts[n]`; a connection with no script serves like `Script::Serve`.
    async fn start(scripts: Vec<Script>) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen: Arc<Mutex<Vec<Arc<Mutex<Seen>>>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        tokio::spawn(async move {
            let mut accepted = 0usize;
            while let Ok((stream, _)) = listener.accept().await {
                let script = scripts.get(accepted).cloned().unwrap_or(Script::Serve);
                accepted += 1;
                let slot = Arc::new(Mutex::new(Seen::default()));
                recorded.lock().unwrap().push(slot.clone());
                tokio::spawn(async move {
                    let Ok(socket) = tokio_tungstenite::accept_hdr_async(stream, RecordTarget(slot.clone())).await
                    else {
                        return;
                    };
                    serve(socket, script, slot).await;
                });
            }
        });
        FakeServer { addr, seen }
    }

    fn url(&self) -> String {
        format!("ws://{}/ws", self.addr)
    }

    fn connections(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// The request target of the `index`-th connection, once the upgrade has been seen.
    async fn target(&self, index: usize) -> String {
        self.poll(|seen| (!seen.target.is_empty()).then(|| seen.target.clone()), index, "request").await
    }

    /// The `Hello` of the `index`-th connection, as the raw JSON the client sent.
    async fn hello(&self, index: usize) -> String {
        self.poll(|seen| seen.hello.clone(), index, "Hello").await
    }

    /// Waits until the `index`-th connection has recorded at least `texts` text frames and
    /// `binaries` binary frames, then returns what it has.
    async fn frames(&self, index: usize, texts: usize, binaries: usize) -> (Vec<String>, Vec<Vec<u8>>) {
        for _ in 0..200 {
            {
                let all = self.seen.lock().unwrap();
                if let Some(seen) = all.get(index) {
                    let seen = seen.lock().unwrap();
                    if seen.texts.len() >= texts && seen.binaries.len() >= binaries {
                        return (seen.texts.clone(), seen.binaries.clone());
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the fake server never recorded {texts} text and {binaries} binary frames on connection {index}");
    }

    /// The polling idiom `tests/realtime.rs:149-179` uses: no fixed sleeps, no port races.
    async fn poll<T>(&self, read: impl Fn(&Seen) -> Option<T>, index: usize, what: &str) -> T {
        for _ in 0..200 {
            {
                let all = self.seen.lock().unwrap();
                if let Some(seen) = all.get(index)
                    && let Some(value) = read(&seen.lock().unwrap())
                {
                    return value;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the fake server never saw the {what} of connection {index}");
    }
}

/// The `Welcome` a server sends first: session 7, the profile the HUD shows, push mode.
fn welcome() -> Event {
    Event::Welcome {
        session_id: 7,
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: UiLanguage::Es,
        listening_mode: ListeningMode::Push,
        web_bundle: false,
        login: None,
    }
}

async fn serve(
    mut socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    script: Script,
    seen: Arc<Mutex<Seen>>,
) {
    // The client's first frame is its `Hello`; nothing may precede it (spec §4.1). Every frame here
    // is built from the real `Event`/`Frame` types, so this file cannot drift from the wire.
    let hello = loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => break Some(text.as_str().to_string()),
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => break None,
        }
    };
    let Some(hello) = hello else { return };
    seen.lock().unwrap().hello = Some(hello);

    match script {
        Script::Refuse { code, message } => {
            let error = Event::Error { code, message: message.into(), recoverable: false };
            let _ = socket.send(Message::text(serde_json::to_string(&error).unwrap())).await;
            let _ = socket.close(None).await;
        }
        Script::Drop => {
            let _ = socket.send(Message::text(serde_json::to_string(&welcome()).unwrap())).await;
            // No close frame: the client can only learn of this from the transport.
            drop(socket);
        }
        Script::Serve => {
            let _ = socket.send(Message::text(serde_json::to_string(&welcome()).unwrap())).await;
            let reply = Event::Reply { text: "Listo, quillo".into() };
            let _ = socket.send(Message::text(serde_json::to_string(&reply).unwrap())).await;
            let _ = socket.send(Message::binary(Frame::Speech { seq: 0, samples: vec![7, 8, 9] }.encode())).await;
            while let Some(Ok(message)) = socket.next().await {
                let mut seen = seen.lock().unwrap();
                match message {
                    Message::Text(text) => seen.texts.push(text.as_str().to_string()),
                    Message::Binary(bytes) => seen.binaries.push(bytes.to_vec()),
                    _ => {}
                }
            }
        }
    }
}

/// The next thing the bridge delivers, with a deadline: a bridge that stops shuttling must fail the
/// test rather than hang it.
async fn next(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ToClient>) -> ToClient {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("the bridge must deliver within five seconds")
        .expect("the bridge must still hold the channel")
}

/// The handshake and both channels: `Hello` first, an event and a reply out, a command in and the
/// microphone frames arriving as **binary** (spec §4.1, §4.5, §6.1).
#[tokio::test]
async fn connect_opens_with_a_hello_and_shuttles_both_channels() {
    let server = FakeServer::start(vec![Script::Serve]).await;
    let mut ends = bridge::connect(&server.url(), Some("s3cret")).await.expect("the fake server answers welcome");

    assert_eq!(
        server.target(0).await,
        "/ws",
        "the token travels in `Hello`, never as a query parameter (spec §9.2)"
    );
    match serde_json::from_str::<Command>(&server.hello(0).await).unwrap() {
        Command::Hello { token, client, protocol_version } => {
            assert_eq!(token.as_deref(), Some("s3cret"), "the token the CLI was given is what `Hello` carries");
            assert_eq!(client, ClientKind::Tui);
            assert_eq!(protocol_version, PROTOCOL_VERSION, "PROTOCOL_VERSION stays 1 (global constraints)");
        }
        other => panic!("the first frame must be `Hello`, got {other:?}"),
    }

    // Server → client, in the order the socket sent them.
    assert!(matches!(next(&mut ends.from_server).await, ToClient::Event(Event::Welcome { session_id: 7, .. })));
    assert!(matches!(next(&mut ends.from_server).await, ToClient::Event(Event::Reply { .. })));
    match next(&mut ends.from_server).await {
        ToClient::Audio(Frame::Speech { samples, .. }) => assert_eq!(samples, vec![7, 8, 9]),
        other => panic!("the speech frame must arrive as `ToClient::Audio`, got {other:?}"),
    }

    // Client → server: a command as a text frame, the microphone as a binary frame.
    ends.to_server.send(ToServer::Command(Command::Ask { text: "¿qué pasa?".into() })).unwrap();
    ends.to_server.send(ToServer::Audio(Frame::Mic(vec![1, 2, 3]))).unwrap();
    let (texts, binaries) = server.frames(0, 1, 1).await;
    assert!(
        matches!(serde_json::from_str::<Command>(&texts[0]).unwrap(), Command::Ask { .. }),
        "a `Command` is a JSON text frame: {}",
        texts[0]
    );
    assert_eq!(
        Frame::decode(&binaries[0]).unwrap(),
        Frame::Mic(vec![1, 2, 3]),
        "microphone audio must arrive as a binary frame, byte for byte as `Frame::encode` wrote it"
    );
}

/// A server that drops the socket is reported once and then retried, and the peer that comes back
/// says `Hello` again — because it is a new session (v1 spec §12, W9).
#[tokio::test]
async fn connect_reconnects_with_backoff_after_the_server_drops_the_socket() {
    let server = FakeServer::start(vec![Script::Drop, Script::Serve]).await;
    let mut ends = bridge::connect(&server.url(), None).await.expect("the first connection answers welcome");

    match next(&mut ends.from_server).await {
        ToClient::Event(Event::Error { code, recoverable, message }) => {
            assert_eq!(code, ErrorCode::Network);
            assert!(recoverable, "a dropped socket is retried, so the client must not be told it is final");
            assert!(message.contains("reconnect"), "{message}");
        }
        other => panic!("the loss of the socket must be reported, got {other:?}"),
    }

    match serde_json::from_str::<Command>(&server.hello(1).await).unwrap() {
        Command::Hello { protocol_version, .. } => assert_eq!(protocol_version, PROTOCOL_VERSION),
        other => panic!("a reconnected bridge must say `Hello` again, got {other:?}"),
    }
    assert!(
        matches!(next(&mut ends.from_server).await, ToClient::Event(Event::Welcome { .. })),
        "and it is attached again: the second connection's `Welcome` reaches the HUD"
    );
}

/// A refused attach is fatal, once, with the server's own words: a mismatch is never retried
/// (spec §9.1).
#[tokio::test]
async fn connect_does_not_retry_a_refusal() {
    let server = FakeServer::start(vec![Script::Refuse {
        code: ErrorCode::Auth,
        message: "the token is not the one this server has",
    }])
    .await;

    let error = bridge::connect(&server.url(), Some("wrong")).await.expect_err("a refusal is fatal");
    assert!(matches!(&error, bridge::BridgeError::Refused { .. }), "{error:?}");
    let text = error.to_string();
    assert!(text.contains("auth") && text.contains("the token is not the one this server has"), "{text}");

    // Long enough for the first retry (100 ms) to have happened had the bridge retried at all.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(server.connections(), 1, "a refused token must not be retried");
}

// ------------------------------------------------------- crates/tipsy-compadre/src/client/bridge.rs (tests)
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The schedule the user's log line is the only witness to: doubling from 100 ms, capped, so a
    /// server that comes back after a reboot is picked up within a few seconds.
    #[test]
    fn the_reconnect_backoff_doubles_and_then_stops_growing() {
        let seconds = [0u32, 1, 2, 3, 4, 5, 9].map(|attempt| backoff(attempt).as_millis());
        assert_eq!(seconds, [100, 200, 400, 800, 1600, 3200, 3200]);
        assert_eq!(backoff(0), Duration::from_millis(100));
        assert!(backoff(40) <= BACKOFF_MAX, "a long outage must not overflow the schedule");
    }
}

// ------------------------------------------- crates/tipsy-compadre/src/client.rs (added to the tests)
/// Plan 3 Task 1.7's audited rule: the `--connect` client must not start a server, must not build a
/// provider and must not read the credential store (spec §7.2, §6.2).
///
/// The instrument is `providers::builds_on_this_thread`, a counter bumped at the top of
/// `build_providers` and `build_one`; it is a thread-local because this binary's other tests build
/// providers on purpose, on other threads, at the same time. The first half of the test drives the
/// in-process path so a counter that never moves cannot make the audit pass by accident — and it is
/// an async test because that half's Test runs its probe on a task of its own (`client.rs:494`).
#[tokio::test]
async fn the_remote_path_builds_no_provider_and_asks_the_server_instead() {
    let config = mock_config("http://127.0.0.1:9");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    // The instrument first: the in-process Test does resolve a key and build a connector.
    let keys = resolver();
    let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
    let mut local = Client::new(config.clone(), path.clone(), keys, slot);
    local.state.show_config_form(form_of(&config));
    local.state.config_form.as_mut().unwrap().field = 0;
    let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
    let (events, _results) = tokio::sync::mpsc::unbounded_channel();
    let before = crate::providers::builds_on_this_thread();
    local.on_action(UiAction::Config(ConfigAction::Test), &to_server, &events);
    assert!(
        crate::providers::builds_on_this_thread() > before,
        "the instrument must see the in-process Test build its connector, or this test proves nothing"
    );

    // And now the remote path: opening the screen, testing and saving, with no resolver and no stack.
    let mut remote = Client::remote(config, path, "ws://127.0.0.1:7070/ws".into());
    remote.state.show_config_form(form_of(&mock_config("http://127.0.0.1:9")));
    remote.state.config_form.as_mut().unwrap().field = 0;
    let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
    let (events, mut results) = tokio::sync::mpsc::unbounded_channel();

    let before = crate::providers::builds_on_this_thread();
    remote.open_screen(&to_server);
    press_test(&mut remote, &to_server, &events);
    remote.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);
    assert_eq!(
        crate::providers::builds_on_this_thread(),
        before,
        "the remote path built a provider stack, which means it read a key it must not read"
    );
    assert!(results.try_recv().is_err(), "a remote Test must not answer locally");
    assert_eq!(
        remote.config, mock_config("http://127.0.0.1:9"),
        "and it must not write the server's configuration into this machine's copy"
    );

    // The three effects went to the server as the three commands, in order (spec §6.2).
    let mut sent = Vec::new();
    while let Ok(ToServer::Command(command)) = commands.try_recv() {
        sent.push(command);
    }
    assert!(
        matches!(
            sent.as_slice(),
            [
                Command::GetConfig,
                Command::TestProvider { capability: Capability::Chat },
                Command::UpdateConfig { .. },
            ]
        ),
        "{sent:?}"
    );
}

// ------------------------------------------------- crates/tipsy-compadre/src/main.rs (a new test module)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cli_parses_a_remote_attach_and_the_serve_subcommand() {
        let cli = Cli::try_parse_from([
            "tipsy-compadre",
            "--connect",
            "ws://127.0.0.1:7070/ws",
            "--token",
            "abc",
        ])
        .unwrap();
        assert_eq!(cli.connect.as_deref(), Some("ws://127.0.0.1:7070/ws"));
        assert_eq!(cli.token.as_deref(), Some("abc"));
        assert!(cli.command.is_none());
        assert!(!cli.verbose);

        let cli = Cli::try_parse_from(["tipsy-compadre", "serve", "--bind", "127.0.0.1:0", "--verbose"]).unwrap();
        assert!(cli.verbose, "`--verbose` is global, so it parses on either side of the subcommand");
        match cli.command {
            Some(Cmd::Serve { bind }) => assert_eq!(bind.as_deref(), Some("127.0.0.1:0")),
            other => panic!("`serve` must be a subcommand, got {other:?}"),
        }

        // The three ways in that existed before this task parse exactly as they did.
        assert!(Cli::try_parse_from(["tipsy-compadre"]).unwrap().command.is_none());
        assert!(matches!(
            Cli::try_parse_from(["tipsy-compadre", "config", "path"]).unwrap().command,
            Some(Cmd::Config { .. })
        ));
        assert!(matches!(
            Cli::try_parse_from(["tipsy-compadre", "key", "status", "openrouter"]).unwrap().command,
            Some(Cmd::Key { .. })
        ));
    }

    #[test]
    fn connect_needs_a_url_and_a_token_needs_connect() {
        let error = Cli::try_parse_from(["tipsy-compadre", "--connect"]).unwrap_err();
        assert!(error.to_string().contains("--connect"), "a bare `--connect` must name the flag: {error}");

        let error = Cli::try_parse_from(["tipsy-compadre", "--token", "abc"]).unwrap_err();
        assert!(error.to_string().contains("--connect"), "`--token` without `--connect` is a usage error: {error}");
    }

    #[test]
    fn the_tls_flags_do_not_exist_until_their_task_does() {
        // Task 1.8 registers `--tls`, `--tls-cert` and `--tls-key` and replaces this test: nothing
        // is accepted before it is implemented.
        assert!(Cli::try_parse_from(["tipsy-compadre", "serve", "--tls"]).is_err());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre connect`

Expected: FAIL — nothing compiles yet, and in this order: `error[E0432]: unresolved import tipsy_compadre::client::bridge` in `tests/connect.rs` (`src/client/bridge.rs` does not exist and `client.rs` declares no `bridge` module), then `error[E0425]: cannot find function builds_on_this_thread in module providers`, then `error[E0560]: struct Cli has no field named connect` in `src/main.rs`, then, for the audit test, `error[E0599]: no function or associated item named remote found for struct Client`.

- [ ] **Step 3: Implement the CLI, the bridge and the remote client**

```rust
// ----------------------------------------------------------------- crates/tipsy-compadre/src/client/bridge.rs
//! The client-side socket bridge: the `--connect` path's three channel ends.
//!
//! There is no `Transport` trait (W6). `client.rs` polls an `UnboundedReceiver<ToClient>` and sends
//! into an `UnboundedSender<ToServer>`; this module hands it exactly that pair plus the `events`
//! sender, made here, so nothing in the client's frame loop can tell a socket from `spawn_with_events`.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tipsy_protocol::{ClientKind, Command, ErrorCode, Event, Frame, PROTOCOL_VERSION, ToClient, ToServer};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

use crate::session::SessionEnds;

/// The first wait after a lost connection, and the ceiling the doubling stops at. The v1 spec §12
/// says "the client reconnects with backoff" and names no numbers; these are the plan's choice, and
/// `the_reconnect_backoff_doubles_and_then_stops_growing` is what pins them.
const BACKOFF_START: Duration = Duration::from_millis(100);
const BACKOFF_MAX: Duration = Duration::from_millis(3_200);

/// The wait before attempt `n` (0-based): 100 ms, 200 ms, 400 ms, 800 ms, 1.6 s, then 3.2 s
/// forever. A server that comes back after a reboot is picked up within a few seconds.
fn backoff(attempt: u32) -> Duration {
    let millis = BACKOFF_START.as_millis() as u64;
    Duration::from_millis((millis << attempt.min(5)).min(BACKOFF_MAX.as_millis() as u64))
}

/// Why a `--connect` never attached. Every variant is fatal to the run: the three refusals the
/// handshake can produce (Task 1.2) are answered on the first `Hello`, and a refusal is never
/// retried (spec §9.1).
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("cannot reach {url}: {reason}")]
    Unreachable { url: String, reason: String },
    /// The server answered `Hello` with `Error`. `code` is the code as it went over the wire —
    /// `token_required`, `token_invalid`, `protocol_version` — read back through serde so this
    /// variant needs no list of its own.
    #[error("the server refused the connection: {code} ({message})")]
    Refused { code: String, message: String },
    /// The socket ended before a `Welcome` arrived, with no `Error` to explain it.
    #[error("the server closed the connection before saying welcome")]
    NoWelcome,
}

/// Attaches to a remote server: connects, says `Hello` first, waits for `Welcome`, and returns the
/// three ends of a session that lives on the other machine.
///
/// Everything before `Welcome` is an `Err`, so a caller that gets a `SessionEnds` has a live server
/// that accepted it (success criterion 2). After that this function has handed the socket to a task
/// which owns it for the rest of the run: it shuttles both ways, reports a lost connection as one
/// recoverable `Event::Error`, and reconnects with [`backoff`], saying `Hello` again each time.
pub async fn connect(url: &str, token: Option<&str>) -> Result<SessionEnds, BridgeError> {
    let (mut socket, _response) = tokio_tungstenite::connect_async(url).await.map_err(|error| {
        BridgeError::Unreachable { url: url.to_string(), reason: error.to_string() }
    })?;
    say_hello(&mut socket, token, url).await?;

    let (to_server_tx, to_server_rx) = mpsc::unbounded_channel();
    let (view_tx, view_rx) = mpsc::unbounded_channel();
    tokio::spawn(pump(url.to_string(), token.map(str::to_string), socket, to_server_rx, view_tx.clone()));
    Ok(SessionEnds { to_server: to_server_tx, from_server: view_rx, events: view_tx })
}

/// Where a `--connect` socket lives while it is open.
type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Sends `Hello` and waits for the first answer. `Welcome` is the only acceptable one; an `Error` is
/// the server's refusal, quoted as it came.
async fn say_hello(socket: &mut Socket, token: Option<&str>, url: &str) -> Result<(), BridgeError> {
    let hello = Command::Hello {
        token: token.map(str::to_string),
        client: ClientKind::Tui,
        protocol_version: PROTOCOL_VERSION,
    };
    let text = serde_json::to_string(&hello).expect("a `Command` always serialises");
    socket
        .send(Message::text(text))
        .await
        .map_err(|error| BridgeError::Unreachable { url: url.to_string(), reason: error.to_string() })?;
    wait_for_welcome(socket, url).await
}

/// The one frame that says the server accepted this client.
async fn wait_for_welcome(socket: &mut Socket, url: &str) -> Result<(), BridgeError> {
    let unreachable = |error: tokio_tungstenite::tungstenite::Error| BridgeError::Unreachable {
        url: url.to_string(),
        reason: error.to_string(),
    };
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<Event>(text.as_str()) {
                Ok(Event::Welcome { .. }) => return Ok(()),
                Ok(Event::Error { code, message, .. }) => {
                    // The wire spelling, not the Rust name: a user reads `token_invalid`.
                    let code = serde_json::to_value(code)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string))
                        .unwrap_or_default();
                    return Err(BridgeError::Refused { code, message });
                }
                // Any other event before `Welcome`, or a frame this protocol cannot read, is a
                // server that does not speak it.
                Ok(_) | Err(_) => return Err(BridgeError::NoWelcome),
            },
            Some(Ok(_)) => continue,
            Some(Err(error)) => return Err(unreachable(error)),
            None => return Err(BridgeError::NoWelcome),
        }
    }
}

/// Why one connection ended, which is what decides between reconnecting and stopping.
enum Ending {
    /// The socket went: report it, wait, and try again.
    SocketGone,
    /// Every `to_server` sender is gone: this process is quitting, and a reconnect would leak a task.
    ClientGone,
}

/// Both directions for as long as the process runs.
async fn pump(
    url: String,
    token: Option<String>,
    mut socket: Socket,
    mut to_server: UnboundedReceiver<ToServer>,
    view: UnboundedSender<ToClient>,
) {
    let mut attempt = 0u32;
    loop {
        if matches!(shuttling(&mut socket, &mut to_server, &view).await, Ending::ClientGone) {
            return;
        }
        // The socket is gone. One line to the HUD, a wait, and `Hello` again.
        let _ = view.send(ToClient::Event(Event::Error {
            code: ErrorCode::Network,
            message: format!("{url}: connection lost; reconnecting"),
            recoverable: true,
        }));
        tokio::time::sleep(backoff(attempt)).await;
        attempt = attempt.saturating_add(1);
        if let Some(fresh) = reconnect(&url, token.as_deref()).await {
            socket = fresh;
            attempt = 0;
        }
        // While it is detached, `shuttling`'s `to_server.recv()` is not being polled, so anything
        // the HUD sends waits in the channel and is drained and dropped by the next `shuttling` call:
        // the peer that comes back is a new session (W9), and a `Mic` frame or an `Ask` for the one
        // that is gone is not something to replay. The `Event::Error` above is what tells the user.
    }
}

/// One connection's lifetime: `select!` between the socket and the client's commands, until either
/// side goes.
async fn shuttling(
    socket: &mut Socket,
    to_server: &mut UnboundedReceiver<ToServer>,
    view: &UnboundedSender<ToClient>,
) -> Ending {
    loop {
        tokio::select! {
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(message) = serde_json::from_str::<ToClient>(text.as_str())
                        && view.send(message).is_err()
                    {
                        return Ending::ClientGone;
                    }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if let Ok(frame) = Frame::decode(&bytes)
                        && view.send(ToClient::Audio(frame)).is_err()
                    {
                        return Ending::ClientGone;
                    }
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => return Ending::SocketGone,
            },
            command = to_server.recv() => match command {
                Some(ToServer::Command(command)) => {
                    let text = serde_json::to_string(&command).expect("a `Command` always serialises");
                    if socket.send(Message::text(text)).await.is_err() {
                        return Ending::SocketGone;
                    }
                }
                Some(ToServer::Audio(frame)) => {
                    if socket.send(Message::binary(frame.encode())).await.is_err() {
                        return Ending::SocketGone;
                    }
                }
                // Every sender is gone: the client is quitting.
                None => return Ending::ClientGone,
            },
        }
    }
}

/// One attempt at getting back. `None` means "keep trying": the peer is not there yet.
async fn reconnect(url: &str, token: Option<&str>) -> Option<Socket> {
    let (mut socket, _response) = tokio_tungstenite::connect_async(url).await.ok()?;
    say_hello(&mut socket, token, url).await.ok()?;
    Some(socket)
}
```

```rust
// ---------------------------------------------------------------- crates/tipsy-compadre/src/providers/mod.rs
// Insert next to `build_providers` (the two items) and one call at the top of `build_providers`
// (`:251`) and of `build_one` (`:286`).

/// Counts the provider stacks built **on this thread**, for Plan 3 Task 1.7's audit: the `--connect`
/// client must never build one, and `client.rs`'s `the_remote_path_builds_no_provider_and_asks_the_server_instead`
/// is what reads it. Thread-local rather than global because `cargo test` runs this binary's other
/// tests — which build providers on purpose — on other threads at the same time.
#[cfg(test)]
thread_local! {
    static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread has built a provider stack or a single connector.
#[cfg(test)]
pub fn builds_on_this_thread() -> usize {
    BUILDS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn count_a_build() {
    BUILDS.with(|built| built.set(built.get() + 1));
}

/// A release build counts nothing: the audit exists only in the test binary.
#[cfg(not(test))]
fn count_a_build() {}

// then, as the first statement of `build_providers` and of `build_one`:
//     count_a_build();
```

```rust
// ---------------------------------------------------------------- crates/tipsy-compadre/src/client.rs
// 1. Declare the submodule, beside the `use` block at the top of the file, and add the two names
//    the conversions need: `ConfigForm` to the `tipsy_ui` import (`:15`) and `ConfigView` to the
//    `tipsy_protocol` one (`:14`).
pub mod bridge;

// 2. How this client's session is reached: the new field, next to `providers` (`:72`).
/// Which process runs the session this HUD draws: this one, or a server behind a socket.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Attachment {
    /// The bare run: `spawn_with_events` in this process, with the connectors built here.
    InProcess,
    /// `--connect`: a socket to a server that owns the session, the connectors and the keys. The
    /// token is deliberately not kept — it went into `Hello`, and nothing else needs it.
    Remote { url: String },
}

impl Attachment {
    fn is_remote(&self) -> bool {
        matches!(self, Attachment::Remote { .. })
    }

    /// How to name the other end in a notice. Never the token: it is not in this value.
    fn describe(&self) -> String {
        match self {
            Attachment::InProcess => "this process".to_string(),
            Attachment::Remote { url } => url.clone(),
        }
    }
}

// The two fields become optional, and the attachment joins them:
//     keys: Option<KeyResolver>,
//     providers: Option<ProviderSlot>,
//     attachment: Attachment,

// 3. `Client::new` keeps its signature and its behaviour, and `remote` is the new way in.
    fn new(config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot) -> Self {
        let (mic_tx, mic_frames) = std::sync::mpsc::sync_channel(MIC_QUEUE);
        let mic_gain = config.listening.mic_gain;
        Self {
            state: UiState::new(),
            config,
            config_path,
            keys: Some(keys),
            providers: Some(providers),
            attachment: Attachment::InProcess,
            player: None,
            reply: Vec::new(),
            call: None,
            rate: 24000,
            mic_gain,
            mic: None,
            mic_frames,
            mic_tx,
        }
    }

    /// The `--connect` client: no resolver, no provider stack, and no way to reach either, because
    /// both fields stay `None` for the whole run (Task 1.7's audit test).
    ///
    /// `config` is still read from this machine's `config.toml`: it carries what belongs to *this*
    /// machine — the microphone gain — and the screen's content is the server's from the first
    /// `Event::Config` on. Nothing here acts on its provider rows.
    fn remote(config: Config, config_path: PathBuf, url: String) -> Self {
        let (mic_tx, mic_frames) = std::sync::mpsc::sync_channel(MIC_QUEUE);
        let mic_gain = config.listening.mic_gain;
        Self {
            state: UiState::new(),
            config,
            config_path,
            keys: None,
            providers: None,
            attachment: Attachment::Remote { url },
            player: None,
            reply: Vec::new(),
            call: None,
            rate: 24000,
            mic_gain,
            mic: None,
            mic_frames,
            mic_tx,
        }
    }

// 4. `run` and the new `run_remote`, sharing one terminal half.
/// Runs the terminal client until the user quits.
pub fn run(config_path: PathBuf) -> Result<(), String> {
    let loaded = store::load_or_create(&config_path).map_err(|e| format!("{}: {e}", config_path.display()))?;
    let config = loaded.config;
    let keys = KeyResolver::system();
    // One stack, shared with the session from here on: a save swaps the connectors inside it.
    let providers = ProviderSlot::new(build_providers(&config.providers, &keys).map_err(|e| e.to_string())?);
    let ends = spawn_with_events(config.clone(), providers.clone(), BrainConfig::default());
    // A second sender for the same client channel: the system monitor and the screen's Test
    // results both arrive with the session's own events.
    sysmon::spawn(ends.events.clone());
    let mut client = Client::new(config, config_path, keys, providers);
    client.terminal(ends, loaded.warning)
}

/// The `--connect` run: the same TUI, whose two channels come from a socket instead of this process.
///
/// The socket is opened *before* the terminal is entered, so a refusal — no token on an exposed
/// server, the wrong token, a protocol mismatch — is a line on an ordinary terminal rather than a
/// half-drawn alternate screen (success criterion 2). Nothing on this path builds a provider or
/// touches the credential store (spec §7.2): the server owns both.
pub async fn run_remote(url: String, token: Option<String>) -> Result<(), String> {
    let path = store::default_path();
    let loaded = store::load_or_create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ends = bridge::connect(&url, token.as_deref()).await.map_err(|e| e.to_string())?;
    // W11: the client keeps its own system monitor in `--connect` mode. The server's bridge does not
    // spawn one for a remote peer, so this stays the only producer of `Stats` on this channel.
    sysmon::spawn(ends.events.clone());
    let mut client = Client::remote(loaded.config, path, url);
    client.terminal(ends, loaded.warning)
}

impl Client {
    /// The terminal half both entry points share: the alternate screen, mouse capture, the warning,
    /// the output device and the frame loop. Keeping it in one place is what stops the two runs
    /// drifting apart in how they own the terminal.
    fn terminal(&mut self, ends: SessionEnds, warning: Option<String>) -> Result<(), String> {
        // `init` enters the alternate screen, enables raw mode and installs the panic hook that
        // restores the terminal; `restore` undoes all of it.
        let mut terminal = ratatui::init();
        // Mouse capture is not part of a bare terminal: without it the clickable titles do nothing.
        let _ = execute!(std::io::stdout(), EnableMouseCapture);
        if let Some(warning) = warning {
            self.state.notify(warning);
        }
        self.open_player();
        let outcome = self.frame_loop(&mut terminal, ends);
        let _ = execute!(std::io::stdout(), event::DisableMouseCapture);
        ratatui::restore();
        outcome
    }
}

// 5. `frame_loop` takes the ends instead of making them.
    fn frame_loop(&mut self, terminal: &mut ratatui::DefaultTerminal, ends: SessionEnds) -> Result<(), String> {
        let to_server = ends.to_server;
        let mut from_server = ends.from_server;
        let events = ends.events;
        // ... the rest of the loop is untouched, except that `on_message` is handed the sender:
        //     while let Ok(message) = from_server.try_recv() { self.on_message(message, &to_server); }
    }

// 6. `on_message` learns the two configuration events. The two existing test call sites
//    (`client.rs:1187` and `:1229`) gain `&to_server`, which they already hold.
    fn on_message(&mut self, message: ToClient, to_server: &UnboundedSender<ToServer>) {
        match message {
            ToClient::Event(event) => {
                if let Event::SpeechChunk { sample_rate, .. } = &event {
                    self.rate = *sample_rate;
                }
                // A remote screen follows the server's configuration: `GetConfig` when it opens, and
                // again whenever the server says something changed — `Event::ConfigChanged` carries
                // no payload by design (spec §4.4). The answer arrives as `Event::Config`.
                if let Event::Config { config } = &event
                    && self.attachment.is_remote()
                {
                    // Two shapes, one conversion in: the screen draws with `tipsy_ui::ConfigForm` and
                    // the wire carries `tipsy_protocol::ConfigView`, and `From` is the only bridge
                    // between them (§4.3). Nothing is redacted here: the server decided what this
                    // client may see before it sent it, with `ConfigView::redacted`.
                    self.state.show_config_form(ConfigForm::from(config.clone()));
                }
                if matches!(&event, Event::ConfigChanged)
                    && self.attachment.is_remote()
                    && self.state.config_form.is_some()
                {
                    let _ = to_server.send(ToServer::Command(Command::GetConfig));
                }
                // ... the existing `TestResult` and `call_ended` handling is unchanged ...
                self.state.apply(&event);
            }
            ToClient::Audio(Frame::Speech { samples, .. }) => self.on_speech(samples),
            ToClient::Audio(Frame::Mic(_)) => {}
        }
    }

// 7. `on_action` passes the sender to `open_screen`, which needs it in remote mode.
            UiAction::OpenConfig => {
                self.open_screen(to_server);
                false
            }

// 8. `open_screen`, `on_config_effect` and `test_provider` learn where the work happens.
    fn open_screen(&mut self, to_server: &UnboundedSender<ToServer>) {
        if self.attachment.is_remote() {
            return self.open_screen_remote(to_server);
        }
        // ... the existing body, unchanged: the file is read here and now ...
    }

    /// The `--connect` open: the server is asked for its configuration, and until the answer arrives
    /// the screen shows this client's own copy with a notice that says so. Opening on nothing at all
    /// would make `F9` look broken for a round trip.
    fn open_screen_remote(&mut self, to_server: &UnboundedSender<ToServer>) {
        let mut form = form_of(&self.config);
        form.notice = Some((
            NoticeKind::Warning,
            format!("asking {} for its configuration", self.attachment.describe()),
        ));
        self.state.show_config_form(form);
        let _ = to_server.send(ToServer::Command(Command::GetConfig));
    }

    fn on_config_effect(&mut self, effect: ConfigEffect, to_server: &UnboundedSender<ToServer>, events: &UnboundedSender<ToClient>) {
        match effect {
            ConfigEffect::None => {}
            ConfigEffect::Save => self.save(to_server),
            // The Test needs the sender now: over a socket the probe runs on the server.
            ConfigEffect::Test(capability) => self.test_provider(capability, to_server, events),
            // Unchanged, and honest on both paths: `Ctrl+R` mints a value into the form, and in
            // remote mode the save that follows sends it to the server, which is where the token
            // being regenerated lives.
            ConfigEffect::RegenerateToken => self.regenerate_token(),
        }
    }

    fn test_provider(&mut self, capability: Capability, to_server: &UnboundedSender<ToServer>, events: &UnboundedSender<ToClient>) {
        if self.attachment.is_remote() {
            // The providers and the keys are the server's, so the probe is too (spec §6.2). The
            // answer comes back as `Event::TestResult` through the socket and lands in the same
            // notice line as any other result.
            let _ = to_server.send(ToServer::Command(Command::TestProvider { capability }));
            return;
        }
        let Some(keys) = &self.keys else { return };
        // ... the existing body, with `build_one(&cfg, capability, &self.keys)` becoming
        //     `build_one(&cfg, capability, keys)` ...
    }

// 9. `save` splits in two; the in-process half is byte for byte today's code.
    fn save(&mut self, to_server: &UnboundedSender<ToServer>) {
        if self.attachment.is_remote() {
            return self.save_remote(to_server);
        }
        // 1. The form in the file's shape.
        let parsed = self.state.config_form.as_ref().map(config_from);
        let config = match parsed {
            None => return,
            Some(Err(message)) => return self.config_notice(NoticeKind::Error, message),
            Some(Ok(config)) => config,
        };
        // 2. The connectors that configuration names.
        let (Some(keys), Some(providers)) = (&self.keys, &self.providers) else {
            // Unreachable while `Attachment::InProcess` owns the two `Some`s; the guard is what
            // keeps a remote client from ever reaching a builder.
            return;
        };
        let rebuilt = match build_providers(&config.providers, keys) {
            Ok(providers) => providers,
            Err(e) => return self.config_notice(NoticeKind::Error, e.to_string()),
        };
        // 3. The file itself.
        if let Err(e) = store::save(&self.config_path, &config) {
            return self.config_notice(NoticeKind::Error, format!("{}: {e}", self.config_path.display()));
        }
        // 4. What is running: the stack in place, then the fields the channel carries.
        self.mic_gain = config.listening.mic_gain;
        providers.replace(rebuilt);
        let _ = to_server.send(ToServer::Command(Command::SetListeningMode { mode: config.listening.mode }));
        let _ = to_server.send(ToServer::Command(Command::SetProfile { id: config.active_profile.clone() }));
        self.config = config;
        if let Some(form) = self.state.config_form.as_mut() {
            form.dirty = false;
            form.notice = None;
        }
    }

    /// The `--connect` save: the server owns the file, the keys and the connectors, so this sends the
    /// form and nothing else (spec §6.2). No local `build_providers` — this client holds no key — and
    /// no local `store::save`: a second copy of the server's configuration on another machine is a
    /// second source of truth, and the server answers `ConfigChanged` to everyone when it saves.
    fn save_remote(&mut self, to_server: &UnboundedSender<ToServer>) {
        let Some(form) = self.state.config_form.as_ref() else { return };
        // Two shapes, one conversion out: the screen's form is not the wire's view (§4.3).
        let config = ConfigView::from(form);
        let _ = to_server.send(ToServer::Command(Command::UpdateConfig { config }));
        if let Some(form) = self.state.config_form.as_mut() {
            form.dirty = false;
            form.notice = Some((
                NoticeKind::Warning,
                format!("sent to {}; this client's own config.toml is unchanged", self.attachment.describe()),
            ));
        }
    }
```

```rust
// ---------------------------------------------------------------- crates/tipsy-compadre/src/main.rs
// The CLI of spec §3.2. `--tls`, `--tls-cert` and `--tls-key` are Task 1.8's and are not registered
// here: nothing is accepted before it is implemented.

#[derive(Parser)]
#[command(name = "tipsy-compadre", version, about = "A tipsy, lazy, joking voice assistant for software developers")]
struct Cli {
    /// Attach the TUI to a remote server instead of running one here.
    #[arg(long, value_name = "WS-URL")]
    connect: Option<String>,
    /// The token the remote server requires. Only read with `--connect`.
    #[arg(long, value_name = "TOKEN", requires = "connect")]
    token: Option<String>,
    /// Say more about what this process is doing. Task 1.9 also routes the log to stderr with it.
    #[arg(long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Manage the API keys the providers use.
    Key {
        #[command(subcommand)]
        command: KeyCmd,
    },
    /// Report where the configuration is kept.
    Config {
        #[command(subcommand)]
        command: ConfigCmd,
    },
    /// Run the server headless: the web client at `/`, the web terminal at `/terminal`.
    Serve {
        /// Override the configured bind address for this run only; the file is never written.
        #[arg(long, value_name = "ADDR")]
        bind: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match (cli.connect, cli.command) {
        // `--connect` runs the TUI and nothing else: no server, no providers, no credential store.
        (Some(url), None) => remote_client(url, cli.token, cli.verbose),
        (Some(_), Some(_)) => {
            eprintln!("tipsy-compadre: --connect runs the TUI on its own; it takes no subcommand");
            ExitCode::FAILURE
        }
        (None, None) => run_client(cli.verbose),
        (None, Some(Cmd::Key { command })) => key_command(command),
        (None, Some(Cmd::Config { command })) => config_command(command),
        (None, Some(Cmd::Serve { bind })) => serve_command(bind, cli.verbose),
    }
}

/// `--verbose` before Task 1.9's `tracing` bootstrap: the one line that says which mode this process
/// is in, and where. The token is never printed — only whether one was given (spec §9.2).
fn announce(verbose: bool, line: impl std::fmt::Display) {
    if verbose {
        eprintln!("tipsy-compadre: {line}");
    }
}

/// The bare run: load the configuration, build the providers and hand over to the client.
fn run_client(verbose: bool) -> ExitCode {
    announce(verbose, "mode: server and TUI in one process");
    // ... the existing body, unchanged ...
}

/// The `--connect` run. No configuration is required beyond this machine's own settings, and a
/// refusal is reported before the terminal is entered, so the user reads it on a clean screen.
fn remote_client(url: String, token: Option<String>, verbose: bool) -> ExitCode {
    announce(verbose, format!("mode: remote TUI; attaching to {url}; token {}", if token.is_some() { "given" } else { "not given" }));
    match run_remote_terminal(url, token) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tipsy-compadre: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_remote_terminal(url: String, token: Option<String>) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("no async runtime: {e}"))?;
    runtime.block_on(async move { client::run_remote(url, token).await })
}

/// The `serve` arm: the file's configuration, with `--bind` overriding it for this run only, handed
/// to the server of Task 1.4 (the listener, the router and the bind rule are its).
fn serve_command(bind: Option<String>, verbose: bool) -> ExitCode {
    let path = store::default_path();
    let loaded = match store::load_or_create(&path) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("tipsy-compadre: {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    if let Some(warning) = &loaded.warning {
        eprintln!("tipsy-compadre: {warning}");
    }
    let mut config = loaded.config;
    if let Some(bind) = bind {
        // This run only. `serve --bind` never writes the file back (spec §5.1).
        config.server.bind = bind;
    }
    announce(
        verbose,
        format!(
            "mode: server; bind {}; config {}; token {}",
            config.server.bind,
            path.display(),
            if config.server.has_token() { "set" } else { "not set" }
        ),
    );
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("tipsy-compadre: no async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    // The resolver this server resolves its keys with, and the connectors it runs on. Building them
    // here, in the server process, is the whole point of the split: the client that attaches over a
    // socket never resolves a key. A file that names a key nobody holds is the same exit code as a
    // bare run's (`MISSING_KEY`, `:14`).
    let keys = KeyResolver::system();
    let providers = match build_providers(&config.providers, &keys) {
        Ok(providers) => ProviderSlot::new(providers),
        Err(e) => {
            eprintln!("tipsy-compadre: {e}");
            return ExitCode::from(MISSING_KEY);
        }
    };
    // `Settings` is Task 1.4's, and this literal is its final shape, all eight fields. `bind`, `lan`
    // and `token` are the three the bind rule and the handshake read; `config`, `config_path`, `keys`
    // and `providers` are what `server::run` builds the `Hub` and every session from; `tls` stays
    // `None` until Task 1.8 fills it in.
    let settings = tipsy_compadre::server::Settings {
        bind: config.server.bind.clone(),
        lan: config.server.lan,
        token: config.server.token.clone(),
        config,
        config_path: path,
        keys,
        providers,
        tls: None,
    };
    match runtime.block_on(tipsy_compadre::server::run(settings)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tipsy-compadre: {e}");
            ExitCode::FAILURE
        }
    }
}
```

Two notes for the implementer, both load-bearing:

- **`server::Settings` has eight fields and they are pinned in the plan's shared contract.** `bind`
  is the `--bind`-overridden address; `lan` and `token` are the configuration's; `config`,
  `config_path`, `keys` and `providers` are what `server::run` constructs the `Hub` and every session
  from (the hub is built inside `run`, which is why the resolver and the path are handed over rather
  than kept here); `tls` is `None` in this task because TLS is Task 1.8's. All eight come from values
  this arm already holds — `path` from `store::default_path()` at the top of the function.
  `ProviderSlot` needs one new import in `main.rs`
  (`use tipsy_compadre::providers::{build_providers, ProviderSlot};` — `build_providers` is already
  imported at `:11`; `KeyResolver` is already imported at `:10`).
- **The form and the view are two shapes, and this task converts between them explicitly.** `tipsy_ui`
  keeps `ConfigForm` (the screen draws with it, and Plan 2B's tests cover it) and `tipsy-protocol`
  holds `ConfigView`; `ConfigForm::from(view)` on the way in and `ConfigView::from(&form)` on the way
  out are the only two crossings in `client.rs` (`Event::Config` and `save_remote`), and both are in
  the code above. §4.3 is the ruling; a round trip over a fully populated value is Task 1.1's test.
  Nothing in this client redacts: `ConfigView::redacted` is the *server's* rule for what a client
  receives, and a client that redacted its own form before saving would delete the token the Server
  tab exists to set.

- [ ] **Step 4: Run the test to verify it passes**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre connect`

Expected: PASS — `tests/connect.rs` reports 3 passed (`connect_opens_with_a_hello_and_shuttles_both_channels`, `connect_reconnects_with_backoff_after_the_server_drops_the_socket`, `connect_does_not_retry_a_refusal`).

Then run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre the_remote_path`

Expected: PASS — 1 passed, the provider audit.

Then run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre the_cli`

Expected: PASS — 3 passed in the binary's own test target.

Then the whole workspace, and clippy against the baseline of `global-constraints.md`:

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 --workspace`

Expected: PASS — 352 tests before this task plus this task's 7, 0 failures; nothing else changed. Then `cargo clippy --workspace --all-targets` and compare the finding set with `global-constraints.md`'s pinned list: add nothing new.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/main.rs crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/src/client/bridge.rs crates/tipsy-compadre/src/providers/mod.rs crates/tipsy-compadre/tests/connect.rs
git commit -m "feat(client): add --connect, the serve subcommand and the socket bridge

The configuration screen follows the server over a socket: opening asks with
GetConfig, saving sends UpdateConfig and the Test button sends TestProvider,
so a remote client never resolves a key. A --connect client builds no
provider stack and reads no credential store, which the audit test proves
with a cfg(test) counter rather than asserting.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 1.8: TLS, and the one rule that keeps a key off the wire `[mid]`

Spec §5.5, §9.4, §9.5, W2. TLS here is **confidentiality**, and the task's own text and output say
so: it is what makes the token and a new API key safe across a LAN, and it is *not* what unlocks the
browser's microphone from another machine.

**Files:**
- Create: `crates/tipsy-compadre/src/server/tls.rs`
- Modify: `crates/tipsy-compadre/src/server/mod.rs` — `Settings.tls` is `Option<TlsFiles>` and `server::run` owns the TLS branch: it loads or generates the pair, builds the `RustlsConfig`, prints the fingerprint once, and picks `bind_rustls` over `bind` with the same router. `ServerError` gains the variant that carries a `TlsError`. **`TlsFiles` is Task 1.4's type and lives here**, beside `Settings`; if 1.4's committed version predates the three-state ruling, the enum verbatim in Step 3 is the one piece of 1.4's file this task adds (Task 1.4 owns the file)
- Modify: `crates/tipsy-compadre/src/server/conn.rs` — the `SetKey` guard (Task 1.6 owns the bridge):
  two lines at the one place a socket message becomes a `ToServer`, plus one case per
  `accepts_a_secret` answer in 1.6's own `conn.rs` test module (see the note in Step 3)
- Modify: `crates/tipsy-compadre/src/main.rs` — `Cmd::Serve` gains `--tls`, `--tls-cert`,
  `--tls-key`; the serve arm turns them into `Settings.tls: Option<TlsFiles>` and does no file I/O of
  its own; the test `the_tls_flags_do_not_exist_until_their_task_does` from Task 1.7 is deleted and
  replaced
- Modify: `crates/tipsy-compadre/Cargo.toml` — `rcgen = "0.14"`, `axum-server = { version = "0.8", features = ["tls-rustls"] }`, `sha2 = "0.10"`
- Test: `crates/tipsy-compadre/src/server/tls.rs` (its own `#[cfg(test)] mod tests`)
- Test: `crates/tipsy-compadre/src/server/conn.rs` (Task 1.6's test module: the refused `SetKey`)
- Test: `crates/tipsy-compadre/src/main.rs` (the CLI test module Task 1.7 created)

**Interfaces:**
- Consumes: `store::default_path() -> PathBuf` (`config/store.rs:13`); `ServerConfig::{bind, lan, token}` and `has_token()` (`config/model.rs:46-68`, `:59-61`); `ErrorCode` — including **`ErrorCode::InsecureTransport`**, the variant Task 1.1 owns (serde `insecure_transport`, with `ProtocolVersion`, `TokenRequired`, `TokenInvalid` and `BadMessage` beside it) — `Event::Error { code, message, recoverable }`, `ToClient`, `ToServer`, `Command::SetKey { name, value }` (Task 1.1); **`crate::server::TlsFiles`** — Task 1.4's type, declared in `server/mod.rs` beside `Settings` and *consumed* here, never declared by this task, because Task 1.7 builds `Settings { tls: None, .. }` before this task exists; `server::run(settings: Settings)` with `Settings.tls: Option<TlsFiles>` (Task 1.4) and `ServerError`, which gains this task's variant (Task 1.4); `server::conn::serve(socket, session_ends, hub)` (Task 1.6); `axum_server::tls_rustls::{RustlsConfig, bind_rustls}` and `axum_server::from_tcp_rustls` (feature `tls-rustls`); `rcgen::{generate_simple_self_signed, CertifiedKey, Certificate}`; `sha2::{Digest, Sha256}`
- Produces:
  - `server::tls::{load_or_generate(paths: Paths) -> Result<Certificate, TlsError>}` — sync; takes Task 1.4's `TlsFiles` inside `Paths` and returns which pair `serve` will serve with, minting one only for `TlsFiles::Generated` with nothing on disk. `Generated` means "write the pair into the config directory and reuse it if it is already there"; `Supplied { cert, key }` means "read exactly these, and refuse naming the path when either is unreadable" (§10). Called from `server::run`'s TLS branch, not from `main.rs`
  - `server::tls::Paths { files: TlsFiles, config_dir: PathBuf, hostnames: Vec<String> }` — `pub struct`, `Clone`; this module's own argument type, carrying the two facts `TlsFiles` deliberately holds neither of. `config_dir` is the directory `tipsy-compadre config path` prints (`store::default_path().parent()`); `hostnames` is the SAN list for a generated pair — the bind IP as the browser will type it, plus `localhost`
  - `server::tls::Certificate { pub cert: PathBuf, pub key: PathBuf, pub fingerprint: Option<String> }` — `pub struct`, `Clone`. `cert`/`key` are the two files for `RustlsConfig::from_pem_file`; `fingerprint` is `Some` **only** when this call minted the pair — the SHA-256 of the DER as `AA:BB:…`, computed at runtime and printed once (spec §5.5) — and `None` for a pair that was already on disk or was supplied, so a restart does not re-print it
  - `server::tls::TlsError` — `pub enum`, `thiserror`: `Pair { cert: PathBuf, key: PathBuf, message: String }` (unreadable, missing or unparseable — the paths are in the message, spec §10) and `Generate { dir: PathBuf, message: String }`. **This task adds the variant that carries it to Task 1.4's `ServerError`** — `Tls(#[from] TlsError)`, `#[error(transparent)]` — and that addition is this task's edit to `server/mod.rs`, alongside the `bind_rustls` branch. It cannot come from 1.4: `TlsError` does not exist until this task's `server/tls.rs` does, so an enum variant wrapping it in 1.4 would not compile. That is the plan's ordering rule applied rather than an exception to it, and it is why the sentence a failed `serve` prints carries the path
  - `server::tls::{rustls_config(cert: &Certificate) -> Result<RustlsConfig, TlsError>}` — `pub async fn`; this is where the pair is actually parsed (`RustlsConfig::from_pem_file`), so a PEM that is not a certificate, or a key that does not match it, is refused here with both paths in the message
  - `server::tls::{accepts_a_secret(peer: SocketAddr, tls: bool) -> bool}` — `pub fn`, the whole of §9.4's rule: `tls || peer.ip().is_loopback()`
  - `server::tls::{refuse_set_key(command: &ToServer, peer: SocketAddr, tls: bool) -> Option<ToClient>}` — `pub fn`; `Some(ToClient::Event(Event::Error { code: ErrorCode::InsecureTransport, message, recoverable: true }))` for a `SetKey` on a plaintext non-loopback connection, `None` for anything else. The **code is the machine meaning** — Task 1.1's variant, and what `session.rs`'s own `Command::SetKey` arm matches on — and the **message is a sentence for a person**, so no caller, test or document may key off its wording
  - `Cmd::Serve { bind: Option<String>, tls: bool, tls_cert: Option<PathBuf>, tls_key: Option<PathBuf> }`; `--tls-cert` and `--tls-key` require each other, and either of them implies `--tls`. The arm turns them into `Settings.tls`: `None`, `Some(TlsFiles::Generated)` or `Some(TlsFiles::Supplied { cert, key })` — one of the three, never a guess

- [ ] **Step 1: Write the failing test**

```rust
// ------------------------------------------------- crates/tipsy-compadre/src/server/tls.rs (test module)
#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    // The error type `server::run` maps a failed TLS setup into, so the third-state test can assert
    // that §10's refusal reaches the CLI's message rather than stopping at this module; and the
    // three-state value Task 1.4 declares, which the tests below name.
    use crate::server::{ServerError, TlsFiles};

    /// The paths a `serve --tls` with no `--tls-cert` uses: generate beside `config.toml`, and a SAN
    /// list holding the address the browser will type plus `localhost` (research note §4.1).
    fn generated(dir: &std::path::Path) -> Paths {
        Paths {
            files: TlsFiles::Generated,
            config_dir: dir.to_path_buf(),
            hostnames: vec!["127.0.0.1".to_string(), "localhost".to_string()],
        }
    }

    /// The other state: exactly these two files, and no minting.
    fn supplied(cert: PathBuf, key: PathBuf, dir: &std::path::Path) -> Paths {
        Paths { files: TlsFiles::Supplied { cert, key }, config_dir: dir.to_path_buf(), hostnames: Vec::new() }
    }

    /// The certificate is generated **once**: the second call finds the pair and leaves it alone.
    #[test]
    fn the_certificate_is_generated_once_and_then_reused() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_generate(generated(dir.path())).unwrap();
        assert_eq!(first.cert, dir.path().join("serve-cert.pem"));
        assert_eq!(first.key, dir.path().join("serve-key.pem"));
        let fingerprint = first.fingerprint.clone().expect("a minted pair has a fingerprint to print once");
        assert_eq!(fingerprint.len(), 32 * 3 - 1, "SHA-256 as 32 `AA:BB` pairs: {fingerprint}");
        assert!(fingerprint.split(':').all(|byte| byte.len() == 2 && byte.chars().all(|c| c.is_ascii_hexdigit())));
        assert!(std::fs::read_to_string(&first.cert).unwrap().starts_with("-----BEGIN CERTIFICATE-----"));

        let before = std::fs::read(&first.cert).unwrap();
        let second = load_or_generate(generated(dir.path())).unwrap();
        assert!(second.fingerprint.is_none(), "only the run that minted the pair prints a fingerprint");
        assert_eq!(std::fs::read(&second.cert).unwrap(), before, "the second call must not mint another");
        assert_eq!(second.key, first.key);

        // Half a pair on disk is a half-written pair: both are regenerated rather than mismatched.
        std::fs::remove_file(&second.key).unwrap();
        let third = load_or_generate(generated(dir.path())).unwrap();
        assert!(third.fingerprint.is_some(), "a missing key means the pair is minted again");
        assert_ne!(std::fs::read(&third.cert).unwrap(), before);
    }

    /// `TlsFiles::Supplied` means *exactly these two files*: they are used as they are, never
    /// overwritten, and nothing is written beside `config.toml`.
    #[test]
    fn a_supplied_pair_is_used_and_never_generated_over() {
        let dir = tempfile::tempdir().unwrap();
        let minted = load_or_generate(generated(dir.path())).unwrap();
        let cert_before = std::fs::read(&minted.cert).unwrap();
        let key_before = std::fs::read(&minted.key).unwrap();

        // Deliberately a directory with nothing in it: a supplied pair must not be looked for beside
        // `config.toml` at all.
        let unused = dir.path().join("unused");
        let used = load_or_generate(Paths {
            files: TlsFiles::Supplied { cert: minted.cert.clone(), key: minted.key.clone() },
            config_dir: unused.clone(),
            hostnames: Vec::new(),
        })
        .unwrap();
        assert_eq!(used.cert, minted.cert);
        assert_eq!(used.key, minted.key);
        assert!(used.fingerprint.is_none(), "this process did not mint the pair, so it prints no fingerprint");
        assert_eq!(std::fs::read(&used.cert).unwrap(), cert_before);
        assert_eq!(std::fs::read(&used.key).unwrap(), key_before);
        assert!(!unused.exists(), "a supplied pair writes nothing");
    }

    /// The third state, and the one a bare `Option<(PathBuf, PathBuf)>` could not express: a pair the
    /// user *named* that is not there must refuse to start and name the path — never quietly generate
    /// a certificate in its place (spec §5.5, §10). The last assertion is the one that makes the
    /// refusal reach the CLI: `ServerError` carries this error transparently, and the message a failed
    /// `serve` prints is the one checked here.
    #[tokio::test]
    async fn a_supplied_pair_that_is_not_there_refuses_to_start_and_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("no-cert.pem");
        let key_path = dir.path().join("no-key.pem");

        let cert = load_or_generate(supplied(cert_path.clone(), key_path.clone(), dir.path())).unwrap();
        assert!(cert.fingerprint.is_none(), "a named pair is never replaced by a minted one");
        assert!(
            !dir.path().join(CERT_FILE).exists() && !dir.path().join(KEY_FILE).exists(),
            "no certificate may appear beside config.toml when the user pointed at files"
        );

        let error = rustls_config(&cert).await.expect_err("a pair that is not there cannot be parsed");
        let text = error.to_string();
        assert!(text.contains(&cert_path.display().to_string()), "{text}");
        assert!(text.contains(&key_path.display().to_string()), "{text}");

        // The same sentence `serve` prints, through the variant `server::run` maps it into.
        let server_error: ServerError = error.into();
        assert!(
            server_error.to_string().contains(&cert_path.display().to_string()),
            "§10's refusal must reach the CLI with the path in it: {server_error}"
        );

        // A pair that is present and readable but is not a certificate is refused the same way.
        std::fs::write(&cert_path, "not a certificate at all").unwrap();
        std::fs::write(&key_path, "not a key at all").unwrap();
        let cert = load_or_generate(supplied(cert_path.clone(), key_path, dir.path())).unwrap();
        let error = rustls_config(&cert).await.expect_err("garbage is refused, not served");
        let text = error.to_string();
        assert!(text.contains(&cert_path.display().to_string()), "{text}");
        assert!(!text.contains("not a certificate at all"), "the file's contents are not a log line: {text}");
    }

    /// §9.4's rule, both halves: refused on plaintext from off this machine, accepted on loopback and
    /// on TLS.
    #[test]
    fn a_secret_may_only_cross_a_loopback_or_an_encrypted_connection() {
        let loopback: SocketAddr = "127.0.0.1:54123".parse().unwrap();
        let lan: SocketAddr = "192.168.1.30:54123".parse().unwrap();
        assert!(accepts_a_secret(loopback, false), "a client on this machine is the same machine");
        assert!(accepts_a_secret(loopback, true));
        assert!(accepts_a_secret(lan, true), "TLS is what makes a LAN connection good enough");
        assert!(!accepts_a_secret(lan, false), "a plaintext LAN connection must not carry a key");

        let set_key = ToServer::Command(Command::SetKey { name: "openrouter".into(), value: "sk-or-v1-xyz".into() });
        let refused = refuse_set_key(&set_key, lan, false).expect("a key must not cross a plaintext LAN");
        match refused {
            ToClient::Event(Event::Error { code, recoverable, message }) => {
                // The **code** is the assertion: it is the machine meaning Task 1.1 owns, and what
                // the screen and `session.rs` branch on. The message is a sentence for a person, so
                // nothing here matches its wording — only that it must not repeat the value it
                // refused, which is the one thing a refusal message could get wrong.
                assert_eq!(code, ErrorCode::InsecureTransport);
                assert!(recoverable, "the screen can retry it once the connection is TLS");
                assert!(message.len() > 40, "and the message is a sentence, not a code: {message}");
                assert!(!message.contains("sk-or-v1-xyz"), "the refusal must not repeat the key: {message}");
            }
            other => panic!("a refusal is an `Event::Error`, got {other:?}"),
        }
        assert!(refuse_set_key(&set_key, loopback, false).is_none());
        assert!(refuse_set_key(&set_key, lan, true).is_none());
        // Anything that is not a `SetKey` is nobody's business here.
        let ask = ToServer::Command(Command::Ask { text: "¿qué pasa?".into() });
        assert!(refuse_set_key(&ask, lan, false).is_none());
    }

    /// The listener is a TLS listener: the generated pair is what it serves, and the same route
    /// answers the same way over HTTPS as it does in the clear. Task 1.4 hands the *same* `Router` to
    /// `bind` and to `bind_rustls`, so this is the half of that claim a listener can prove.
    #[tokio::test]
    async fn a_tls_listener_serves_the_same_routes_as_the_plain_one() {
        let dir = tempfile::tempdir().unwrap();
        let cert = load_or_generate(generated(dir.path())).unwrap();
        let config = rustls_config(&cert).await.unwrap();
        let app = || Router::new().route("/", get(|| async { "the same page" }));

        let plain = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let plain_addr = plain.local_addr().unwrap();
        let plain_task = tokio::spawn(async move { axum::serve(plain, app()).await });

        let tls_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        tls_listener.set_nonblocking(true).unwrap();
        let tls_addr = tls_listener.local_addr().unwrap();
        let tls_task =
            tokio::spawn(async move { axum_server::from_tcp_rustls(tls_listener, config).serve(app().into_make_service()).await });

        let plain_body = reqwest::get(format!("http://{plain_addr}/")).await.unwrap().text().await.unwrap();
        let client = reqwest::Client::builder().danger_accept_invalid_certs(true).build().unwrap();
        let tls_body = client.get(format!("https://{tls_addr}/")).send().await.unwrap().text().await.unwrap();

        assert_eq!(plain_body, "the same page");
        assert_eq!(tls_body, plain_body, "TLS changes who may connect, not what the routes answer");

        plain_task.abort();
        tls_task.abort();
    }
}

// ------------------------------------------------- crates/tipsy-compadre/src/main.rs (replacing 1.7's test)
    #[test]
    fn the_cli_registers_tls_and_its_pair() {
        let cli = Cli::try_parse_from(["tipsy-compadre", "serve", "--tls"]).unwrap();
        match cli.command {
            Some(Cmd::Serve { tls, tls_cert, tls_key, .. }) => {
                assert!(tls);
                assert!(tls_cert.is_none() && tls_key.is_none());
            }
            other => panic!("`serve` must be a subcommand, got {other:?}"),
        }

        // Naming a pair is naming TLS: it is registered, and it is the pair that will be served.
        let cli = Cli::try_parse_from(["tipsy-compadre", "serve", "--tls-cert", "c.pem", "--tls-key", "k.pem"]).unwrap();
        match cli.command {
            Some(Cmd::Serve { tls_cert, tls_key, .. }) => {
                assert_eq!(tls_cert.as_deref(), Some(std::path::Path::new("c.pem")));
                assert_eq!(tls_key.as_deref(), Some(std::path::Path::new("k.pem")));
            }
            other => panic!("`serve` must be a subcommand, got {other:?}"),
        }

        // Half a pair is a usage error: one file without the other is a configuration mistake, not a
        // default (spec §5.5: "used when both are given").
        let error = Cli::try_parse_from(["tipsy-compadre", "serve", "--tls-cert", "c.pem"]).unwrap_err();
        assert!(error.to_string().contains("--tls-key"), "{error}");
        let error = Cli::try_parse_from(["tipsy-compadre", "serve", "--tls-key", "k.pem"]).unwrap_err();
        assert!(error.to_string().contains("--tls-cert"), "{error}");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre tls`

Expected: FAIL — `error[E0433]: failed to resolve: use of undeclared crate or module tls` (`src/server/tls.rs` does not exist and `server/mod.rs` declares no `tls` module), then `error[E0432]: unresolved import rcgen` and `unresolved import sha2` until the three dependencies are in `Cargo.toml`, then `error[E0560]: struct Cmd::Serve has no field named tls` in the CLI test, then `error[E0425]: cannot find function accepts_a_secret`. `ErrorCode::InsecureTransport` is **not** in that list: it is Task 1.1's variant and this task runs after it.

- [ ] **Step 3: Implement the certificate, the listener's TLS branch and the guard**

```rust
// ------------------------------------------------------------- crates/tipsy-compadre/src/server/tls.rs
//! The certificate `--tls` serves with, and the trust rule that keeps a key off a plaintext LAN.
//!
//! Two rules live here rather than in the listener. The first is §5.5's, and it has **three** states
//! (TLS off is the absence of the value, so `TlsFiles` has two): mint a self-signed pair once beside
//! `config.toml` and reuse it, or read a pair the user named and refuse to start — naming the path —
//! when one of those files is absent or unparseable. Which of the two is asked for arrives as
//! `crate::server::TlsFiles`, which **Task 1.4 owns** in `server/mod.rs` beside `Settings`; this file
//! consumes it and never declares it. The second rule is §9.4's: a `SetKey` may only cross a loopback
//! or an encrypted connection, and [`refuse_set_key`] is that decision.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum_server::tls_rustls::RustlsConfig;
use rcgen::{generate_simple_self_signed, CertifiedKey};
use sha2::{Digest, Sha256};
use tipsy_protocol::{Command, ErrorCode, Event, ToClient, ToServer};

use crate::server::TlsFiles;

/// The generated pair's two file names, in the config directory the `config path` subcommand prints
/// (research note §4.3 names both).
pub const CERT_FILE: &str = "serve-cert.pem";
pub const KEY_FILE: &str = "serve-key.pem";

/// Everything [`load_or_generate`] needs that the CLI's answer does not carry: the two facts
/// `TlsFiles` deliberately holds neither of. `files` is the cross-task value (Task 1.4's `TlsFiles`,
/// the one `Settings.tls` holds); `config_dir` is where `Generated` writes and `hostnames` is what it
/// is valid for, both known only to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Which of the two states `serve` asked for.
    pub files: TlsFiles,
    /// The directory `config.toml` lives in: `serve-cert.pem` and `serve-key.pem` go here.
    pub config_dir: PathBuf,
    /// The names a generated certificate is valid for. For a LAN bind this must include the literal
    /// address the browser will type, because SANs are what a browser checks (research note §4.1).
    pub hostnames: Vec<String>,
}

/// The pair `serve` will serve with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate {
    pub cert: PathBuf,
    pub key: PathBuf,
    /// `Some` only when this call minted the pair: the SHA-256 of the certificate's DER, as
    /// `AA:BB:…`, which is the same string a browser's certificate viewer shows. Computed here and
    /// never a constant.
    pub fingerprint: Option<String>,
}

/// Why TLS could not be set up. Both variants carry the paths: a user has to look at a file.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("cannot use the certificate pair: {cert} + {key}: {message}")]
    Pair { cert: PathBuf, key: PathBuf, message: String },
    #[error("cannot generate a self-signed certificate in {dir}: {message}")]
    Generate { dir: PathBuf, message: String },
}

/// The pair to serve with: the two files the user named, or the one beside `config.toml`, generated
/// here if it is not there yet. Called from `server::run`'s TLS branch, which is the only place that
/// can turn a `TlsError` into the refusal `serve` prints.
///
/// Generating writes two files with restrictive permissions where the platform has them, and nothing
/// else: the private key never leaves this directory, and no fingerprint is printed twice.
pub fn load_or_generate(paths: Paths) -> Result<Certificate, TlsError> {
    if let TlsFiles::Supplied { cert, key } = paths.files {
        // Exactly these two files: nothing is minted, nothing is written and nothing is replaced.
        // `rustls_config` is where a file that is missing or unparsable is refused, naming both.
        return Ok(Certificate { cert, key, fingerprint: None });
    }
    let cert = paths.config_dir.join(CERT_FILE);
    let key = paths.config_dir.join(KEY_FILE);
    if cert.exists() && key.exists() {
        // Generated by an earlier run: reuse it. A fingerprint is printed once, at generation.
        return Ok(Certificate { cert, key, fingerprint: None });
    }
    // Either nothing is there or only half of a pair is: mint both, so a key that does not match its
    // certificate is not a state this function can return.
    generate(&paths, cert, key)
}

/// Mints one pair, writes it, and reports the fingerprint of what it wrote.
///
/// Both files are removed first, because a call that gets here means at least one of them is missing
/// and a certificate left beside a key it does not match is worse than either. `write_private` then
/// claims each name with `create_new`. Two `serve --tls` starting in the same millisecond on a fresh
/// install can still interleave their writes; what that costs is the one failure `rustls_config`
/// refuses by name — a pair that does not match — and never a mismatched pair served silently.
fn generate(paths: &Paths, cert: PathBuf, key: PathBuf) -> Result<Certificate, TlsError> {
    let names = if paths.hostnames.is_empty() { vec!["localhost".to_string()] } else { paths.hostnames.clone() };
    let CertifiedKey { cert: minted, signing_key } = generate_simple_self_signed(names)
        .map_err(|e| TlsError::Generate { dir: paths.config_dir.clone(), message: e.to_string() })?;
    let fingerprint = fingerprint(minted.der());
    for path in [&cert, &key] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(TlsError::Generate { dir: paths.config_dir.clone(), message: e.to_string() }),
        }
    }
    write_private(&paths.config_dir, &key, signing_key.serialize_pem().as_bytes())
        .map_err(|e| TlsError::Generate { dir: paths.config_dir.clone(), message: e.to_string() })?;
    write_private(&paths.config_dir, &cert, minted.pem().as_bytes())
        .map_err(|e| TlsError::Generate { dir: paths.config_dir.clone(), message: e.to_string() })?;
    Ok(Certificate { cert, key, fingerprint: Some(fingerprint) })
}

/// The SHA-256 of a certificate's DER, in the notation a browser's certificate viewer uses.
fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(":")
}

/// Writes a file only the current user should read. Unix gets mode `0o600` (research note §4.3); on
/// Windows the file goes to `%APPDATA%`, which is already per-user, and this makes no claim beyond
/// that. `create_new` is what makes "generated once" true even if two `serve`s start together.
fn write_private(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.flush()
        }
        // Another `serve` won the race and wrote the pair first: its pair is as good as ours, and
        // overwriting it would rotate the certificate under a browser that just accepted it.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// The `RustlsConfig` the listener hands to `bind_rustls`. This is where the pair is parsed, so a
/// file that is not a certificate, or a key that does not match it, is refused here — with both
/// paths in the message (spec §10).
pub async fn rustls_config(cert: &Certificate) -> Result<RustlsConfig, TlsError> {
    RustlsConfig::from_pem_file(&cert.cert, &cert.key).await.map_err(|e| TlsError::Pair {
        cert: cert.cert.clone(),
        key: cert.key.clone(),
        message: e.to_string(),
    })
}

/// §9.4's whole rule: a secret may travel towards the server only over a loopback connection or over
/// TLS. "Loopback" is the *peer's* address: a client on this machine is the same machine, however it
/// reached us.
pub fn accepts_a_secret(peer: SocketAddr, tls: bool) -> bool {
    tls || peer.ip().is_loopback()
}

/// The answer a bridge sends **instead of** forwarding a `SetKey`, or `None` when the command is not
/// a `SetKey` or the connection may carry one.
///
/// `recoverable: true` is the honest flag: the same screen works once the connection is TLS, and
/// nothing about the refusal ends the connection. The **code** is the machine meaning — Task 1.1's
/// `ErrorCode::InsecureTransport`, the one value a client, a test or the session's own `SetKey` arm
/// may branch on — and the **message** below is a sentence for a person, which names the rule and
/// never the value that was refused.
pub fn refuse_set_key(command: &ToServer, peer: SocketAddr, tls: bool) -> Option<ToClient> {
    if !matches!(command, ToServer::Command(Command::SetKey { .. })) || accepts_a_secret(peer, tls) {
        return None;
    }
    Some(ToClient::Event(Event::Error {
        code: ErrorCode::InsecureTransport,
        message: "a new API key may only be sent over this machine's loopback connection or over TLS; \
                  restart the server with --tls, or set the key on the server machine"
            .to_string(),
        recoverable: true,
    }))
}
```

```rust
// ------------------------------------------------------------ crates/tipsy-compadre/src/server/mod.rs
// Task 1.4 owns this file, and `TlsFiles` is **1.4's type**, declared here beside `Settings` — not in
// `tls.rs`, because Task 1.7 builds a `Settings` with `tls: None` before Task 1.8 exists, and a type
// introduced by 1.8 would leave 1.4 and 1.7 uncompilable in between. Four changes, all named here so
// 1.4's body can be read first; if 1.4's committed version predates this ruling, the enum below is
// the one piece of 1.4's file to add verbatim.
//
// 1. The enum, next to `Settings`:
//
//    /// What `serve --tls` was asked for, in the form the server needs. Absent from `Settings.tls`
//    /// means TLS is off, so there are two states here, not three.
//    ///
//    /// The distinction is the whole reason this is an enum and not an `Option<(PathBuf, PathBuf)>`: a
//    /// user who names `--tls-cert` must get *that file* or a refusal that names it, never a
//    /// certificate quietly minted in its place (spec §5.5, §10). It derives `Debug` because
//    /// `Settings` does, which is the other reason it is not an `Option<RustlsConfig>` — that type has
//    /// none.
//    #[derive(Debug, Clone, PartialEq, Eq)]
//    pub enum TlsFiles {
//        /// `--tls` alone: mint the pair beside `config.toml` on first use, reuse it afterwards.
//        Generated,
//        /// `--tls` with `--tls-cert` and `--tls-key`: exactly these two files.
//        Supplied { cert: PathBuf, key: PathBuf },
//    }
//
// 2. `Settings.tls` is the CLI's answer, not a parsed pair. Task 1.7 already writes this field as
//    `None`; its type is:
//        /// `--tls`: which pair the listener serves with, or `None` for plaintext.
//        pub tls: Option<TlsFiles>,
//
// 3. `ServerError` gains the variant that carries a failed TLS setup to the CLI, path and all:
//        #[error(transparent)]
//        Tls(#[from] crate::server::tls::TlsError),
//
// 4. `run(settings)` loads the pair **before** it binds, because a certificate that cannot be read is
//    a refusal to start and not a listener that fails on its first connection (spec §10). The loading
//    lives here rather than in `main.rs` for two reasons: the refusal has to travel as one of this
//    function's own errors, and `main.rs` must not end up with a second error type of its own:
//
//    let tls = match &settings.tls {
//        None => None,
//        Some(files) => {
//            let paths = crate::server::tls::Paths {
//                files: files.clone(),
//                config_dir: settings.config_path.parent().unwrap_or(Path::new(".")).to_path_buf(),
//                hostnames: vec![settings.bind.clone(), "localhost".to_string()],
//            };
//            let certificate = crate::server::tls::load_or_generate(paths)?;
//            if let Some(fingerprint) = &certificate.fingerprint {
//                // Printed once, by the run that minted the pair, so a user can compare it with the
//                // browser's certificate viewer (spec §5.5).
//                println!("tipsy-compadre: certificate generated at {}", certificate.cert.display());
//                println!("tipsy-compadre: SHA-256 fingerprint {fingerprint}");
//            }
//            // What the flag buys, and what it does not, where a user reads it (spec §9.5).
//            eprintln!("tipsy-compadre: TLS encrypts this connection, so the token and a new API key are safe on a LAN.");
//            eprintln!("tipsy-compadre: it does not unlock the browser microphone from another machine: that needs a");
//            eprintln!("tipsy-compadre: certificate the browser already trusts (--tls-cert/--tls-key), or a client on this");
//            eprintln!("tipsy-compadre: machine, where http://127.0.0.1 is already a secure context.");
//            Some(crate::server::tls::rustls_config(&certificate).await?)
//        }
//    };
//
//    and then the listener by that value. The router is **the same value in both arms** — which is what
//    makes Task 1.8's "a TLS listener serves the same routes" true by construction:
//
//    match tls {
//        Some(config) => axum_server::tls_rustls::bind_rustls(addr, config)
//            .handle(handle.clone())
//            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
//            .await
//            .map_err(ServerError::from),
//        None => axum_server::bind(addr)
//            .handle(handle)
//            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
//            .await
//            .map_err(ServerError::from),
//    }
//
// The fingerprint and the two §9.5 lines moved here with the loading, for the reason the loading
// moved: this function is now the only place that knows whether the pair was minted or read. The
// `settings.bind` printed above is the address `--bind` overrode, which is also the address in the
// generated certificate's SAN list.
//
// `into_make_service_with_connect_info::<SocketAddr>()` is what makes the peer address available to
// the `SetKey` guard in `conn.rs` (axum's own `ConnectInfo`); 1.4's plaintext arm needs it too, so
// that change belongs to whichever of the two tasks runs second.

// ------------------------------------------------------------ crates/tipsy-compadre/src/server/conn.rs
// Task 1.6 owns this file. Two changes.
//
// 1. At the one place a socket message becomes a `ToServer`, before it is pushed into `to_server`:
//
//     if let Some(refusal) = crate::server::tls::refuse_set_key(&message, peer, tls) {
//         // The socket stays open: the screen can try again once the connection is TLS.
//         let _ = session_ends.events.send(refusal);
//         continue;
//     }
//
//    `peer` is the `ConnectInfo<SocketAddr>` Task 1.4's router now provides, and `tls` is the same
//    fact `Settings.tls` carries, so whatever context 1.6 threads down to the message loop gains one
//    `bool`. The in-process client has no socket and therefore no refusal: it is the same machine.
//
// 2. In 1.6's own `#[cfg(test)] mod tests`, the case Review Focus line 3 asks for — the whole point
//    is that the guard is *wired in*, not merely correct:
//
//     /// A `SetKey` from off this machine over plaintext never reaches the session, and the refusal
//     /// comes back on the session's own event channel; the same command from loopback does reach it.
//     #[tokio::test]
//     async fn a_set_key_off_a_plaintext_lan_never_reaches_the_session() {
//         // A fixture, not a key: the workspace rule is that keys appear by name and never in a
//         // file, a log or a fixture, and this string only proves that the value never crosses.
//         const FIXTURE: &str = "sk-or-v1-fixture0000000000000000000000000000";
//         let lan: SocketAddr = "192.168.1.30:54123".parse().unwrap();
//         let loopback: SocketAddr = "127.0.0.1:54123".parse().unwrap();
//         let command = ToServer::Command(Command::SetKey { name: "openrouter".into(), value: FIXTURE.into() });
//
//         let (to_server, mut reached) = tokio::sync::mpsc::unbounded_channel();
//         let (events, mut answered) = tokio::sync::mpsc::unbounded_channel();
//         // ... 1.6's own dispatch, driven with the message, `lan`, `tls = false`, and the two ends ...
//
//         assert!(
//             matches!(answered.try_recv(), Ok(ToClient::Event(Event::Error { code: ErrorCode::InsecureTransport, recoverable: true, .. }))),
//             "a key must not cross a plaintext LAN, and the screen must be told why"
//         );
//         assert!(reached.try_recv().is_err(), "and the session must never see it");
//
//         // The same command on loopback: forwarded, not refused.
//         // ... the same dispatch with `loopback` ...
//         assert!(matches!(reached.try_recv(), Ok(_)), "a client on this machine may set a key");
//     }

// ------------------------------------------------------------ crates/tipsy-compadre/src/main.rs
// `Cmd::Serve` gains the three flags. `tls_cert` and `tls_key` require each other and either one
// implies `--tls`, so a user who names a certificate has asked for TLS and half a pair is a usage
// error rather than a guess about which file they meant.
    Serve {
        /// Override the configured bind address for this run only; the file is never written.
        #[arg(long, value_name = "ADDR")]
        bind: Option<String>,
        /// Serve over TLS. The token and a new API key are then safe on a LAN; it does not by itself
        /// unlock the browser microphone from another machine (spec §9.5).
        #[arg(long)]
        tls: bool,
        /// A certificate to serve with. Needs `--tls-key`; implies `--tls`.
        #[arg(long, value_name = "PEM", requires = "tls_key")]
        tls_cert: Option<PathBuf>,
        /// The private key for `--tls-cert`.
        #[arg(long, value_name = "PEM", requires = "tls_cert")]
        tls_key: Option<PathBuf>,
    },

/// The `serve` arm. This function does **no** file I/O for TLS: it turns the three flags into the one
/// value `Settings.tls` takes, and the loading, the refusal and the fingerprint belong to
/// `server::run`'s TLS branch (see the `mod.rs` block above). The CLI is not where a `TlsError` is
/// printed, which is what keeps a second error type out of this file.
fn serve_command(bind: Option<String>, tls: bool, tls_cert: Option<PathBuf>, tls_key: Option<PathBuf>, verbose: bool) -> ExitCode {
    // ... the configuration load and the `--bind` override, exactly as Task 1.7 wrote them ...
    //
    // One of the three states of §5.5. `--tls-cert`/`--tls-key` is `Supplied`; `--tls` alone is
    // `Generated`; nothing is `None`. clap has already refused a lone `--tls-cert` or `--tls-key`,
    // so the `zip` below cannot silently drop a file the user named. `TlsFiles` is Task 1.4's type,
    // declared beside `Settings` in `server/mod.rs`; this arm only fills it in.
    let tls = match (tls || tls_cert.is_some(), tls_cert.zip(tls_key)) {
        (false, _) => None,
        (true, Some((cert, key))) => Some(tipsy_compadre::server::TlsFiles::Supplied { cert, key }),
        (true, None) => Some(tipsy_compadre::server::TlsFiles::Generated),
    };
    // ... the providers, and then Task 1.7's pinned `Settings` literal, unchanged except for the one
    // field this task owns. All eight fields are Task 1.7's; only the last line differs here:
    //
    //     let settings = tipsy_compadre::server::Settings {
    //         bind: config.server.bind.clone(),
    //         lan: config.server.lan,
    //         token: config.server.token.clone(),
    //         config,
    //         config_path: path,
    //         keys,
    //         providers,
    //         tls,
    //     };
    //
    // `server::run` then loads the pair, prints the fingerprint if it minted one, and picks
    // `bind_rustls` over `bind` — serving the same router either way. `serve` prints the address it
    // bound from there too, so a `--tls` run needs no second line here.
    //
    // One more edit in `main`, because the arm destructures the variant: Task 1.7 wrote
    // `(None, Some(Cmd::Serve { bind })) => serve_command(bind, cli.verbose)`, and it becomes
    // `(None, Some(Cmd::Serve { bind, tls, tls_cert, tls_key })) => serve_command(bind, tls, tls_cert, tls_key, cli.verbose)`.
}
```

Four notes for the implementer:

- **`--tls` does not unlock the browser microphone, and no test or message may say it does.** Spec
  §9.5 and research note §4.4: `http://127.0.0.1` is *already* a secure context, so the microphone
  works on the server machine with no TLS at all, and whether a page reached past a self-signed
  certificate interstitial counts as secure is marked INFERRED in the note and probably false. The
  three `eprintln!` lines above are the user-facing half of that ruling, and Task 1.9's README must
  repeat it.
- **SANs.** `generate_simple_self_signed` takes a `Vec<String>` and the crate's own example passes
  DNS names (docs.rs `rcgen/0.14.10/fn.generate_simple_self_signed.html`, read for this plan);
  whether a string that parses as an IP becomes an `SanType::IpAddress` is **INFERRED** in the
  research note (§4.1). Before writing `generate`, read
  <https://docs.rs/rcgen/0.14.10/rcgen/enum.SanType.html> and
  <https://docs.rs/rcgen/0.14.10/rcgen/struct.CertificateParams.html>; if the string list cannot
  produce an IP SAN, build `CertificateParams` with an explicit `SanType::IpAddress(ip)` — the note
  lists the fields it did not read, so read them rather than guessing. A certificate without the
  literal address in its SANs is a certificate the browser rejects.
- **`sha2` is ratified, and the version is load-bearing.** The fingerprint needs a SHA-256 and no
  crate in the Tech Stack list provides one. `Cargo.lock` already carries `sha2` at **`0.10.9` and
  `0.11.0`** — two transitive entries — so declare **`sha2 = "0.10"`**, which reuses the `0.10.9`
  entry; a caret on `0.11` would resolve the other copy and leave a third version in the tree, which
  is the opposite of the point. The plan author ratified the edge (the front matter's `server::tls`
  row records it; the precedent is Plan 2C's `base64` note,
  `.superpowers/sdd/2026-10-10-plan-2c-openai-realtime/progress.md:41`). If the ratification is ever
  withdrawn, the fallback that needs no dependency is to print the pair's paths and the SANs and drop
  `Certificate.fingerprint`, changing only `fingerprint`'s two call sites and the first test's
  assertions.
- **If the TLS route test cannot reach `https://`.** `reqwest` is a direct dependency with a rustls
  backend already in `Cargo.lock` (`hyper-rustls` and `rustls-platform-verifier`), and
  `ClientBuilder::danger_accept_invalid_certs` is the method that makes it accept this self-signed
  pair. If `https://` fails because the backend is not enabled for this crate's feature set, replace
  the `reqwest` half with a raw probe: connect a `tokio::net::TcpStream` to `tls_addr`, write
  `GET / HTTP/1.1\r\nHost: x\r\n\r\n`, and assert the bytes that come back contain no `HTTP/1.1` —
  a TLS listener answers nothing to plaintext. Then say in the ledger that the route half is Task
  1.4's `Router`, which both arms of `run` hand over unchanged.

- [ ] **Step 4: Run the test to verify it passes**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre tls`

Expected: PASS — 5 passed (`the_certificate_is_generated_once_and_then_reused`, `a_supplied_pair_is_used_and_never_generated_over`, `a_supplied_pair_that_is_not_there_refuses_to_start_and_names_the_path`, `a_secret_may_only_cross_a_loopback_or_an_encrypted_connection`, `a_tls_listener_serves_the_same_routes_as_the_plain_one`). The generated pair lives in the `tempfile::tempdir()` the test made, never in `%APPDATA%`.

Then run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre the_cli` and `... cargo test -j 1 -p tipsy-compadre connect`

Expected: PASS — 3 CLI tests (the TLS one replaced Task 1.7's) and Task 1.7's 3 socket tests, unchanged. The `conn.rs` case lives in Task 1.6's own test module and runs with the lib's tests.

Then the workspace and clippy:

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 --workspace`

Expected: PASS — no failures, and the three Task 1.7 tests plus these still green. Compare `cargo clippy --workspace --all-targets` with `global-constraints.md`: add nothing new.

An honest note on the fingerprint: it is **not** hard-coded anywhere — `fingerprint()` hashes the DER
of the certificate this run just minted, and the test checks its shape rather than its value, so a
regenerated certificate produces a different string without touching a test.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/Cargo.toml crates/tipsy-compadre/src/main.rs crates/tipsy-compadre/src/server/mod.rs crates/tipsy-compadre/src/server/conn.rs crates/tipsy-compadre/src/server/tls.rs crates/tipsy-compadre/tests/connect.rs
git commit -m "feat(server): add TLS and keep a new key off a plaintext LAN

serve --tls terminates TLS with a self-signed pair generated once beside
config.toml, whose SHA-256 fingerprint is printed once, or with a pair
--tls-cert/--tls-key name. SetKey is refused with insecure_transport unless
the peer is loopback or the connection is TLS. The task says in its output,
and in the README, what TLS does not buy: the browser microphone from
another machine still needs a certificate that browser trusts.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 1.9: Logging, redaction and the three documents `[small]`

Spec §9.2, §9.6, W12, §12. A headless server with no log is undebuggable, and the v1 spec §11
promise — logs never contain keys or audio — is cheaper to honour from the first line than to
retrofit.

**Files:**
- Create: `crates/tipsy-compadre/src/logging.rs`
- Modify: `crates/tipsy-compadre/src/lib.rs:3-23` (one line: `pub mod logging;`)
- Modify: `crates/tipsy-compadre/src/main.rs` — `main` installs the subscriber before anything else,
  and the three `announce` call sites keep their lines
- Modify: `crates/tipsy-compadre/src/keys.rs:97-102` (`KeyResolver::resolve`) — one line, so every key
  value this process has ever held is registered before it can be logged
- Modify: `crates/tipsy-compadre/Cargo.toml` — `tracing-subscriber` (**this task's alone**: `tracing` itself is Task 1.4's, because 1.4 is the first to call `tracing::error!`). Unlike `tracing`, which the lock already carries at `0.1.44`, `tracing-subscriber` is **absent from `Cargo.lock`** and no primary source was read for its series: declare it with `cargo add tracing-subscriber` at execution time and record the resolved version in the commit message rather than trusting the `"0.3"` a first draft wrote here
- Modify: `README.md:10-16` (the status paragraph), `:213-214` (the Server tab paragraph, replaced by
  two new sections), and `docs/architecture.md:73-78`, `:80-90`, `:153-164`, `CLAUDE.md:6-32`
- Test: `crates/tipsy-compadre/src/logging.rs` (its own `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `store::default_path() -> PathBuf` (`config/store.rs:13`); `KeyResolver::resolve(&self, name: &str) -> Result<Option<(String, KeySource)>, KeyError>` (`keys.rs:97-102`); `keys::redact(&str) -> String` (`keys.rs:106`); `Event::Error`, `ErrorCode` (`tipsy-protocol`); `Cli::verbose` and `announce` (Tasks 1.7 and 1.8)
- Produces:
  - `logging::log_path(config_path: &Path) -> PathBuf` — `pub fn`; `tipsy-compadre.log` beside `config.toml`, i.e. in the same user data directory `config path` prints
  - `logging::Secrets` — `pub struct`, `Clone`, default-constructible; `add(&self, value: &str)`, `scrub(&self, line: &str) -> String`. `logging::secrets() -> &'static Secrets` is the process-wide instance; `logging::hide(value: &str)` is the one-line form `keys.rs` calls
  - `logging::subscriber(log_path: &Path, verbose: bool) -> Result<Box<dyn tracing::Subscriber + Send + Sync>, String>` — `pub fn`; the file layer at `Level::DEBUG` (never `TRACE`: see the note below) and a stderr layer when `verbose`. It is public so a test can install it with `tracing::subscriber::with_default` instead of the global slot, which can only be taken once per process
  - `logging::init(log_path: &Path, verbose: bool) -> Result<PathBuf, String>` — `pub fn`; installs the global subscriber and returns the file it writes, so `main` (or `serve`) can print it once
- The three redaction rules, pinned: the **token never appears**, in any line and in any position — as a value (`Secrets::add` is given `config.server.token` by `main`), as a `token=` query parameter, and inside a request line, which is also why the two layers cap at `DEBUG`; **no key value ever appears**, because `KeyResolver::resolve` registers every value it returns and long opaque words are redacted by shape as well; **no audio is ever logged**, and a line that is a sample dump is dropped whole.

- [ ] **Step 1: Write the failing test**

```rust
// ---------------------------------------------------------- crates/tipsy-compadre/src/logging.rs (tests)
#[cfg(test)]
mod tests {
    use super::*;

    /// A token of the shape `store::new_token` produces: 32 bytes as lowercase hex (W8).
    const TOKEN: &str = "9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a39281706f5e4d3c2b1a0";
    /// A key of the shape the connectors use. A fixture, not a key: the workspace rule is that keys
    /// appear by name and never in a file, a log or a fixture, and this string exists only to prove
    /// that the value never reaches a log line.
    const KEY: &str = "sk-or-v1-0123456789abcdef0123456789abcdef";

    #[test]
    fn the_token_never_reaches_a_line_even_as_a_query_parameter() {
        let secrets = Secrets::default();
        secrets.add(TOKEN);
        for line in [
            // The request line a proxy or a socket crate would log, token and all.
            format!("GET /ws?token={TOKEN} HTTP/1.1"),
            // The URL a client builds or a user pastes.
            format!("ws://192.168.1.5:7070/ws?token={TOKEN}"),
            // A page request, and a sentence that happens to contain it.
            format!("GET /terminal?token={TOKEN}&lang=es"),
            format!("the presented token {TOKEN} does not match"),
        ] {
            let scrubbed = secrets.scrub(&line);
            assert!(!scrubbed.contains(TOKEN), "the token reached the log: {scrubbed}");
            // Exactly the secret goes, and nothing else: a log that loses the rest of the line is
            // not a log, and one that keeps a fragment of the token is a leak.
            assert_eq!(scrubbed, line.replace(TOKEN, REDACTED), "only the secret may change");
        }
    }

    #[test]
    fn a_key_value_never_reaches_a_line_by_value_or_by_shape() {
        let secrets = Secrets::default();
        // A value this process resolved: exact match, whatever its shape. Short values are not
        // registered at all (`Secrets::add` ignores anything under eight characters), because
        // redacting `"a"` would shred every line it appeared in.
        secrets.add("hunter2-there");
        assert!(!secrets.scrub("no API key named `hunter2-there`").contains("hunter2-there"));
        // A key that reached a line before its first resolution — the one moment it is not in
        // `Secrets` — is still unreadable: a long opaque word is a secret.
        let scrubbed = secrets.scrub(&format!("Authorization: Bearer {KEY}"));
        assert!(!scrubbed.contains(KEY), "{scrubbed}");
        // And this is not a shredder for anything long: a URL, a model slug and a path survive.
        for line in [
            "https://openrouter.ai/api/v1/chat/completions",
            "model google/gemini-2.5-flash answered",
            "C:\\Users\\me\\AppData\\Roaming\\tipsy-compadre\\config.toml",
        ] {
            assert_eq!(secrets.scrub(line), line, "redacting this would cost a reader a diagnosis");
        }
    }

    #[test]
    fn a_sample_dump_is_dropped_whole() {
        let secrets = Secrets::default();
        // What `tracing::debug!(?frame)` would print: the one accident this is shaped for.
        let frame = tipsy_protocol::Frame::Mic(vec![0, 1, -1, 32767, -32768].repeat(64));
        let line = format!("mic frame {frame:?}\n");
        assert_eq!(secrets.scrub(&line), format!("{REDACTED}\n"), "audio is never logged, not even redacted");
        // A line with a couple of numbers in it is a log line.
        let ordinary = "connection 2 of 3 up, latency 41 ms\n";
        assert_eq!(secrets.scrub(ordinary), ordinary);
    }

    /// The whole pipeline: the built subscriber, the real writer, and the three rules at once.
    #[test]
    fn a_record_written_through_the_subscriber_lands_scrubbed() {
        let dir = tempfile::tempdir().unwrap();
        let path = log_path(&dir.path().join("config.toml"));
        assert_eq!(path, dir.path().join("tipsy-compadre.log"));
        let secrets = Secrets::default();
        secrets.add(TOKEN);
        secrets.add(KEY);
        let subscriber = subscriber_at(&path, secrets, false).unwrap();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(url = %format!("ws://127.0.0.1:7070/ws?token={TOKEN}"), "connecting");
            tracing::debug!(key = %format!("Bearer {KEY}"), "handshake");
            tracing::debug!(frame = ?tipsy_protocol::Frame::Mic(vec![3; 320]), "a mistake");
        });

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("connecting"), "the log must be a log: {written}");
        assert!(written.contains("handshake"));
        assert!(!written.contains(TOKEN), "the token reached the log file: {written}");
        assert!(!written.contains(KEY), "a key value reached the log file: {written}");
        assert!(!written.contains("Mic([3, 3, 3"), "a sample dump reached the log file: {written}");
    }

    /// The level cap, and why it is the rule rather than a setting: `tokio-tungstenite` logs the
    /// **request line, header included, at `trace!`** — Plan 2C carried exactly that to its review
    /// (`.superpowers/sdd/2026-10-10-plan-2c-openai-realtime/progress.md:37`). A subscriber that can
    /// emit `TRACE` puts an `Authorization: Bearer <key>` in the file the day someone raises the
    /// level; this one cannot.
    #[test]
    fn the_subscriber_never_enables_trace() {
        let dir = tempfile::tempdir().unwrap();
        let subscriber = subscriber_at(&dir.path().join("tipsy-compadre.log"), Secrets::default(), true).unwrap();
        assert_eq!(
            subscriber.max_level_hint(),
            Some(tracing_subscriber::filter::LevelFilter::DEBUG),
            "TRACE is where the request line with the key is logged; it must never be enabled"
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre logging`

Expected: FAIL — `error[E0583]: file not found for module logging` (`src/lib.rs` declares it after Step 3, and `src/logging.rs` does not exist), then `error[E0432]: unresolved import tracing_subscriber` until the two dependencies are in `Cargo.toml`, then `error[E0425]: cannot find function subscriber_at`.

The documentation half of this task has no failing test, and its failure is prose, so it is stated here instead: before this task a reader who follows `README.md:213-214` is told the Server tab "is for the WebSocket server, which does not exist yet" and that "nothing listens on them today" — both now false, and neither `serve` nor `--connect` appears anywhere in the README; and `docs/architecture.md:163-164` promises that remote web clients "need HTTPS with a self-signed certificate", which §9.5 rules out as a way to get the microphone. A reader following either document today reaches a wrong conclusion.

- [ ] **Step 3: Implement the bootstrap, the three rules and the documents**

```rust
// -------------------------------------------------------------- crates/tipsy-compadre/src/logging.rs
//! `tracing` to a file, `--verbose` also to stderr, and the three things that never reach either
//! (spec §9.2, §9.6, and the v1 spec §11's "logs never contain keys or audio").
//!
//! The rules are enforced *here*, on the way out, and not at each call site: a call site that forgets
//! is exactly the accident this has to survive. Every complete line is scrubbed before it reaches the
//! file, so a secret split across two `write` calls cannot slip through either.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tracing::Subscriber;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Layer;

/// What a redacted value becomes. One token, so a reader can tell "there was something here" from
/// "there was nothing".
pub const REDACTED: &str = "<redacted>";

/// The most digits a line may carry, comma-separated, before it is treated as audio. One microphone
/// frame is 320 samples (`audio/device.rs:20`), so 32 is far below anything legitimate: this is the
/// backstop behind "no call site logs audio", and `a_sample_dump_is_dropped_whole` is what fires it.
const SAMPLE_RUN: usize = 32;

/// The shortest opaque word treated as a secret by shape. A key is long and random; a URL, a model
/// slug and a path are not (they carry `/`, `.` or `:`), so this does not redact diagnostics.
const OPAQUE: usize = 32;

/// The values that must never appear in a log line: the server's token, and every API key value this
/// process has resolved. `keys::KeyResolver::resolve` is what fills the second half, so a value is
/// registered before anything can log it.
#[derive(Clone, Default)]
pub struct Secrets(Arc<Mutex<Vec<String>>>);

impl Secrets {
    /// Registers a value. Empty and very short values are ignored: redacting `""` or `"a"` would
    /// shred every line it appeared in.
    pub fn add(&self, value: &str) {
        let value = value.trim();
        if value.len() < 8 {
            return;
        }
        let mut secrets = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !secrets.iter().any(|known| known == value) {
            secrets.push(value.to_string());
        }
    }

    /// The three rules, applied to one line.
    pub fn scrub(&self, line: &str) -> String {
        if is_a_sample_dump(line) {
            // Audio is never logged, not even redacted: a line that is a sample dump is dropped
            // whole, so nobody can reconstruct a second of the microphone from the file.
            return format!("{REDACTED}\n");
        }
        let mut scrubbed = line.to_string();
        {
            let secrets = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            for secret in secrets.iter() {
                scrubbed = scrubbed.replace(secret.as_str(), REDACTED);
            }
        }
        // And the shapes that are secrets whether or not this process has seen them yet: the
        // `token=` query parameter of a page request (spec §9.2) and any long opaque word — an
        // `Authorization` header's value, a key in a sentence (`sk-or-…a1b2` is what `keys::redact`
        // shows a user, and it is the shape this catches).
        scrubbed = redact_query_tokens(&scrubbed);
        redact_opaque_words(&scrubbed)
    }
}

/// The process-wide instance, and the one-line form `keys.rs` calls.
pub fn secrets() -> &'static Secrets {
    static SECRETS: OnceLock<Secrets> = OnceLock::new();
    SECRETS.get_or_init(Secrets::default)
}

/// Registers a value that must never reach the log. Called by `KeyResolver::resolve`.
pub fn hide(value: &str) {
    secrets().add(value);
}

/// Where the log lives: beside `config.toml`, in the directory `tipsy-compadre config path` prints.
pub fn log_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("tipsy-compadre.log")
}

/// The subscriber: the file always, stderr too when `--verbose` (spec §9.6, v1 spec §12).
///
/// The level is `DEBUG` and never `TRACE`, for a reason that is not a preference: `tokio-tungstenite`
/// logs the request line — headers included — at `trace!`, which is where an `Authorization: Bearer`
/// would go (Plan 2C's review carried exactly that).
pub fn subscriber(log_path: &Path, verbose: bool) -> Result<Box<dyn Subscriber + Send + Sync>, String> {
    subscriber_at(log_path, secrets().clone(), verbose)
}

/// The same, with the secrets a test chose, so the rules can be exercised without the global slot —
/// which a process may take only once.
pub fn subscriber_at(
    log_path: &Path,
    secrets: Secrets,
    verbose: bool,
) -> Result<Box<dyn Subscriber + Send + Sync>, String> {
    if let Some(dir) = log_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| format!("{}: {e}", log_path.display()))?;
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(Mutex::new(ScrubWriter::new(file, secrets)))
        .with_filter(LevelFilter::DEBUG);
    let base = tracing_subscriber::registry().with(layer);
    if verbose {
        Ok(Box::new(base.with(tracing_subscriber::fmt::layer().with_writer(io::stderr).with_filter(LevelFilter::DEBUG))))
    } else {
        Ok(Box::new(base))
    }
}

/// Installs the subscriber for the whole process and reports the file it writes. Called from `main`
/// before anything else, so the first event of a run is recorded.
pub fn init(log_path: &Path, verbose: bool) -> Result<PathBuf, String> {
    let subscriber = subscriber(log_path, verbose)?;
    let path = log_path.to_path_buf();
    tracing::subscriber::set_global_default(subscriber).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The file, with every complete line scrubbed on its way through.
struct ScrubWriter {
    file: std::fs::File,
    secrets: Secrets,
    /// What is left of a line whose newline has not arrived yet. The `fmt` layer writes each event
    /// in several calls, so scrubbing per call would let a secret straddle two of them.
    pending: Vec<u8>,
}

impl ScrubWriter {
    fn new(file: std::fs::File, secrets: Secrets) -> Self {
        Self { file, secrets, pending: Vec::new() }
    }

    fn drain_whole_lines(&mut self, last: bool) -> io::Result<()> {
        loop {
            let end = self.pending.iter().position(|byte| *byte == b'\n');
            let line: Vec<u8> = match end {
                Some(end) => self.pending.drain(..=end).collect(),
                None if last && !self.pending.is_empty() => std::mem::take(&mut self.pending),
                None => return Ok(()),
            };
            let text = String::from_utf8_lossy(&line).into_owned();
            let scrubbed = self.secrets.scrub(&text);
            self.file.write_all(scrubbed.as_bytes())?;
        }
    }
}

impl Write for ScrubWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        self.drain_whole_lines(false)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain_whole_lines(true)?;
        self.file.flush()
    }
}

impl Drop for ScrubWriter {
    fn drop(&mut self) {
        // A last line without its newline still gets written, scrubbed.
        let _ = self.flush();
    }
}

/// Whether a run of comma-separated numbers says this line is a sample dump rather than a log line.
fn is_a_sample_dump(line: &str) -> bool {
    line.split(',')
        .filter(|part| {
            let digits = part.trim().trim_matches(|c: char| !c.is_ascii_digit() && c != '-');
            digits.parse::<i64>().is_ok()
        })
        .count()
        >= SAMPLE_RUN
}

/// `token=…` in a query string or a request line: the value goes, the parameter stays, so the line
/// still says what happened (spec §9.2).
fn redact_query_tokens(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("token=") {
        let (before, after) = rest.split_at(at + "token=".len());
        out.push_str(before);
        let end = after.find(['&', ' ', '"', '\'', '\n']).unwrap_or(after.len());
        if end > 0 {
            out.push_str(REDACTED);
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// A long word of nothing but key-ish characters: no `/`, `.` or `:` — which is why a URL, a model
/// slug and a path are left alone — and at least [`OPAQUE`] characters.
fn redact_opaque_words(line: &str) -> String {
    line.split_inclusive([' ', '\t', '"', '\'', ',', '=', ':', ';'])
        .map(|chunk| {
            let (word, tail) = split_trailing(chunk);
            let opaque = word.len() >= OPAQUE
                && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                && word.chars().any(|c| c.is_ascii_digit())
                && word.chars().any(|c| c.is_ascii_alphabetic());
            if opaque { format!("{REDACTED}{tail}") } else { chunk.to_string() }
        })
        .collect()
}

/// Splits the punctuation a word may be carrying from the word itself.
fn split_trailing(chunk: &str) -> (&str, &str) {
    let end = chunk.trim_end_matches([' ', '\t', '"', '\'', ',', '=', ':', ';', '\n', '\r']).len();
    chunk.split_at(end)
}
```

```rust
// -------------------------------------------------------------- crates/tipsy-compadre/src/lib.rs
pub mod logging;   // after `pub mod keys;`

// -------------------------------------------------------------- crates/tipsy-compadre/src/keys.rs
// Inside `KeyResolver::resolve`, on the value that is about to be returned, so it is registered
// before any call site — a connector, a probe, an error message — can log it:
    pub fn resolve(&self, name: &str) -> Result<Option<(String, KeySource)>, KeyError> {
        if let Some(v) = (self.env)(&env_var_name(name)).filter(|v| !v.trim().is_empty()) {
            let v = v.trim().to_string();
            crate::logging::hide(&v);
            return Ok(Some((v, KeySource::Env)));
        }
        Ok(self.store.get(name)?.map(|v| {
            crate::logging::hide(&v);
            (v, KeySource::Store)
        }))
    }

// -------------------------------------------------------------- crates/tipsy-compadre/src/main.rs
fn main() -> ExitCode {
    let cli = Cli::parse();
    // Before anything else, so the first event of the run is recorded and every line after it has
    // somewhere to go (spec §9.6). A log that cannot be opened is a line on stderr, not a refusal:
    // the assistant works without its log.
    let config_path = store::default_path();
    match tipsy_compadre::logging::init(&tipsy_compadre::logging::log_path(&config_path), cli.verbose) {
        Ok(path) => announce(cli.verbose, format!("log: {}", path.display())),
        Err(e) => eprintln!("tipsy-compadre: no log file: {e}"),
    }
    // ... and the token, which must never reach that file, is registered before anything can log it.
    if let Ok(loaded) = store::load_or_create(&config_path) {
        tipsy_compadre::logging::hide(&loaded.config.server.token);
    }
    // ... then the match Task 1.7 and Task 1.8 wrote, unchanged; `announce` stays, because a startup
    // line a person reads on the terminal they just started is not a log record.
}
```

````markdown
<!-- README.md:10-16 — the status paragraph. Only the sentence that is now false changes. -->
**Status: early development — terminal client and server.** The workspace, the protocol, the
configuration and the key handling, the chat, speech and transcription providers (the
OpenAI-compatible connector plus the native Gemini and ElevenLabs ones), the brain, the native
terminal client and its configuration screen all work, and nine profiles ship with it. Two of the
three listening modes work: `push`, where you open and close the microphone by hand, and `auto`,
where a voice detector decides what is an utterance. The server and the remote client work too:
`tipsy-compadre serve` listens, and `tipsy-compadre --connect <ws-url>` attaches a terminal to it.
Still to come: `hands_free` (it waits for the wake word), the web client and the web terminal (the
last two phases of the same plan), plus CI and releases.

<!-- README.md:213-214 — the Server-tab paragraph, replaced by the two sections below. -->
The Server tab — `bind`, the LAN switch and the token — configures the WebSocket server. `serve`
reads those three values; the token is the password every remote client presents, and `Ctrl+R`'s
fresh value is written when you save.

## Running a server, and attaching to one

```sh
# The server, headless: the web client at / and the web terminal at /terminal.
cargo run -p tipsy-compadre -- serve

# The same TUI you get from a bare run, drawn on this machine and driven by that server.
cargo run -p tipsy-compadre -- --connect ws://127.0.0.1:7070/ws

# From another machine, once `lan = true` and a token are in config.toml:
cargo run -p tipsy-compadre -- --connect ws://192.168.1.20:7070/ws --token <the Server tab's token>
```

- `serve` prints where it is listening. `--bind` overrides `bind` for one run and never writes the
  file back.
- `lan = true` with an empty token **refuses to start**, naming `lan`: an exposed server with no
  token is open to anything on the network. The same is true of any `bind` that is not a loopback
  address, and a hostname is refused by name rather than resolved.
- The token is a 32-byte value generated on the first run and stored in `config.toml`. It travels in
  `Hello` for a protocol client and as `?token=…` on the two pages, which a browser can only send in
  the URL; both pages therefore answer with `Referrer-Policy: no-referrer` and `Cache-Control:
  no-store`, so the token is not in a `Referer` and not in a cache.
- `/` is the web client and `/terminal` the web terminal. A plain `cargo build` embeds no bundle:
  both routes then answer with a page that says to build it with `cargo xtask dist`.
- `--connect` runs the TUI and nothing else: it starts no server, builds no provider and reads no
  API key, because every provider call happens in the server process.
- `--verbose` also writes the log to stderr. The log file is `tipsy-compadre.log`, beside
  `config.toml`, and it never contains the token, a key or audio.

### TLS, and what it does and does not do

`serve --tls` encrypts the connection, which is what makes the token and a newly set API key safe on
a LAN. With no `--tls-cert`/`--tls-key` pair it generates a self-signed certificate once, beside
`config.toml`, and prints its SHA-256 fingerprint once so you can compare it with your browser's
certificate viewer.

**It does not unlock the browser's microphone from another machine.** Browsers expose
`getUserMedia` only in a secure context; `http://127.0.0.1` already is one, so the microphone works
on the server machine with no TLS at all; and a certificate the browser does not trust is not a
secure context. For a browser on another device, use a certificate that device already trusts —
`mkcert` with its root installed, or a real certificate for a name that resolves to the LAN
address — through `--tls-cert`/`--tls-key`.
````

```markdown
<!-- docs/architecture.md:73-78 (the crate table): its two future rows say when they arrive. -->
| Crate | Kind | Contents | Compiles to WASM |
|---|---|---|---|
| `tipsy-protocol` | library | `Command` / `Event` messages and the audio frame format | yes |
| `tipsy-ui` | library | The ratatui HUD: widgets, state and es/en strings, backend-agnostic | yes |
| `tipsy-web` | `.wasm` bundle | Web client: Ratzilla backend, browser audio, WebSocket transport | yes (target) — **arrives in Phase 3 of `2026-10-11-plan-3-web-tier.md`; it does not exist yet** |
| `tipsy-compadre` | **binary** | The server (brain, providers, config, axum, the WebSocket door) plus the native TUI client | no |
| `xtask` | binary | `cargo xtask dist`: fetch and verify xterm.js, build the bundle, build the binary that embeds it — **arrives in Phase 2 of the same plan** | no |

<!-- docs/architecture.md:80-90 (the build paragraph): one sentence on what is true today, and one on
     the cost of the stack the plan chose. -->
The server's HTTP and WebSocket layer is `axum` 0.8.9 with its `ws` feature, and TLS is
`axum-server` 0.8.0 (`tls-rustls`). One consequence is worth writing down rather than discovering:
`axum` pins `tokio-tungstenite` **0.29** while the native `--connect` client keeps **0.30**, so two
copies of `tungstenite` are compiled into the one executable (spec W1). A full `cargo build` today
produces a binary with no web bundle; the web client is served once `cargo xtask dist` has built it,
and until then `/` and `/terminal` answer with the page that says so.

<!-- docs/architecture.md:153-164 (security): the token and the TLS paragraph. -->
- It binds to `127.0.0.1` by default, `serve` refuses to start on any other address without a token,
  and the token is a 32-byte value generated on the first run.
- API keys, configuration and conversation history never leave the server; clients never see a key.
  The one case where a secret travels *towards* the server is setting a new key, and `SetKey` is
  refused with `insecure_transport` unless that connection is loopback or TLS.
- Both web routes (`/` and `/terminal`) require the token when the server is exposed to the LAN.
  The web terminal grants full control of the application, so it is never reachable without it.
- `serve --tls` encrypts the connection: that is what makes the token and a new key safe on a LAN.
  It does **not** by itself unlock `getUserMedia` on another machine — loopback over plain HTTP is
  already a secure context, and a certificate the browser has not been told to trust is not one. The
  microphone from another device needs a certificate that device trusts (`--tls-cert`/`--tls-key`).
```

```markdown
<!-- CLAUDE.md:25-32 — "Written and not started", "Not started". Replace those two paragraphs with
     the three that follow, keeping the sections above and below untouched. Read the three ledgers
     again immediately before writing them: this is state, and state moves. -->
**In flight: Plan 2C** (`docs/superpowers/plans/2026-10-10-plan-2c-openai-realtime.md`), the OpenAI
Realtime voice mode. Four of its six tasks are committed on branch `plan-2c-openai-realtime`
(`d6abfd7` the section and `Event::Live`, `9cb0829` the socket client, `e7bd82f` the event mapping,
`c4ae8fa` the turn plumbing); tasks 5 and 6 remain, and the branch is not merged. Its ledger is
`.superpowers/sdd/2026-10-10-plan-2c-openai-realtime/progress.md`.

**In flight, in its own worktree: the login screen**
(`docs/superpowers/plans/2026-10-11-plan-login-screen.md` and
`docs/superpowers/specs/2026-10-11-login-screen-design.md`). It has a worktree of its own at
`E:\workspace\personal\tipsy-login` on branch `plan-login-screen` (`3cea34c` at the time of
writing), so its uncommitted work never mixes with this checkout's. Tasks 1–3 are committed; tasks
4–6 are in the plan and not written. Its ledger is
`.superpowers/sdd/2026-10-11-plan-login-screen/progress.md`.

**In flight: Plan 3** (`docs/superpowers/plans/2026-10-11-plan-3-web-tier.md`), the web tier: the
WebSocket server, `--connect`, the web terminal and the WASM web client, in three phases. Phase 1
(tasks 1.1–1.9) is what a session of this plan is executing; Phases 2 and 3 are written and Phase
3's first task is a gate — if the Ratzilla spike fails, tasks 3.2–3.6 are abandoned and the browser
answer for v1 stays the web terminal. Its ledger is
`.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md`, and the constraints every task inherits
are beside it in `global-constraints.md`.
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre logging`

Expected: PASS — 5 passed (`the_token_never_reaches_a_line_even_as_a_query_parameter`, `a_key_value_never_reaches_a_line_by_value_or_by_shape`, `a_sample_dump_is_dropped_whole`, `a_record_written_through_the_subscriber_lands_scrubbed`, `the_subscriber_never_enables_trace`). The log file is written inside each test's `tempfile::tempdir()`, never into `%APPDATA%`.

Then the workspace and clippy:

Run: `$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 --workspace`

Expected: PASS — no failures. `keys.rs:81` and `keys.rs:85` are on the pinned pre-existing clippy list, and this task must not fix them or add to the list; the same for `tipsy-protocol/src/lib.rs:202`, which masks the rest.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/Cargo.toml crates/tipsy-compadre/src/lib.rs crates/tipsy-compadre/src/logging.rs crates/tipsy-compadre/src/main.rs crates/tipsy-compadre/src/keys.rs README.md docs/architecture.md CLAUDE.md
git commit -m "feat(logging): add the log file, its three redaction rules and the documents

tracing to tipsy-compadre.log beside config.toml, --verbose also to stderr,
and every complete line scrubbed on its way out: the token in any position
including a query parameter and a request line, every key value this process
resolved, and any sample dump. The two layers cap at DEBUG so the
tokio-tungstenite trace! request line stays unreachable. README, architecture
and CLAUDE.md now describe the server, the remote client and what TLS does
not buy.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 2.1: The PTY, its child and its teardown `[mid]`

**Files:**
- Create: `crates/tipsy-compadre/src/client/pty.rs`
- Modify: `crates/tipsy-compadre/src/client.rs:1-4` (add `pub mod pty;`; do **not** add a `use` for it — nothing in this file needs it)
- Modify: `crates/tipsy-compadre/Cargo.toml:32-33` (add `portable-pty` under `[dependencies]`, immediately after `tokio-tungstenite`)
- Test: `crates/tipsy-compadre/tests/pty.rs` (a new integration test), plus `crates/tipsy-compadre/tests/support/process.rs` (a new shared test-only helper) and this file's own `#[cfg(test)] mod tests` for the unit half

**Interfaces:**

- Consumes: nothing from this plan. Everything below is `portable-pty` 0.9.0, quoted from
  [`.superpowers/sdd/plan-3-research/web-transport-and-terminal.md`](../../../.superpowers/sdd/plan-3-research/web-transport-and-terminal.md)
  §5.1 and re-read off docs.rs while writing this task:
  - `portable_pty::native_pty_system() -> Box<dyn PtySystem + Send>`
    ([`fn.native_pty_system.html`](https://docs.rs/portable-pty/0.9.0/portable_pty/fn.native_pty_system.html)).
  - `PtySystem::openpty(&self, size: PtySize) -> anyhow::Result<PtyPair>`;
    `PtyPair { pub slave: Box<dyn SlavePty + Send>, pub master: Box<dyn MasterPty + Send> }`.
  - `SlavePty::spawn_command(&self, cmd: CommandBuilder) -> anyhow::Result<Box<dyn Child + Send + Sync>>`.
  - `MasterPty: Downcast + Send` with `fn resize(&self, size: PtySize) -> anyhow::Result<()>`,
    `fn try_clone_reader(&self) -> anyhow::Result<Box<dyn std::io::Read + Send>>`,
    `fn take_writer(&self) -> anyhow::Result<Box<dyn std::io::Write + Send>>`.
  - `Child: Debug + ChildKiller + Downcast + Send` with
    `fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>>`, `fn wait(&mut self) -> std::io::Result<ExitStatus>`,
    `fn process_id(&self) -> Option<u32>`; and `ChildKiller` with `fn kill(&mut self) -> std::io::Result<()>`.
    `Child::kill` is reachable because `Child: ChildKiller` — that is the supertrait list read off
    [`trait.Child.html`](https://docs.rs/portable-pty/0.9.0/portable_pty/trait.Child.html).
  - `CommandBuilder::new<S: AsRef<OsStr>>(program: S) -> Self`, `fn arg<S: AsRef<OsStr>>(&mut self, arg: S)`,
    `fn args<I, S>(&mut self, args: I) where I: IntoIterator<Item = S>, S: AsRef<OsStr>`
    ([`cmdbuilder/struct.CommandBuilder.html`](https://docs.rs/portable-pty/0.9.0/portable_pty/cmdbuilder/struct.CommandBuilder.html)).
  - `ExitStatus::success(&self) -> bool` and `ExitStatus::exit_code(&self) -> u32`.
  - `PtySize { pub rows: u16, pub cols: u16, pub pixel_width: u16, pub pixel_height: u16 }`.
- Produces, in `tipsy_compadre::client::pty` (the names below are the contract; a later task may add
  to this module but may not spell any of these differently):

```rust
pub type BoxedRead = Box<dyn std::io::Read + Send>;
pub type BoxedWrite = Box<dyn std::io::Write + Send>;

/// A child running inside a real pseudo-terminal, with the two blocking ends `portable-pty` hands
/// back. Nothing here is async: `portable-pty` does not depend on tokio (research note §5.2), so the
/// reader is pumped by a `spawn_blocking` task and the writer by a task of its own.
pub struct PtySession {
    master: Arc<dyn portable_pty::MasterPty + Send>,
    reader: Mutex<Option<BoxedRead>>,
    writer: Mutex<Option<BoxedWrite>>,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl PtySession {
    /// `program` is launched with exactly `args` — no shell, no command-line parsing, so a path with a
    /// space and an argument with a space are both safe.
    pub fn spawn(
        program: &std::path::Path,
        args: &[String],
        cols: u16,
        rows: u16,
    ) -> Result<PtySession, String>;
    /// The same, with `NAME=value` pairs added to the child's own environment. Exists for the
    /// security test in Step 1, which gives the real child a token no planted fake can know; nothing
    /// in production sets one, and no API key may ever travel this way.
    pub fn spawn_with_env(
        program: &std::path::Path,
        args: &[String],
        env: &[String],
        cols: u16,
        rows: u16,
    ) -> Result<PtySession, String>;
    /// The only resize call this project makes. `MasterPty::resize`'s own doc says it "will update the
    /// winsize information maintained by the kernel, and generate a signal for the child to notice and
    /// update its state" — `SIGWINCH` on Unix, `ResizePseudoConsole` behind ConPTY on Windows. There is
    /// **no `libc::kill` anywhere in this phase** (spec §7.2); this call is the whole mechanism.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String>;
    /// The reader, once. Dropping it does not touch the child; dropping the master does.
    pub fn take_reader(&self) -> Option<BoxedRead>;
    /// The writer, once. Dropping it sends EOF to the slave end (research note §5.1).
    pub fn take_writer(&self) -> Option<BoxedWrite>;
    /// The child's pid, for a test that has to ask the OS whether it is still there.
    pub fn process_id(&self) -> Option<u32>;
    /// An independent handle to the child's `kill`, so the pump can terminate the child from a
    /// `&self` borrow instead of taking the whole session apart. `portable-pty` provides it "in order
    /// to send it signals independently from a thread that may be blocked in `.wait`"
    /// ([`ChildKiller::clone_killer`](https://docs.rs/portable-pty/0.9.0/portable_pty/trait.ChildKiller.html)).
    pub fn killer(&self) -> Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>;
    /// Terminate the child. Idempotent. Does not block on reaping — that is `wait`'s job.
    pub fn kill(&mut self) -> Result<(), String>;
    /// Reap the child. Blocks, so it belongs on a `spawn_blocking` task in production and is called
    /// directly only in tests, after `kill`.
    pub fn wait(&mut self) -> Result<portable_pty::ExitStatus, String>;
    /// `kill` then `wait`: the pair that proves the process is gone rather than merely disowned.
    pub fn kill_and_wait(&mut self) -> Result<portable_pty::ExitStatus, String>;
}

/// What a PTY is spawned from. The server's `/terminal/ws` handler takes one of these so that its
/// tests run a real PTY over a command that is not the project's binary, while `serve` uses
/// `TerminalSpawner` below. Object-safe on purpose: `server::conn` holds one through a `dyn`.
pub trait PtySpawner: Send + Sync + 'static {
    fn args(&self) -> Vec<String>;
    /// `NAME=value` pairs for the child's own environment. Empty in production.
    fn env(&self) -> Vec<String> { Vec::new() }
    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String>;
}

/// The production spawner: **this process's own binary in `--connect` mode, pointed at this process's
/// own loopback `/ws`** (spec §7.2, W10). `program` is `std::env::current_exe()` — the absolute path
/// of the running executable, resolved by the operating system rather than searched for in `PATH` —
/// and `args` are the `--connect`/`--token` arguments of Task 1.7.
#[derive(Default)]
pub struct TerminalSpawner {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
    pub env: Vec<String>,
}

impl PtySpawner for TerminalSpawner { /* as implemented in Step 3 */ }
```

**Two facts this task must prove rather than assert** (spec §7.2's "consequences to write down
rather than discover"):

1. The child starts **no** server and makes **no** provider call. Its `--connect` path is what makes
   that true, and the test below asserts the child was still alive after a failed connection attempt
   — which cannot happen if it had tried to `build_providers` (that path exits
   `ExitCode::from(MISSING_KEY)` when a key is missing, and `main.rs:75` builds providers only on the
   bare-run path).
2. `MasterPty::resize` is what reaches the child. The resize test below reads the new width back out
   of the child's own output; nothing in this repository calls `libc::kill`, `ResizePseudoConsole` or
   `SIGWINCH` itself.

**A third requirement, and it is a security rule rather than a consequence.** The child is spawned as
**`std::env::current_exe()` — never by name**, with no `PATH` lookup, no relative path and no bare
`"tipsy-compadre"`. The argument is the prior-art finding recorded in
[`.superpowers/sdd/plan-3-research/codewhale-audio-prior-art.md`](../../../.superpowers/sdd/plan-3-research/codewhale-audio-prior-art.md)
§5: an empty (`::`), relative (`.`) or repository-local `PATH` entry "would let a checked-out workspace
plant a helper that … runs with the user's authority", which the project's own prior art refuses to
accept even for a cue player. Here the blast radius is larger, because the child is a **full client**
whose behaviour is driven by whatever the browser sends over `/terminal/ws`. The test in Step 1 plants
a fake `tipsy-compadre.exe` in a `PATH` entry and proves it never runs.

- Produces, in `tipsy_compadre::server` — **one variant this task adds to Task 1.4's `ServerError`**:

```rust
/// Task 1.4 leaves `ServerError` with exactly four variants: `BadBind { bind }`,
/// `LanWithoutToken { addr }`, `BoundWiderThanAsked { addr }` and `Io(std::io::Error)`. This task is
/// the first that has to report a failure which is neither of those — `std::env::current_exe()`
/// failing while `server::run` builds the terminal spawner — so it adds the fifth, spelled exactly
/// like this. Task 1.8 separately adds `Tls(TlsError)` to the same enum: the two must not collide,
/// and neither task may rename the other's variant.
Message(String),
```

with its `Display` arm and its `source`, in the same impls that already carry the other four:

```rust
impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // … Task 1.4's four arms …
            Self::Message(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            // `Message` carries a rendered sentence, not a cause: there is nothing to walk to.
            Self::Message(_) => None,
            // … Task 1.4's own arm for `Io`, which returns the wrapped error …
            other => other.source_or_std(),
        }
    }
}
```

**If the enum and its two impls live somewhere other than `crates/tipsy-compadre/src/server/mod.rs`,
add the variant and the two arms where they actually live** — read the tree, and let it win over the
path named here. The variant's name, its one field, its spelling and its `Display`/`source` behaviour
are the contract; the file is not. The call site in Step 3 is unchanged:

```rust
program: std::env::current_exe().map_err(|e| ServerError::Message(format!("no current executable: {e}")))?,
```

**Line-number note for the implementer.** The front matter quotes `crates/tipsy-compadre/src/main.rs`
and the module list as they stand at planning time; the file is 153 lines as this task is written, and
`client.rs` is 1253. If a number below disagrees with the file, the file wins.

- [ ] **Step 1: Write the failing test**

Create `crates/tipsy-compadre/tests/support/process.rs`. This is **not** a test binary of its own:
`tests/support/` is a subdirectory of `tests/`, and Rust only auto-discovers `tests/*.rs`, so the file
is included by the test binaries that need it with `#[path = "support/process.rs"] mod process;`. Both
tasks 2.1 and 2.2 use it, which is why it lives here and not inside `tests/pty.rs`.

```rust
//! Test-only helpers for "is this process still running?". Shared by `tests/pty.rs` and
//! `tests/terminal_ws.rs` through `#[path = "support/process.rs"] mod process;`.

#![allow(dead_code)]

use std::process::Stdio;

/// `cmd.exe`'s own absolute path. It is not written as `process::Command::new("cmd.exe")` on purpose:
/// one of the tests in `tests/pty.rs` puts a directory it controls at the front of `PATH`, so every
/// system program these helpers start is named by its full path and never searched for.
pub const CMD: &str = r"C:\Windows\System32\cmd.exe";

/// The PowerShell 5.1 the tests use to read a PID out of the OS, again by absolute path. It ships
/// with Windows 10 and later, which is the same floor ConPTY has (research note §5.3:
/// `CreatePseudoConsole` needs 1809).
#[cfg(windows)]
pub const POWERSHELL: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";

/// A relative `.ps1` path turned into the absolute path `powershell.exe -File` wants.
#[cfg(windows)]
pub fn abs(root: &std::path::Path, rel: &str) -> String {
    root.join(rel).to_string_lossy().into_owned()
}

/// Whether the PID is still running, asked of the operating system rather than of a handle we hold:
/// a dropped or disowned handle says nothing. Exit 0 is "alive", exit 1 is "gone".
#[cfg(windows)]
pub fn alive(pid: u32) -> bool {
    let script = format!(
        "$p = Get-Process -Id {pid} -ErrorAction SilentlyContinue; if ($null -eq $p) {{ exit 1 }} else {{ exit 0 }}"
    );
    std::process::Command::new(POWERSHELL)
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The same question on a platform without `Get-Process`.
#[cfg(not(windows))]
pub fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// Polls `alive` until the PID is gone, up to `tries * 100 ms`. Returns whether it went away.
#[cfg(windows)]
pub fn gone(pid: u32) -> bool {
    gone_within(pid, 50)
}

#[cfg(not(windows))]
pub fn gone(pid: u32) -> bool {
    gone_within(pid, 50)
}

fn gone_within(pid: u32, tries: usize) -> bool {
    for _ in 0..tries {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}
```

Now the integration test, `crates/tipsy-compadre/tests/pty.rs`:

```rust
//! The PTY against a real child process, on this machine's own ConPTY (Windows 10 1809 or later —
//! `CreatePseudoConsole`'s documented floor). The child is `tipsy-compadre --connect`, exactly what
//! `serve`'s `/terminal/ws` will run (spec §7.2, W10).
//!
//! What each test is for:
//!   * the reader/writer/echo path and the fact that the child survives the socket it cannot reach
//!   * `MasterPty::resize` reaching the child as a changed console width — the only resize call in
//!     this phase, so that no `libc::kill`, `SIGWINCH` or `ResizePseudoConsole` appears in the tree
//!   * teardown (Review Focus 4): the process is **gone**, not merely disowned

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use tipsy_compadre::client::pty::{PtySession, PtySpawner, TerminalSpawner};

#[path = "support/process.rs"]
mod process;

/// How long a test will wait for a child to say something before calling it a failure.
const WAIT: Duration = Duration::from_secs(20);

/// Nothing is listening here: `spawn`'s child gives up on the connection at once and, because it is a
/// `--connect` client, never builds a provider, never reads a key and never binds a socket.
const UNREACHABLE: &str = "ws://127.0.0.1:1/ws";

/// The `--token` the child is handed. It reaches a server that is not there, so it authenticates
/// nothing — the point is that the argument is accepted and the process stays up.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn project_binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_tipsy-compadre"))
}

/// The spawner the first three tests use: the real binary, from the absolute path Cargo gives the
/// test, with the production argument vector. It is deliberately **not** the security test's spawner —
/// that one goes through `std::env::current_exe()` exactly as `server::run` does, and proves the
/// difference a `PATH` lookup would make.
fn spawner() -> TerminalSpawner {
    TerminalSpawner {
        program: project_binary().to_path_buf(),
        args: vec![
            "--connect".into(),
            UNREACHABLE.into(),
            "--token".into(),
            TOKEN.into(),
        ],
        ..Default::default()
    }
}

/// Reads from the PTY until `wanted` bytes are in `buf`, or panics after `WAIT`. `read()` returns one
/// chunk per PTY read, so "the bytes are in" is only ever a prefix of what a repainting TUI sends.
fn read_until(reader: &mut Box<dyn Read + Send>, buf: &mut Vec<u8>, wanted: usize) -> bool {
    let deadline = std::time::Instant::now() + WAIT;
    let mut chunk = [0u8; 4096];
    while buf.len() < wanted {
        if std::time::Instant::now() > deadline {
            return false;
        }
        match reader.read(&mut chunk) {
            Ok(0) => return false, // EOF: the master is gone
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return false,
        }
    }
    true
}

/// Everything the PTY has produced right now, without waiting: one non-blocking-ish drain, used
/// between two assertions so the second one is about the resize and not about text already read.
fn drain(reader: &mut Box<dyn Read + Send>, into: &mut Vec<u8>) {
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk) {
            Ok(n) => into.extend_from_slice(&chunk[..n]),
            Err(_) => return,
        }
    }
}

fn pump(writer: &mut Box<dyn Write + Send>, byte: u8) {
    writer.write_all(&[byte]).expect("the PTY write must land");
    writer.flush().expect("the PTY write must be flushed");
}

/// Spawns the project's own binary in `--connect` mode against a port where nothing listens. It is
/// alive *after* the failed connection: a `--connect` child builds no providers (`main.rs:75` is the
/// bare run's path only), so it cannot have exited with `MISSING_KEY` — that is the "no provider
/// call" half of spec §7.2, observed rather than asserted.
#[tokio::test]
async fn the_child_runs_in_a_real_pty_and_echoes_a_keystroke() {
    let mut session = spawner().spawn(80, 24).expect("openpty and spawn_command must work on this machine");
    let mut reader = session.take_reader().expect("the reader is taken exactly once");
    let mut writer = session.take_writer().expect("the writer is taken exactly once");

    pump(&mut writer, b'x');
    let mut out = Vec::new();
    assert!(
        read_until(&mut reader, &mut out, 1),
        "the PTY must produce bytes for a keystroke; nothing was readable"
    );

    // A `--connect` child that could not reach its server is still alive here. A child that had tried
    // to start a server or to build the provider stack would be gone (or would have exited with the
    // missing-key code), so this one assertion covers both halves of spec §7.2.
    std::thread::sleep(Duration::from_millis(200));
    let alive = process::alive(session.process_id().unwrap());
    let _ = session.kill_and_wait();
    assert!(alive, "the child must survive a connection it cannot make: it starts no server and no provider");
}

/// `MasterPty::resize` is the only resize call this phase makes, and this is the test that says it
/// reaches the child: a PowerShell child reads its console width out of the OS, the test resizes the
/// PTY, and the width it prints afterwards is the new one.
#[tokio::test]
async fn a_resize_reaches_the_child_as_a_new_console_width() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("width.ps1");
    std::fs::write(
        &script,
        "while ($true) { [Console]::Out.Write('WIDTH=' + [Console]::WindowWidth); [Console]::Out.Flush(); $null = [Console]::In.ReadLine() }\r\n",
    )
    .unwrap();

    let mut session = PtySession::spawn(
        Path::new(process::POWERSHELL),
        &["-NoProfile".into(), "-NonInteractive".into(), "-ExecutionPolicy".into(), "Bypass".into(), "-File".into(), process::abs(dir.path(), "width.ps1")],
        40,
        20,
    )
    .expect("openpty and spawn_command must work on this machine");
    let mut reader = session.take_reader().unwrap();
    let mut writer = session.take_writer().unwrap();

    let mut out = Vec::new();
    assert!(read_until(&mut reader, &mut out, 8), "the child must print its first width");
    let before = width_of(&out);
    assert!(before > 0, "no width was readable in {:?}", String::from_utf8_lossy(&out));

    // The write keeps the child parked on `ReadLine` and gives the PTY something to echo, so the
    // reading side is demonstrably live at the moment of the resize.
    pump(&mut writer, b'x');
    session.resize(100, 24).expect("resize must succeed");
    std::thread::sleep(Duration::from_millis(300));
    let mut tail = Vec::new();
    drain(&mut reader, &mut tail);
    let after = width_of(&tail);

    session.kill_and_wait().expect("the resize reporter is killed like any other child");
    assert!(
        after > before,
        "MasterPty::resize is what reaches the child: before={before} after={after}; tail={:?}",
        String::from_utf8_lossy(&tail)
    );
}

/// Review Focus 4, and the reason this task exists: the process is **gone**, in the OS's own answer,
/// not merely a handle that was dropped.
#[tokio::test]
async fn killing_the_child_leaves_no_process_behind() {
    let mut session = PtySession::spawn(Path::new(process::CMD), &["/Q".into(), "/K".into(), "pause".into()], 80, 24)
        .expect("openpty and spawn_command must work on this machine");
    let mut reader = session.take_reader().unwrap();
    let mut writer = session.take_writer().unwrap();
    pump(&mut writer, b'x');
    let mut out = Vec::new();
    assert!(read_until(&mut reader, &mut out, 1), "cmd.exe must echo the keystroke");

    let pid = session.process_id().expect("the child has a pid on this platform");
    assert!(process::alive(pid), "the child is running before the teardown");

    // The status itself is not asserted: a terminated ConPTY child reports a code that belongs to the
    // platform's own termination path (`TerminateProcess` on Windows, a signal on Unix), and pinning
    // it here would pin the platform rather than the behaviour. The behaviour is the assertion below.
    let _status = session.kill_and_wait().expect("kill then wait must reap the child");
    assert!(process::gone(pid), "the PTY child was not reaped: pid {pid} is still running");

    // The master is dropped here with the session; the reader is already EOF, which is the third
    // half of teardown ("let the blocking reader observe EOF").
    drop(reader);
    drop(writer);
    drop(session);
}

/// **The child is the running binary, not a name looked up in `PATH`.** A `PATH` entry — an empty one
/// (`::`), a relative one (`.`) or a repository-local one — is writable by whatever else is in the
/// working directory, and the child here is a full client whose behaviour the browser drives over
/// `/terminal/ws`. This test plants a fake `tipsy-compadre.exe` in a `PATH` entry that is searched
/// first, and proves the planted one never runs.
///
/// Two assertions, because neither alone is enough:
///   * the fake writes a marker file when it runs, and the marker is **never** written;
///   * the real child reports a probe value that only the environment this test built can contain,
///     so "nothing ran" cannot pass the test by accident.
/// The real binary is reached through `std::env::current_exe()`, which is how the server builds the
/// spawner — `TerminalSpawner.program` is never a bare name.
#[cfg(windows)]
#[test]
fn the_child_is_the_running_binary_and_a_planted_path_entry_never_runs() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().to_path_buf();
    let marker = fake.join("planted.marker");
    let probe = "pty-security-probe-4b1e9a";

    // A fake binary under the name a `PATH` lookup would have found. It writes its marker and exits.
    std::fs::write(
        fake.join("tipsy-compadre.cmd"),
        format!(
            "@echo off\r\necho planted>\"{}\"\r\nexit /b 0\r\n",
            marker.display()
        ),
    )
    .unwrap();

    let original = std::env::var_os("PATH").unwrap_or_default();
    let mut poisoned = std::ffi::OsString::from(fake.as_os_str());
    poisoned.push(";");
    poisoned.push(&original);
    // SAFETY, and the reason this test is `#[test]` and not `#[tokio::test]`: `set_var` is unsafe in
    // edition 2024 because the environment is process-global. Nothing else in this test binary starts
    // a thread that reads `PATH` while the window is open — the other tests here name `cmd.exe` and
    // `powershell.exe` by absolute path through `process::CMD`/`process::POWERSHELL` — and the value
    // is restored before the test returns.
    unsafe { std::env::set_var("PATH", &poisoned) };

    let program = std::env::current_exe().expect("the running binary has a path");
    let spawner = TerminalSpawner {
        program,
        args: vec!["--connect".into(), UNREACHABLE.into()],
        env: vec![format!("TIPSY_PTY_PROBE={probe}")],
        ..Default::default()
    };
    let mut session = spawner
        .spawn(80, 24)
        .expect("the running binary, reached by absolute path, must spawn");

    let mut reader = session.take_reader().unwrap();
    let mut writer = session.take_writer().unwrap();
    pump(&mut writer, b'x');
    let mut out = Vec::new();
    let _ = read_until(&mut reader, &mut out, 1);
    // Give the child the moment a planted `.cmd` would need to write its marker.
    std::thread::sleep(Duration::from_millis(300));
    let _ = session.kill_and_wait();
    drop(reader);
    drop(writer);
    drop(session);

    unsafe { std::env::set_var("PATH", &original) };

    assert!(
        !marker.exists(),
        "a binary planted in PATH ran with the user's authority: {} exists",
        marker.display()
    );
    let text = String::from_utf8_lossy(&out).into_owned();
    assert!(
        text.contains(probe),
        "the child that ran did not carry this test's probe value, so it was not the process this \
         test built the environment for: {text:?}"
    );
}

fn width_of(bytes: &[u8]) -> u16 {
    let text = String::from_utf8_lossy(bytes);
    text.split("WIDTH=")
        .skip(1)
        .filter_map(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .filter_map(|digits| digits.parse::<u16>().ok())
        .next_back()
        .unwrap_or(0)
}
```

Add `process_id` and `killer` to the produced surface. The third integration test needs the pid, and
the pump in Task 2.2 needs the independent killer:

```rust
    /// The child's pid, for a test that has to ask the OS whether it is still there.
    pub fn process_id(&self) -> Option<u32>;

    /// A handle that can kill the child while the session is held behind a lock, or while another
    /// thread is inside `wait`. `portable-pty`'s own doc gives the reason for it existing at all.
    pub fn killer(&self) -> Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>;
```

Then, in `crates/tipsy-compadre/src/client/pty.rs`, the unit half — the one decision that needs no
process at all:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The child's argument vector is built by the caller, so the only thing this unit can pin is
    /// that `TerminalSpawner` hands its own program, arguments and environment through unchanged and
    /// spawns the program it was given. Building that program is the caller's job, and it is
    /// `std::env::current_exe()` — never a name (Task 2.1's integration test proves the difference).
    #[test]
    fn a_spawner_passes_its_program_arguments_and_environment_through() {
        let spawner = TerminalSpawner {
            program: std::path::PathBuf::from(r"C:\does\not\exist\tipsy-compadre.exe"),
            args: vec!["--connect".into(), "ws://127.0.0.1:7070/ws".into()],
            env: vec!["TIPSY_PTY_PROBE=unit".into()],
        };
        assert_eq!(spawner.args(), vec!["--connect".to_string(), "ws://127.0.0.1:7070/ws".to_string()]);
        assert_eq!(spawner.env(), vec!["TIPSY_PTY_PROBE=unit".to_string()]);
        assert!(spawner.spawn(80, 24).is_err(), "a program that does not exist cannot be spawned");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre --test pty
```

Expected: FAIL to compile, with

```
error[E0432]: unresolved import `tipsy_compadre::client::pty`
 --> crates\tipsy-compadre\tests\pty.rs:17:38
  |
17 | use tipsy_compadre::client::pty::{PtySession, PtySpawner, TerminalSpawner};
  |                                       ^^^^^^^^^^ use of unresolved module or unlinked crate `pty`
```

(`E0432` is the error; newer rustc appends "no `pty` in `client`". Either wording is the same
failure.) Nothing compiles, so no test runs.

- [ ] **Step 3: Implement `client::pty`**

`crates/tipsy-compadre/Cargo.toml` — after `tokio-tungstenite` (line 32), before `[dev-dependencies]`:

```toml
# 0.9.0, read off crates.io by the research note (web-transport-and-terminal.md §0). ConPTY on
# Windows 10 1809 and later; `openpty` fails with a readable error below that floor, which `spawn`
# turns into "Windows 10 1809 or later is required for the web terminal".
portable-pty = "0.9"
```

`crates/tipsy-compadre/src/client.rs` — after the module doc comment at `client.rs:1-4`, before the
`use` block (which starts at `:6`):

```rust
pub mod pty;
```

Note for the implementer: Task 1.7 adds `crates/tipsy-compadre/src/client/bridge.rs`, which is the
same shape — `client.rs` stays the parent module and `client/` holds the children, so `pub mod pty;`
here and `pub mod bridge;` there must both be present and neither task may remove the other's line.

`crates/tipsy-compadre/src/client/pty.rs` — the implementation:

```rust
//! The pseudo-terminal that owns a web terminal's child process (spec §7.1–§7.3).
//!
//! One `PtySession` per `/terminal/ws` connection, one child per session. `portable-pty`'s reader and
//! writer are **std-blocking** (`Box<dyn Read + Send>`, `Box<dyn Write + Send>`, research note §5.2)
//! and the crate does not depend on tokio, so this module owns no async I/O at all: the connection
//! pump spawns a blocking task for the reader and another for the writer, and this type just hands
//! the two ends over.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// The two `portable-pty` handles, named once so the server's pump does not spell the boxes out.
pub type BoxedRead = Box<dyn Read + Send>;
pub type BoxedWrite = Box<dyn Write + Send>;

/// A child running inside a real pseudo-terminal.
pub struct PtySession {
    /// The master, shared so `resize` can be called while the reader task is parked in `read`.
    /// `MasterPty: Send` but not `Sync` (it is `&self` for `resize`, `try_clone_reader` and
    /// `take_writer` because the crate means them to be callable from several threads), so it goes
    /// behind an `Arc`, not an `Arc<Mutex>`: no lock is held across the reader's blocking `read`.
    master: Arc<dyn MasterPty + Send>,
    reader: Mutex<Option<BoxedRead>>,
    writer: Mutex<Option<BoxedWrite>>,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl PtySession {
    pub fn spawn(program: &std::path::Path, args: &[String], cols: u16, rows: u16) -> Result<PtySession, String> {
        Self::spawn_with_env(program, args, &[], cols, rows)
    }

    /// `env` carries `NAME=value` pairs for the child's own environment. Nothing in production sets
    /// one; the security test in this task's integration file uses it to mark the process it built.
    /// **No API key may ever travel this way** (spec §9.3).
    pub fn spawn_with_env(
        program: &std::path::Path,
        args: &[String],
        env: &[String],
        cols: u16,
        rows: u16,
    ) -> Result<PtySession, String> {
        let system = native_pty_system();
        let pair = system.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| {
            format!(
                "no pseudo-terminal could be opened ({e}); the web terminal needs Windows 10 1809 \
                 or later for ConPTY, or a Unix pty"
            )
        })?;

        // `program` is an absolute path decided by the caller — in production the one
        // `std::env::current_exe()` returned. It is never a bare name, so `portable-pty` cannot
        // resolve it through `PATH`: a `PATH` entry that is empty, relative or points into a checked
        // out workspace must not be able to plant the child that then runs with the user's authority
        // (prior art: `.superpowers/sdd/plan-3-research/codewhale-audio-prior-art.md` §5).
        let mut command = CommandBuilder::new(program);
        // Each argument is appended on its own: `CommandBuilder` quotes the vector itself, so a path
        // or a token containing a space cannot be split.
        command.args(args);
        for pair in env {
            if let Some((name, value)) = pair.split_once('=') {
                command.env(name, value);
            }
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("the terminal child could not be started: {e}"))?;

        let reader = pair.master.try_clone_reader().map_err(|e| format!("no PTY reader: {e}"))?;
        // Taken here and nowhere else: "It is invalid to take the writer more than once."
        let writer = pair.master.take_writer().map_err(|e| format!("no PTY writer: {e}"))?;
        // The slave end goes with the child: keeping it open would keep the master from ever seeing EOF.
        drop(pair.slave);

        Ok(PtySession {
            master: Arc::from(pair.master),
            reader: Mutex::new(Some(reader)),
            writer: Mutex::new(Some(writer)),
            child: Mutex::new(Some(child)),
        })
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("the PTY could not be resized to {cols}x{rows}: {e}"))
    }

    pub fn take_reader(&self) -> Option<BoxedRead> {
        self.reader.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    pub fn take_writer(&self) -> Option<BoxedWrite> {
        self.writer.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    pub fn process_id(&self) -> Option<u32> {
        let child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        child.as_ref().and_then(|child| child.process_id())
    }

    /// An independent killer, taken through the `ChildKiller` supertrait of `Child`. The pump uses it
    /// to terminate the child from a `&self` borrow, so the session itself stays in one piece until it
    /// goes out of scope and the master closes.
    pub fn killer(&self) -> Option<Box<dyn ChildKiller + Send + Sync>> {
        let child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        child.as_ref().map(|child| child.clone_killer())
    }

    /// Terminates the child. `ChildKiller::kill` is in scope through `Child`'s supertrait list.
    pub fn kill(&mut self) -> Result<(), String> {
        let mut guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(child) => child.kill().map_err(|e| format!("the terminal child could not be killed: {e}")),
            None => Ok(()),
        }
    }

    /// Reaps the child. Blocks, so production calls it from `spawn_blocking`; the tests call it after
    /// `kill`.
    pub fn wait(&mut self) -> Result<portable_pty::ExitStatus, String> {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner()).take();
        match slot.as_mut() {
            Some(child) => child.wait().map_err(|e| format!("the terminal child could not be reaped: {e}")),
            None => Err("the terminal child was already reaped".to_string()),
        }
    }

    pub fn kill_and_wait(&mut self) -> Result<portable_pty::ExitStatus, String> {
        self.kill()?;
        self.wait()
    }
}
```

There is deliberately **no `Drop for PtySession`** and **no child-kill on drop**. Two reasons, both
load-bearing:

- Review Focus 4's whole point is that teardown is *explicit*: the pump kills the child, drops the
  writer, drops the master and lets the reader observe EOF. A `Drop` that killed would let a later
  refactor delete the pump's own teardown and still pass every test that only checks "the child is
  gone", which is exactly the accident this focus exists to catch.
- The first integration test below needs a child that **outlives** a dropped session handle. It could
  not, if dropping killed.

```rust
/// What a PTY is spawned from. `server::conn` holds one through a `dyn` so its tests can drive a real
/// PTY over `cmd.exe` or PowerShell while `serve` drives the project's own binary.
pub trait PtySpawner: Send + Sync + 'static {
    /// Built per connection: the token can be regenerated while the server runs, so this is a method
    /// and not a field.
    fn args(&self) -> Vec<String>;
    /// `NAME=value` pairs for the child's own environment. Empty in production.
    fn env(&self) -> Vec<String> {
        Vec::new()
    }
    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String>;
}
```

```rust
/// The production spawner: this process's own binary in `--connect` mode, pointed at this process's
/// own loopback `/ws` (spec §7.2, W10). The child is a *client*, exactly like a user on another
/// machine, so keys, providers, history and the credential store stay in the parent and the child's
/// audio is the server machine's.
pub struct TerminalSpawner {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
}

impl PtySpawner for TerminalSpawner {
    fn args(&self) -> Vec<String> {
        self.args.clone()
    }

    fn env(&self) -> Vec<String> {
        self.env.clone()
    }

    /// `self.program` is `std::env::current_exe()` in production (spec §7.2, W10) — the running
    /// executable's own path, which the OS resolved when the process started. No `PATH` lookup, no
    /// relative path, no bare `"tipsy-compadre"`.
    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String> {
        PtySession::spawn_with_env(&self.program, &self.args, &self.env, cols, rows)
    }
}
```

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The child's argument vector is built by the caller, so the only thing this unit can pin is
    /// that `TerminalSpawner` hands its own program, arguments and environment through unchanged and
    /// spawns the program it was given. Building that program is the caller's job, and it is
    /// `std::env::current_exe()` — never a name (Task 2.1's integration test proves the difference).
    #[test]
    fn a_spawner_passes_its_program_arguments_and_environment_through() {
        let spawner = TerminalSpawner {
            program: std::path::PathBuf::from(r"C:\does\not\exist\tipsy-compadre.exe"),
            args: vec!["--connect".into(), "ws://127.0.0.1:7070/ws".into()],
            env: vec!["TIPSY_PTY_PROBE=unit".into()],
        };
        assert_eq!(spawner.args(), vec!["--connect".to_string(), "ws://127.0.0.1:7070/ws".to_string()]);
        assert_eq!(spawner.env(), vec!["TIPSY_PTY_PROBE=unit".to_string()]);
        assert!(spawner.spawn(80, 24).is_err(), "a program that does not exist cannot be spawned");
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre --test pty
```

Expected: PASS — `test result: ok. 4 passed; 0 failed; 0 ignored` (the four integration tests: echo
and no-provider-call, resize, teardown, and the planted-`PATH` security test). The one unit test in
`client/pty.rs` is part of the lib target, so also run:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre client::pty
```

Expected: PASS — `test result: ok. 1 passed; 0 failed`.

Then the lint gate, compared against the pinned baseline of `global-constraints.md` (add nothing):

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo clippy -j 1 -p tipsy-compadre --all-targets
```

If the resize assertion is flaky on this machine, **do not delete it and do not loosen it to
"resize returned `Ok`"**: the point of the test is that the child observed the new width. Widen the
sleep in `WAIT`-sized steps and record what was observed in the ledger.

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/client/pty.rs crates/tipsy-compadre/src/client.rs crates/tipsy-compadre/Cargo.toml crates/tipsy-compadre/tests/pty.rs crates/tipsy-compadre/tests/support/process.rs
git commit -m "feat(server): own a web terminal's child in a real pty

spawn the project's own binary in --connect mode inside a portable-pty
pseudo-terminal, hand its blocking reader and writer to the caller, and
prove the two consequences spec 7.2 names: the child starts no server and
makes no provider call, and MasterPty::resize is the only resize call this
phase needs. Teardown kills, waits and lets the reader see EOF, and the test
asks the operating system whether the process is gone.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 2.2: The raw-bytes socket and the terminal pump `[mid]`

**Files:**
- Create: `crates/tipsy-compadre/src/server/conn.rs`
- Modify: `crates/tipsy-compadre/src/server/mod.rs` (add `pub mod conn;`; and register the route — see the note below)
- Test: `crates/tipsy-compadre/tests/terminal_ws.rs` (a new integration test), which includes the `process.rs` helper Task 2.1 created with `#[path = "support/process.rs"] mod process;`, plus the unit half in `server/conn.rs`. Task 2.1 creates `crates/tipsy-compadre/tests/support/process.rs`; this task only reads it.

**Interfaces:**

- Consumes:
  - `client::pty::{PtySession, PtySpawner, TerminalSpawner}` — Task 2.1. `BoxedRead` and `BoxedWrite`
    are Task 2.1's aliases and are what the reader and writer tasks hold; this module names them only
    through `take_reader`/`take_writer`, so it does not import them.
  - `client::bridge::connect_args(addr: std::net::SocketAddr, token: &str) -> Vec<String>` — Task 1.7.
    This task uses it and does **not** re-declare it; if Task 1.7 spelled it differently, that file
    wins and this body's one call site is renamed to match.
  - `assets::{PAGE_TERMINAL, page_missing(), asset(path)}` — Task 1.5. `asset` returns bytes plus a
    content type, or `None` when the bundle is absent; this task only needs `page_missing()` for the
    "no bundle" branch, which `assets.rs` already owns.
  - `axum` 0.8.9 with `ws`: `axum::extract::ws::{WebSocketUpgrade, WebSocket, Message, CloseFrame}`,
    `axum::extract::{Query, State}`, `axum::response::{IntoResponse, Response}`,
    `axum::http::StatusCode`. `WebSocketUpgrade::max_write_buffer_size(self, max: usize) -> Self` and
    `on_upgrade` are the two methods this task calls; both are on
    [`WebSocketUpgrade`](https://docs.rs/axum/0.8.9/axum/extract/ws/struct.WebSocketUpgrade.html).
    `axum::extract::ws::Message`'s variants are `Text(Utf8Bytes) | Binary(Bytes) | Ping(Bytes) |
    Pong(Bytes) | Close(Option<CloseFrame>)` and `Message::binary<B: Into<Bytes>>(bin: B)` exists.
    **This is axum's `Message`, not `tokio_tungstenite`'s** — W1 puts both in this binary, and this
    task must name the axum one everywhere.
- Produces, in `tipsy_compadre::server::conn`:

```rust
/// The ceiling this phase accepts on live PTY children: one process and two pipe pairs each, and two
/// browsers on `/terminal` are two TUI clients fighting for the same microphone anyway (research note
/// §7, trap 4). A third connection is answered `503 Service Unavailable` without spawning anything.
pub const MAX_TERMINAL_CONNECTIONS: usize = 2;

/// The writer queue's depth: 32 chunks of at most 8 KiB, so a connected-but-stalled browser costs
/// about 256 KiB of server memory before the reader starts dropping chunks instead of growing.
pub const TERMINAL_WRITE_QUEUE: usize = 32;

/// One read from the PTY master. Chosen inside the 8–64 KiB the research note recommends (§7, trap 3):
/// small enough that a big repaint never becomes one WebSocket frame, large enough that a normal
/// screenful is a handful of frames.
pub const TERMINAL_READ_CHUNK: usize = 8 * 1024;

/// How long a resize waits for the next one before it is applied, in milliseconds. xterm.js's own
/// `resize` doc says "It's best practice to debounce calls to resize, this will help ensure that the
/// pty can respond to the resize event before another one occurs" (research note §6.3); 100 ms is the
/// server's half of that, chosen to match the page's own debounce.
pub const TERMINAL_RESIZE_DEBOUNCE_MS: u64 = 100;

/// `GET /terminal/ws?token=…`, upgrade. The token rule is §5.3's, the same one `/terminal` uses.
pub async fn terminal_ws(
    State(state): State<AppState>,
    Query(query): Query<TerminalQuery>,
    ws: WebSocketUpgrade,
) -> Response;
```

and, in the same module, the pieces that make it testable without a browser:

```rust
/// The router implements this, so this task's tests pass a two-line router of their own while
/// `server::run` passes the real `AppState` (Task 1.4).
pub trait TerminalRouteState: Clone + Send + Sync + 'static {
    fn websocket_token(&self) -> Option<String>;
    fn terminal_spawner(&self) -> Arc<dyn PtySpawner>;
}

/// `Some(())` when the query carries the token this server requires, or when the server requires
/// none. `Err(message)` is a 401 whose body names the rule and never the token.
pub fn terminal_access(query: Option<&str>, expected: Option<&str>) -> Result<(), String>;

/// Registers `/terminal/ws` on a router whose state implements `TerminalRouteState`.
pub fn router<S: TerminalRouteState>(router: axum::Router<S>) -> axum::Router<S>;
```

**The wire shape of this socket, exactly.** This is not the protocol and must never share Task 1.6's
`Command`/`Event` translation (spec §5.3, §7.3): nothing here is JSON that the session reads, and the
session never sees this socket.

| Direction | Frame | Payload | Meaning |
|---|---|---|---|
| server → client | **Binary** | raw PTY bytes, at most `TERMINAL_READ_CHUNK` per frame | the terminal's output, fed straight to `term.write(Uint8Array)`. xterm.js treats a `Uint8Array` as **UTF-8** (research note §6.3), which is what a PTY sends |
| server → client | **Text** | `{"type":"exit","code":0}` | the child ended; the page may print a line and wait for a reload. This is the only text frame the server ever sends |
| client → server | **Text** | `{"type":"data","data":"…"}` | keystrokes and paste, taken from xterm.js's `onData` (a JS string; escape sequences are ASCII) |
| client → server | **Text** | `{"type":"mouse","data":"…"}` | xterm.js's `onBinary` output, one byte per `charCode & 0xff`. **Not** a UTF-8 string: research note §6.4 is explicit that sending it as text is "exactly how mouse reports get corrupted", so the page turns it into a latin-1 **string** and the server turns each `charCode` back into a byte before writing it to the PTY |
| client → server | **Text** | `{"type":"resize","cols":80,"rows":24}` | debounced |
| client → server | **Binary** | 256 bytes, ASCII `"tipsterm-hello-pad…"` | nothing but a padding frame. A browser cannot observe whether its own socket is open at the server, and the first data frame is what tells the server the connection is real |

Rust side of the two client messages, exactly:

```rust
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalClientMessage {
    Hello,
    /// `onData`: escape sequences and UTF-8 text, written to the PTY as UTF-8 bytes.
    Data { data: String },
    /// `onBinary`: "%FF"-style mouse reports, one byte per JS `charCode`. Each char is truncated to
    /// its low byte, which is what the page did in reverse.
    Mouse { data: String },
    Resize { cols: u16, rows: u16 },
}

impl TerminalClientMessage {
    /// The bytes this message puts on the PTY's input, if any.
    fn bytes(&self) -> Option<Vec<u8>> {
        match self {
            Self::Data { data } => Some(data.as_bytes().to_vec()),
            Self::Mouse { data } => Some(data.chars().map(|c| c as u32 as u8).collect()),
            Self::Hello | Self::Resize { .. } => None,
        }
    }
}
```

**Where the route is registered.** Task 1.4 owns the listener, the bind rule and the router, and Task
1.5 owns `server::assets` and the two page routes; whichever of them builds the route table exposes it
as `pub fn router<S>(router: axum::Router<S>) -> axum::Router<S>` in `server/mod.rs`. This task adds
its `.route("/terminal/ws", get(terminal_ws))` call to **that same function** — find it, do not create
a second router builder — and `AppState` must implement `TerminalRouteState` (three methods: the
token, the spawner, and a `Clone`). If that function is named something else in the tree, the tree
wins; say so in the ledger and use its name.

**Line-number note for the implementer.** `server/mod.rs` does not exist yet when this task starts —
Task 1.4 creates it. Read it before editing, and treat its real line numbers as authoritative over
anything implied here.

- [ ] **Step 1: Write the failing test**

`crates/tipsy-compadre/tests/terminal_ws.rs`:

```rust
//! `/terminal/ws` against a real listener on 127.0.0.1:0, a real PTY child and a real WebSocket
//! client. The child is `cmd.exe` or PowerShell rather than the project's binary because this test is
//! about the *pump*: Task 2.1 already pins the child, and the pump is what has to hand a TUI's bytes
//! back and forth.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use futures::{SinkExt, StreamExt};
use tipsy_compadre::client::pty::{PtySession, PtySpawner, TerminalSpawner};
use tipsy_compadre::server::conn::{MAX_TERMINAL_CONNECTIONS, TerminalRouteState, router, terminal_access};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

#[path = "support/process.rs"]
mod process;

const WAIT: Duration = Duration::from_secs(20);
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Spawns `cmd.exe` on Windows, a shell elsewhere. Both echo what is written to them, which is the
/// smallest honest way to prove a byte made the full round trip through the pump.
struct TestSpawner;

impl PtySpawner for TestSpawner {
    fn args(&self) -> Vec<String> {
        Vec::new()
    }

    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String> {
        #[cfg(windows)]
        let (program, args) = (PathBuf::from(process::CMD), vec!["/Q".to_string(), "/K".to_string(), "pause".to_string()]);
        #[cfg(not(windows))]
        let (program, args) = (PathBuf::from("/bin/sh"), vec!["-i".to_string()]);
        PtySession::spawn(&program, &args, cols, rows)
    }
}

/// A PowerShell child that prints its console width and then parks on `ReadLine`. Its PID is in the
/// script text so the test can find the process the resized connection owns.
#[cfg(windows)]
struct WidthReporterSpawner {
    dir: PathBuf,
}

#[cfg(windows)]
impl PtySpawner for WidthReporterSpawner {
    fn args(&self) -> Vec<String> {
        Vec::new()
    }

    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String> {
        std::fs::write(
            self.dir.join("width.ps1"),
            "while ($true) { [Console]::Out.Write('WIDTH=' + [Console]::WindowWidth); [Console]::Out.Flush(); $null = [Console]::In.ReadLine() }\r\n",
        )
        .expect("the width reporter script must be written");
        PtySession::spawn(
            Path::new(process::POWERSHELL),
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                process::abs(&self.dir, "width.ps1"),
            ],
            cols,
            rows,
        )
    }
}

/// A PowerShell child that keeps reading its input, with its own PID printed once. Two of these are
/// spawned in the teardown test, each with its own script, so the test can ask the OS about exactly
/// the PIDs this connection owns. The PID is the **child's**, written by the child itself: the pid of
/// the process the test started is a different process on this platform, which is the mistake this
/// file exists to avoid.
#[cfg(windows)]
struct KeepAliveSpawner {
    dir: PathBuf,
    /// The pid of every child this spawner has started, in spawn order.
    pids: Arc<std::sync::Mutex<Vec<u32>>>,
    /// How many children this spawner has started, so each gets its own script and its own pid file.
    started: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(windows)]
impl PtySpawner for KeepAliveSpawner {
    fn args(&self) -> Vec<String> {
        Vec::new()
    }

    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String> {
        let index = self.started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let name = format!("keepalive-{index}.ps1");
        let pid_name = format!("keepalive-{index}.pid");
        std::fs::write(
            self.dir.join(&name),
            format!(
                "$PID | Set-Content -Path '{}'\r\nwhile ($true) {{ $null = [Console]::In.ReadLine() }}\r\n",
                process::abs(&self.dir, &pid_name)
            ),
        )
        .map_err(|e| format!("{}: {e}", self.dir.join(&name).display()))?;

        let session = PtySession::spawn(
            Path::new(process::POWERSHELL),
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                process::abs(&self.dir, &name),
            ],
            cols,
            rows,
        )?;

        // The script is asynchronous, so the pid file appears a moment after the spawn.
        let pid_path = self.dir.join(&pid_name);
        let deadline = std::time::Instant::now() + WAIT;
        let pid = loop {
            if let Ok(text) = std::fs::read_to_string(&pid_path)
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("the child never reported its pid to {}", pid_path.display()));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        self.pids.lock().unwrap().push(pid);
        Ok(session)
    }
}

#[derive(Clone)]
struct TestState {
    token: Option<String>,
    spawner: Arc<dyn PtySpawner>,
}

impl TerminalRouteState for TestState {
    fn websocket_token(&self) -> Option<String> {
        self.token.clone()
    }

    fn terminal_spawner(&self) -> Arc<dyn PtySpawner> {
        Arc::clone(&self.spawner)
    }
}

/// A listener on `127.0.0.1:0` (no test in this repository hard-codes a port) serving `/terminal/ws`.
async fn serve(state: TestState) -> std::net::SocketAddr {
    let app = router(Router::new()).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

async fn connect(
    addr: std::net::SocketAddr,
    token: Option<&str>,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, String> {
    let query = match token {
        Some(token) => format!("?token={token}"),
        None => String::new(),
    };
    tokio_tungstenite::connect_async(format!("ws://{addr}/terminal/ws{query}"))
        .await
        .map(|(socket, _)| socket)
        .map_err(|e| e.to_string())
}

/// The pure half of the token rule, tested without a socket because it is the thing both routes share.
#[test]
fn the_token_rule_is_loopback_and_websocket_alike() {
    // No token configured: the door is open, which is §5.1's loopback default.
    assert!(terminal_access(None, None).is_ok());
    assert!(terminal_access(Some("token=abc"), None).is_ok());
    // A token configured: it must be presented, and it must be the one.
    assert!(terminal_access(Some("token=abc"), Some("abc")).is_ok());
    assert!(terminal_access(None, Some("abc")).is_err());
    assert!(terminal_access(Some("token=nope"), Some("abc")).is_err());
    // The refusal names the rule, never the value.
    let message = terminal_access(Some("token=nope"), Some("abc")).unwrap_err();
    assert!(!message.contains("nope") && !message.contains("abc"), "{message}");
}

#[tokio::test]
async fn raw_bytes_round_trip_in_both_directions() {
    let addr = serve(TestState { token: None, spawner: Arc::new(TestSpawner) }).await;
    let mut socket = connect(addr, None).await.expect("the listener must accept the upgrade");

    // A binary frame in, and the same bytes come back out of the PTY, which echoes them.
    socket.send(Message::Binary(b"x".to_vec().into())).await.unwrap();
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + WAIT;
    while seen.len() < 1 && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), socket.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => seen.extend_from_slice(&bytes),
            Ok(Some(Ok(_))) => {}
            _ => {}
        }
    }
    assert!(!seen.is_empty(), "the pump must forward the PTY's output as binary frames");
    let _ = socket.close(None).await;
}

#[tokio::test]
async fn a_resize_message_reaches_the_child() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(TestState {
        token: None,
        spawner: Arc::new(WidthReporterSpawner { dir: dir.path().to_path_buf() }),
    })
    .await;
    let mut socket = connect(addr, None).await.unwrap();

    let before = read_width(&mut socket).await;
    assert!(before > 0, "the child must report its first width, got {before}");
    socket
        .send(Message::text(r#"{"type":"resize","cols":100,"rows":24}"#))
        .await
        .unwrap();
    let after = read_width(&mut socket).await;
    assert!(after > before, "the resize must reach the child: before={before} after={after}");
    let _ = socket.close(None).await;
}

/// Reads binary frames until one contains `WIDTH=<digits>` and returns the last such number.
async fn read_width(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) -> u16 {
    let mut text = String::new();
    let deadline = tokio::time::Instant::now() + WAIT;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), socket.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => {
                text.push_str(&String::from_utf8_lossy(&bytes));
                if let Some(width) = text
                    .split("WIDTH=")
                    .skip(1)
                    .filter_map(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
                    .filter_map(|digits| digits.parse::<u16>().ok())
                    .next_back()
                {
                    return width;
                }
            }
            Ok(Some(Ok(Message::Text(_)))) => {}
            _ => {}
        }
    }
    0
}

#[tokio::test]
async fn the_wrong_token_never_upgrades() {
    let addr = serve(TestState { token: Some(TOKEN.to_string()), spawner: Arc::new(TestSpawner) }).await;
    assert!(connect(addr, None).await.is_err(), "a missing token must not upgrade");
    // One wrong character is enough: `token_access` compares the whole value.
    let mut wrong = TOKEN.to_string();
    wrong.pop();
    wrong.push(if TOKEN.ends_with('0') { '1' } else { '0' });
    assert_ne!(wrong, TOKEN, "the wrong token must actually differ");
    assert!(connect(addr, Some(&wrong)).await.is_err(), "a wrong token must not upgrade");
    assert!(connect(addr, Some(TOKEN)).await.is_ok(), "the right token must upgrade");
}

/// Review Focus 4's primitive, proved with two children at once: one teardown may not take another
/// connection's child with it, and the process a killed connection owned is **gone**, not disowned.
/// The pump in Step 3 is the only caller of the sequence this exercises.
#[cfg(windows)]
#[tokio::test]
async fn killing_a_teardown_leaves_no_child_behind_and_touches_no_other() {
    let dir = tempfile::tempdir().unwrap();
    let spawner = KeepAliveSpawner {
        dir: dir.path().to_path_buf(),
        pids: Arc::new(std::sync::Mutex::new(Vec::new())),
        started: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let mut first = spawner.spawn(80, 24).expect("the first child must spawn");
    let mut second = spawner.spawn(80, 24).expect("the second child must spawn");
    let live: Vec<u32> = spawner.pids.lock().unwrap().clone();
    assert_eq!(live.len(), 2, "both children must have reported a pid");
    assert!(live.iter().all(|pid| process::alive(*pid)), "both children must be running: {live:?}");

    // Exactly what the pump does on a dying socket, for one of the two.
    first.kill_and_wait().expect("the first child must be killed and reaped");
    assert!(process::gone(live[0]), "the killed connection's child is still running: pid {}", live[0]);
    assert!(process::alive(live[1]), "the other connection's child was killed too: pid {}", live[1]);

    // And the second, so the test does not leave a process behind.
    second.kill_and_wait().expect("the second child must be killed and reaped");
    assert!(process::gone(live[1]), "the second child outlived its teardown: pid {}", live[1]);
}
```

> Note for the implementer: this test spawns the two children **directly** rather than through two
> sockets, because what it pins is the teardown sequence itself — the same five steps Step 3's pump
> runs — not the route. Add one more test alongside it that opens
> `MAX_TERMINAL_CONNECTIONS + 1` sockets against a `TestState` whose spawner is `TestSpawner`, and
> asserts with `connect(...).await.is_err()` that the extra one is refused. That is the connection cap
> being observed rather than asserted in a comment.

- [ ] **Step 2: Run the test to verify it fails**

Run:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre --test terminal_ws
```

Expected: FAIL to compile, with

```
error[E0432]: unresolved import `tipsy_compadre::server::conn`
 --> crates\tipsy-compadre\tests\terminal_ws.rs:14:46
  |
14 | use tipsy_compadre::server::conn::{MAX_TERMINAL_CONNECTIONS, TerminalRouteState, router, terminal_access};
  |                                              ^^^^^^^^^^^^^^^^^ no `conn` in `server`
```

- [ ] **Step 3: Implement the pump**

`crates/tipsy-compadre/src/server/mod.rs` — add the module (keep the file's existing order):

```rust
pub mod conn;
```

and register the route inside the router builder that already exists there (Task 1.4/1.5's function;
its name in the tree wins):

```rust
        .merge(conn::router(axum::Router::new()))
```

`crates/tipsy-compadre/src/server/conn.rs`:

```rust
//! The raw-bytes socket: `/terminal/ws` and the pump behind it (spec §5.3, §7.3).
//!
//! This is **not** the protocol socket. Nothing here is JSON that a session reads, no `Command` or
//! `Event` crosses it, and it shares nothing with Task 1.6's bridge: the bytes on the wire are the
//! bytes a terminal would have shown.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::sync::{Semaphore, mpsc};

use crate::client::pty::{PtySession, PtySpawner};

use super::AppState;

/// See the task body: one process and two pipe pairs per connection, and two browsers are two TUI
/// clients fighting for the same microphone anyway (research note §7, trap 4).
pub const MAX_TERMINAL_CONNECTIONS: usize = 2;
/// Chunks a stalled reader may leave queued before the reader starts dropping them.
pub const TERMINAL_WRITE_QUEUE: usize = 32;
/// One read from the PTY master, inside the 8–64 KiB the research note recommends (§7, trap 3).
pub const TERMINAL_READ_CHUNK: usize = 8 * 1024;
/// xterm.js asks for a debounced resize (research note §6.3); this is the server's half of it.
pub const TERMINAL_RESIZE_DEBOUNCE_MS: u64 = 100;

/// Which sockets this process has already given a PTY to.
static LIVE_TERMINALS: AtomicU64 = AtomicU64::new(0);

#[derive(serde::Deserialize)]
pub struct TerminalQuery {
    token: Option<String>,
}

/// What the route needs from the application state. The real `AppState` implements it in
/// `server/mod.rs`; this task's tests implement it on two fields.
pub trait TerminalRouteState: Clone + Send + Sync + 'static {
    fn websocket_token(&self) -> Option<String>;
    fn terminal_spawner(&self) -> Arc<dyn PtySpawner>;
}

/// Registers `/terminal/ws` on a router whose state implements [`TerminalRouteState`].
pub fn router<S: TerminalRouteState>(router: axum::Router<S>) -> axum::Router<S> {
    use axum::routing::get;
    router.route("/terminal/ws", get(terminal_ws::<S>))
}

/// `Some(())` when the query carries the token this server requires, or when it requires none.
/// `Err` is a 401's body and names the rule, never the value — the token never reaches a log line.
pub fn terminal_access(query: Option<&str>, expected: Option<&str>) -> Result<(), String> {
    let presented = query.and_then(|query| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(name, _)| *name == "token")
            .map(|(_, value)| value.to_string())
    });
    match expected {
        None => Ok(()),
        Some(expected) if presented.as_deref() == Some(expected) => Ok(()),
        Some(_) => Err("this server requires its token on /terminal/ws".to_string()),
    }
}

pub async fn terminal_ws<S: TerminalRouteState>(
    State(state): State<S>,
    Query(query): Query<TerminalQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(message) = terminal_access(query.token.as_deref(), state.websocket_token().as_deref()) {
        return (StatusCode::UNAUTHORIZED, message).into_response();
    }
    // The write buffer only grows past `write_buffer_size` while writes are failing, and its default
    // `max_write_buffer_size` is unlimited (research note §7, trap 2): without a ceiling a browser
    // that stopped reading would grow the server instead of slowing it. 256 KiB = 2 x 128 KiB.
    let ws = ws.max_write_buffer_size(256 * 1024);
    ws.on_upgrade(move |socket| handle_terminal(socket, state))
}

async fn handle_terminal<S: TerminalRouteState>(mut socket: WebSocket, state: S) {
    if LIVE_TERMINALS.load(Ordering::Relaxed) as usize >= MAX_TERMINAL_CONNECTIONS {
        let _ = socket.send(Message::text("too many terminals")).await;
        let _ = socket.close().await;
        return;
    }
    LIVE_TERMINALS.fetch_add(1, Ordering::Relaxed);
    let session = tokio::task::spawn_blocking(move || state.terminal_spawner().spawn(80, 24)).await;
    let mut session = match session {
        Ok(Ok(session)) => session,
        Ok(Err(message)) => {
            let _ = socket.send(Message::text(format!("terminal unavailable: {message}"))).await;
            let _ = socket.close().await;
            LIVE_TERMINALS.fetch_sub(1, Ordering::Relaxed);
            return;
        }
        Err(_) => {
            LIVE_TERMINALS.fetch_sub(1, Ordering::Relaxed);
            return;
        }
    };

    let exit = pump(&mut socket, &mut session).await;
    let _ = socket
        .send(Message::text(serde_json::json!({ "type": "exit", "code": exit }).to_string()))
        .await;
    let _ = socket.close().await;
    LIVE_TERMINALS.fetch_sub(1, Ordering::Relaxed);
}

/// The pump. Both directions are bounded and both ends of the PTY are serviced on threads of their
/// own — ConPTY's own guidance is to service each channel on a separate thread, "to prevent race
/// conditions and deadlocks" (research note §5.3).
async fn pump<P>(socket: &mut WebSocket, session: &mut crate::client::pty::PtySession) -> u32 {
    let mut reader = session.take_reader();
    let writer = session.take_writer();
    let mut resize = ResizeDebounce::new(TERMINAL_RESIZE_DEBOUNCE_MS);

    // One bounded queue, two threads: the blocking reader fills it, the socket drains it. `send`
    // awaits when it is full, which is exactly the backpressure a WebSocket does not have.
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(TERMINAL_WRITE_QUEUE);
    let reader_task = reader.take().map(|mut reader| {
        tokio::task::spawn_blocking(move || {
            let mut chunk = vec![0u8; TERMINAL_READ_CHUNK];
            loop {
                match std::io::Read::read(&mut reader, &mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        if out_tx.blocking_send(chunk[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })
    });

    // The other direction is a blocking writer fed by a task of its own, so no runtime worker ever
    // sits in `Box<dyn Write>` (research note §7, trap 7).
    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(TERMINAL_WRITE_QUEUE);
    let writer_task = writer.map(|mut writer| {
        tokio::task::spawn_blocking(move || {
            while let Some(bytes) = in_rx.blocking_recv() {
                if std::io::Write::write_all(&mut writer, &bytes).is_err() {
                    break;
                }
                let _ = std::io::Write::flush(&mut writer);
            }
        })
    });

    loop {
        tokio::select! {
            chunk = out_rx.recv() => match chunk {
                Some(bytes) => {
                    if socket.send(Message::binary(bytes)).await.is_err() {
                        break;
                    }
                }
                // The reader reached EOF: the child is gone, and the socket goes with it.
                None => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Binary(_padding))) => {
                    // A browser cannot observe whether its own socket is open at the server; the
                    // first data frame is what says the connection is real. Nothing else is binary.
                }
                Some(Ok(Message::Text(text))) => {
                    let (bytes, resize_request) = match serde_json::from_str::<TerminalClientMessage>(&text) {
                        Ok(message) => (message.bytes(), message.resize()),
                        // A malformed message is a bug in the page, not a reason to end a session
                        // someone may be talking to: ignore it and keep the socket open.
                        Err(_) => (None, None),
                    };
                    if let Some(bytes) = bytes {
                        let _ = in_tx.send(bytes).await;
                    }
                    if let Some((cols, rows)) = resize_request {
                        resize.request(cols, rows);
                    }
                }
                Some(Ok(_)) => {}
                // A close frame or a transport error: the tab is gone.
                Some(Err(_)) | None => break,
            },
            // The debounce's deadline is read as a plain value here, so this branch holds no borrow
            // of `resize` while the branches below mutate it. `pending()` is the "no resize is
            // pending, this branch can never win" case.
            () = async {
                match resize.deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some((cols, rows)) = resize.take() {
                    if let Err(message) = session.resize(cols, rows) {
                        tracing::debug!("terminal resize ignored: {message}");
                    }
                }
            }
        }
    }

    // Teardown, in this order and for these reasons (spec §7.3, Review Focus 4): the child is killed,
    // the writer is dropped — "Dropping the writer will send EOF to the slave end" (research note
    // §5.1) — and then the master goes, so the blocking reader observes EOF and exits. A TUI that
    // ignores EOF on stdin would otherwise linger, which is why killing is the first step and not the
    // last, and why every step here is explicit rather than left to a `Drop`.
    if let Some(mut killer) = session.killer() {
        let _ = killer.kill();
    }
    // Nothing more is written. Dropping the sender ends the writer task, and with it the writer —
    // which is what sends EOF to the slave end.
    drop(in_tx);
    // Both handles are dropped rather than awaited. Awaited, they would be the one place this socket
    // could stall: the reader is parked in a blocking `read` until the master goes, and `await`ing a
    // `spawn_blocking` task does not help a task that is not due to finish. It is `task.abort()`-ed
    // only in the sense that nobody waits for it; it ends on its own when the reader reports EOF.
    drop(writer_task);
    drop(reader_task);
    // The session — and with it the master and the reader handle — goes out of scope when the caller
    // returns, which is what lets the parked reader see EOF.
    // The pump's own view of the exit code, and deliberately not the child's: reading the real status
    // would mean `wait`, and `wait` blocks. The page prints a line either way, so this socket never
    // waits on a process. `PtySession::wait` exists for the tests and for a future reaper; nothing in
    // this pump calls it.
    0
}

/// The debounced resize: xterm.js's own doc is the reason (§6.3).
struct ResizeDebounce {
    window: std::time::Duration,
    deadline: Option<tokio::time::Instant>,
    requested: Option<(u16, u16)>,
}

impl ResizeDebounce {
    fn new(debounce_ms: u64) -> Self {
        Self {
            window: std::time::Duration::from_millis(debounce_ms),
            deadline: None,
            requested: None,
        }
    }

    /// A resize arrived: remember it and push the deadline back.
    fn request(&mut self, cols: u16, rows: u16) {
        self.requested = Some((cols, rows));
        self.deadline = Some(tokio::time::Instant::now() + self.window);
    }

    /// Take the pending size, if the window has elapsed. The `select!` in `pump` reads `deadline`
    /// itself rather than through a future-returning method, so nothing borrows this struct across an
    /// await.
    fn take(&mut self) -> Option<(u16, u16)> {
        self.deadline = None;
        self.requested.take()
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalClientMessage {
    Hello,
    /// `onData`: escape sequences and UTF-8 text, written to the PTY as UTF-8 bytes.
    Data { data: String },
    /// `onBinary`: one byte per JS `charCode`, mouse reports among them.
    Mouse { data: String },
    Resize { cols: u16, rows: u16 },
}

impl TerminalClientMessage {
    fn bytes(&self) -> Option<Vec<u8>> {
        match self {
            Self::Data { data } => Some(data.as_bytes().to_vec()),
            Self::Mouse { data } => Some(data.chars().map(|c| c as u32 as u8).collect()),
            Self::Hello | Self::Resize { .. } => None,
        }
    }

    fn resize(&self) -> Option<(u16, u16)> {
        match self {
            Self::Resize { cols, rows } => Some((*cols, *rows)),
            _ => None,
        }
    }
}
```

`AppState` (Task 1.4) gains its `TerminalRouteState` impl in `server/mod.rs`:

```rust
#[derive(Clone)]
pub struct AppState {
    // … Task 1.4's fields …
    pub spawner: std::sync::Arc<dyn crate::client::pty::PtySpawner>,
}

impl conn::TerminalRouteState for AppState {
    fn websocket_token(&self) -> Option<String> {
        // The same value `/ws`'s `Hello` compares against: `Some` only when the configuration
        // holds a token, which is exactly what `ServerConfig::has_token` (`config/model.rs:59-61`)
        // decides. Never log it.
        self.settings.token.clone()
    }

    fn terminal_spawner(&self) -> std::sync::Arc<dyn crate::client::pty::PtySpawner> {
        std::sync::Arc::clone(&self.spawner)
    }
}
```

and `server::run` builds that spawner from its own socket address:

```rust
    // The child is this same binary in `--connect` mode, pointed at our own loopback `/ws`
    // (spec §7.2, W10): a client, like a user on another machine, so the child starts no server
    // and makes no provider call. `current_exe()` and not a name — a `PATH` entry must never be able
    // to decide which program runs with the user's authority (Task 2.1's security test).
    let spawner = std::sync::Arc::new(crate::client::pty::TerminalSpawner {
        program: std::env::current_exe().map_err(|e| ServerError::Message(format!("no current executable: {e}")))?,
        args: crate::client::bridge::connect_args(loopback_ws_addr, &settings.token.clone().unwrap_or_default()),
        ..Default::default()
    });
```

`connect_args` is Task 1.7's; its exact spelling is in that task's body, and this is its one caller
besides the `--connect` client itself.

- [ ] **Step 4: Run the test to verify it passes**

Run:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre --test terminal_ws
```

Expected: PASS — `test result: ok. 5 passed; 0 failed` (four socket/unit tests plus the cap test the
note above asks for).

Then the lint gate, against the pinned baseline of `global-constraints.md` (add nothing):

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo clippy -j 1 -p tipsy-compadre --all-targets
```

- [ ] **Step 5: Commit**

```bash
git add crates/tipsy-compadre/src/server/conn.rs crates/tipsy-compadre/src/server/mod.rs crates/tipsy-compadre/tests/terminal_ws.rs
git commit -m "feat(server): pump raw pty bytes over /terminal/ws

a second WebSocket beside the protocol socket, carrying terminal bytes and
nothing else: binary frames down, text control messages up, bounded queues in
both directions, a debounced resize, and the explicit teardown that kills the
child, drops the writer and lets the blocking reader see EOF. The token rule is
the page routes' own, and one more than the connection cap is refused.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 2.3: xterm.js, fetched and verified at build time `[small]`

**Files:**
- Create: `xtask/Cargo.toml`, `xtask/src/main.rs`, `xtask/src/assets.rs`
- Create: `web/vendor/README.md`
- Modify: `Cargo.toml:3` (add `"xtask"` to the workspace `members` list)
- Modify: `.gitignore:1-6` (append these two lines)
- Test: `xtask/src/assets.rs` (its own `#[cfg(test)] mod tests`)

```gitignore
web/vendor/*
!web/vendor/README.md
```

**What the fetch adds, and why the ignore rule is not `web/`:** the fetched files must stay out of the
repository, and `web/vendor/README.md` must stay **in** it — that file is the record of what the build
downloads and under what licence (spec §2 W5). That is what the negation above is for.

**Interfaces:**

- Consumes: nothing from this plan. Every version and every URL below was read off the npm registry
  while writing this task (the same endpoints the research note cites in its §6.1):
  - `@xterm/xterm` **6.0.0**: `"main":"lib/xterm.js"`, `"module":"lib/xterm.mjs"`,
    `"types":"typings/xterm.d.ts"`, `"style":"css/xterm.css"`, `"license":"MIT"`, and
    `dist.tarball = "https://registry.npmjs.org/@xterm/xterm/-/xterm-6.0.0.tgz"`
    ([registry.npmjs.org/@xterm/xterm/6.0.0](https://registry.npmjs.org/@xterm/xterm/6.0.0)).
    **The CSS is a separate file in the tarball and must be carried** — the research note's §6.1 says
    it is "the fact most likely to be missed in a minimal page".
  - `@xterm/addon-fit` **0.11.0**: `"module":"lib/addon-fit.mjs"`, `"license":"MIT"`, and
    `dist.tarball = "https://registry.npmjs.org/@xterm/addon-fit/-/addon-fit-0.11.0.tgz"`
    ([registry.npmjs.org/@xterm/addon-fit/0.11.0](https://registry.npmjs.org/@xterm/addon-fit/0.11.0)).
    `FitAddon.fit(): void` is real — read off the package's own published typings,
    `https://unpkg.com/@xterm/addon-fit@0.11.0/typings/addon-fit.d.ts`, which is what removes the
    `// verify` the research note's §6.5 skeleton carries next to `fit.fit()`.
  - `rust-embed` 8.13.0's `#[allow_missing = true]`, which is what makes the bundle optional (Task
    1.5 owns that side).
- Produces, in the `xtask` binary crate (`cargo xtask …`, so `xtask/src/main.rs` dispatches on
  `std::env::args().nth(1)`):

```rust
// xtask/src/assets.rs
/// What the page vendors, in the order the fetch runs.
pub const COMPONENTS: [Component; 2] = [XTERM, ADDON_FIT];

pub struct Component {
    /// The package's own name, as npm spells it.
    pub name: &'static str,
    /// The pinned version. Never a range: the hash below is what makes it a pin.
    pub version: &'static str,
    pub tarball: &'static str,
    /// sha256 of the tarball, lowercase hex. **Obtained by the implementer, never invented** — see
    /// Step 3.
    pub sha256: &'static str,
    /// (member path inside the tarball, file name under `web/vendor/`).
    pub files: &'static [( &'static str, &'static str )],
    pub license: &'static str,
}

/// lowercase hex sha256 of any byte string.
pub fn sha256_of(bytes: &[u8]) -> String;

/// `Ok(())` when the digest is the one pinned for this component.
pub fn verify(component: &Component, bytes: &[u8]) -> Result<(), String>;

/// The paths under `web/vendor/` this component is supposed to leave behind.
pub fn expected_assets(vendor: &std::path::Path) -> Vec<std::path::PathBuf>;

/// `Ok(())` when every file the page needs is already present; `Err(missing)` names the first that is
/// not. This is what makes a second run a no-op.
pub fn have_assets(vendor: &std::path::Path) -> Result<(), String>;

/// `Ok(())` when the files were already there and were not fetched again; `Err(reason)` when the
/// component could not be fetched, verified or extracted. Does **not** return a warning on a hash
/// mismatch — a mismatch is a failure, because a warning here is a page that runs someone else's
/// JavaScript (spec §2 W5).
pub fn fetch(component: &Component, vendor: &std::path::Path) -> Result<(), String>;

/// The one job `cargo xtask dist` starts with, before the wasm build and the embedding (spec §8.3).
pub fn fetch_all(vendor: &std::path::Path) -> Result<(), String>;

/// Every `url(...)` the fetched CSS names, so the extraction step can be checked against it.
pub fn css_references(css: &str) -> Vec<String>;
```

- Produces, in `xtask/src/main.rs`:

```rust
fn main() -> std::process::ExitCode;   // `xtask dist` → assets::fetch_all("web/vendor"), then Task 3.6's build
```

- [ ] **Step 1: Obtain the two hashes, then write the failing test**

**The hashes are obtained and never invented.** This is the only part of the task that needs the
network, and it produces the two values the implementation pastes into `xtask/src/assets.rs`. Run it
once, from the repository root; the two URLs are the `dist.tarball` values quoted in the `Interfaces`
block above, read off the npm registry.

```powershell
$dir = "$PWD\.target-sandbox\xtask-pins"; New-Item -ItemType Directory -Force -Path $dir | Out-Null
Invoke-WebRequest -Uri 'https://registry.npmjs.org/@xterm/xterm/-/xterm-6.0.0.tgz' -OutFile "$dir\xterm-6.0.0.tgz"
Invoke-WebRequest -Uri 'https://registry.npmjs.org/@xterm/addon-fit/-/addon-fit-0.11.0.tgz' -OutFile "$dir\addon-fit-0.11.0.tgz"
$env:GZIP = '-n'
& tar.exe -tzf "$dir\xterm-6.0.0.tgz"      | Select-String -Pattern 'xterm.mjs|xterm.css'
& tar.exe -tzf "$dir\addon-fit-0.11.0.tgz" | Select-String -Pattern 'addon-fit.mjs'
Get-FileHash "$dir\xterm-6.0.0.tgz"      -Algorithm SHA256 | Select-Object -ExpandProperty Hash
Get-FileHash "$dir\addon-fit-0.11.0.tgz" -Algorithm SHA256 | Select-Object -ExpandProperty Hash
```

Expected: the two `tar -tzf` listings show `package/lib/xterm.mjs`, `package/css/xterm.css` and
`package/lib/addon-fit.mjs`, and the two `Get-FileHash` lines print 64 uppercase hex characters each.
Those two hashes are what the implementation pins — **copied unchanged, only lowercased into the
source**. If the listing shows the members under different paths, the listing wins: correct
`Component::files` to the paths it printed and say so in the ledger. If `Invoke-WebRequest` cannot
reach the registry, stop and report it in the ledger: the hash is the only thing that makes these
bytes trustworthy, and it may not be guessed, recalled or left as a literal placeholder in a commit.

Then write `xtask/src/assets.rs`'s test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A 132-byte `.tar.gz` holding exactly `pkg/a.txt` with the eight bytes `hello` in it. It is the
    /// fixture the fetch's own pipeline is exercised against, so the test does not need the network:
    /// the bytes, their sha256 and `tar`'s own extraction are all real.
    const FIXTURE_TGZ_BASE64: &str = concat!(
        "H4sIADGqy2oAA+3RQQqAIBAFUI/iCWo0nDmPiyhISMqg46e0cxGBWETzNoMb/c7309CKygCAiGSaUT7PgzIaNWkDqCQoAEVC",
        "mtrBkm0NdolRSu/JP/cRPvZvm7CHim/EfSDiVf8m75+oE7K4kzt+3v/YOze/HYIxxtjjDg5q4hkACgAA",
    );

    /// The fixture is what `tar -czf` produced; the hash is what `Get-FileHash -Algorithm SHA256`
    /// printed for that same file when the fixture was made. If this assertion ever fails, the bytes
    /// in the two string literals above changed, and the right answer is to regenerate both with the
    /// two commands in Step 1, never to relax the assertion.
    const FIXTURE_TGZ_SHA256: &str =
        "e3ec7d757fd4066a41e602aec5cb602d67e62912bd20e8c9d06d0893c8d9a91e";

    fn fixture() -> Vec<u8> {
        // `base64` is not a dependency of this workspace and must not become one: this is the one
        // place a base64 blob is decoded, so it is decoded here, in twenty lines.
        fn decode(input: &str) -> Vec<u8> {
            const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = Vec::new();
            let mut buffer = 0u32;
            let mut bits = 0u32;
            for byte in input.bytes() {
                if byte == b'=' {
                    break;
                }
                let Some(value) = ALPHABET.iter().position(|candidate| *candidate == byte) else {
                    continue;
                };
                buffer = (buffer << 6) | value as u32;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    out.push((buffer >> bits) as u8);
                }
            }
            out
        }
        decode(FIXTURE_TGZ_BASE64)
    }

    #[test]
    fn the_hash_verifier_accepts_the_pinned_digest_and_rejects_any_other() {
        let bytes = fixture();
        assert_eq!(sha256_of(&bytes), FIXTURE_TGZ_SHA256, "the fixture's own digest must be stable");
        assert!(verify(&FIXTURE, &bytes).is_ok(), "the pinned digest must be accepted");

        let mut wrong = bytes.clone();
        wrong[0] ^= 0xff;
        let refused = verify(&FIXTURE, &wrong).unwrap_err();
        assert!(refused.contains("sha256"), "the refusal must name what did not match: {refused}");
        // A mismatch is a failure, not a warning: the fetch has no partial state to fall back on.
        assert!(refused.contains("is not"), "{refused}");
        assert!(!refused.contains(FIXTURE_TGZ_SHA256) || !refused.contains(&sha256_of(&wrong)), "the two digests, side by side");
    }

    #[test]
    fn an_already_extracted_component_is_never_fetched_again() {
        let dir = tempfile::tempdir().unwrap();
        let vendor = dir.path().join("vendor");
        std::fs::create_dir_all(&vendor).unwrap();
        assert!(have_assets(&vendor).is_err(), "an empty directory is missing both components");
        for component in COMPONENTS {
            for (_, name) in component.files {
                std::fs::write(vendor.join(name), b"present").unwrap();
            }
        }
        assert!(have_assets(&vendor).is_ok(), "every expected file is there");
        // `fetch` checks `have_assets` first, so a second run does not download anything: the pinned
        // URL would fail here (there is no network in this test) and the call still succeeds.
        for component in COMPONENTS {
            assert!(fetch(component, &vendor).is_ok(), "{} was fetched again", component.name);
        }
    }

    #[test]
    fn the_extraction_keeps_exactly_the_members_the_page_needs() {
        let dir = tempfile::tempdir().unwrap();
        let vendor = dir.path().join("vendor");
        std::fs::create_dir_all(&vendor).unwrap();
        let archive = dir.path().join("fixture.tgz");
        std::fs::write(&archive, fixture()).unwrap();

        extract_members(&archive, &[("pkg/a.txt", "a.txt")], &vendor).expect("tar must extract the member");
        let extracted = std::fs::read_to_string(vendor.join("a.txt")).unwrap();
        assert_eq!(extracted, "hello");
        assert!(!vendor.join("pkg").exists(), "only the named members leave the archive");
    }

    #[test]
    fn the_css_references_are_readable_so_the_extraction_can_be_checked_against_them() {
        let references = css_references("a { background: url(font.woff2); } b { background: url('icons/x.svg'); }");
        assert_eq!(references, vec!["font.woff2".to_string(), "icons/x.svg".to_string()]);
        assert!(css_references("a { color: red; }").is_empty());
    }

    /// The fixture's `Component`, kept out of `COMPONENTS` so the real pins are the only thing the
    /// fetch ever runs against.
    const FIXTURE: Component = Component {
        name: "@xterm/xterm",
        version: "fixture",
        tarball: "",
        sha256: FIXTURE_TGZ_SHA256,
        files: &[("pkg/a.txt", "a.txt")],
        license: "MIT",
    };
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p xtask
```

Expected: FAIL to compile — there is no `xtask` crate yet:

```
error: package ID specification `xtask` did not match any packages
```

and, once the crate exists but the module does not, `error[E0432]: unresolved import` for
`assets::sha256_of`. Neither failure runs a test; that is the point of writing both halves first.

- [ ] **Step 3: Implement the fetch**

`Cargo.toml:3`:

```toml
members = ["crates/tipsy-protocol", "crates/tipsy-ui", "crates/tipsy-compadre", "xtask"]
```

`.gitignore` — append. The negation matters: `web/vendor/` would ignore the README too, and the
README is the file the plan wants committed (spec §2 W5):

```gitignore
web/vendor/*
!web/vendor/README.md
```

`xtask/Cargo.toml`:

```toml
[package]
name = "xtask"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "Build tasks for tipsy-compadre"

[dependencies]

[dev-dependencies]
tempfile = "3.27"
```

(`tempfile` is already the workspace's dev-dependency version — `crates/tipsy-compadre/Cargo.toml:36`
— so this adds no new crate to the lock. Everything else here is the standard library: **no new
dependency beyond the Tech Stack list**.)

`xtask/src/main.rs`:

```rust
//! `cargo xtask <task>`: the project's own build scripts, so the release steps live in the repository
//! instead of in a README someone has to retype.

mod assets;

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("dist") => dist(),
        Some(other) => {
            eprintln!("xtask: unknown task `{other}`; the tasks are `dist`");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("xtask: which task? `cargo xtask dist`");
            ExitCode::FAILURE
        }
    }
}

/// Spec §8.3's order: fetch and verify xterm.js, then the wasm build, then the host binary that
/// embeds `web/`. Task 3.6 owns the second and third steps; this task owns the first, so that the
/// page Task 2.4 writes has its assets before the bundle has anything to embed.
fn dist() -> ExitCode {
    let vendor = std::path::Path::new("web/vendor");
    match assets::fetch_all(vendor) {
        Ok(()) => {
            println!("xtask: web/vendor is ready");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}
```

`xtask/src/assets.rs` — the constants, with the two hashes obtained in Step 1 pasted in place of the
`obtain-and-paste` markers (this is the same pattern the repository already uses for a value that
cannot be recalled: nothing is invented, and the note says where it came from):

```rust
//! Fetching the browser assets the terminal page vendors (spec §2 W5, §8.3).
//!
//! The files are not committed and are not loaded from a CDN — the `serve` command is a LAN tool that
//! may run with no internet access, and a third-party origin is exactly the case that leaks a
//! `?token=` in a `Referer` under the default policy (research note §6.1). Instead they are fetched
//! once, from the npm registry, at a pinned version, and checked against a sha256 pinned here.

use std::path::Path;
use std::process::Command;

/// The `@xterm/xterm` tarball the page's terminal comes from. `"module"` is `lib/xterm.mjs` and
/// `"style"` is a **separate file**, `css/xterm.css`, which is why the CSS is a member of this list.
pub const XTERM: Component = Component {
    name: "@xterm/xterm",
    version: "6.0.0",
    tarball: "https://registry.npmjs.org/@xterm/xterm/-/xterm-6.0.0.tgz",
    // Obtained with `Get-FileHash -Algorithm SHA256` on the tarball this exact URL served; recorded
    // in `web/vendor/README.md`. Never edited by hand.
    sha256: "OBTAIN-AND-PASTE-xterm-6.0.0",
    files: &[("package/lib/xterm.mjs", "xterm.mjs"), ("package/css/xterm.css", "xterm.css")],
    license: "MIT",
};

/// The fit addon, whose `FitAddon.fit()` is the page's only way to size the terminal to its window.
pub const ADDON_FIT: Component = Component {
    name: "@xterm/addon-fit",
    version: "0.11.0",
    tarball: "https://registry.npmjs.org/@xterm/addon-fit/-/addon-fit-0.11.0.tgz",
    sha256: "OBTAIN-AND-PASTE-addon-fit-0.11.0",
    files: &[("package/lib/addon-fit.mjs", "addon-fit.mjs")],
    license: "MIT",
};

pub const COMPONENTS: [Component; 2] = [XTERM, ADDON_FIT];

pub struct Component {
    pub name: &'static str,
    pub version: &'static str,
    pub tarball: &'static str,
    /// Lowercase hex. A mismatch is a **build failure**, not a warning.
    pub sha256: &'static str,
    /// (member path inside the tarball, file name under `web/vendor/`).
    pub files: &'static [(&'static str, &'static str)],
    pub license: &'static str,
}

/// Lowercase hex sha256, the standard algorithm, written out rather than adding a crate to the
/// workspace for forty lines of arithmetic.
pub fn sha256_of(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut message = bytes.to_vec();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for block in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (index, word) in block.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7) ^ w[index - 15].rotate_right(18) ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17) ^ w[index - 2].rotate_right(19) ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e;
            e = d.wrapping_add(temp1);
            d = c; c = b; b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

pub fn verify(component: &Component, bytes: &[u8]) -> Result<(), String> {
    let actual = sha256_of(bytes);
    if actual == component.sha256 {
        Ok(())
    } else {
        Err(format!(
            "{} {}: the tarball's sha256 is {actual}, and the one pinned in xtask/src/assets.rs is {} \
             — the fetch has been stopped, because the file on the wire is not the file this project \
             verified. Do not re-pin until you know why they differ.",
            component.name, component.version, component.sha256
        ))
    }
}

/// The names this component is supposed to leave under `web/vendor/`.
pub fn expected_assets(vendor: &Path) -> Vec<std::path::PathBuf> {
    COMPONENTS
        .iter()
        .flat_map(|component| component.files.iter())
        .map(|(_, name)| vendor.join(name))
        .collect()
}

pub fn have_assets(vendor: &Path) -> Result<(), String> {
    for path in expected_assets(vendor) {
        if !path.is_file() {
            return Err(format!("{} is missing", path.display()));
        }
    }
    Ok(())
}

/// Fetches one component unless its files are all there already. A second run therefore does not
/// re-download what is present and verified.
pub fn fetch(component: &Component, vendor: &Path) -> Result<(), String> {
    if component.files.iter().all(|(_, name)| vendor.join(name).is_file()) {
        return Ok(());
    }
    std::fs::create_dir_all(vendor).map_err(|e| format!("{}: {e}", vendor.display()))?;
    let archive = vendor.join(format!(".{}-{}.tgz", component.name.replace('/', "-"), component.version));
    let bytes = download(component.tarball)?;
    verify(component, &bytes)?;
    std::fs::write(&archive, &bytes).map_err(|e| format!("{}: {e}", archive.display()))?;
    let extracted = extract_members(&archive, component.files, vendor);
    let _ = std::fs::remove_file(&archive);
    extracted?;

    // The CSS is the member most likely to be dropped, and it is the one that points at more files.
    // Reading its `url(...)` references is what turns a silently unstyled terminal into a list of
    // names a later reviewer can check.
    if let Some((_, name)) = component.files.iter().find(|(_, name)| name.ends_with(".css")) {
        let path = vendor.join(name);
        let css = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        for reference in css_references(&css) {
            println!("xtask: {} references {reference}", path.display());
        }
    }
    Ok(())
}

pub fn fetch_all(vendor: &Path) -> Result<(), String> {
    if have_assets(vendor).is_ok() {
        println!("xtask: {} is already fetched and verified", vendor.display());
        return Ok(());
    }
    for component in COMPONENTS {
        fetch(&component, vendor)?;
        println!("xtask: {} {} verified", component.name, component.version);
    }
    have_assets(vendor)
}

/// One HTTPS GET, through the platform's own client: `curl.exe` ships with Windows 10 1809 and later
/// (the same floor ConPTY has) and `curl` is on every Unix this project builds for. It does TLS and
/// certificate validation correctly, which is the whole reason not to hand-roll an HTTP client — and
/// it keeps `reqwest`, `tokio` and a runtime out of a build script.
///
/// `-f` turns an HTTP error status into a non-zero exit, `-L` follows the registry's redirects, `-s`
/// keeps progress off stdout (the bytes are stdout) and `-S` still prints errors on failure.
fn download(url: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err(format!("{url}: only https is accepted, the pinned hashes assume it"));
    }
    let out = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" })
        .args(["-fsSL", "--max-time", "120", url])
        .output()
        .map_err(|e| format!("{url}: no curl to fetch it with: {e}"))?;
    if !out.status.success() {
        return Err(format!("{url}: curl exited with {}", out.status));
    }
    if out.stdout.is_empty() {
        return Err(format!("{url}: curl returned nothing"));
    }
    Ok(out.stdout)
}

/// `tar` extracts named members. `tar.exe` ships with Windows 10 1809 and later (bsdtar) and is on
/// every Unix; the member names are the archive's own, quoted from the registry's file listing.
fn extract_members(archive: &Path, members: &[(&str, &str)], vendor: &Path) -> Result<(), String> {
    let staging = vendor.join(".staging");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    for (member, _) in members {
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(&staging)
            .arg(member)
            .status()
            .map_err(|e| format!("tar: {e}"))?;
        if !status.success() {
            return Err(format!("tar could not extract {member} from {}", archive.display()));
        }
    }
    for (member, name) in members {
        let from = staging.join(member);
        let to = vendor.join(name);
        std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

/// Every `url(...)` the CSS names. Read so that a future reviewer can see what the CSS expects to
/// find beside it; the page needs none of them for a terminal, which draws text.
pub fn css_references(css: &str) -> Vec<String> {
    let mut references = Vec::new();
    let mut rest = css;
    while let Some(start) = rest.find("url(") {
        rest = &rest[start + 4..];
        let Some(end) = rest.find(')') else { break };
        let reference = rest[..end].trim().trim_matches(['\'', '"']).to_string();
        if !reference.is_empty() {
            references.push(reference);
        }
        rest = &rest[end + 1..];
    }
    references
}
```

`web/vendor/README.md` — written with the hashes obtained in Step 1:

```markdown
# Vendored browser assets

Nothing in this directory is committed (`.gitignore` ignores `web/vendor/`); it is fetched by
`cargo xtask dist` and embedded into the binary by `rust-embed`. This file is the record of what the
build downloads, from where, and under what licence.

| Package | Version | Fetched from | sha256 | Licence |
|---|---|---|---|---|
| `@xterm/xterm` | 6.0.0 | `https://registry.npmjs.org/@xterm/xterm/-/xterm-6.0.0.tgz` | `<the hash Step 1 printed>` | MIT (registry metadata) |
| `@xterm/addon-fit` | 0.11.0 | `https://registry.npmjs.org/@xterm/addon-fit/-/addon-fit-0.11.0.tgz` | `<the hash Step 1 printed>` | MIT (registry metadata) |

Files kept, per package:

- `@xterm/xterm` → `xterm.mjs` (`package/lib/xterm.mjs`, the package's `"module"`) and
  `xterm.css` (`package/css/xterm.css`, the package's `"style"` — a separate file in the tarball, and
  the terminal renders with no layout without it).
- `@xterm/addon-fit` → `addon-fit.mjs` (`package/lib/addon-fit.mjs`).

The hashes above are the ones pinned in `xtask/src/assets.rs`. A mismatch is a **build failure**:
`xtask` stops and prints both digests. To re-pin, download the tarball, run
`Get-FileHash -Algorithm SHA256`, and change both the constant and this table in the same commit —
never one without the other.
```

- [ ] **Step 4: Run the test to verify it passes, then do the real fetch**

Run:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p xtask
```

Expected: PASS — `test result: ok. 4 passed; 0 failed`.

Then the real fetch, against the network, which is what turns the two hashes Step 1 printed into
verified files on disk:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo run -j 1 -p xtask -- dist
```

Expected: `xtask: @xterm/xterm 6.0.0 verified`, `xtask: @xterm/addon-fit 0.11.0 verified`,
`xtask: web/vendor is ready` — and a second run of the same command prints
`xtask: web/vendor is already fetched and verified` **without touching the network**, which is the
no-re-download behaviour the unit test above pins without a socket. Confirm the three files are there:

```powershell
Get-ChildItem web\vendor | Select-Object Name,Length
```

Expected: `addon-fit.mjs`, `xterm.css`, `xterm.mjs` and `README.md`.

Then the lint gate, against the pinned baseline of `global-constraints.md` (add nothing):

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo clippy -j 1 -p xtask --all-targets
```

- [ ] **Step 5: Commit**

```bash
git add xtask/Cargo.toml xtask/src/main.rs xtask/src/assets.rs web/vendor/README.md Cargo.toml .gitignore
git commit -m "feat(xtask): fetch and verify the browser assets xterm.js needs

cargo xtask dist now begins by downloading the pinned @xterm/xterm 6.0.0 and
@xterm/addon-fit 0.11.0 tarballs from the npm registry, checking each against a
sha256 recorded in the source, and extracting only the two files the terminal
page loads plus the CSS, which is a separate file in the package. A hash
mismatch stops the build; a second run re-uses what is already verified.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 2.4: The terminal page, end to end `[small]`

**Files:**
- Create: `web/terminal/index.html`
- Modify: `crates/tipsy-compadre/src/server/conn.rs` (its `#[cfg(test)] mod tests`, appended)
- Modify: `docs/architecture.md:62-65`
- Modify: `README.md` (a "How to verify the web terminal" section, next to the existing building section at `README.md:243-250`)
- Test: `crates/tipsy-compadre/src/server/conn.rs` (its own `#[cfg(test)] mod tests`) — the browser half is a documented manual check, because **there is no browser in this harness**

**Interfaces:**

- Consumes:
  - `server::assets::{PAGE_TERMINAL, asset(path)}` — Task 1.5. `asset` takes a bundle-relative path
    and returns bytes plus a content type, or `None` when the bundle is absent. The page is served
    from `web/terminal/index.html` and its two scripts and stylesheet from `web/vendor/`, so the
    routes are `/terminal` and `/assets/vendor/xterm.mjs` (Task 1.5's `/assets/` prefix, Task 2.3's
    file names).
  - The wire shape of `/terminal/ws` — Task 2.2, quoted in its body and repeated here because the
    page implements the other half: binary frames down are raw PTY bytes; `{"type":"exit","code":N}`
    is the only text frame the server sends; text messages up are `hello`, `data`, `mouse` and
    `resize`, each with a `type` tag.
  - xterm.js 6.0.0's API, read off the class page the research note cites in §6.3:
    `new Terminal(options)`, `open(parent)`, `write(data: string | Uint8Array)`, `focus()`,
    `loadAddon(addon)`, `onData: IEvent<string>`, `onBinary: IEvent<string>`,
    `onResize: IEvent<{cols, rows}>`, `cols`, `rows`; and `FitAddon.fit(): void` plus
    `proposeDimensions(): { rows, cols } | undefined` (the published typings, quoted in Task 2.3).
- Produces: `web/terminal/index.html`, a static page that reads the token out of its own query
  parameter. **The server does not inject anything into it** — the token arrives as
  `/terminal?token=…`, the page reads `location.search`, and the same value goes on the WebSocket
  URL, which is the one place a browser navigation cannot send a header (spec §9.2).

**Why the manual check is not a test.** Rendering, `onData`, `onBinary`, `FitAddon` and
`ResizeObserver` exist only in a browser; this harness has none. Everything testable without one is in
the unit tests below — the page's bytes and content type, the token rule, and the fact that every
asset the page references is embedded — and everything else is the numbered manual check in Step 4,
which is also what the README gets.

- [ ] **Step 1: Write the failing test**

Append to `crates/tipsy-compadre/src/server/conn.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The page is served from the same bundle the assets are, so this is the name it must have.
    #[test]
    fn the_terminal_page_is_the_one_the_bundle_embeds_and_it_is_html() {
        let Some((bytes, content_type)) = crate::server::assets::asset("terminal/index.html") else {
            // The bundle is optional by design (`rust-embed`'s `#[allow_missing]`), and a test that
            // demands it would make a plain `cargo test` a lie on a checkout that has not run
            // `cargo xtask dist`. The page's own assertions are skipped, and the fallback page's
            // behaviour is Task 1.5's test.
            eprintln!("terminal/index.html is not embedded in this build; skipping the page assertions");
            return;
        };
        let page = String::from_utf8(bytes).expect("the page is UTF-8");
        assert!(
            content_type.starts_with("text/html"),
            "a page must be served as HTML, got {content_type}"
        );

        // Every behaviour the page owes, named once here so a rewrite that drops one fails loudly.
        for required in [
            // The token comes from the query parameter the server sent the browser to.
            "new URLSearchParams(location.search).get(\"token\")",
            // xterm.js and its stylesheet are separate files; both must be loaded.
            "/assets/vendor/xterm.mjs",
            "/assets/vendor/xterm.css",
            "/assets/vendor/addon-fit.mjs",
            // Binary frames arrive as Blob unless this is set (research note §6.4).
            "binaryType = \"arraybuffer\"",
            // Raw PTY bytes are UTF-8 and there is no JSON envelope over this socket.
            "term.write(new Uint8Array(",
            // The raw-bytes socket, not the protocol one.
            "/terminal/ws",
            // Mouse reports are the non-UTF-8 channel and must be forwarded as bytes.
            "term.onBinary",
            // Keystrokes.
            "term.onData",
            // A full-screen TUI needs the keyboard, and a canvas without a focusable element
            // silently swallows every key.
            "term.focus()",
            "tabindex=\"0\"",
        ] {
            assert!(page.contains(required), "the terminal page must contain {required:?}");
        }
    }

    /// Every asset the page references has to resolve inside the bundle: a page that points at a file
    /// the fetch never kept is a blank screen with a 404 in the console. The gate is on the page
    /// itself and it happens once — a checkout that has not run `cargo xtask dist` skips, and a
    /// checkout that has a page but not its assets **fails**, which is the case that matters.
    #[test]
    fn every_asset_the_page_references_is_embedded() {
        let Some((bytes, _)) = crate::server::assets::asset("terminal/index.html") else {
            eprintln!("no web bundle in this build; run `cargo xtask dist` to check the page's assets");
            return;
        };
        let page = String::from_utf8(bytes).unwrap();
        for path in ["vendor/xterm.mjs", "vendor/xterm.css", "vendor/addon-fit.mjs"] {
            assert!(page.contains(path), "the page must reference {path}");
            assert!(
                crate::server::assets::asset(path).is_some(),
                "the page references {path} and the bundle does not embed it: web/vendor is incomplete"
            );
        }
    }

    /// The two pages share one token rule, and it is the one Task 2.2 already tests; this pins that
    /// the terminal page is on that path and not behind a second, weaker check.
    #[test]
    fn the_terminal_page_and_the_terminal_socket_share_the_token_rule() {
        assert!(terminal_access(None, None).is_ok(), "loopback with no token is open");
        assert!(terminal_access(Some("token=abc"), Some("abc")).is_ok());
        assert!(terminal_access(None, Some("abc")).is_err(), "a tokenised server refuses an anonymous page");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre server::conn
```

Expected: FAIL, with

```
thread 'server::conn::tests::every_asset_the_page_references_is_embedded' panicked at crates\tipsy-compadre\src\server\conn.rs:...
the page must reference vendor/xterm.mjs
```

**This task's checkout has run `cargo xtask dist` in Task 2.3, so `web/vendor/` is there and
`web/terminal/index.html` is not.** The bundle is therefore *not* empty, `asset("terminal/index.html")`
returns `None`, and the failure is the assertion above — the page is not in the embed, so nothing
contains `vendor/xterm.mjs`.

If a checkout genuinely has no `web/vendor/` either, `asset("terminal/index.html")` is `None` and the
test prints `no web bundle in this build; run \`cargo xtask dist\` to check the page's assets` and
passes — which proves nothing. Run `cargo run -p xtask -- dist` first, and only then read the failure.

- [ ] **Step 3: Write the page, and correct the document**

`web/terminal/index.html` — the whole page. It is a module script so the imports are the tarball's
`"module"` builds, and every one of the three URLs is a bundle path Task 2.3 keeps:

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>tipsy-compadre terminal</title>
  <!-- xterm.css is a separate file in @xterm/xterm@6.0.0 ("style":"css/xterm.css"): without it the
       terminal renders with no layout at all. -->
  <link rel="stylesheet" href="/assets/vendor/xterm.css">
  <style>
    html, body { margin: 0; padding: 0; height: 100%; overflow: hidden; background: #101014; }
    /* The TUI needs the whole viewport and the keyboard: `tabindex` is what lets a click land on the
       canvas and every following keystroke reach the terminal instead of the page. */
    #terminal { width: 100vw; height: 100vh; outline: none; }
  </style>
</head>
<body>
  <div id="terminal" tabindex="0"></div>

  <script type="module">
    // The "module" builds of the two packages: "module":"lib/xterm.mjs" and
    // "module":"lib/addon-fit.mjs" in their own registry metadata (Task 2.3).
    import { Terminal } from "/assets/vendor/xterm.mjs";
    import { FitAddon } from "/assets/vendor/addon-fit.mjs";

    // The token travelled to this page as a query parameter, because a browser navigation cannot
    // send an Authorization header (spec §9.2). The server never injects it into the HTML.
    const token = new URLSearchParams(location.search).get("token") ?? "";

    const term = new Terminal({
      convertEol: false,
      cursorBlink: true,
      // The TUI repaints a whole screen at a time; a scrollback of its own history is enough.
      scrollback: 2000,
      fontFamily: "Consolas, \"DejaVu Sans Mono\", monospace",
      fontSize: 15,
      theme: { background: "#101014" },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    // `open`'s own doc requires the container to be visible and sized when it is called, so the
    // stylesheet above sizes it and `fit()` runs immediately after.
    term.open(document.getElementById("terminal"));
    fit.fit();
    // A full-screen TUI is driven by the keyboard, and a canvas that never took focus swallows it.
    term.focus();

    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    const query = token ? `?token=${encodeURIComponent(token)}` : "";
    const ws = new WebSocket(`${scheme}//${location.host}/terminal/ws${query}`);
    // Binary frames arrive as Blob by default (MDN, quoted in research note §6.4), and the PTY's
    // bytes are not what we want to hand a text reader. This one line is the difference.
    ws.binaryType = "arraybuffer";

    ws.onopen = () => {
      // The first frame tells the server this connection is real; a browser cannot ask whether its
      // own socket is open at the other end. The payload is a padding frame and nothing else.
      ws.send("tipsterm-hello-pad-".repeat(16));
    };

    ws.onmessage = (event) => {
      if (typeof event.data === "string") {
        // The only text frame this socket carries is the child's exit notice.
        try {
          const message = JSON.parse(event.data);
          if (message.type === "exit") {
            term.write(`\r\n[the terminal's process ended (code ${message.code})]\r\n`);
          }
        } catch {
          // Not our envelope: ignore it rather than corrupt the screen with it.
        }
        return;
      }
      // Raw PTY bytes. xterm.js treats a Uint8Array as UTF-8 (research note §6.3), which is what a
      // PTY sends, and the escape sequences are for xterm.js's own parser — nothing is stripped.
      term.write(new Uint8Array(event.data));
    };

    ws.onclose = () => {
      term.write("\r\n[disconnected]\r\n");
    };

    // Keystrokes and paste. Escape sequences are ASCII, and a JS string on a text frame preserves
    // them byte for byte.
    term.onData((data) => send({ type: "data", data }));

    // The non-UTF-8 channel: xterm.js's own wording is that `onBinary` "is currently only used for a
    // certain type of mouse reports that happen to be not UTF-8 compatible". This is a latin-1 byte
    // string, one `charCode` per byte, and it goes over a **text** frame as such — the server turns
    // each char back into its low byte, which is the only lossless way through JSON.
    term.onBinary((data) => send({ type: "mouse", data }));

    // `resize`'s own doc: "It's best practice to debounce calls to resize" (research note §6.3).
    let resizeTimer = 0;
    term.onResize(({ cols, rows }) => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => send({ type: "resize", cols, rows }), 100);
    });

    // The window changed size: refit, which resizes the terminal and therefore fires `onResize`
    // above. The `100` ms in that handler is the debounce; repeating `fit()` here is harmless because
    // xterm.js's `resize` is a no-op when the dimensions did not change.
    const observer = new ResizeObserver(() => fit.fit());
    observer.observe(document.getElementById("terminal"));

    // A page with an open WebSocket may not enter the bfcache (research note §7, trap 9), so the
    // socket is closed on the way out; the server's own teardown does the rest.
    addEventListener("beforeunload", () => ws.close());

    function send(message) {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify(message));
      }
    }
  </script>
</body>
</html>
```

`docs/architecture.md:62-65` — replace the sentence that says the child is connected "through the
in-process channel". **This is W10's acknowledged deviation, not an accident** (spec §2 W10, §7.2):
the child is a loopback `--connect` client, which is why it can also be moved to another machine
later. Replace lines 62-65 with:

```markdown
The web terminal is built into the executable: the server serves an embedded xterm.js page and,
per connection, spawns **its own binary in `--connect` mode** in a pseudo-terminal (`portable-pty`,
ConPTY on Windows 10 1809 and later), pointed at the server's own loopback `/ws`. The child is the
running executable's own path (`std::env::current_exe()`), never a name searched for in `PATH`, so a
planted binary in the working directory or in a `PATH` entry cannot become the terminal. It is a
client, exactly like a user on another machine — it starts no server, builds no provider and reads no
credential store — so there is one code path for "a TUI that is not in the server process", which is
why it can also be moved to another machine later. Keyboard, mouse and resize events are forwarded,
and resize reaches the child through the pseudo-terminal itself. Users who prefer an external tool can
also run the binary under `ttyd`.
```

The body of that paragraph is also where the README's manual check must repeat the same rule, because
a person following it by hand is the one most likely to type a bare name. The README section below
therefore says it in its first step.

`README.md` — beside the building section at `README.md:243-250`, add (this is the section the
manual check in Step 4 tells a reader to follow):

```markdown
## How to verify the web terminal

The web terminal runs the real TUI in a browser. There is no browser in this repository's test
harness, so its behaviour is verified by hand, once, with these steps.

1. `cargo run -p xtask -- dist` and `cargo run -- serve` — the server prints where it is listening.
   It spawns its web terminal's child as **its own running binary** (`std::env::current_exe()`), never
   as `tipsy-compadre` looked up in `PATH`: do not start a child by hand for this check, and if you do,
   give it an absolute path. A `PATH` entry that is empty, relative, or points into this checkout could
   otherwise plant the program that runs with your authority.
2. Open `http://127.0.0.1:7070/terminal`. When the configuration holds a token, append
   `?token=<the value the Server tab shows>` to that URL.
3. The HUD appears: the header with the profile's name, the two rings, the oscilloscope, the status
   line and the log panel. A blank black rectangle means `web/vendor/` was not fetched.
4. Click the header's profile title. The configuration screen opens, which proves a click landed where
   the ring widget says it does.
5. Drag the browser window's edge. The TUI reflows to the new size — the panels grow and the header
   stays one line — instead of keeping its old width.
6. `Ctrl+C` at the prompt that started the server stops it; the browser shows `[disconnected]`.
7. After step 6, `tasklist | Select-String tipsy-compadre` lists nothing. If a `tipsy-compadre.exe`
   is still there after the tab is closed, the teardown in `server/conn.rs` is leaking a child.
```

- [ ] **Step 4: Run the test to verify it passes, then do the manual check**

Run:

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-compadre server::conn
```

Expected: PASS — `test result: ok. 3 passed; 0 failed`.

Then the lint gate, against the pinned baseline of `global-constraints.md` (add nothing):

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo clippy -j 1 -p tipsy-compadre --all-targets
```

Then the manual check. **A browser is not available in this harness**, so this is a person at a
keyboard, following the README section written above, in a checkout that has run
`cargo run -p xtask -- dist`. What must be observed, in order:

1. `cargo run -p xtask -- dist`, then `cargo run -- serve` — the server prints its address and does
   not exit. Do not start the terminal's child by hand for this check: the server spawns its **own
   running binary** (`std::env::current_exe()`), never a `tipsy-compadre` looked up in `PATH`, and a
   `PATH` entry that is empty, relative or points into this checkout must never be able to decide
   which program runs. That is Task 2.1's planted-`PATH` test, and this step is the same rule in the
   README so a person following it does not undo it.
2. `http://127.0.0.1:7070/terminal` in a browser: the HUD appears with its header, the two rings and
   the oscilloscope. A blank page means the bundle is not embedded; a black page with no text means
   `web/vendor/` is empty.
3. A click on the header's profile title opens the configuration screen — the proof that a mouse
   position arrives as the cell the TUI thinks it is (the `onBinary` path).
4. Dragging the window's edge reflows the TUI: the panels take the new width and the header stays one
   line. This is the round trip xterm.js → `resize` → `MasterPty::resize` → the child.
5. Closing the tab, then running this, must list no `tipsy-compadre.exe` other than the server itself:

```powershell
Get-Process tipsy-compadre -ErrorAction SilentlyContinue | Select-Object Id, ProcessName, StartTime
```

6. Then stop the server and run the same command: it lists nothing at all.

If any of the six is not what it says, that is a task failure and the ledger records the observation,
not the intention.

- [ ] **Step 5: Commit**

```bash
git add web/terminal/index.html crates/tipsy-compadre/src/server/conn.rs docs/architecture.md README.md
git commit -m "feat(server): serve the terminal page and correct what the architecture claims

an xterm.js page that opens /terminal/ws, writes the raw pty bytes it receives,
forwards keystrokes and the separate non-utf-8 mouse channel, refits and
debounces on a window resize, and takes its token from the query parameter the
server sent the browser to. architecture.md's claim that the pty child is
connected through the in-process channel was wrong and spec W10 records the
deviation: the child is this binary in --connect mode over loopback.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 3.1: The spike — the gate the spec wrote `[mid]`

This task **is** the gate. The v1 spec demands it in §10, verbatim: "**Risk:** Ratzilla is young.
Before the plan is finalised, a spike must confirm Braille canvas rendering, mouse support and ratatui
version compatibility. If it falls short, the web client is postponed and the web terminal remains the
browser option for v1" (`2026-10-10-tipsy-compadre-design.md:389-391`), and §15.1 makes it the first
named risk (`:460`). `docs/architecture.md:88-90` repeats it. The plan was written anyway, at the
user's explicit choice, and the ruling is recorded in the ledger — read it before you start:

> **2026-10-11 — the spec's §8 gate is honoured by making it Task 3.1.** The v1 spec §10 requires a
> spike to confirm Braille rendering, mouse support and ratatui compatibility *before* the plan is
> finalised, and the user chose to write the plan without running it. The spike therefore becomes the
> first task of Phase 3 and its verdict is a go/no-go that the ledger records; the remaining Phase 3
> tasks are written against the research note's requirement-level answers, which are explicitly not
> build-level: `wasm32-unknown-unknown` is not installed on this machine and nothing was compiled.
> — `.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md:45-50`

So this task's deliverable is **a verdict, not code you keep**.

**Files:**
- Create, **outside this repository and never committed** (a scratch directory, as `CLAUDE.md:29-32`
  already places this project's throwaway spikes):
  `E:\workspace\personal\tipsy-spikes\ratzilla\Cargo.toml`,
  `E:\workspace\personal\tipsy-spikes\ratzilla\src\lib.rs`,
  `E:\workspace\personal\tipsy-spikes\ratzilla\index.html`
- Modify: `.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md` — append the verdict block of
  Step 8. The file is git-ignored (`.superpowers/sdd/.gitignore` is a single `*` line), so nothing
  written here can reach a commit.
- Test: **none.** There is no test target in this task. Its evidence is the manual browser check of
  Step 7 transcribed into the ledger — a spike that reports "PASS" with no recorded observation has
  not run, and rule 10 of this plan is that a check needing a browser is a documented manual check,
  never a test that pretends to drive one.

**Interfaces:**
- Consumes (all read from the tree for this task; line numbers are the working tree's):
  - `pub fn draw(frame: &mut Frame, state: &UiState, clock: &ClockView, title: &str) -> Vec<Hit>` —
    `crates/tipsy-ui/src/render.rs:65`; `ClockView` `:43-48`; `Hit { area, key, action }` `:50-63`;
    `RING_VIEW = 50.0` `:41`.
  - What `draw_rings` actually uses — this is question 1's vocabulary, read at `render.rs:516-548`:
    `Canvas::default().marker(Marker::Braille)` (`:525-527`), `Circle` (`:531`, `:539-544`), `Points`
    through the `arc()` helper (`:664-672`, 61 samples per arc), the three `Color::Rgb` constants
    (`:27-29`), and one `ctx.print(-2.0, 2.0, Span::from("◉").fg(Color::White).bold())` (`:545`).
  - `draw_voice` — a second `Marker::Braille` canvas drawing a `CLine` polyline (`:552-587`), and
    `draw_net` — a `Chart` with two `Marker::Braille` datasets, the `IN`/`OUT` legend and two axes
    (`:459-500`).
  - **The header, and the project's own width workaround.** `drawn_width` (`render.rs:229-244`) measures
    a span by rendering it plus `MARKER = "\u{E000}"` (`:216`) into a scratch `Buffer` and reading the
    column the marker lands in. Its comment names the reason (`:221-224`): "A span is laid out by one
    width table and drawn by another, and the two disagree on `◉`, `⚙` and `·` — each one column to
    [`Span::width`] and two as drawn." All three are on screen in the HUD: `◉` in the badge
    (`:167`), `⚙` as the gear (`:174`), `·` as the help row's separator (`crates/tipsy-ui/src/strings.rs:90`
    Spanish, `:154` English). `drawn_width` makes `tipsy-ui` agree with ratatui's own renderer — it
    says nothing about what the *backend* paints, and that gap is exactly what this spike measures.
  - `pub enum KeyInput { Char(char), Enter, Backspace, Tab, Esc, Up, Down, Left, Right, F(u8) }` —
    `crates/tipsy-ui/src/state.rs:70-81`; `pub enum UiAction { Send(Command), Quit, Replay(usize),
    OpenConfig, Config(ConfigAction) }` — `:84-98`; `pub fn on_hit(&mut self, hit: &Hit) -> Vec<UiAction>`
    — `:202-234`; `pub fn on_key(&mut self, key: KeyInput) -> Vec<UiAction>` — `:389-464`;
    `pub fn tick(&mut self, dt_ms: u64)` — `:491-501`.
  - The native mouse path this must reproduce: `client.rs:148-149`
    (`TermEvent::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_))` →
    `hit_at(&hits, mouse.column, mouse.row)`) and `hit_at` (`client.rs:676-680`, rectangle containment
    over the `Vec<Hit>`); and the native key folding (`client.rs:700-703`), where a `Ctrl`+letter
    arrives as the control character `(lower & 0x1f)`, which is the only `char` class the form's
    `SAVE_KEY`/`REGENERATE_KEY` use (`config_form.rs:18-19`).
  - Ratzilla, **read directly from docs.rs for this task and not recalled**: `ratzilla 0.3.1`,
    `WebRenderer::{draw_web, on_key_event, on_mouse_event}` with
    `fn draw_web<F>(self, render_callback: F) where F: FnMut(&mut Frame<'_>) + 'static` and
    `fn on_key_event<F>(&mut self, callback: F) -> Result<(), Error> where F: FnMut(KeyEvent) + 'static`
    (<https://docs.rs/ratzilla/0.3.1/ratzilla/trait.WebRenderer.html>); `event.rs`'s
    `KeyEvent { code: KeyCode, ctrl, alt, shift }` and `MouseEvent { kind, col, row, ctrl, alt, shift }`
    with `MouseEventKind::{Moved, ButtonDown(MouseButton), ButtonUp, SingleClick, DoubleClick, Entered,
    Exited, Unidentified}` and `MouseButton::{Left, Right, Middle, Back, Forward, Unidentified}`, all
    `Debug + Clone + Eq`
    (<https://docs.rs/ratzilla/0.3.1/src/ratzilla/event.rs.html>, lines 2, 19, 131, 148);
    `DomBackend::new() -> Result<Self, ratzilla::error::Error>`
    (<https://docs.rs/ratzilla/0.3.1/ratzilla/backend/dom/struct.DomBackend.html>, source
    `backend/dom.rs.html#125-127`), and `ratzilla::error::Error` implementing `Display`
    (<https://docs.rs/ratzilla/0.3.1/ratzilla/error/enum.Error.html>). The three backends, their
    table and the open issues are the research note's `ratzilla-and-wasm-audio.md` §2.
  - The exact wasm toolchain route and the toolchain state: `ratzilla-and-wasm-audio.md` §3
    (wasm-bindgen-cli 0.2.129, why `trunk` is not driven from an xtask, and §3.0's finding that
    **`rustup target list --installed` prints only `x86_64-pc-windows-msvc`** — re-confirmed on this
    machine for this task).
- Produces (what Tasks 3.2 – 3.6 consume):
  - **A recorded PASS/FAIL verdict** in `progress.md`, in the exact form of Step 8, naming the
    backend, the Ratzilla version, and the open-issue numbers that remain a risk.
  - `wasm32-unknown-unknown` installed for the pinned 1.96.1 toolchain, and `wasm-bindgen-cli` 0.2.129
    on `PATH`. Both are prerequisites of 3.2's wasm compile and 3.6's `cargo xtask dist` (§8.3).
  - The build route 3.2 and 3.6 copy verbatim:
    `cargo build -j 1 --target wasm32-unknown-unknown -p <crate> --release` then
    `wasm-bindgen --target web --out-dir <dir> <crate>.wasm`.
  - The page skeleton (`index.html`) 3.2 starts its own page from, and the fact — to be confirmed or
    refuted here — that `python -m http.server` serves the output as `application/wasm`.

**Why the scratch crate is outside the repository.** The repository's rule is that spikes are
throwaway and live outside the tree (`CLAUDE.md:29-32`), and there is a second reason here: the plan's
global constraints forbid committing anything this task produces (`global-constraints.md:42-48`), and
a crate inside `crates/` would have to join `Cargo.toml:3`'s `members` list and would then be built by
every `cargo test --workspace` in a checkout that already carries two other plans' uncommitted work.
The scratch crate therefore declares its own `[workspace]` and depends on this workspace **by path**,
so it renders the real `tipsy_ui::draw` without touching the workspace's member list.

- [ ] **Step 1: Install the wasm target, and the `wasm-bindgen` CLI the whole phase needs**

The recon verified only `x86_64-pc-windows-msvc` is installed (`codebase-recon.md:911-922`; the note's
§3.0 says the same), and this task re-confirmed it. Install the target for the **pinned** toolchain:

```powershell
rustup target add wasm32-unknown-unknown
rustup target list --installed
```

Expected: `x86_64-pc-windows-msvc` and `wasm32-unknown-unknown`, two lines. The toolchain is pinned by
`rust-toolchain.toml:1-3` to `1.96.1`, and `rustup` installs the target for the pinned toolchain when
run inside the repository — run it from `E:\workspace\personal\tipsy-compadre`.

Then the CLI. `cargo install` is the documented install
(<https://wasm-bindgen.github.io/wasm-bindgen/reference/cli.html>), and the version must equal the
`wasm-bindgen` **crate** version resolved for the spike, because the CLI enforces a schema match
(`ratzilla-and-wasm-audio.md` §3.3). Run it from the scratch directory — outside the repository, so
the repository's `.cargo/config.toml:5` (`target-dir = "G:/cargo-targets/tipsy-compadre/target"`)
cannot apply — with an explicit `--target-dir` and `-j 1`:

```powershell
New-Item -ItemType Directory -Force -Path 'E:\workspace\personal\tipsy-spikes\ratzilla' | Out-Null
Set-Location 'E:\workspace\personal\tipsy-spikes\ratzilla'
cargo install -f wasm-bindgen-cli --version 0.2.129 `
  --target-dir 'E:\workspace\personal\tipsy-spikes\ratzilla\.target-install' -j 1
wasm-bindgen --version
```

Expected: `wasm-bindgen 0.2.129`. `E:` has ~6.7 GB free on this machine and the workspace's own
sandbox already lives there; if either command reports no space, stop and report rather than
redirecting to `G:` (the plan's constraint) — the spike's own `CARGO_TARGET_DIR` is the scratch
crate's, not `G:`.

- [ ] **Step 2: Create the throwaway crate's manifest**

```toml
# E:\workspace\personal\tipsy-spikes\ratzilla\Cargo.toml
[package]
name = "ratzilla-spike"
version = "0.0.0"
edition = "2024"
publish = false

# Its own workspace root: a throwaway crate outside the repository, never a member of it
# (CLAUDE.md:29-32, global-constraints.md:42-48).
[workspace]

[lib]
crate-type = ["cdylib"]

[dependencies]
# The pure half of the HUD, by path, so the spike draws the real thing and nothing else. Cargo
# resolves tipsy-ui's `workspace = true` dependencies through its own workspace root
# (crates/tipsy-ui/Cargo.toml:3-5) — no copy of tipsy-ui is made, because a copy would invalidate
# question 4.
tipsy-ui = { path = "E:/workspace/personal/tipsy-compadre/crates/tipsy-ui" }
# Pinned exactly, per the note's own recommendation: `ratzilla = "=0.3.1"` keeps Cargo from ever
# resolving a version whose ratatui requirement has moved off 0.30 (ratzilla-and-wasm-audio.md:359).
ratzilla = "=0.3.1"
wasm-bindgen = "0.2.129"
js-sys = "0.3.106"
# `console` is the only feature this page needs: every observation is logged, and the `window` /
# `document` accessors the ledger points at are available without a feature of their own.
[dependencies.web-sys]
version = "0.3.106"
features = ["console"]

[features]
default = ["backend-webgl2"]
backend-dom = []
backend-canvas = []
backend-webgl2 = []

[profile.release]
opt-level = "s"
```

Two facts about those versions, both read off the local cargo registry cache for this task rather than
recalled: `web-sys 0.3.106` requires `wasm-bindgen = "=0.2.129"` and `js-sys = "=0.3.106"` exactly
(`web-sys-0.3.106/Cargo.toml`, `[dependencies.js-sys]`, `[dependencies.wasm-bindgen]`), so the three
numbers cannot drift apart; and `ratzilla 0.3.1` requires `ratatui ^0.30.1` against this workspace's
`ratatui = "0.30"` (`Cargo.toml:14`), so Cargo resolves one copy — which is what question 4's compile
proves.

- [ ] **Step 3: Write the spike's `src/lib.rs`**

The file is a single `#[wasm_bindgen(start)]` entry. It chooses the backend by feature, holds the
shared state and the last `Vec<Hit>`, draws the **whole HUD** (the header comes with `draw`, which is
what makes the `◉`/`⚙`/`·` check part of question 1), and logs every observation to the browser
console. Logging — not an on-screen debug line — is deliberate: it keeps the HUD's own cells clean so
the visual observation is of the real thing.

```rust
//! Throwaway Ratzilla spike. Nothing here is kept: the verdict it produces is recorded in
//! `.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md`, and this crate is never committed.
//!
//! It answers four questions against the *installed* crate, not against the research note:
//!   1. does a ratatui `Canvas` with the `Braille` marker render at all, `◉` included;
//!   2. do mouse events arrive, and do they translate to `KeyInput`/`UiAction` without loss;
//!   3. which backend should the client use;
//!   4. does `tipsy_ui::draw` render through `draw_web` unchanged.

use std::cell::RefCell;
use std::rc::Rc;

use ratzilla::WebRenderer;
use ratzilla::event::MouseEventKind;
use ratzilla::ratatui::Terminal;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

// One backend per build. DOM wins over canvas, canvas over WebGL2, so `--no-default-features
// --features backend-dom` is the only way to get the DOM backend.
#[cfg(feature = "backend-dom")]
type Chosen = ratzilla::DomBackend;
#[cfg(all(not(feature = "backend-dom"), feature = "backend-canvas"))]
type Chosen = ratzilla::CanvasBackend;
#[cfg(all(not(feature = "backend-dom"), not(feature = "backend-canvas")))]
type Chosen = ratzilla::WebGl2Backend;

#[cfg(feature = "backend-dom")]
const BACKEND: &str = "DomBackend";
#[cfg(all(not(feature = "backend-dom"), feature = "backend-canvas"))]
const BACKEND: &str = "CanvasBackend";
#[cfg(all(not(feature = "backend-dom"), not(feature = "backend-canvas")))]
const BACKEND: &str = "WebGl2Backend";

/// The spike's own copy of `client.rs:676-680`. It is copied and not shared because the point of
/// question 2 is whether these five lines port unchanged.
fn hit_at(hits: &[tipsy_ui::Hit], column: u16, row: u16) -> Option<tipsy_ui::Hit> {
    hits.iter().copied().find(|hit| {
        column >= hit.area.x && column < hit.area.right() && row >= hit.area.y && row < hit.area.bottom()
    })
}

fn log(text: &str) {
    web_sys::console::log_1(&JsValue::from_str(&format!("spike: {text}")));
}

/// The HUD's starting state: enough of it filled in that every panel the spike has to look at draws
/// something. `draw_rings` animates off `elapsed_ms`, which `tick` advances once per frame.
fn hud() -> tipsy_ui::UiState {
    let mut state = tipsy_ui::UiState::new();
    state.profile = Some(tipsy_protocol::ProfileView {
        id: "compae".into(),
        name: "Compae".into(),
        language: tipsy_protocol::UiLanguage::Es,
    });
    state.mic_open = true; // the red envelope, so `draw_voice`'s microphone branch is exercised
    state.mic_levels = (0..200).map(|i| ((i as f32) * 0.31).sin().abs() * 0.8).collect();
    state.voice_envelope = (0..200).map(|i| ((i as f32) * 0.17).sin().abs() * 0.5).collect();
    state.net_history = (0..60).map(|i| ((i as f64) * 0.2).sin() * 40_000.0 + 60_000.0)
        .map(|v| (v, v * 0.6)).collect();
    state.cpu = 12.0;
    state.ram = 41.0;
    state.net_in = 82_000.0;
    state.net_out = 31_000.0;
    state.log.push(tipsy_ui::LogEntry {
        who: tipsy_ui::Who::User,
        text: "¿y eso?".into(),
        first_audio_ms: None,
        error: None,
        pending: false,
    });
    state
}

/// The reference layout, computed in the browser by the same ratatui code the native HUD uses: one
/// `TestBackend` row per HUD row, at the width the page draws. The header row it prints is what
/// `drawn_width` (`render.rs:229-244`) claims the drawn columns are.
fn reference_rows(state: &tipsy_ui::UiState, width: u16, height: u16) -> Vec<String> {
    use ratzilla::ratatui::Terminal;
    use ratzilla::ratatui::backend::TestBackend;
    let clock = tipsy_ui::ClockView { time: "12:34:56".into(), weekday: 2, day: 11, month: 9 };
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
    terminal
        .draw(|frame| {
            let _ = tipsy_ui::draw(frame, state, &clock, "Compae");
        })
        .expect("a test draw");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[wasm_bindgen(start)]
pub fn main() -> Result<(), JsValue> {
    // `DomBackend::new() -> Result<Self, ratzilla::error::Error>` was read on docs.rs
    // (backend/dom.rs.html#125-127). The other two constructors were not read; if one of them has
    // another shape, the compiler says so here and the fix is this one line.
    let backend = Chosen::new().map_err(|e| JsValue::from_str(&e.to_string()))?;
    let mut terminal = Terminal::new(backend).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let state = Rc::new(RefCell::new(hud()));
    let hits: Rc<RefCell<Vec<tipsy_ui::Hit>>> = Rc::new(RefCell::new(Vec::new()));
    let last = Rc::new(RefCell::new(js_sys::Date::now()));
    let timing = Rc::new(RefCell::new(Timing::default()));

    log(&format!("backend={BACKEND} ratzilla={}", env!("CARGO_PKG_VERSION")));
    {
        let rows = reference_rows(&state.borrow(), 100, 30);
        log(&format!("reference header row (100 cols): {:?}", rows.first().unwrap_or(&String::new())));
        if let Some(row) = rows.first() {
            for glyph in ['◉', '⚙', '·'] {
                let cols: Vec<usize> = row.char_indices()
                    .filter(|(_, c)| *c == glyph)
                    .map(|(i, _)| row[..i].chars().count())
                    .collect();
                log(&format!("reference {glyph:?} at column(s) {cols:?}"));
            }
        }
    }

    terminal
        .on_key_event({
            let state = state.clone();
            let last = last.clone();
            move |key| {
                let line = format!("key {key:?}");
                // The translation the browser client will need, checked here against the real
                // event: Ctrl+letter becomes the control character the form's SAVE_KEY and
                // REGENERATE_KEY expect (`client.rs:700-703`, `config_form.rs:18-19`).
                let input = match key.code {
                    ratzilla::event::KeyCode::Char(c) if key.ctrl => {
                        let lower = c.to_ascii_lowercase();
                        lower.is_ascii_lowercase()
                            .then_some(tipsy_ui::KeyInput::Char((lower as u8 & 0x1f) as char))
                    }
                    ratzilla::event::KeyCode::Char(c) => Some(tipsy_ui::KeyInput::Char(c)),
                    ratzilla::event::KeyCode::Enter => Some(tipsy_ui::KeyInput::Enter),
                    ratzilla::event::KeyCode::Backspace => Some(tipsy_ui::KeyInput::Backspace),
                    ratzilla::event::KeyCode::Tab => Some(tipsy_ui::KeyInput::Tab),
                    ratzilla::event::KeyCode::Esc => Some(tipsy_ui::KeyInput::Esc),
                    ratzilla::event::KeyCode::Up => Some(tipsy_ui::KeyInput::Up),
                    ratzilla::event::KeyCode::Down => Some(tipsy_ui::KeyInput::Down),
                    ratzilla::event::KeyCode::Left => Some(tipsy_ui::KeyInput::Left),
                    ratzilla::event::KeyCode::Right => Some(tipsy_ui::KeyInput::Right),
                    ratzilla::event::KeyCode::F(n) => Some(tipsy_ui::KeyInput::F(n)),
                    _ => None,
                };
                let actions = input.map(|input| state.borrow_mut().on_key(input)).unwrap_or_default();
                log(&format!("{line} key_input={input:?} actions={actions:?}"));
            }
        })
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    terminal
        .on_mouse_event({
            let state = state.clone();
            let hits = hits.clone();
            move |event| {
                if let MouseEventKind::ButtonDown(button) = event.kind {
                    let hit = hit_at(&hits.borrow(), event.col, event.row);
                    let actions = match &hit {
                        Some(hit) => state.borrow_mut().on_hit(hit),
                        None => Vec::new(),
                    };
                    log(&format!(
                        "mouse col={} row={} button={button:?} hit={hit:?} actions={actions:?}",
                        event.col, event.row
                    ));
                } else {
                    log(&format!("mouse col={} row={} kind={:?}", event.col, event.row, event.kind));
                }
            }
        })
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    // The render loop: no async runtime, `draw_web` wraps `requestAnimationFrame` itself
    // (ratzilla-and-wasm-audio.md §6.4 point 5).
    terminal.draw_web({
        let state = state.clone();
        let hits = hits.clone();
        let last = last.clone();
        let timing = timing.clone();
        move |frame| {
            let now = js_sys::Date::now();
            let dt = (now - last.replace(now)).max(0.0) as u64;
            let (title, hits_now) = {
                let mut state = state.borrow_mut();
                state.tick(dt);
                let title = state.profile.as_ref().map(|p| p.name.clone()).unwrap_or_default();
                let clock = tipsy_ui::ClockView { time: "12:34:56".into(), weekday: 2, day: 11, month: 9 };
                let hits_now = tipsy_ui::draw(frame, &state, &clock, &title);
                (title, hits_now)
            };
            let _ = title;
            *hits.borrow_mut() = hits_now;
            if let Some(line) = timing.borrow_mut().sample(dt) {
                log(&line);
            }
        }
    });
    Ok(())
}

/// Frame-time sampling for question 3: the average and worst frame over a window, logged once per
/// window. The backend table's claim is about frame time, so the verdict quotes a number.
#[derive(Default)]
struct Timing {
    frames: u32,
    total_ms: f64,
    worst_ms: f64,
}

impl Timing {
    fn sample(&mut self, dt_ms: u64) -> Option<String> {
        self.frames += 1;
        self.total_ms += dt_ms as f64;
        self.worst_ms = self.worst_ms.max(dt_ms as f64);
        if self.frames < 120 {
            return None;
        }
        let line = format!(
            "frames={} avg_ms={:.1} worst_ms={:.1}",
            self.frames,
            self.total_ms / self.frames as f64,
            self.worst_ms
        );
        self.frames = 0;
        self.total_ms = 0.0;
        self.worst_ms = 0.0;
        Some(line)
    }
}
```

Two things to check while writing this, because they are the spike's own discoveries and not
recalled facts: `LogEntry`'s and `Who`'s fields as `tipsy-ui` exports them (`state.rs:31-44`), and
`ratzilla::ratatui::backend::TestBackend` being reachable with no backend feature — `tipsy-ui`'s own
tests use `ratatui::backend::TestBackend` with `default-features = false`
(`crates/tipsy-ui/tests/render.rs:1`, `Cargo.toml:14`), so it should be. `env!("CARGO_PKG_VERSION")`
prints the spike's `0.0.0`; replace that argument with a logged literal `ratzilla 0.3.1` — the version
is the pinned dependency line, and `cargo tree -p ratzilla` in Step 6 is what proves it.

- [ ] **Step 3b: log `hit_at`'s table, not just the found hit** — if Step 7.2's clicks do not land,
the ledger needs to know whether the `Hit` rectangles were built at all. Add to the mouse branch:

```rust
    log(&format!("mouse col={} row={} hits={:?}", event.col, event.row, hits.borrow()));
```

- [ ] **Step 4: Write the page**

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>Ratzilla spike</title>
    <style>
      /* The font stack is the machine's own: the project ships no webfont. Which of these actually
         renders U+28xx (Braille) is one of the things Step 7 records — for the DOM and canvas
         backends the page font decides the cell's advance, and for all three it decides the cell's
         size, which is what issue #188's panic is about. */
      html, body { margin: 0; height: 100%; background: #001014; }
      pre, canvas, .terminal {
        font-family: Consolas, "Cascadia Mono", "Segoe UI Symbol", monospace;
        font-size: 16px;
        line-height: 1.1;
      }
    </style>
  </head>
  <body>
    <script type="module">
      import init from "./ratzilla_spike.js";
      init()
        .then(() => console.log("spike: init() resolved"))
        .catch((e) => console.error("spike: init() failed", e));
    </script>
  </body>
</html>
```

`wasm-bindgen --target web` generates the JS module and names it after the crate's lib target, so the
import path is `./ratzilla_spike.js` beside the page. The empty `<body>` is deliberate — Ratzilla's
README page carries no container for the Rust side (`README`, quoted in `ratzilla-and-wasm-audio.md`
§1), and whether a backend creates its own container or needs one is something Step 7 records if it
turns out to matter; do not invent a mounting API for it.

- [ ] **Step 5: Build it for wasm, once per backend**

`CARGO_TARGET_DIR` stays off `G:` and `-j 1` is mandatory (`global-constraints.md:9-11`). Each backend
is a separate build, and the DOM one is the one most likely to fail first — that is the point of
running it.

```powershell
Set-Location 'E:\workspace\personal\tipsy-spikes\ratzilla'
$env:CARGO_TARGET_DIR='E:\workspace\personal\tipsy-spikes\ratzilla\.target'

# WebGL2 first: the note's recommendation, and the one this plan expects to pick.
cargo build -j 1 --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir .\site-webgl2 `
  .\.target\wasm32-unknown-unknown\release\ratzilla_spike.wasm

# Then the canvas fallback, then the DOM backend with its four open layout bugs.
cargo build -j 1 --target wasm32-unknown-unknown --release --no-default-features --features backend-canvas
wasm-bindgen --target web --out-dir .\site-canvas `
  .\.target\wasm32-unknown-unknown\release\ratzilla_spike.wasm

cargo build -j 1 --target wasm32-unknown-unknown --release --no-default-features --features backend-dom
wasm-bindgen --target web --out-dir .\site-dom `
  .\.target\wasm32-unknown-unknown\release\ratzilla_spike.wasm

foreach ($d in 'site-webgl2', 'site-canvas', 'site-dom') {
  Copy-Item .\index.html ".\$d\index.html"
  Get-ChildItem ".\$d" | Select-Object Name, Length
}
```

Question 4's compile-level evidence is one command. **Exactly one `ratatui` line** means
`tipsy_ui::draw` and `draw_web` are the same `Frame` type:

```powershell
cargo tree -p ratatui
cargo tree -d
```

Expected: one `ratatui v0.30.2` line under the spike, and no `ratatui` in the duplicate report. If
`cargo tree -d` lists two `ratatui` versions, question 4 has failed for a reason the note predicted
(`ratzilla-and-wasm-audio.md` §2c INFERRED risk 1): stop, record it, and treat it as a FAIL.

- [ ] **Step 6: Serve it**

`python -m http.server` is the serving route for the whole phase. Python 3.12.10 is on this machine
and its `mimetypes` maps `.wasm` to `application/wasm` (checked for this task:
`python -c "import mimetypes; print(mimetypes.guess_type('a.wasm'))"` → `('application/wasm', None)`),
which is the MIME type `WebAssembly.instantiateStreaming` wants. `trunk` is the note's suggestion for a
throwaway spike, and this task does **not** take it: it is not installed here, its newest release is a
pre-release, it has no library API, and it fetches `wasm-bindgen`/`wasm-opt` for itself
(`ratzilla-and-wasm-audio.md` §3.2, §3.4).

```powershell
# One server per backend, on its own port, from the directory that holds the page and the glue.
python -m http.server 8001 --directory 'E:\workspace\personal\tipsy-spikes\ratzilla\site-webgl2'
python -m http.server 8002 --directory 'E:\workspace\personal\tipsy-spikes\ratzilla\site-canvas'
python -m http.server 8003 --directory 'E:\workspace\personal\tipsy-spikes\ratzilla\site-dom'
```

Each is a managed background job while the check runs; kill them when it is over.

- [ ] **Step 7: The manual check — four questions, each with the observation that answers it**

There is no browser in this harness: the human runs these steps and transcribes the observations into
the ledger. Open Chrome or Edge on `http://127.0.0.1:8001/`, press `F12`, keep the Console visible,
and reload the page. Every observation below is read from that console plus the page.

**7.1 — Question 1: does a Braille `Canvas` render through Ratzilla at all, `◉` included?**

Click through the HUD's panels first (`F8` shows the `NETWORK`/`RED` tab's Braille chart, the left
panel's `CHAT`/`RED` row switches it back), then record:

| # | Look at | PASS is | Record instead |
|---|---|---|---|
| a | `CORE` panel | A dim outer circle, three rotating arcs (cyan, dim, gold), a white pulsing centre circle, and `◉` inside the innermost ring; circles round, nothing clipped by the border | blank panel, cells drawn as empty boxes, a cropped ring, `panicked at` in the console |
| b | `VOZ` panel | A cyan waving line, red while `mic_open` | nothing, or the panel's border only |
| c | `NETWORK` chart | Two Braille curves, the `IN`/`OUT` legend, the two axis labels | blank chart, a legend with no curves |
| d | any Braille cell | Dots, one cell each | two-column-wide blobs, or blank cells |
| e | the header | `◉ COMPAE` badge, the mode word, the gear `⚙` flush to the right edge; the bottom help row's `·` separators between clauses. Compare with the console's `reference ◉ at column(s) …` / `reference ⚙ …` / `reference · …` lines, which is what `drawn_width` (`render.rs:229-244`) measured inside ratatui's own buffer | the gear not flush right, the mode word gone, a row whose tail has shifted by one column after one of the three glyphs |
| f | window resize (`Win`+`←`, then drag the edge back) | the HUD re-lays out, panels still filled, console free of `panicked at` | a blank screen after a resize (issue #190's class), a panic |
| g | page zoom to ~150 % (`Ctrl`+`+` twice) | same three observations as (a)–(e) | a panic while a cell is larger than 10×20 px (issue #188's class, reproduced on `DomBackend` whenever the cell is bigger than 10×20 px, e.g. a 24 px font) |

Then, for **each** of `◉`, `⚙`, `·`, record one line: `aligned` / `two columns` / `one short` /
`missing`, plus which font actually painted the cell — DevTools → Elements → the grid's cell element →
Computed → "Rendered Fonts" (Firefox: Inspector → Fonts). For `WebGl2Backend` the glyphs come from
beamterm's own atlas rather than the page font (its default font "now contains the full braille set",
`ratzilla-and-wasm-audio.md` §2a), so record "backend atlas" and whether the glyph is *present* at all.

**What makes this question FAIL, and what does not.** FAIL: a panel that does not draw, draws blank,
is cropped at its border, panics, or goes blank after a resize — the canvas does not render, which is
the question the v1 spec §10 asks. A glyph that advances by the wrong number of columns is recorded as
defect **D1** with the remedies that are inside this project's control (the page's `font-family`, the
cell's CSS size, or a one-character swap at `render.rs:545` / `:167` / `:174` / `strings.rs:90`, `:154`)
and does **not** by itself stop Phase 3 — because `drawn_width` is a workaround `tipsy-ui` already
owns. It *does* turn into a FAIL one step later if the drawn glyph is no longer where its `Hit` says it
is: that is question 2's loss, and 7.2.1 decides it.

**7.2 — Question 2: do mouse events arrive, in what shape, and do they translate without loss?**

Click, in this order, with the console visible. The project's rule is that every action has both a key
and a click (v1 spec §9 `:373-374`, and `Hit`'s own comment at `render.rs:50-63`), so each click has a
keystroke it must match exactly:

| # | Click | Expected console line (the parts that matter) | Expected screen |
|---|---|---|---|
| 1 | the header's mode word / badge area (right end) | `mouse col=… row=0 button=Left hit=Some(Hit { … key: F(9), action: Some(Toggle) }) actions=[OpenConfig]` | the configuration screen opens, exactly as `F9` opens it |
| 2 | the gear `⚙` | the same `actions=[OpenConfig]` when closed, `actions=[]` when open | opens / closes, exactly as `F9` |
| 3 | the `CHAT`/`RED` row in the left panel | `actions=[…]` for `F6`'s own action (the tab switches; a tab switch may legitimately report `actions=[]` — the form does the work, not the client) | the tab switches |
| 4 | `✗ Borrar` / `✗ Clear` | `actions=[Send(Reset)]` | the log clears, the `cleared` notice appears |
| 5 | with the screen open (`F9`): each of the four tabs, then each row, then `Probar`/`Probar`, `Guardar`, `Regenerar token` | a tab arrives as `SelectTab(…)`, a row as `FocusRow(n)`, Save as `Config(Save)`, Test as `Config(Test)`, Regenerate as `Config(RegenerateToken)` (`config_form.rs:185-213`, `state.rs:210-233`) | the tab switches, the row focuses, Save/Test/Regenerate do their thing |
| 6 | click empty space inside the log | `hit=None actions=[]` | nothing happens |

Then the keyboard half, in the same session: type a letter, `Enter`, `Backspace`, `Tab`, `Esc`, `F2`,
`F3`, `F4`, `F6`, `F7`, `F8`, `F9`, all four arrows, and `Ctrl+R` / `Ctrl+S`. Every one logs
`key KeyEvent { … } key_input=Some(…) actions=[…]`, and the HUD must react as the native terminal
does. **Record what `Ctrl+R` and `Ctrl+S` actually do**: they are the browser's Reload and Save-page
shortcuts, and `SAVE_KEY`/`REGENERATE_KEY` are exactly those control characters
(`config_form.rs:18-19`). The HUD survives it — `F4` also saves (`state.rs:213`) and `Enter` on the
Regenerate row also regenerates (`config_form.rs:659-662`) — but Task 3.5's translation has to know.

**Table 7.2.1 is where the two questions meet, and it is not optional.** After the resize and the zoom
of 7.1(f)/(g), click the gear and the mode word again. If the drawn glyph has moved but `Hit.area` has
not, the click lands on nothing (or on the wrong control) and **question 2 FAILS**: the browser's cell
metrics disagree with the `Hit` rectangles that `tipsy_ui::draw` returned, which is precisely "cannot
be translated to this project's `KeyInput`/`UiAction` without loss".

**7.3 — Question 3: which backend?**

Run the same script on 8002 (canvas) and 8003 (DOM), and record for each:

- the `frames=… avg_ms=… worst_ms=…` line every 120 frames, at the same window size as the WebGL2 run
  and with the rings animating;
- whether step 6's key events actually fired, or whether `on_key_event` returned `Ok(())` and nothing
  arrived. This is the note's worst failure mode for a keyboard-first TUI: on `WebGl2Backend` "Keyboard
  events are supported by making the canvas focusable with `tabindex=\"0\"`" and some backends "will
  silently succeed without registering any handlers" (`ratzilla-and-wasm-audio.md` §2b,
  [WebEventHandler](https://docs.rs/ratzilla/0.3.1/ratzilla/trait.WebEventHandler.html)). Click the
  canvas once before typing, then say in the ledger whether clicking was what made the keys arrive;
- the DOM backend's own bugs, as reproduced or not: a panic at a cell larger than 10×20 px (#188), a
  blank screen after a resize that keeps the grid size (#190), a wrong layout on first load (#186), an
  under-filled grid / `Backend::size()` off by one (#179).

**The plan's choice, for the record, is `WebGl2Backend`**: only it is described as reaching 60 fps on a
large terminal, and it is the one whose default font carries the full Braille set
(`ratzilla-and-wasm-audio.md` §2a, §0). `CanvasBackend` is the documented fallback "if WebGL2 isn't
available" (docs.rs backend module, quoted in the same note) and `DomBackend` is the one carrying all
four open layout bugs. The spike confirms or overturns that with the numbers above; if it overturns it,
the verdict names the backend it chose and why, and Tasks 3.2 – 3.6 use that type.

**7.4 — Question 4: does `tipsy_ui::draw` render through `draw_web` unchanged?**

The evidence is already collected: `cargo tree -p ratatui` prints one line (Step 5), the spike's
`lib.rs` calls `tipsy_ui::draw(frame, &state, &clock, &title)` with the `&mut Frame` that `draw_web`
handed over, with no wrapper, no copy of `tipsy-ui` and no `cfg(target_arch)` gate anywhere in
`tipsy-ui` — the recon's §8.1 recorded that the crate has no `cfg` gates at all and only two `std::`
imports, `render.rs:7` and `state.rs:3`. Record: "compiles and draws unchanged: yes/no", plus the
`cargo tree` output verbatim.

- [ ] **Step 8: Record the verdict in the ledger, in exactly this form**

Append to `.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md`, under a new heading. Fill every
field; a field you cannot fill is evidence that the check did not run.

```markdown
## Task 3.1 — the Ratzilla spike: **PASS**

Run on <date> on this machine, browser <name and version> at <window size>, `rustc 1.96.1`.

- **Q1 Braille canvas: PASS.** <one line per observation 7.1 a–d, plus the resize and the zoom>
- **Q1 glyph metrics:** `◉` <aligned / two columns / missing>, `⚙` <…>, `·` <…>; rendered by <font
  Rendered Fonts reported, or "WebGl2 backend atlas">. Defect D1 = <none, or what it is and which of
  the four one-line remedies applies>. `Hit.area` still matched the drawn glyphs after the resize and
  the zoom: <yes/no — no is a Q2 FAIL>.
- **Q2 mouse: PASS.** `ButtonDown(MouseButton::Left)` arrived with grid `(col,row)` at every one of
  the seven clicks; `hit_at(&hits, col, row)` returned the control that was clicked in every case; the
  seven expected `actions=[…]` are recorded above. `Ctrl+R`/`Ctrl+S` = <what the browser did>.
- **Q3 backend: `WebGl2Backend`.** frame samples: <the three `frames=… avg_ms=… worst_ms=…` lines>.
  Key events fired <with / without> clicking the canvas first: <yes/no>. DOM backend reproduced
  <#188 / #190 / #186 / #179 / none of them>: <what was seen>.
- **Q4 `draw` unchanged: PASS.** `cargo tree -p ratatui` → <one line, verbatim>. No `cfg(target_arch)`
  gate was needed in `tipsy-ui`.
- **Versions pinned:** `ratzilla = "=0.3.1"`, `ratatui 0.30.2`, `wasm-bindgen 0.2.129`, `web-sys
  0.3.106`. **Build route:** `cargo build -j 1 --target wasm32-unknown-unknown --release` +
  `wasm-bindgen --target web --out-dir <dir>`.
- **Open risks that remain, by number:** #135 (multi-width glyphs, all three backends — this is the
  class D1 belongs to) and <#188 or #190 if this verdict chose `DomBackend`; if it chose
  `WebGl2Backend`, say so and name the `WebEventHandler` silent-key-failure warning instead of a
  second number — do not invent one>.
```

If any question failed, the heading is `## Task 3.1 — the Ratzilla spike: **FAIL**`, and the block
records the same observations plus the one that failed.

- [ ] **Step 9: The exit rule — read it before you write anything else**

- **Q1 or Q2 FAIL → Phase 3 stops here.** Tasks 3.2 – 3.6 are abandoned, and the delivered browser
  answer for v1 is the **web terminal** — exactly what the v1 spec §10 (`:389-391`) and §15.1 (`:460`)
  allow, and what the ledger already says ("if the spike fails, tasks 3.2 – 3.6 are abandoned and the
  browser answer for v1 is the web terminal", `progress.md:18-20`). The ledger line for the task after
  this one is not written, and nothing in `crates/` changes.
- **Q1 and Q2 PASS → Phase 3 continues** with Task 3.2, which consumes the backend name, the pinned
  versions and the build route recorded above.
- **Q3 never stops the phase on its own.** It picks a type, not a go/no-go: the three backends are
  interchangeable behind `WebRenderer`, and a later switch is a one-line change in `tipsy-web/src/lib.rs`.

- [ ] **Step 10: What must not happen**

- **Nothing from the scratch directory is committed**, copied into `crates/`, or added to
  `Cargo.toml:3`'s `members`. This task produces **no commit at all**: the only file it touches inside
  the repository is the git-ignored ledger, and an empty commit is not a commit of this task's work.
- `git status --porcelain` must list no file this task created or changed. The checkout is already
  dirty with the 2C and login plans' work (`global-constraints.md:42-48`); that list is not yours to
  change, and `git stash`, `git restore`, `git checkout -- .` and `git reset` are forbidden here.
- `cargo fmt` is never run (`global-constraints.md:39-40`), and no `cargo test --workspace` is
  started: the spike's builds are its own `CARGO_TARGET_DIR`, and the shared target-directory lock is
  the reason (`global-constraints.md:12`).
- The scratch tree stays until Task 3.2 has copied the build route and the page skeleton from it. If
  it is deleted later, that is a manual decision on the literal path
  `E:\workspace\personal\tipsy-spikes\ratzilla`, applied to nothing else.

**A note on the two line-number families in this body.** The Ratzilla citations are docs.rs line
numbers on a fetched page (`event.rs` lines 2/19/131/148, `backend/dom.rs.html#125-127`); the
`tipsy-ui`, `tipsy-compadre` and `tipsy-protocol` citations are the working tree's, and they were read
for this task. Where the two disagree, the tree wins, and a one-line note in the ledger is the right
place to say so.

### Task 3.2: `tipsy-web` and its socket `[mid]`

This task creates the crate and makes it speak the protocol over `web-sys`'s `WebSocket`. **It adds no
`Transport` trait** (spec W6, §3.3): the in-process channel already is the protocol, so the browser's
job is to produce the same three ends `SessionEnds` has, with a socket on the far side instead of
`spawn_with_events` (`session.rs:46-52`). It also changes nothing on the server: a web client is a
client of the same `/ws` route with `ClientKind::Web` in its `Hello` (spec §4.1, §3.1).

**Files:**
- Create: `crates/tipsy-web/Cargo.toml`
- Create: `crates/tipsy-web/src/lib.rs` — the `#[wasm_bindgen(start)]` entry, the shared cells and the
  render loop
- Create: `crates/tipsy-web/src/wire.rs` — the pure half: JSON text and binary frames, and the rule
  that a malformed message is never fatal. Its own `#[cfg(test)] mod tests` is where the host-runnable
  tests live.
- Create: `crates/tipsy-web/src/socket.rs` — the `web-sys` WebSocket, its closures, `Hello`, and
  reconnection
- Create: `web/client/index.html` — the minimal host page. **Task 3.5 owns the finished page and
  modifies this file**; this task creates the smallest page that lets the socket be exercised.
- Modify: `Cargo.toml:3` — add `"crates/tipsy-web"` to `members`
- Modify: `crates/tipsy-ui/src/strings.rs` — the two connection notices (see Step 1's note on reuse)
- Test: `crates/tipsy-web/src/wire.rs` — its own `#[cfg(test)] mod tests`, run on the host target

**Interfaces:**
- Consumes:
  - `PROTOCOL_VERSION = 1` `tipsy-protocol/src/lib.rs:7`; `ClientKind::{Tui, Web, WebTerminal}` `:9-15`;
    `Command` (13 variants, tagged snake_case) `:107-130`; `Event` (13 variants) `:132-172`;
    `Frame::{Mic(Vec<i16>), Speech{seq, samples}}` `:174-180`; `FrameError` `:182-190`;
    `Frame::encode` `:209-225`; `Frame::decode` `:227-241`; `ToServer` `:244-248`; `ToClient` `:251-255`
  - `tipsy_ui::{draw, UiState, UiAction, KeyInput, ClockView, Hit}` — `crates/tipsy-ui/src/render.rs:65`,
    `:43-48`, `:50-63`, `crates/tipsy-ui/src/state.rs:84-98`, `:70-81`
  - The shape being reproduced, read before writing: `SessionEnds`' three `pub` fields
    `crates/tipsy-compadre/src/session.rs:46-52`; the native client's poll loop `client.rs:166-168`
    (a non-blocking drain once per frame, 33 ms at `:31`); and the native bridge's contract, spec §3.3:
    "turns `Message::Text` into `Command` and `Message::Binary` into `Frame`".
  - The research note's shapes, which this task verifies by compiling: `web-sys`'s
    `WebSocket::new(url)`, `set_binary_type(BinaryType::Arraybuffer)`, `send_with_str`, `send_with_u8_array`,
    `set_onopen/set_onmessage/set_onerror/set_onclose` and the `CONNECTING/OPEN/CLOSING/CLOSED`
    constants (`ratzilla-and-wasm-audio.md` §6.2, read off docs.rs); the MDN facts the note quotes —
    `binaryType` defaults to `"blob"` (`§6.1`), `send()` before `open` throws `InvalidStateError`
    (`§6.1`), the browser `WebSocket` has no backpressure (`web-transport-and-terminal.md` §7.1); and
    `spawn_local`/`JsFuture` for everything that is a `Promise` while `draw_web` needs no async
    runtime at all (`§6.3`, `§6.4` point 5).
- Produces:
  - `pub fn start() -> Result<(), JsValue>` — `#[wasm_bindgen(start)]`, the wasm entry point
    (the front matter's shared contract pins this name).
  - `pub struct SocketEnds { pub from_server: Rc<RefCell<VecDeque<ToClient>>>, pub events: Rc<RefCell<VecDeque<ToClient>>>, pub to_server: Outbox }`
    — the browser's `SessionEnds`: the same three ends, the same payloads, the same names.
  - `pub fn connect(url: &str, token: Option<String>, queue: Rc<RefCell<VecDeque<ToClient>>>) -> Result<Outbox, JsValue>`
  - `pub fn spawn_connection(url: String, token: Option<String>, queue: Rc<RefCell<VecDeque<ToClient>>>, state: Rc<RefCell<UiState>>)`
  - `pub struct Outbox` with `pub fn send(&self, message: ToServer) -> Result<(), WireError>` and
    `pub fn is_open(&self) -> bool`
  - `pub fn url_from_location() -> Result<String, JsValue>` and `pub fn token_from_location() -> Option<String>`
  - `crates/tipsy-web/src/wire.rs`: `pub fn event_from_text(&str) -> Result<Event, WireError>`,
    `pub fn frame_from_bytes(&[u8]) -> Result<Frame, WireError>`,
    `pub fn command_to_text(&Command) -> Result<String, WireError>`,
    `pub fn frame_to_bytes(&Frame) -> Vec<u8>`, `pub enum WireError`
  - the crate's own manifest, which Tasks 3.3 – 3.6 extend with their own features, and the
    `wasm32-unknown-unknown` build route they reuse.

**Which crate may depend on what.** `crates/tipsy-web/Cargo.toml` depends on `tipsy-protocol`,
`tipsy-ui`, `serde`, `serde_json`, `thiserror`, `ratzilla`, `wasm-bindgen`, `web-sys`, `js-sys` and
`wasm-bindgen-futures` — and on **nothing else**. In particular it must not depend on
`tipsy-compadre`, whose closure holds `tokio`, `cpal`, `rodio`, `keyring`, `sysinfo` and `reqwest`, and
the global constraints keep that closure out of the wasm artifact
(`global-constraints.md:57-58`, spec §3.1). The reverse direction is what the constraints already say:
`tipsy-protocol` and `tipsy-ui` never gain `tokio`, `crossterm` or the rest, so the two pure crates
stay shared and this one stays the only wasm-specific consumer.

- [ ] **Step 1: Create the manifest, and prove the crate can be tested on the host before writing code**

`crate-type = ["cdylib", "rlib"]`, not `["cdylib"]`: the `cdylib` is what `wasm-bindgen` processes for
the browser, and the `rlib` is what lets `cargo test -p tipsy-web` link the lib into a host test
binary. The workspace has no `crate-type` anywhere today (`codebase-recon.md:1.1`, §11.2), so this is
the first one and it is what makes the pure half testable at all.

```toml
# crates/tipsy-web/Cargo.toml
[package]
name = "tipsy-web"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "The browser client: Ratzilla backend, web-sys WebSocket, browser audio"

# `cdylib` is the artifact wasm-bindgen processes; `rlib` is what the host test binary links.
# Both are needed, and the workspace has no other crate-type (codebase-recon.md §1.1, §11.2).
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
tipsy-protocol.workspace = true
tipsy-ui.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
# Pinned as Task 3.1's verdict recorded them: `=0.3.1` because a bump to ratatui 0.31 while the
# workspace stays on 0.30 would silently produce two `Frame` types (ratzilla-and-wasm-audio.md:359).
ratzilla = "=0.3.1"
wasm-bindgen = "0.2.129"
js-sys = "0.3.106"
wasm-bindgen-futures = "0.4.79"

[dependencies.web-sys]
version = "0.3.106"
features = [
  # The socket and the page's own location.
  "Window", "Location", "UrlSearchParams", "Document", "Element", "HtmlElement", "Node", "console",
  "WebSocket", "BinaryType", "MessageEvent",
]
```

Then add the member and prove the host build **before** writing any code:

```toml
# Cargo.toml:3 — the only line of this file that changes
members = ["crates/tipsy-protocol", "crates/tipsy-ui", "crates/tipsy-compadre", "crates/tipsy-web"]
```

```rust
// crates/tipsy-web/src/lib.rs, at this point nothing but the module list
//! The browser client.
pub mod socket;
pub mod wire;
```

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
cargo test -j 1 -p tipsy-web
```

Expected: it compiles and runs 0 tests. **This step is the decision point the plan's brief names.** A
`web-sys`-based crate *does* build for the host target: web-sys's generated items are plain
`extern "C"` declarations with no `target_arch` gate (`web-sys-0.3.106/src/lib.rs` and
`src/features/gen_WebSocket.rs` were read in this machine's cargo registry cache for this task), and
`wasm-bindgen`'s macro emits a non-wasm fallback for every import —
`#[cfg(not(all(target_family = "wasm", not(target_os = "wasi"))))] unsafe fn … { panic!("cannot convert
to JsValue outside of the Wasm target") }` at
`wasm-bindgen-macro-support-0.2.129/src/codegen.rs:466-469`, with the same pattern at `:5273-5276`. The
consequences are absolute and shape the rest of this task: **the host build compiles, the pure tests
run, and no test may construct or touch a `web_sys` value** — that path panics by design outside wasm.
If this step fails to compile at all, stop and report: the pure mapping would have to move to a
host-buildable module before any of it is written, and that is a change to this task's shape, not a
detail to improvise.

- [ ] **Step 2: Write the failing tests for the wire half**

Create `crates/tipsy-web/src/wire.rs` with the module doc, the fixtures and the tests only — no
`WireError`, no functions. That is what makes Step 3 fail for the right reason, the same order Plan 2C's
Task 3 used (`docs/superpowers/plans/2026-10-10-plan-2c-openai-realtime.md:1746`).

```rust
//! Wire → value and value → wire: the pure half of the browser client.
//!
//! Text frames carry the protocol's own JSON (`Command` out, `Event` in) and binary frames carry
//! `Frame::encode`'s bytes, exactly as the native bridge does (spec §3.3, §4.5). Nothing here touches
//! a socket, a DOM node or a clock, which is what makes every shape this client trusts a string in
//! the tests below — and what lets them run on the host target.

#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::{
        AssistantState, Capability, ClientKind, ErrorCode, Event, Frame, FrameError, ListeningMode, ProfileView,
        UiLanguage, PROTOCOL_VERSION,
    };

    /// Every variant of `Event`, built by hand. Adding a variant to `tipsy-protocol` means adding it
    /// here — and that is the point: `event_from_text` refuses a tag it does not know, so this list is
    /// what keeps the client from silently ignoring a new event.
    fn every_event() -> Vec<Event> {
        vec![
            Event::Welcome {
                session_id: 7,
                profile: ProfileView { id: "compae".into(), name: "Compae".into(), language: "es-ES".into() },
                ui_language: UiLanguage::Es,
                listening_mode: ListeningMode::Push,
                web_bundle: true,
                login: None,
            },
            Event::Status { state: AssistantState::Thinking, detail: Some("pensando".into()) },
            Event::Heard { text: "¿y eso?".into() },
            Event::ReplyDelta { text: "Ozú, ".into() },
            Event::Reply { text: "Ozú, quillo.".into() },
            Event::SpeechChunk { seq: 3, sample_rate: 24_000 },
            Event::Done { first_audio_ms: Some(410), total_ms: 1_930 },
            Event::Stats { cpu: 12.5, ram: 41.0, net_in: 1_024.0, net_out: 512.0 },
            Event::Error { code: ErrorCode::Network, message: "sin red".into(), recoverable: true },
            Event::TestResult { capability: Capability::Chat, ok: true, latency_ms: 220, message: "ok".into() },
            Event::Live { active: false },
            Event::LoginHeard { text: "hola".into() },
            Event::LoginVerdict { ok: true, name: Some("Compae".into()) },
        ]
    }

    #[test]
    fn every_event_survives_a_round_trip_through_the_wire() {
        for event in every_event() {
            let text = serde_json::to_string(&event).expect("an event serialises");
            assert_eq!(event_from_text(&text).expect("and parses back"), event, "{text}");
        }
    }

    #[test]
    fn an_unknown_event_type_is_a_tag_not_a_crash() {
        // A newer server's event, which this build has never heard of: Plan 2C's Task 3 rule — data,
        // not death (docs/superpowers/plans/2026-10-10-plan-2c-openai-realtime.md:1732).
        assert_eq!(
            event_from_text(r#"{"type":"volume_changed","level":3}"#),
            Err(WireError::UnknownType("volume_changed".into()))
        );
        assert_eq!(event_from_text(r#"{"level":3}"#), Err(WireError::UnknownType(String::new())));
    }

    #[test]
    fn a_known_event_with_a_broken_payload_names_its_tag() {
        let error = event_from_text(r#"{"type":"speech_chunk"}"#).unwrap_err();
        match error {
            WireError::Event { tag, detail } => {
                assert_eq!(tag, "speech_chunk");
                assert!(detail.contains("seq"), "{detail}");
            }
            other => panic!("expected a named event error, got {other:?}"),
        }
    }

    #[test]
    fn a_text_frame_that_is_not_json_is_an_error_not_a_panic() {
        assert!(matches!(event_from_text("not json at all"), Err(WireError::Json(_))));
    }

    #[test]
    fn the_text_of_a_command_is_the_protocols_json() {
        // Field order is the derive's, and `token: None` is `null` — pinned so a protocol change
        // cannot pass unnoticed.
        assert_eq!(
            command_to_text(&Command::Hello {
                token: None,
                client: ClientKind::Web,
                protocol_version: PROTOCOL_VERSION,
            })
            .unwrap(),
            r#"{"type":"hello","token":null,"client":"web","protocol_version":1}"#
        );
        assert_eq!(command_to_text(&Command::ListenStart).unwrap(), r#"{"type":"listen_start"}"#);
        assert_eq!(
            command_to_text(&Command::Ask { text: "¿y eso?".into() }).unwrap(),
            r#"{"type":"ask","text":"¿y eso?"}"#
        );
    }

    #[test]
    fn binary_frames_are_the_protocols_own_bytes() {
        // `audio/device.rs:18-20`'s 320-sample frame, little-endian, behind the 0x01 kind byte
        // (`tipsy-protocol/src/lib.rs:192-193`, `:209-225`).
        assert_eq!(frame_to_bytes(&Frame::Mic(vec![1, -2])), vec![0x01, 0x01, 0x00, 0xFE, 0xFF]);
        assert_eq!(frame_from_bytes(&[0x01, 0x01, 0x00]).unwrap(), Frame::Mic(vec![1]));
    }

    #[test]
    fn a_binary_frame_that_is_not_audio_is_an_error() {
        assert_eq!(frame_from_bytes(&[]), Err(WireError::Frame(FrameError::Empty)));
        assert_eq!(frame_from_bytes(&[0x09, 0, 0]), Err(WireError::Frame(FrameError::UnknownKind(9))));
        assert_eq!(frame_from_bytes(&[0x01, 0x00]), Err(WireError::Frame(FrameError::Malformed)));
    }
}
```

- [ ] **Step 3: Make them pass**

```rust
// crates/tipsy-web/src/wire.rs — the half above the tests
use tipsy_protocol::{
    Command, Event, Frame, FrameError, PROTOCOL_VERSION,
};

/// The `type` tag of every `Event` this build knows. `event_from_text` refuses anything else as
/// [`WireError::UnknownType`], which the socket turns into silence rather than a notice: a newer
/// server may send more than this client understands, and that must not break a running HUD
/// (v1 spec §12; Plan 2C Task 3's "an unknown `type` is data, not death").
pub const KNOWN_EVENT_TAGS: [&str; 13] = [
    "welcome",
    "status",
    "heard",
    "reply_delta",
    "reply",
    "speech_chunk",
    "done",
    "stats",
    "error",
    "test_result",
    "live",
    "login_heard",
    "login_verdict",
];

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum WireError {
    #[error("the text frame is not JSON: {0}")]
    Json(String),
    #[error("unknown event type `{0}`")]
    UnknownType(String),
    #[error("`{tag}` did not parse: {detail}")]
    Event { tag: String, detail: String },
    #[error("the binary frame is not audio: {0}")]
    Frame(#[from] FrameError),
}

/// One text frame → one `Event`. The tag is read first so that an event this build does not know is
/// told apart from an event whose payload it cannot read: the first is ignored, the second is shown.
pub fn event_from_text(text: &str) -> Result<Event, WireError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| WireError::Json(e.to_string()))?;
    let tag = value.get("type").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    if !KNOWN_EVENT_TAGS.contains(&tag.as_str()) {
        return Err(WireError::UnknownType(tag));
    }
    serde_json::from_value(value).map_err(|e| WireError::Event { tag, detail: e.to_string() })
}

/// One binary frame → one `Frame`. The bytes are the protocol's own (`Frame::decode`), so the
/// browser and the native client read the same audio.
pub fn frame_from_bytes(bytes: &[u8]) -> Result<Frame, WireError> {
    Frame::decode(bytes).map_err(WireError::Frame)
}

pub fn command_to_text(command: &Command) -> Result<String, WireError> {
    serde_json::to_string(command).map_err(|e| WireError::Json(e.to_string()))
}

pub fn frame_to_bytes(frame: &Frame) -> Vec<u8> {
    frame.encode()
}
```

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
cargo test -j 1 -p tipsy-web wire::
```

Expected: 7 passed, 0 failed. `PROTOCOL_VERSION` is imported in the tests and used by the `Hello`
assertion, which is also what keeps the pinned `1` honest (`tipsy-protocol/src/lib.rs:7`).

- [ ] **Step 4: Write the socket**

Three facts shape this file, all from the research note: `binaryType` defaults to `"blob"`, so it
**must** be set to `Arraybuffer` before any message arrives (`ratzilla-and-wasm-audio.md` §6.1, §6.4
point 1); `send()` while `CONNECTING` throws `InvalidStateError`, and data sent while `CLOSING`/`CLOSED`
is silently discarded (`§6.1`) — so `Outbox::send` queues nothing and reports; and the browser
`WebSocket` has **no backpressure** (`web-transport-and-terminal.md` §7.1), which is why the inbound
side is a queue the render loop drains once per frame rather than a callback that draws.

```rust
//! The browser's socket, and the three ends the browser's equivalent of `SessionEnds` has.
//!
//! The native client's `SessionEnds` (`crates/tipsy-compadre/src/session.rs:46-52`) is
//! `{ to_server, from_server, events }` over `tokio::sync::mpsc`. There is no trait here either
//! (spec W6): `SocketEnds` is that struct's browser shape, and the payloads are the same
//! `ToServer`/`ToClient`, so the server needs no change for a web client.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use tipsy_protocol::{ClientKind, Command, ToClient, ToServer, PROTOCOL_VERSION};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::wire::{self, WireError};

/// Reconnect delays, in milliseconds, after each close; the last value is used for every attempt
/// after it. The v1 spec §12 promises "the client reconnects with backoff" without a schedule, so
/// this is the plan's choice, and 500 ms doubling to 16 s is small enough for a server restart to be
/// invisible and slow enough not to hammer a server that is down.
pub const RECONNECT_MS: [u32; 6] = [500, 1_000, 2_000, 4_000, 8_000, 16_000];

/// The browser's `SessionEnds`: the same three ends, the same payloads.
#[derive(Clone)]
pub struct SocketEnds {
    /// What the socket's `message` callback pushed, in arrival order: the equivalent of
    /// `SessionEnds::from_server`, drained by the render loop instead of by `try_recv`
    /// (`crates/tipsy-compadre/src/client.rs:166-168` is the loop being mirrored).
    pub from_server: Rc<RefCell<VecDeque<ToClient>>>,
    /// A second handle to the same queue, for producers that live beside the socket — the browser's
    /// shape of `SessionEnds::events`, which exists for the same reason: `sysmon` and the screen's
    /// Test results push into the client's queue behind the session's back. Nothing pushes here yet;
    /// Task 3.4's playback bookkeeping is the first candidate.
    pub events: Rc<RefCell<VecDeque<ToClient>>>,
    /// The browser's `SessionEnds::to_server`.
    pub to_server: Outbox,
}

/// The browser's `UnboundedSender<ToServer>`: `send` takes the same two payloads the native client
/// passes and puts them on the wire as a text or a binary frame (spec §3.3).
#[derive(Clone)]
pub struct Outbox {
    ws: web_sys::WebSocket,
}

impl Outbox {
    pub fn is_open(&self) -> bool {
        self.ws.ready_state() == web_sys::WebSocket::OPEN
    }

    pub fn send(&self, message: ToServer) -> Result<(), WireError> {
        match message {
            ToServer::Command(command) => {
                let text = wire::command_to_text(&command)?;
                // `send()` while CONNECTING throws InvalidStateError, and anything sent while
                // CLOSING/CLOSED is silently discarded (MDN, quoted in ratzilla-and-wasm-audio.md
                // §6.1). Checking first turns both into one honest error.
                if !self.is_open() {
                    return Err(WireError::Json("the socket is not open".into()));
                }
                self.ws.send_with_str(&text).map_err(|e| WireError::Json(error_text(&e)))
            }
            ToServer::Audio(frame) => {
                if !self.is_open() {
                    return Err(WireError::Json("the socket is not open".into()));
                }
                self.ws
                    .send_with_u8_array(&wire::frame_to_bytes(&frame))
                    .map_err(|e| WireError::Json(error_text(&e)))
            }
        }
    }
}

/// A JS exception's readable text, for a `WireError`. `Reflect::get` is used rather than
/// `js_sys::Error`'s accessors because the thrown value is not always an `Error`.
pub fn error_text(value: &JsValue) -> String {
    js_sys::Reflect::get(value, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .or_else(|| value.as_string())
        .unwrap_or_else(|| format!("{value:?}"))
}

/// The socket URL, from the page's own location: `http`→`ws`, `https`→`wss`, same host and port,
/// path `/ws` (spec §5.3). The token does **not** travel here: it goes in `Hello` (spec §9.2).
pub fn url_from_location() -> Result<String, JsValue> {
    let location = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?.location();
    let scheme = if location.protocol().unwrap_or_default() == "https:" { "wss" } else { "ws" };
    Ok(format!("{scheme}://{}/ws", location.host()?))
}

/// The token the page was opened with (`?token=…`). The server puts it there because a browser cannot
/// set a header on a navigation (spec §9.2), and this client sends it back in `Hello` — never in a
/// log line (spec §9.6).
pub fn token_from_location() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get("token")
}

/// Opens one socket. `Hello` is the first message (spec §4.1) and it goes out on `open`, because
/// `send()` before that throws.
pub fn connect(
    url: &str,
    token: Option<String>,
    queue: Rc<RefCell<VecDeque<ToClient>>>,
) -> Result<Outbox, JsValue> {
    let ws = web_sys::WebSocket::new(url)?;
    // Required for the binary branch: the documented default is "blob" (MDN, §6.1).
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let outbox = Outbox { ws: ws.clone() };

    ws.set_onopen(Some(
        Closure::once_into_js({
            let outbox = outbox.clone();
            move || {
                let hello = ToServer::Command(Command::Hello {
                    token: token.clone(),
                    client: ClientKind::Web,
                    protocol_version: PROTOCOL_VERSION,
                });
                if let Err(error) = outbox.send(hello) {
                    web_sys::console::warn_1(&JsValue::from_str(&format!("tipsy-web: {error}")));
                }
            }
        })
        .unchecked_ref(),
    ));

    ws.set_onmessage(Some(
        Closure::wrap(Box::new({
            let queue = queue.clone();
            move |event: web_sys::MessageEvent| {
                let data = event.data();
                let message = if let Some(buffer) = data.dyn_ref::<js_sys::ArrayBuffer>() {
                    // Binary: the only binary traffic is audio (spec §4.5).
                    let bytes = js_sys::Uint8Array::new(buffer).to_vec();
                    wire::frame_from_bytes(&bytes).map(ToClient::Audio)
                } else if let Some(text) = data.as_string() {
                    wire::event_from_text(&text).map(ToClient::Event)
                } else {
                    Err(WireError::Json("neither a string nor an ArrayBuffer".into()))
                };
                match message {
                    Ok(message) => queue.borrow_mut().push_back(message),
                    // A malformed message NEVER kills the render loop. An unknown event type is
                    // silent, by design; a known event that will not parse is a warning in the
                    // console and nothing else (v1 spec §12; Plan 2C Task 3's precedent).
                    Err(WireError::UnknownType(tag)) => {
                        web_sys::console::debug_1(&JsValue::from_str(&format!("tipsy-web: ignoring {tag}")));
                    }
                    Err(error) => {
                        web_sys::console::warn_1(&JsValue::from_str(&format!("tipsy-web: {error}")));
                    }
                }
            }
        }) as Box<dyn FnMut(web_sys::MessageEvent)>)
        .into_js_value()
        .unchecked_ref(),
    ));

    Ok(outbox)
}

/// Keeps a connection up: one `connect`, and on every close another one after the next backoff.
/// `state` is where the two notices go; the backoff resets when a `Welcome` arrives, which the
/// render loop reports through [`note_welcome`].
pub fn spawn_connection(
    url: String,
    token: Option<String>,
    queue: Rc<RefCell<VecDeque<ToClient>>>,
    state: Rc<RefCell<tipsy_ui::UiState>>,
) {
    let attempt = Rc::new(RefCell::new(0usize));
    spawn_local(async move {
        let mut delay = 0u32;
        loop {
            if delay > 0 {
                gloo_sleep(delay).await; // see the note below: a `Promise` + `JsFuture`, no new crate
            }
            match connect(&url, token.clone(), queue.clone()) {
                Ok(outbox) => {
                    // The close handler owns the next attempt; `attempt` records which backoff.
                    let state = state.clone();
                    let attempt = attempt.clone();
                    let _ = outbox;
                    *attempt.borrow_mut() += 1;
                    delay = RECONNECT_MS[(*attempt.borrow() - 1).min(RECONNECT_MS.len() - 1)];
                    let _ = state;
                    // The close/error hooks are installed on the next line of the real
                    // implementation; they set `delay` and let this loop go round again. The shape
                    // is the native bridge's (spec §6.1), the schedule is `RECONNECT_MS`.
                    break;
                }
                Err(error) => {
                    state.borrow_mut().notify("tipsy-web: the socket could not be opened");
                    web_sys::console::warn_1(&JsValue::from_str(&error_text(&error)));
                    delay = RECONNECT_MS[(*attempt.borrow()).min(RECONNECT_MS.len() - 1)];
                    *attempt.borrow_mut() += 1;
                }
            }
        }
    });
}
```

Three deliberate notes on that file, because they are the honest parts:

1. **The `set_onmessage` closures are leaked deliberately.** `Closure::into_js_value` hands ownership
   to JS because the socket outlives the call; the alternative — storing the `Closure` next to the
   socket — has to be carried through `SocketEnds` and is what Task 3.3 does for the microphone,
   where the node is dropped on purpose. `Closure::once_into_js` is the `onopen` one: `Hello` is sent
   once per connection.
2. **The reconnect loop as written is a sketch of the control flow, and the missing piece is the
   `onclose`/`onerror` hooks.** They are three lines each (`set_onclose`, `set_onerror`), they must be
   installed in `connect` beside the other two, and the loop needs to *await* the close rather than
   `break`: use a `futures`-free equivalent — an `Rc<RefCell<Option<js_sys::Function>>>` resolved by a
   `Closure` that calls a stored `FnOnce` — or restructure the loop so that each iteration installs
   the hooks and blocks on a `oneshot`-shaped `Rc<Cell<bool>>` polled by a `setTimeout` promise. **Write
   whichever of the two you implement as real code with a comment, and record in the ledger which one
   it was**: the plan pins the behaviour (reconnect with `RECONNECT_MS`, a notice on
   `strings.web_offline`, a cleared notice on `strings.web_online`) and the schedule, and leaves this
   one control-flow shape to the implementer because it is mechanical and both forms are three lines
   from here.
3. **`gloo_sleep` is not a crate this plan has.** There is no `gloo` and no `TimeoutFuture` in the
   tech stack, and none may be added. The sleep is two web-sys calls wrapped in a promise —
   `set_timeout_with_callback_and_timeout_and_arguments_0` needs the `Window` the crate already has
   (`Window::set_timeout_with_callback_and_timeout_and_arguments_0`, feature `Window`), and the promise
   is awaited with `JsFuture`:

```rust
/// A delay with no new dependency: `window.setTimeout` into a `Promise`, and `JsFuture` over it.
async fn sleep(ms: u32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let window = web_sys::window().expect("a window to set a timeout on");
        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32)
            .expect("a timer");
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
```

- [ ] **Step 5: The two connection notices, in both languages**

Add to `crates/tipsy-ui/src/strings.rs`: two fields to `Strings` (`:5-71`), and one line in each of
`ES` (`:73-135`) and `EN` (`:137-199`):

```rust
    /// The browser (and `--connect`) client lost its socket and is retrying. v1 spec §12: "the client
    /// reconnects with backoff" and tells the user in the log.
    pub web_offline: &'static str,
    /// The socket is up again.
    pub web_online: &'static str,
```

```rust
    web_offline: "Sin conexión: reintentando…",
    web_online: "Conexión restablecida",
```

```rust
    web_offline: "Connection lost: retrying…",
    web_online: "Connection restored",
```

**Reuse rule, because Task 1.7's native `--connect` bridge needs the same two messages.** If
`strings.rs` already carries fields with these meanings when this task runs, use those field names
instead of adding a second pair — one concept, one name — and write the name you used in the ledger.
`UILanguage`'s table is the project's only i18n mechanism (v1 spec §9 `:371-372`), so a hard-coded
English notice inside `tipsy-web` would be a violation, not a shortcut.

- [ ] **Step 6: `lib.rs` — the entry point, the shared cells and the render loop**

```rust
//! The browser client's entry point.
//!
//! `start()` is what `wasm-bindgen --target web`'s generated `init()` calls. The render loop is the
//! native client's frame loop with the socket's queue in place of `from_server`
//! (`crates/tipsy-compadre/src/client.rs:118-194`): tick, drain, draw. `draw_web` wraps
//! `requestAnimationFrame` itself, so the loop needs **no async runtime** — only the socket's
//! callbacks do, and those use `spawn_local` (ratzilla-and-wasm-audio.md §6.4 point 5).

pub mod socket;
pub mod wire;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use tipsy_protocol::{Frame, ToClient};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    // The backend Task 3.1's verdict named; the ledger is the authority. The three backends are
    // interchangeable behind `WebRenderer`, so this is the one line a later switch changes.
    let backend = ratzilla::WebGl2Backend::new().map_err(|e| JsValue::from_str(&e.to_string()))?;
    let mut terminal = ratzilla::ratatui::Terminal::new(backend)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let state = Rc::new(RefCell::new(tipsy_ui::UiState::new()));
    let hits: Rc<RefCell<Vec<tipsy_ui::Hit>>> = Rc::new(RefCell::new(Vec::new()));
    let queue: Rc<RefCell<VecDeque<ToClient>>> = Rc::new(RefCell::new(VecDeque::new()));

    let url = socket::url_from_location()?;
    let token = socket::token_from_location();
    // Task 3.5 registers the key and mouse hooks here, before `draw_web` consumes the terminal.
    socket::spawn_connection(url, token, queue.clone(), state.clone());
    // Task 3.3 builds the microphone's gate here. The one rule that crosses into this file: the
    // `AudioContext` is created **and resumed inside the click handler**, together with the
    // `getUserMedia` call, or the user gets a silent client that looks started. Two independent
    // sources say so — MDN's autoplay policy as quoted at `ratzilla-and-wasm-audio.md` §5.2, and the
    // prior art's shipped page (`codewhale-audio-prior-art.md` §3, `pet/src/ui/pet.ts:173`:
    // "`audio = new AudioContext(); await audio.resume()` inside the sound button's handler").

    let mut last = js_sys::Date::now();
    terminal.draw_web({
        let state = state.clone();
        let hits = hits.clone();
        move |frame| {
            let now = js_sys::Date::now();
            let dt = (now - last).max(0.0) as u64;
            last = now;
            let title = {
                let mut state = state.borrow_mut();
                state.tick(dt);
                // The frame loop drains everything that arrived since the last frame, exactly as the
                // native loop does at `client.rs:166-168`.
                let mut queue = queue.borrow_mut();
                while let Some(message) = queue.pop_front() {
                    match message {
                        ToClient::Event(event) => state.apply(&event),
                        // Playback is Task 3.4's: the frames are dropped here on purpose.
                        ToClient::Audio(Frame::Speech { .. }) => {}
                        ToClient::Audio(Frame::Mic(_)) => {}
                    }
                }
                state.profile.as_ref().map(|p| p.name.clone()).unwrap_or_default()
            };
            let clock = clock_view();
            let drawn = {
                let state = state.borrow();
                tipsy_ui::draw(frame, &state, &clock, &title)
            };
            *hits.borrow_mut() = drawn;
        }
    });
    Ok(())
}

/// The HUD's clock, from the page's own `Date`. `ClockView.weekday` numbers the week from Monday
/// (`render.rs:43-48`, built natively at `client.rs:710-720`) and `js_sys::Date::get_day()` numbers
/// it from Sunday, so the browser subtracts one day's worth with `(day + 6) % 7`.
fn clock_view() -> tipsy_ui::ClockView {
    let now = js_sys::Date::new_0();
    tipsy_ui::ClockView {
        time: format!("{:02}:{:02}:{:02}", now.get_hours(), now.get_minutes(), now.get_seconds()),
        weekday: ((now.get_day() + 6) % 7) as u8,
        day: now.get_date() as u8,
        month: now.get_month() as u8,
    }
}
```

The two `js_sys::Date` accessors used here are real and were read in this machine's registry for this
task (`js-sys-0.3.106/src/lib.rs`: `Date::new_0` `:6429`, `Date::now` `:6505`, `get_day` `:6311`,
`get_date` `:6304`, `get_hours` `:6323`, `get_minutes` `:6335`, `get_month` `:6342`, `get_seconds`
`:6348`), and `get_day`'s Sunday-based numbering is MDN's.

- [ ] **Step 7: The minimal host page**

```html
<!doctype html>
<html lang="es">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>tipsy-compadre</title>
    <style>
      html, body { margin: 0; height: 100%; background: #001014; }
      pre, canvas { font-family: Consolas, "Cascadia Mono", "Segoe UI Symbol", monospace; font-size: 16px; }
    </style>
  </head>
  <body>
    <script type="module">
      import init from "./tipsy_web.js";
      init().catch((e) => console.error("tipsy-web: init failed", e));
    </script>
  </body>
</html>
```

The page is the bundle's host page and its URL is where `?token=` arrives (spec §9.2). **Task 3.5 owns
the finished page** — the layout, the styles, whatever the gate needs — and modifies this file; this
task creates the smallest page the socket can be exercised in.

- [ ] **Step 8: The wasm build and the host tests**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
cargo test -j 1 -p tipsy-web
cargo build -j 1 -p tipsy-web --target wasm32-unknown-unknown --release
cargo clippy -j 1 -p tipsy-web --all-targets
```

Expected: the tests pass (7), the wasm target builds, and clippy adds **nothing** to the pinned
pre-existing list in `global-constraints.md:26-37` — the rule is "compare clippy's finding set with that
list and add nothing new", and a new finding in `crates/tipsy-web/` is this task's to fix inside its own
commit. `cargo fmt` is never run (`global-constraints.md:39-40`).

- [ ] **Step 9: The browser half — documented now, runnable at Task 3.6**

The socket cannot be exercised in a browser before the bundle exists, because the page and `/ws` have
to be the same origin and only the embedded bundle serves the page from the server's own port (spec
§8.3, Task 3.6). The steps and the expected observations are fixed here so that 3.6 runs *this* script
rather than inventing one:

1. `cargo run -j 1 -p tipsy-compadre -- serve --verbose` and read the address it prints (success
   criterion 1, `2026-10-11-web-tier-design.md:41-43`). On a fresh install `lan = false` and no token is
   needed: loopback is trusted (spec §9.1).
2. Open `http://127.0.0.1:<port>/` in a real browser with the console open. **Expected:** the HUD is
   there, the console shows no `tipsy-web: …` warning, and the server's log shows a client connecting
   with `ClientKind::Web`.
3. Type `hola` and press `Enter`. **Expected:** the console logs the `{"type":"ask","text":"hola"}`
   text frame going out (add a `debug_1` in `Outbox::send` behind a `const TRACE: bool = false` if you
   want it visible), the server answers `Welcome` then `Heard`/`ReplyDelta`/`Reply`, and the reply text
   appears in the HUD's log. **Audio does not play**: playback is Task 3.4's, and dropping `Frame::Speech`
   is this task's deliberate behaviour.
4. Stop the server (`Ctrl+C`) with the page open. **Expected:** the `web_offline` notice appears, the
   console shows the failed connection attempt, the page keeps drawing, and no `panicked at` appears.
5. Start the server again on the same port. **Expected:** within the backoff schedule (`RECONNECT_MS`)
   the socket reconnects, `Hello` is sent, `Welcome` arrives, and the `web_online` notice replaces the
   offline one.
6. Open two tabs. **Expected:** two `session_id`s in the two `Welcome`s (W9), each tab's own history.
7. Open `http://127.0.0.1:<port>/?token=<wrong>` on a loopback bind. **Expected:** the server's
   `token_invalid` error arrives as `Event::Error`, this client shows it as a notice rather than
   dropping it, and no reconnect storm follows.

- [ ] **Step 10: Commit**

One commit, only the files this task names:

```
feat(web): add tipsy-web and its protocol socket

crates/tipsy-web is the browser client's cdylib: wire.rs is the pure
Command/Event/Frame mapping with its host-runnable tests, socket.rs is the
web-sys WebSocket with Hello first and reconnection with backoff, and lib.rs
is the wasm entry point whose render loop drains the socket's queue exactly
as the native client drains SessionEnds. No server change is needed: the
text frames are the protocol's own JSON and the binary frames are
Frame::encode's bytes.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
```

`git add` exactly `crates/tipsy-web/`, `Cargo.toml`, `crates/tipsy-ui/src/strings.rs` and
`web/client/index.html`. The checkout holds two other plans' uncommitted work
(`global-constraints.md:42-48`); a `git add -A` here would commit somebody else's files.

### Task 3.3: The browser microphone `[mid]`

The browser has to do in WebAudio what `crates/tipsy-compadre/src/audio/device.rs` does in cpal:
mono, 16 kHz, PCM16, **320 samples (20 ms) per frame**, so that the server's frame handling is
untouched (`audio/device.rs:18-20`: `MIC_SAMPLE_RATE = 16000`, `MIC_FRAME_SAMPLES = 16000 / 50`). The
frames go up as the **existing** `0x01` binary frames (`tipsy-protocol/src/lib.rs:192-193`, `:209-225`)
inside `ToServer::Audio(Frame::Mic(..))` — no protocol change, no new frame kind.

Two shapes this task takes from the prior-art note
(`.superpowers/sdd/plan-3-research/codewhale-audio-prior-art.md`), and they decide its structure:

- **Capability first, and failure is data** (§4's last row, which lands in this task and Task 3.5).
  The shipped desktop client "reads `GET /v1/voice` first and disables its dictation affordance when
  `available` is false", and every failure "answer[s] `200` with `ok: false` + a machine-readable
  `reason`" (`crates/tui/src/runtime_api/voice.rs:9-12`, routes `:14-18`, quoted at the note's §2); the
  browser side does the same from the data it holds — `if (frame.audioUnavailable && sound) { sound =
  false; $('sound').textContent = 'Sound unavailable'; … }` (`pet/public/shared.html:45`, §3). So the
  **"press to start" gate is a capability check with a reason**, not a button that fails on first use:
  before the page offers the microphone it decides whether it *can* (`navigator.mediaDevices` exists at
  all — it is `undefined` in an insecure context — and what `permissions.query({name:"microphone"})`
  answers), and an unavailable microphone draws the affordance **disabled with a short es/en reason**
  taken from `tipsy-ui/src/strings.rs`. A reason is a value from the table, never a string built at
  the call site, and the pairing is the same shape `Welcome.web_bundle` already uses (spec §5.4) —
  which is why the data type and the two strings are in this body and not in Task 3.5's.
- **A pure core and a thin presenter, with the invariants named.** Prior art's `pet-audio.ts:10-12`:
  "Core emits score events; WebAudio / AVAudioEngine only present their PCM. Calling at any display
  cadence produces the same score when all world ticks are supplied. Calling twice for a world tick
  cannot retrigger a voice." The browser's audio is this phase's core; here the presenter is the
  `AudioWorklet`, which only hands samples over, while the float-to-`i16` conversion, the 320-sample
  boundary and the carry of a partial render quantum are pure Rust with host-runnable tests. That split
  is what makes the tests in Step 1 runnable at all in a harness with no browser.

**Files:**
- Create: `crates/tipsy-web/src/mic.rs` — the pure half (the permission mapping, the frame assembler,
  the resampler and the sample conversion) with its own `#[cfg(test)] mod tests`, plus the WebAudio
  wiring
- Create: `web/client/mic-worklet.js` — the `AudioWorkletProcessor`, which must be a real file: the
  note could not verify that a `Blob` URL may be passed to `addModule`
  (`ratzilla-and-wasm-audio.md` §4.4, "UNVERIFIED (single fetcher)")
- Modify: `crates/tipsy-web/src/lib.rs` — `pub mod mic;`, the "press to start" gate, and the mic's
  frames into the same `Outbox`
- Modify: `crates/tipsy-web/Cargo.toml` — the web-sys features this task needs
- Modify: `crates/tipsy-ui/src/strings.rs` — the six microphone strings, es and en
- Test: `crates/tipsy-web/src/mic.rs` — its own `#[cfg(test)] mod tests`, run with
  `cargo test -j 1 -p tipsy-web mic::`

**Interfaces:**
- Consumes:
  - `pub const MIC_SAMPLE_RATE: u32 = 16000;` and `pub const MIC_FRAME_SAMPLES: usize = (MIC_SAMPLE_RATE as usize) / 50;`
    — `crates/tipsy-compadre/src/audio/device.rs:18-20`, and the capture shape those numbers come from:
    downmix by averaging the channels (`:186-189`), `resample_linear(&mono, rate, MIC_SAMPLE_RATE)`
    (`:190`), clamp and convert with `(s.clamp(-1.0, 1.0) * 32767.0) as i16` (`:191`), emit exactly
    `MIC_FRAME_SAMPLES` and never block the audio thread (`:192-198`).
  - `pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32>` — `crates/tipsy-compadre/src/audio/pcm.rs:36-55`;
    `pub fn f32_to_i16(samples: &[f32]) -> Vec<i16>` — `:20-22`; `pub fn rms(samples: &[f32]) -> f32` —
    `:28-33`. **These three cannot be called from here** (this crate must not depend on
    `tipsy-compadre`, Step 1 of Task 3.2), so `mic.rs` carries its own copies with a comment naming the
    twin, and the tests below pin the same results.
  - `tipsy_ui::UiState::{push_mic_level, mic_open, notify}` — `crates/tipsy-ui/src/state.rs:503`,
    `:105`, `:166`; the native client's own use of them, `crates/tipsy-compadre/src/client.rs:780`.
  - `ToServer::Audio(Frame::Mic(samples))` and `Outbox::send` — Task 3.2's
    `crates/tipsy-web/src/socket.rs`.
  - The MDN facts the note quotes and this task is built on: `getUserMedia` is **secure context only**,
    and `navigator.mediaDevices` is `undefined` in an insecure context (so the `TypeError` state is a
    real state, not a hypothetical); the eight documented rejection names `AbortError`,
    `InvalidStateError`, `NotAllowedError`, `NotFoundError`, `NotReadableError`, `OverconstrainedError`,
    `SecurityError`, `TypeError`; `permissions.query({name:"microphone"})` and
    `PermissionStatus.state` ∈ `granted|denied|prompt`; `AudioContextOptions.sampleRate` is a *request*
    and `BaseAudioContext.sampleRate` is what the context actually runs at — "sample-rate converters
    are not supported"; the render quantum is 128 frames and "you *must always check the size of the
    sample array* rather than assuming a particular size"; `AudioWorklet` is the replacement for the
    deprecated `ScriptProcessorNode`; and the autoplay rule "Create or resume context from inside a
    user gesture" — all `ratzilla-and-wasm-audio.md` §4.1–§4.6 and §5.2, each with its MDN link.
  - The loopback-is-a-secure-context finding, which is why this works at `http://127.0.0.1:7070/` with
    no TLS: `web-transport-and-terminal.md` §4.4 and spec §9.5.
  - The prior art's two shapes and its own citation for the gesture rule:
    `.superpowers/sdd/plan-3-research/codewhale-audio-prior-art.md` §3 (`pet/src/core/pet-audio.ts:10-12`
    for the pure core, `pet/public/shared.html:45` for availability-as-data, and
    `pet/src/ui/pet.ts:173` for `new AudioContext(); await audio.resume()` **inside** the handler) and
    §4 (the import table's last row, which is why the capability shape belongs to this task). The
    gesture rule therefore has two independent sources — `ratzilla-and-wasm-audio.md` §5.2's MDN
    autoplay quotation and this shipped page — and the body treats it as settled.
- Produces:
  - `pub const WORKLET_URL: &str = "/assets/mic-worklet.js";`
  - `pub enum MicState { Prompt, Granted, Denied, NoDevice, Busy, Unsupported, Failed }` and
    `pub fn mic_state_for(rejection: &str) -> MicState`
  - `pub struct MicAvailability { pub available: bool, pub reason: Option<&'static str> }` with
    `pub fn probe(devices_present: bool, permission: MicState, strings: &tipsy_ui::Strings) -> Self`
    and `pub fn from_rejection(name: &str, strings: &tipsy_ui::Strings) -> Self` — the capability
    answer the gate acts on; `reason` is always a value out of the `Strings` table, never a message
    assembled at the call site (the prior art's `ok: false` + `reason` shape, §4)
  - `pub struct FrameAssembler` with `pub const FRAME_SAMPLES: usize = 320`,
    `pub fn new()`, `pub fn push_block(&mut self, channels: &[Vec<f32>]) -> Vec<Vec<i16>>`,
    `pub fn pending_len(&self) -> usize`
  - `pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32>`,
    `pub fn f32_to_i16(samples: &[f32]) -> Vec<i16>`, `pub fn rms(samples: &[f32]) -> f32`
  - `pub struct Capture` (owns the `AudioContext`, the `MediaStream`, the `AudioWorkletNode` and the
    port `Closure`) with `pub fn context(&self) -> &web_sys::AudioContext` — **Task 3.4 plays the reply
    through this same context**, which is why it is exposed rather than kept private
  - `pub fn press_to_start(state: Rc<RefCell<UiState>>, outbox: Outbox)` — the one function that may be
    called from a click handler, and the only place `getUserMedia` and the context are created
  - `pub mod mic;` in `crates/tipsy-web/src/lib.rs`

**The frame boundary lives in Rust, not in the worklet.** 320 is not a multiple of the 128-frame
render quantum (320 = 2×128 + 64 — the arithmetic the note records at §4.5), and the note also warns
that the block size may not be 128 at all. If the accumulation happened in JavaScript it would be
untestable in this harness; in Rust it is a pure function with tests, and the worklet is reduced to
"forward whatever you were handed". The cost is one `postMessage` per quantum instead of one per frame
(2.5×, ~1 KB each) and that is the trade this plan takes deliberately. The split is the prior art's
core/presenter sentence with its two invariants applied here, and saying them out loud is the point:
**calling at any cadence produces the same frames** (the assembler is driven by sample counts, never by
the 33 ms frame or the browser's paint rate — a 128-frame quantum, a 400-sample block and a 480-sample
48 kHz block all land in the same 320-sample output), and **a repeated input cannot retrigger** (the
assembler has no sequence numbers and no timers; it appends and emits, so feeding the same block twice
produces two frames and never a duplicated frame — the retrigger bug class the note warns about belongs
to Task 3.4's playback queue, which does have a cursor).

- [ ] **Step 1: Write the failing tests for the pure half**

Same order as Task 3.2: `crates/tipsy-web/src/mic.rs` gets the module doc and the tests only, so Step 2
fails for the right reason.

```rust
//! The browser microphone: WebAudio capture into the protocol's 320-sample 16 kHz frames.
//!
//! The pure half is here (the permission mapping, the frame assembler, the resampler and the sample
//! conversion) and the WebAudio wiring is below it. The split is not decoration: there is no browser
//! in this harness, so everything that can be a test is one, and the tests run on the host target
//! (Task 3.2 Step 1).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_render_quantum_is_not_a_frame() {
        // 128 frames = 8 ms at 16 kHz (MDN, quoted at ratzilla-and-wasm-audio.md §4.5).
        let mut assembler = FrameAssembler::new();
        assert!(assembler.push_block(&[vec![0.0; 128]]).is_empty());
        assert_eq!(assembler.pending_len(), 128);
    }

    #[test]
    fn three_quanta_make_one_frame_and_leave_sixty_four_samples() {
        // 320 = 2 × 128 + 64: 320 is not a multiple of the render quantum, which is exactly why the
        // remainder has to be carried.
        let mut assembler = FrameAssembler::new();
        assert!(assembler.push_block(&[vec![0.0; 128]]).is_empty());
        assert!(assembler.push_block(&[vec![0.0; 128]]).is_empty());
        let frames = assembler.push_block(&[vec![0.0; 128]]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 320);
        assert_eq!(assembler.pending_len(), 64);
    }

    #[test]
    fn a_partial_quantum_is_carried_and_never_dropped() {
        let mut assembler = FrameAssembler::new();
        // A block larger than a frame emits a frame and keeps the rest.
        let frames = assembler.push_block(&[vec![0.5; 400]]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 320);
        assert_eq!(assembler.pending_len(), 80);
        // Then 240 more completes the second frame exactly.
        let frames = assembler.push_block(&[vec![0.5; 240]]);
        assert_eq!(frames.len(), 1);
        assert_eq!(assembler.pending_len(), 0);
    }

    #[test]
    fn a_block_that_crosses_two_frames_returns_both() {
        let mut assembler = FrameAssembler::new();
        let frames = assembler.push_block(&[vec![0.0; 640]]);
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|frame| frame.len() == 320));
        assert_eq!(assembler.pending_len(), 0);
    }

    #[test]
    fn two_channels_are_averaged_like_the_native_capture() {
        // `audio/device.rs:186-189` averages the channels into mono before resampling.
        let mut assembler = FrameAssembler::new();
        let frames = assembler.push_block(&[vec![1.0; 320], vec![-1.0; 320]]);
        assert_eq!(frames, vec![vec![0i16; 320]]);
    }

    #[test]
    fn the_context_rate_is_resampled_to_sixteen_kilohertz() {
        // The read-back caveat: `AudioContextOptions.sampleRate` is a request and the context may run
        // at anything (ratzilla-and-wasm-audio.md §4.3). A 48 kHz context has to reach 16 kHz here.
        let block = vec![0.25f32; 480];
        let mono = resample_linear(&block, 48_000, 16_000);
        assert_eq!(mono.len(), 160);
        // Three 48 kHz blocks are exactly one 16 kHz frame, with nothing left over.
        let mut assembler = FrameAssembler::new();
        let mut frames = Vec::new();
        for _ in 0..3 {
            frames.extend(assembler.push_block(&[resample_linear(&block, 48_000, 16_000)]));
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 320);
        assert_eq!(assembler.pending_len(), 0);
    }

    #[test]
    fn the_resampler_matches_the_native_one() {
        // `crates/tipsy-compadre/src/audio/pcm.rs:36-55`, pinned by its own arithmetic: out_len =
        // 2 * 4 / 2 = 4, step = 0.5.
        assert_eq!(resample_linear(&[0.0, 1.0], 2, 4), vec![0.0, 0.5, 1.0, 1.0]);
        assert_eq!(resample_linear(&[1.0, 2.0], 16_000, 16_000), vec![1.0, 2.0]);
        assert!(resample_linear(&[], 48_000, 16_000).is_empty());
    }

    #[test]
    fn samples_convert_the_way_the_native_capture_converts_them() {
        // `pcm.rs:20-22` and `audio/device.rs:191` are the same expression; the browser's copy must
        // not round differently.
        assert_eq!(f32_to_i16(&[0.0, 1.0, -1.0]), vec![0, 32767, -32767]);
        assert_eq!(f32_to_i16(&[2.0, -2.0]), vec![32767, -32767]);
        assert_eq!(rms(&[1.0, -1.0]), 1.0);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn the_eight_rejection_names_map_to_the_states_the_hud_shows() {
        // MDN's documented list, quoted at ratzilla-and-wasm-audio.md §4.1. Only `NotAllowedError`
        // means the user said no; the rest are device or context trouble, and treating them all as a
        // denial would lie in the gate's own message.
        assert_eq!(mic_state_for("NotAllowedError"), MicState::Denied);
        assert_eq!(mic_state_for("SecurityError"), MicState::Denied);
        assert_eq!(mic_state_for("TypeError"), MicState::Unsupported); // no `navigator.mediaDevices`
        assert_eq!(mic_state_for("NotFoundError"), MicState::NoDevice);
        assert_eq!(mic_state_for("NotReadableError"), MicState::Busy);
        assert_eq!(mic_state_for("OverconstrainedError"), MicState::Unsupported);
        assert_eq!(mic_state_for("AbortError"), MicState::Failed);
        assert_eq!(mic_state_for("InvalidStateError"), MicState::Failed);
        assert_eq!(mic_state_for("SomethingNewError"), MicState::Failed);
    }

    #[test]
    fn the_gate_is_disabled_with_a_reason_instead_of_failing_on_first_use() {
        // Capability first: the affordance is offered only when the microphone can actually open, and
        // when it cannot, the reason is a value out of the table (the prior art's shape,
        // codewhale-audio-prior-art.md §4).
        let es = tipsy_ui::strings(tipsy_protocol::UiLanguage::Es);
        let en = tipsy_ui::strings(tipsy_protocol::UiLanguage::En);
        assert_eq!(
            MicAvailability::probe(true, MicState::Prompt, es),
            MicAvailability { available: true, reason: None }
        );
        assert_eq!(
            MicAvailability::probe(true, MicState::Granted, es),
            MicAvailability { available: true, reason: None }
        );
        let denied = MicAvailability::probe(true, MicState::Denied, es);
        assert!(!denied.available);
        assert_eq!(denied.reason, Some(es.mic_denied));
        // No `navigator.mediaDevices` at all is its own reason: MDN's second documented half is that
        // the object is `undefined` in an insecure context, so telling the user "denied" there would
        // send them to the wrong setting (ratzilla-and-wasm-audio.md §4.1).
        let insecure = MicAvailability::probe(false, MicState::Unsupported, en);
        assert!(!insecure.available);
        assert_eq!(insecure.reason, Some(en.mic_no_api));
        // The same shape answers a rejection that arrives later, from the exception's name.
        assert_eq!(
            MicAvailability::from_rejection("NotFoundError", es),
            MicAvailability { available: false, reason: Some(es.mic_nodevice) }
        );
        assert_eq!(
            MicAvailability::from_rejection("NotReadableError", es),
            MicAvailability { available: false, reason: Some(es.mic_busy) }
        );
        // Every reason is one of the table's own values, in both languages: a `&'static str` that is
        // not in either table cannot be produced by these two constructors.
        for availability in [
            MicAvailability::probe(false, MicState::Failed, es),
            MicAvailability::probe(true, MicState::Denied, es),
            MicAvailability::probe(true, MicState::NoDevice, es),
            MicAvailability::probe(true, MicState::Busy, es),
            MicAvailability::probe(true, MicState::Failed, es),
        ] {
            assert!(!availability.available);
            assert!(availability.reason.is_some());
        }
    }
}
```

- [ ] **Step 2: Make them pass, and show the numbers**

```rust
// crates/tipsy-web/src/mic.rs — the half above the tests

/// 20 ms at 16 kHz, the frame the protocol promises (`audio/device.rs:18-20`).
pub const FRAME_SAMPLES: usize = 320;
/// The context rate the project wants. It is a **request**: what the context actually runs at is read
/// back from `AudioContext::sample_rate()` and resampled if it differs
/// (ratzilla-and-wasm-audio.md §4.3).
pub const MIC_RATE: u32 = 16_000;

/// The worklet module. `/assets/` is where the bundle serves the client's files (spec §5.3); if Task
/// 1.5's `server::assets::asset(path)` ends up serving them under another prefix, this one constant
/// changes — nothing else does.
pub const WORKLET_URL: &str = "/assets/mic-worklet.js";

/// The three permission states plus the four ways a device can fail, as the gate shows them. The v1
/// spec §12 rule is one sentence: a microphone that will not open disables listening with a warning
/// and typing still works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicState {
    /// `PermissionStatus.state == "prompt"`: show "press to start".
    Prompt,
    Granted,
    /// The user said no, or the context is not secure.
    Denied,
    NoDevice,
    Busy,
    /// The browser ignored our constraints, or there is no `navigator.mediaDevices` at all.
    Unsupported,
    /// Everything else, including the two transient rejections.
    Failed,
}

pub fn mic_state_for(rejection: &str) -> MicState {
    match rejection {
        "NotAllowedError" | "SecurityError" => MicState::Denied,
        "NotFoundError" => MicState::NoDevice,
        "NotReadableError" => MicState::Busy,
        "OverconstrainedError" | "TypeError" => MicState::Unsupported,
        _ => MicState::Failed,
    }
}

/// What the gate acts on: can this page offer the microphone, and if not, why — as data.
///
/// The prior art's shape (`codewhale-audio-prior-art.md` §2 and §4: failures "answer `200` with
/// `ok: false` + a machine-readable `reason`", and the browser disables and relabels the control from
/// the data it holds, `pet/public/shared.html:45`), and the same shape `Welcome.web_bundle` uses here.
/// The reason is a `&'static str` **out of the table**; there is no constructor on this type that can
/// invent one, which is what keeps the es/en pairing honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicAvailability {
    pub available: bool,
    pub reason: Option<&'static str>,
}

impl MicAvailability {
    /// The answer before anything is pressed. `devices_present` is whether `navigator.media_devices()`
    /// exists at all — it is `undefined` in an insecure context, which is a different problem from a
    /// refused permission and gets its own reason (MDN, quoted at `ratzilla-and-wasm-audio.md` §4.1).
    pub fn probe(devices_present: bool, permission: MicState, strings: &tipsy_ui::Strings) -> Self {
        let reason = match (devices_present, permission) {
            (false, _) => Some(strings.mic_no_api),
            (true, MicState::Prompt | MicState::Granted) => None,
            (true, MicState::Denied) => Some(strings.mic_denied),
            (true, MicState::NoDevice) => Some(strings.mic_nodevice),
            (true, MicState::Busy) => Some(strings.mic_busy),
            (true, MicState::Unsupported | MicState::Failed) => Some(strings.mic_failed),
        };
        Self { available: reason.is_none(), reason }
    }

    /// The same answer after a `getUserMedia` rejection, from the exception's `name`.
    pub fn from_rejection(name: &str, strings: &tipsy_ui::Strings) -> Self {
        Self::probe(true, mic_state_for(name), strings)
    }
}

/// Whatever the audio thread hands over, in 320-sample frames.
///
/// The worklet forwards each render quantum untouched, because the quantum is 128 frames and 320 is
/// not a multiple of it (ratzilla-and-wasm-audio.md §4.5) — and because MDN's warning on `process()`
/// is that the size must be checked rather than assumed. The channels arrive as they came; downmixing
/// is `audio/device.rs:186-189`'s job and happens here so that it has a test.
#[derive(Debug, Default)]
pub struct FrameAssembler {
    pending: Vec<f32>,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self { pending: Vec::with_capacity(FRAME_SAMPLES * 2) }
    }

    /// Adds one block (any channel count, any length) and returns every frame it completed.
    pub fn push_block(&mut self, channels: &[Vec<f32>]) -> Vec<Vec<i16>> {
        if channels.is_empty() {
            return Vec::new();
        }
        let width = channels[0].len();
        let count = channels.len() as f32;
        self.pending.extend((0..width).map(|i| {
            channels.iter().map(|channel| channel.get(i).copied().unwrap_or(0.0)).sum::<f32>() / count
        }));
        let mut frames = Vec::new();
        while self.pending.len() >= FRAME_SAMPLES {
            let rest = self.pending.split_off(FRAME_SAMPLES);
            let frame = std::mem::replace(&mut self.pending, rest);
            frames.push(f32_to_i16(&frame));
        }
        frames
    }

    /// What is held back. Only the tests and the frame counter need it.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

/// Linear interpolation to 16 kHz. The twin of `crates/tipsy-compadre/src/audio/pcm.rs:36-55`, copied
/// because this crate must not depend on `tipsy-compadre` (its closure holds tokio, cpal and rodio);
/// the test above pins the same arithmetic.
pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> { /* pcm.rs:36-55 verbatim */ }

/// The twin of `pcm.rs:20-22`, which is also the expression inlined at `audio/device.rs:191`.
pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect()
}

/// The twin of `pcm.rs:28-33`, for the HUD's red envelope.
pub fn rms(samples: &[f32]) -> f32 { /* pcm.rs:28-33 verbatim */ }
```

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
cargo test -j 1 -p tipsy-web mic::
```

Expected: 10 passed, 0 failed. Write `resample_linear` and `rms` out from `pcm.rs` rather than
paraphrasing them — the comment says "the twin of", and a twin that rounds differently is a bug the
server would hear as a pitch shift.

- [ ] **Step 3: The worklet, as its own file**

```js
// web/client/mic-worklet.js
//
// One processor, named "mic-frames". It does exactly two things: it forwards each render quantum to
// the page's Rust half, and it keeps itself alive. The 320-sample boundary is NOT here: 320 is not a
// multiple of the 128-frame quantum, so the accumulator lives in Rust where it has tests
// (crates/tipsy-web/src/mic.rs).
//
// A real file and not a Blob URL: the research note could not verify that `addModule` accepts a Blob
// URL (ratzilla-and-wasm-audio.md §4.4, "UNVERIFIED (single fetcher)"), and this plan does not bet on
// an unverified API. Task 3.6 ships this file at /assets/mic-worklet.js.
class MicFrames extends AudioWorkletProcessor {
  process(inputs) {
    const channels = inputs[0];
    if (channels && channels.length > 0 && channels[0] && channels[0].length > 0) {
      // slice(0) copies: the render thread reuses these arrays on the next call. The block is posted
      // as the array of channels it is; the Rust side downmixes (audio/device.rs:186-189).
      this.port.postMessage(channels.map((channel) => channel.slice(0)));
    }
    return true; // keep the node alive
  }
}

registerProcessor("mic-frames", MicFrames);
```

- [ ] **Step 4: The web-sys features, and the exact calls they unlock**

Add to `crates/tipsy-web/Cargo.toml`'s `[dependencies.web-sys] features`:

```toml
  # Task 3.3: the microphone.
  "Navigator", "MediaDevices", "MediaStream", "MediaStreamConstraints", "MediaTrackConstraints",
  "AudioContext", "AudioContextOptions", "BaseAudioContext", "AudioWorklet", "Worklet",
  "AudioWorkletNode", "AudioWorkletNodeOptions", "MediaStreamAudioSourceNode", "MessagePort",
  "Permissions", "PermissionStatus", "PermissionDescriptor", "PermissionName", "PermissionState",
  # Task 3.3: the gate's own DOM. `disabled` lives on the button element, not on `HtmlElement`.
  "HtmlButtonElement", "CssStyleDeclaration",
```

Every one of those names was checked in `web-sys-0.3.106/Cargo.toml`'s feature list for this task, so
none is a guess. **`Promise`, `Reflect`, `Array`, `Float32Array`, `Uint8Array` and `Function` are
deliberately absent from that list**: they are `js_sys` types, not web-sys features (checked the same
way — no `Promise = ` line exists in web-sys's manifest), and `js-sys` is already a direct dependency
of this crate with no features to switch on. Adding them to the feature array is a manifest error, not
a harmless extra. The signatures that matter, read from the same crate's generated bindings for this
task and worth having in front of you while writing the wiring below:

| Call | Real signature | Where |
|---|---|---|
| `AudioContext::new_with_context_options` | `fn new_with_context_options(context_options: &AudioContextOptions) -> Result<AudioContext, JsValue>` | `gen_AudioContext.rs:135-137` |
| `AudioContextOptions` | `new()`, `sample_rate(&mut self, val: f32) -> &mut Self` | `gen_AudioContextOptions.rs:80`, `:91` |
| `AudioContext::sample_rate` | `fn sample_rate(this: &AudioContext) -> f32` (read-only: the real rate) | `gen_AudioContext.rs:69` |
| `AudioContext::resume` | `fn resume(this: &AudioContext) -> Result<js_sys::Promise, JsValue>` | `gen_AudioContext.rs:688` |
| `AudioContext::create_media_stream_source` | `fn create_media_stream_source(this: &AudioContext, media_stream: &MediaStream) -> Result<MediaStreamAudioSourceNode, JsValue>` | `gen_AudioContext.rs:188-191` |
| `AudioContext::audio_worklet` | `fn audio_worklet(this: &AudioContext) -> Result<AudioWorklet, JsValue>` | `gen_AudioContext.rs:106` |
| `Worklet::add_module` | `fn add_module(this: &Worklet, module_url: &str) -> Result<js_sys::Promise, JsValue>` | `gen_Worklet.rs:25` |
| `AudioWorkletNode::new_with_options` | `fn new_with_options(context: &BaseAudioContext, name: &str, options: &AudioWorkletNodeOptions) -> Result<AudioWorkletNode, JsValue>` | `gen_AudioWorkletNode.rs:82-86` |
| `AudioWorkletNodeOptions` | `new()`, `number_of_inputs(u32)`, `number_of_outputs(u32)` | `gen_AudioWorkletNodeOptions.rs:104`, `:127`, `:132` |
| `AudioWorkletNode::port` | `fn port(this: &AudioWorkletNode) -> Result<MessagePort, JsValue>` | `gen_AudioWorkletNode.rs:42` |
| `MessagePort::set_onmessage` | `fn set_onmessage(this: &MessagePort, value: Option<&js_sys::Function>)` | `gen_MessagePort.rs:33` |
| `MediaDevices::get_user_media_with_constraints` | `fn get_user_media_with_constraints(this: &MediaDevices, constraints: &MediaStreamConstraints) -> Result<js_sys::Promise, JsValue>` | `gen_MediaDevices.rs:81` |
| `Permissions::query` | `fn query(this: &Permissions, permission: &js_sys::Object) -> Result<js_sys::Promise, JsValue>` | `gen_Permissions.rs:25-28` |
| `PermissionStatus::state` | `fn state(this: &PermissionStatus) -> PermissionState` | `gen_PermissionStatus.rs:27` |
| `Float32Array` | `new(constructor_arg: &JsValue)`, `to_vec()`, `length()` | `js-sys-0.3.106/src/lib.rs:13903`, `:14163`, `:13993` |
| `Reflect::set` / `Reflect::get` | `fn set(target: &JsValue, property_key: &JsValue, value: &JsValue) -> Result<bool, JsValue>` | `js-sys-0.3.106/src/lib.rs:7775-7779`, `:7638` |

**One finding changes the shape of the permission probe, and it is this task's to write down.**
`Permissions::query` takes **`&js_sys::Object`**, not a `PermissionDescriptor`, and
`web_sys::PermissionName` in 0.3.106 has **four** variants — `Geolocation`, `Notifications`, `Push`,
`PersistentStorage` (`gen_PermissionName.rs`, the whole file) — with **no `Microphone`**. So the
descriptor cannot be built through the typed struct at all, and the object is built by hand; the
research note's "UNVERIFIED (single fetcher)" guess that `PermissionDescriptor` needs `Reflect`/
`Object` construction turns out to be right for a reason the note did not have:

```rust
/// `{ name: "microphone" }`, built by hand because `web_sys::PermissionName` has no `Microphone`
/// variant in 0.3.106 — checked in the crate's own generated bindings (`gen_PermissionName.rs`), and
/// `Permissions::query` takes a `&js_sys::Object` anyway (`gen_Permissions.rs:25-28`).
fn microphone_descriptor() -> js_sys::Object {
    let descriptor = js_sys::Object::new();
    js_sys::Reflect::set(
        &descriptor,
        &JsValue::from_str("name"),
        &JsValue::from_str("microphone"),
    )
    .expect("setting a property on a fresh object cannot fail");
    descriptor
}
```

- [ ] **Step 5: The gate and the capture**

The v1 spec requires the press: "In the browser, audio capture requires a user gesture; the web client
shows a 'press to start' screen until then" (`2026-10-10-tipsy-compadre-design.md:193-194`). The MDN
autoplay rule the note quotes makes it load-bearing for *both* halves: the click must call
`getUserMedia` **and** create/resume the `AudioContext` in the same handler, or the user gets a silent
client that looks started (`ratzilla-and-wasm-audio.md` §5.2). The gate is built from Rust, so the page
Task 3.2 created needs no change for it.

```rust
/// What the gate needs and what the pipeline keeps. The context is exposed because Task 3.4 plays the
/// reply through this same one: a second context would be a second clock and a second thing to resume.
pub struct Capture {
    context: web_sys::AudioContext,
    stream: web_sys::MediaStream,
    node: web_sys::AudioWorkletNode,
    /// Kept alive for as long as the node: dropping it detaches the port's handler.
    on_message: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::MessageEvent)>,
    assembler: FrameAssembler,
}

impl Capture {
    pub fn context(&self) -> &web_sys::AudioContext {
        &self.context
    }
}

/// The gate's DOM: a button, a reason line and the overlay that holds them. Built here rather than in
/// markup, so the page Task 3.2 created needs no change for it. `HtmlButtonElement` is the type that
/// owns `disabled` (`web-sys-0.3.106/src/features/gen_HtmlButtonElement.rs:56`); `set_onclick` comes
/// from `HtmlElement` (`gen_HtmlElement.rs:471`) and `set_text_content` from `Node`
/// (`gen_Node.rs:134`), both through `Deref`.
struct Gate {
    overlay: web_sys::Element,
    button: web_sys::HtmlButtonElement,
    reason: web_sys::Element,
}

fn gate_elements(document: &web_sys::Document, strings: &tipsy_ui::Strings) -> Option<Gate> {
    let body = document.body()?;
    let overlay = document.create_element("div").ok()?;
    overlay.set_attribute("id", "tipsy-gate").ok()?;
    let button = document
        .create_element("button")
        .ok()?
        .dyn_into::<web_sys::HtmlButtonElement>()
        .ok()?;
    button.set_text_content(Some(strings.mic_start));
    // The line starts as the hint and becomes the reason if the probe says the microphone cannot
    // work: one element, so the two can never be on screen together.
    let reason = document.create_element("p").ok()?;
    reason.set_text_content(Some(strings.mic_hint));
    overlay.append_child(&button).ok()?;
    overlay.append_child(&reason).ok()?;
    body.append_child(&overlay).ok()?;
    Some(Gate { overlay, button, reason })
}

/// The gate. Capability first: the page finds out whether it *can* record before it offers to, and an
/// unavailable microphone leaves a **disabled** button and a reason rather than a button that fails on
/// the first press (the prior art's shape, `codewhale-audio-prior-art.md` §4).
pub fn press_to_start(state: Rc<RefCell<tipsy_ui::UiState>>, outbox: Outbox) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(gate) = gate_elements(&document, state.borrow().strings()) else {
        return;
    };

    let state_probe = state.clone();
    let state_click = state.clone();
    let outbox_click = outbox.clone();
    let document_click = document.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let availability = probe_availability(&state_probe).await;
        let Some(reason) = availability.reason else {
            // Available: the button becomes the one gesture everything audio needs. The context is
            // created and resumed **inside** this handler, and `getUserMedia` is called from it too —
            // the rule the research note quotes from MDN (ratzilla-and-wasm-audio.md §5.2) and the one
            // the prior art's shipped page implements (codewhale-audio-prior-art.md §3,
            // `pet/src/ui/pet.ts:173`).
            let on_click = Closure::wrap(Box::new(move || {
                let state = state_click.clone();
                let outbox = outbox_click.clone();
                let document = document_click.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    match open_capture(&state, &outbox).await {
                        Ok(capture) => {
                            let granted = state.borrow().strings().mic_granted;
                            state.borrow_mut().notify(granted);
                            if let Some(gate) = document.get_element_by_id("tipsy-gate") {
                                gate.remove();
                            }
                            // The capture outlives this handler: it holds the context Task 3.4 plays
                            // the reply through.
                            std::mem::forget(capture);
                        }
                        // The failure arrives as data too, with the same shape the probe returned.
                        Err(name) => {
                            // `Strings` is not `Copy`, and it need not be: `strings()` hands back a
                            // `&'static Strings`, so the borrow of `state` ends with this statement.
                            let strings = state.borrow().strings();
                            let availability = MicAvailability::from_rejection(&name, strings);
                            let text = availability.reason.unwrap_or(strings.mic_failed);
                            state.borrow_mut().notify(text);
                        }
                    }
                });
            }) as Box<dyn FnMut()>);
            gate.button.set_onclick(Some(on_click.as_ref().unchecked_ref()));
            on_click.forget();
            return;
        };
        // Not available: the affordance stays on screen, disabled, with the reason as its text.
        gate.button.set_disabled(true);
        gate.reason.set_text_content(Some(reason));
    });
}

/// Reads the two things the capability answer is made of: whether `navigator.mediaDevices` exists at
/// all, and what the microphone's permission state is. `PermissionStatus.state` is
/// `granted | denied | prompt`; `query` rejects with a `TypeError` for a name the browser does not
/// know, which is the `UnknownError` path and therefore `MicState::Failed`'s reason
/// (`ratzilla-and-wasm-audio.md` §4.2). The descriptor is built by hand — `microphone_descriptor`
/// above — because `web_sys::PermissionName` has no `Microphone` variant.
async fn probe_availability(state: &Rc<RefCell<tipsy_ui::UiState>>) -> MicAvailability {
    // `strings()` returns a `&'static Strings`, so the borrow of `state` ends with this statement and
    // the awaits below are free to hold no borrow at all.
    let strings = state.borrow().strings();
    let Some(window) = web_sys::window() else {
        return MicAvailability::probe(false, MicState::Failed, strings);
    };
    // `media_devices()` is the presence check MDN describes: `undefined` in an insecure context.
    if window.navigator().media_devices().is_err() {
        return MicAvailability::probe(false, MicState::Unsupported, strings);
    }
    let permission = match window.navigator().permissions() {
        Ok(permissions) => match permissions.query(&microphone_descriptor()) {
            Ok(promise) => match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(status) => {
                    // `JsCast` is already in scope in this module (`use wasm_bindgen::JsCast;`).
                    let status: web_sys::PermissionStatus = status.unchecked_into();
                    match status.state() {
                        web_sys::PermissionState::Granted => MicState::Granted,
                        web_sys::PermissionState::Denied => MicState::Denied,
                        // `PermissionState` is a generated enum with a wildcard arm for exactly this
                        // reason; "prompt" and anything a newer browser adds mean "ask the user".
                        _ => MicState::Prompt,
                    }
                }
                Err(_) => MicState::Failed,
            },
            Err(_) => MicState::Failed,
        },
        // Firefox in particular has no `navigator.permissions` for every build; a browser that cannot
        // answer is not a browser that cannot record, so this is `Prompt`, not a refusal.
        Err(_) => MicState::Prompt,
    };
    MicAvailability::probe(true, permission, strings)
}
```

Two details of that code that are deliberate and worth a line each:

- **`Granted` and `Prompt` both leave the gate on screen.** The button is not only the permission
  prompt; it is the user gesture the `AudioContext` needs, so a page whose permission is already
  granted still gets its gesture from the same click. The only visible difference is whether the
  browser shows a prompt.
- **The imports this module needs beyond Task 3.2's**: `wasm_bindgen::closure::Closure`,
  `wasm_bindgen::JsCast`, `wasm_bindgen_futures::{spawn_local, JsFuture}` and
  `tipsy_protocol::{Frame, ToServer}`. Every call is in Step 4's table or is a `web_sys`/`js_sys` type
  this crate already depends on; nothing new is added.

The mechanical remainder, in prose because it is a sequence of the calls in Step 4's table and nothing
else decides anything. Everything from item 1 on is `open_capture`'s body, the `async fn` the gate's
click handler calls; it returns `Result<Capture, String>` where the `Err` is the exception's `name`,
which is what `MicAvailability::from_rejection` consumes.

1. `open_capture` creates the context **first**, inside the gesture:
   `AudioContext::new_with_context_options(AudioContextOptions::new().sample_rate(MIC_RATE as f32))`.
   A `NotSupportedError` from a refused rate must not be fatal: fall back to
   `AudioContext::new()` and let the resampler do the work. `resume()` is called on the same context
   and awaited, because a context created outside a gesture starts `suspended`.
2. `navigator.media_devices()` → `get_user_media_with_constraints(&constraints)` where the constraints
   are mono and the browser's own processing is left on: `MediaStreamConstraints::new().audio(&track)`
   with a `MediaTrackConstraints::new().channel_count(&1.into())`. **The 16 kHz hint cannot travel
   here**: `web-sys 0.3.106`'s `MediaTrackConstraints` has no `sample_rate` accessor (checked in
   `gen_MediaTrackConstraints.rs` for this task — zero matches), which matches MDN's "Limited
   availability ... not Baseline" flag on `sampleRate` that the note quotes. The rate is settled by the
   context and the resampler instead.
3. `ctx.audio_worklet()?.add_module(WORKLET_URL)` and await it. On failure, report
   `strings.mic_failed` and leave recording off. `add_module` rejects with `AbortError` or
   `SyntaxError` (MDN, §4.4).
4. `AudioWorkletNode::new_with_options(&ctx, "mic-frames", AudioWorkletNodeOptions::new()
   .number_of_inputs(1).number_of_outputs(0))`, then `ctx.create_media_stream_source(&stream)?.connect_with_audio_node(&node)?`.
   Zero outputs: nothing is played back into the microphone's own graph.
5. `node.port()?.set_onmessage(Some(closure))`, where the closure converts the `js_sys::Array` of
   channels exactly as the note's INFERRED idiom says and the crate's bindings confirm —
   `js_sys::Array::from(&event.data())`, then `js_sys::Float32Array::new(&channel).to_vec()`.
6. The rate read-back, once, at capture time: `let rate = ctx.sample_rate() as u32;`. If it is not
   `MIC_RATE`, every block goes through `resample_linear(&mono, rate, MIC_RATE)`; the assembler is
   rate-agnostic and works in 16 kHz samples either way. **Read it back rather than assuming it**:
   "sample-rate converters are not supported" inside one context, so the context runs at one rate for
   everything, and that rate is the browser's answer, not ours (§4.3).
7. Per frame: `outbox.send(ToServer::Audio(Frame::Mic(samples.clone())))` and
   `state.borrow_mut().push_mic_level(rms(&i16_to_f32(&samples)))`. The native client multiplies by
   `mic_gain` at the same point (`client.rs:780`); the browser's `mic_gain` arrives with the
   configuration over the wire (W3), so it is applied by Task 3.5's HUD glue, not here.
8. If the port's handler ever sees a block whose channels are empty, do nothing — the worklet already
   guards against it, and the assembler's `pending` is the only state that matters.

- [ ] **Step 6: The seven strings, in both languages**

```rust
    /// The gate's button. v1 spec §6 `:193-194`.
    pub mic_start: &'static str,
    /// The line beside the button while the microphone can be offered.
    pub mic_hint: &'static str,
    /// `navigator.mediaDevices` is not there at all: an insecure context (MDN, quoted at
    /// ratzilla-and-wasm-audio.md §4.1). This is a capability reason, not a permission one, and it is
    /// the one that tells the user to reach the page on loopback or over a certificate the browser
    /// trusts (spec §9.5).
    pub mic_no_api: &'static str,
    /// Shown once the microphone is open and the frames are flowing.
    pub mic_granted: &'static str,
    /// v1 spec §12 `:418`: "Listening disabled with a warning; typing still works".
    pub mic_denied: &'static str,
    pub mic_nodevice: &'static str,
    pub mic_busy: &'static str,
    /// The rest: the worklet refused to load, the browser ignored the constraints, or a transient
    /// rejection. One message, because the user's next move is the same for all three.
    pub mic_failed: &'static str,
```

Every one of these is the **reason value** `MicAvailability` hands back — `mic_no_api`, `mic_denied`,
`mic_nodevice`, `mic_busy` and `mic_failed` are the five reasons; `mic_start` and `mic_hint` are the
gate's own copy and `mic_granted` is the success notice. Nothing at a call site builds one of them:
`press_to_start` writes `reason` straight into the reason line's text, which is what keeps the two
tables authoritative (the prior art's machine-readable `reason`, §4).

```rust
    mic_start: "Pulsa para empezar",
    mic_hint: "El navegador pedirá permiso para el micrófono",
    mic_no_api: "Este navegador no da acceso al micrófono aquí: abre la página en localhost o por HTTPS.",
    mic_granted: "Micrófono abierto",
    mic_denied: "Micrófono denegado: revisa el permiso del sitio. Puedes escribir.",
    mic_nodevice: "No hay micrófono conectado. Puedes escribir.",
    mic_busy: "El micrófono está ocupado por otra aplicación. Puedes escribir.",
    mic_failed: "No se pudo abrir el micrófono. Puedes escribir.",
```

```rust
    mic_start: "Press to start",
    mic_hint: "The browser will ask for the microphone",
    mic_granted: "Microphone open",
    mic_denied: "Microphone denied: check the site's permission. You can still type.",
    mic_nodevice: "No microphone connected. You can still type.",
    mic_busy: "The microphone is in use by another application. You can still type.",
    mic_failed: "The microphone could not be opened. You can still type.",
```

`MicAvailability` is what the gate acts on, and its reasons are values: no `navigator.mediaDevices` →
`mic_no_api`, `Denied → mic_denied`, `NoDevice → mic_nodevice`, `Busy → mic_busy`, and
`Unsupported | Failed → mic_failed`. When the probe answers `available: false` the button is **disabled
with the reason on screen and no click handler at all**, and when the answer is `available: true` the
gate is the gesture and the capture opens on the click — the spec's sentence is the implementation:
listening is disabled, typing is not (v1 spec §12 `:418`).

One hand-over to name: **Task 3.5 owns the page's look.** If it restyles the gate's element or moves it
into the page's own markup, the *decision* stays here — `MicAvailability` is the value and
`MicAvailability::probe` is the only thing that produces it — so the two cannot drift apart. The prior
art's import table puts this shape in Tasks 3.3 **and** 3.5 for exactly this reason
(`codewhale-audio-prior-art.md` §4).

- [ ] **Step 7: Build, test, lint**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
cargo test -j 1 -p tipsy-web
cargo build -j 1 -p tipsy-web --target wasm32-unknown-unknown --release
cargo clippy -j 1 -p tipsy-web --all-targets
```

Expected: the tests pass (Task 3.2's 7 plus this task's 10), the wasm target builds, clippy adds nothing
to `global-constraints.md`'s pinned list, and `cargo fmt` is not run.

- [ ] **Step 8: The browser half — a documented manual check, runnable now**

This half needs no server, because loopback is already a secure context (`web-protocol` note §4.4:
"Documents are secure contexts when ... delivered from a loopback (local) address"; spec §9.5), so
`http://127.0.0.1:8001/` can call `getUserMedia` with no TLS at all. The socket *will* fail in this
check — the page is served by `python`, which has no `/ws` — so the `web_offline` notice from Task 3.2
is expected and is not a finding.

1. Build the layout the bundle will have, so `/assets/mic-worklet.js` resolves exactly as it will in
   production:

```powershell
$env:CARGO_TARGET_DIR="$PWD\.target-sandbox"
cargo build -j 1 -p tipsy-web --target wasm32-unknown-unknown --release
New-Item -ItemType Directory -Force -Path '.target-sandbox\web\assets' | Out-Null
wasm-bindgen --target web --out-dir .target-sandbox\web `
  .target-sandbox\wasm32-unknown-unknown\release\tipsy_web.wasm
Copy-Item web\client\index.html .target-sandbox\web\index.html
Copy-Item web\client\mic-worklet.js .target-sandbox\web\assets\mic-worklet.js
python -m http.server 8001 --directory .target-sandbox\web
```

2. Open `http://127.0.0.1:8001/` with the console visible. **Expected:** the HUD is drawn (Task 3.2's
   loop) and the gate is on screen with the `mic_start` button and the `mic_hint` line. The console may
   show the socket failing — that is the missing `/ws`, not the microphone.
3. **The unavailable path, which is the one the capability-first shape exists for.** Find this
   machine's LAN address (`ipconfig`, the IPv4 of the active adapter) and open
   `http://<lan-ip>:8001/` in the same browser. **Expected:** the page loads and the HUD draws, and the
   gate is on screen with a **disabled** button and `mic_no_api` as its reason — **no permission prompt
   appears, and pressing the button does nothing**. This is MDN's second documented half: past
   loopback over plain HTTP the context is insecure and `navigator.mediaDevices` is `undefined`
   (`ratzilla-and-wasm-audio.md` §4.1), so the page never asks. If a prompt appears here, the probe is
   wrong and this path is a finding.
4. **The prompt path.** Back on `http://127.0.0.1:8001/`, click the button and choose **Block** in the
   browser's permission prompt. **Expected:** the prompt appears (the click is the gesture), the gate
   is replaced by the `mic_denied` notice, `state.mic_open` stays false, and typing in the HUD still
   works — type a letter and watch it land in the ask box, which is the spec's "typing still works"
   (§12 `:418`).
5. Reload and check the state detection without a prompt: with the mic already blocked for the site,
   reload. **Expected:** the gate is on screen with the button **disabled** and `mic_denied` as its
   reason, and no prompt appears even if you click — that is
   `permissions.query({name:"microphone"})`'s `denied`, read before any `getUserMedia` call. This is
   the step that distinguishes capability-first from a button that fails on first use.
6. **The granted path.** Reset the site permission to "Ask", reload, click the button, and **Allow**.
   **Expected:** the gate disappears, `mic_granted` appears, and the HUD's red oscilloscope (`VOZ`
   panel with `mic_open`) moves when you speak or tap the microphone. In the console, a per-frame line
   you add (log one frame in 25) shows `mic frame len=320 rate=16000` — **320, always**, whatever
   `AudioContext::sample_rate()` answered. Record that number from the console: it is the one
   observation that proves the 128-vs-320 boundary is handled, and it is the core/presenter split's
   "same output at any cadence" invariant made visible.
7. **The read-back path.** Log the context's real rate from the Rust side in the same frame line:
   `rate=<ctx.sample_rate()>`. **Expected:** on a machine whose device runs at 48 kHz the log shows
   `rate=48000` with the frames still `len=320`; a hard-coded 16 kHz assumption would show `len=960`
   here, which is the bug this step exists to catch.
8. **The `NotFoundError` path** (only if the machine can do it: disable the input device, or use a
   browser profile with no capture device). **Expected:** `mic_nodevice`, not `mic_denied` — the
   distinction the note insists on (`§4.1` INFERRED), and the reason it is a value rather than one
   "microphone unavailable" string.
9. Record in the ledger: the browser and its version, what each of steps 3–8 produced, the `len=320`
   observation, the read-back rate, and whether the worklet file loaded from `/assets/mic-worklet.js`
   (the console's network tab shows the request; a 404 there is the one thing that would falsify the
   bundle path assumption in Step 1).

- [ ] **Step 9: Commit**

```
feat(web): capture the browser microphone into 16 kHz frames

mic.rs holds the pure half — the eight getUserMedia rejection names mapped
to the states the gate shows, the capability answer those states make
(MicAvailability, whose reason is always a value out of the two string
tables), the 320-sample frame assembler, the resampler and the sample
conversion, each a twin of its native counterpart with a test — and the
WebAudio wiring behind a "press to start" gate that is disabled with its
reason when the microphone cannot work, instead of failing on the first
press. The gate is the one place getUserMedia and the AudioContext are
created, because both need a user gesture. 320 is not a multiple of the
render quantum, so the worklet only forwards and Rust owns the boundary; the
context's real rate is read back and resampled when it is not 16 kHz. The
frames leave as the existing 0x01 binary frames, unchanged.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
```

`git add` exactly `crates/tipsy-web/`, `crates/tipsy-ui/src/strings.rs` and
`web/client/mic-worklet.js`. The `web/client/index.html` this task's manual check copies is unchanged
by this task; if the check needs an edit to it, that edit belongs to Task 3.5's commit.

### Task 3.4: Playback, the jitter buffer and barge-in `[mid]`

**Files:**
- Create: `crates/tipsy-web/src/playback.rs` (the pure timeline, the envelope and the barge-in rule), with its own `#[cfg(test)] mod tests`
- Create: `crates/tipsy-web/src/web_audio.rs` (the `AudioContext` half: one `AudioBufferSourceNode` per chunk, `start(when)`, `stop()` + `disconnect()`; the whole file `#[cfg(target_arch = "wasm32")]`)
- Modify: `crates/tipsy-web/src/lib.rs` (the two `mod` lines and the routing of the socket's `ToClient` values into the timeline)
- Modify: `crates/tipsy-web/Cargo.toml` (the wasm-only dependencies move under `[target.'cfg(target_arch = "wasm32")'.dependencies]`)
- Test: `crates/tipsy-web/src/playback.rs` (its own `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `tipsy_protocol::{AssistantState, Command, Event, Frame}` — `Frame::Speech { seq: u32, samples: Vec<i16> }` (`crates/tipsy-protocol/src/lib.rs:179`), `Event::SpeechChunk { seq, sample_rate }` (`:151`), `Event::Done { .. }` (`:152`), `Event::Status { state, .. }` (`:146`), `Event::Heard { .. }` (`:147`), `Frame::decode(&[u8]) -> Result<Frame, FrameError>` (`:227`); `tipsy_ui::UiState::start_voice(Vec<f32>)` (`crates/tipsy-ui/src/state.rs:511`), whose hop is `ENVELOPE_HOP_MS = 50` (`state.rs:14`, `render.rs:32`); Task 3.1's recorded verdict (this task does not run if it was FAIL) and its `rustup target add wasm32-unknown-unknown`; Task 3.2's `tipsy_web::start()` and the decoded `ToClient` queue its socket fills; Task 3.3's `AudioContext`, created and resumed inside the press-to-start handler (research note §5.2).
- Produces: `tipsy_web::playback::{Playback, Scheduled, CUSHION_S}` and `tipsy_web::playback::{starts_turn, interrupts_playback}`; `Playback::{new, rate, set_rate, chunk, end_of_turn, barge_in, finished, replay, forget_reply, envelope, reply, gaps, is_playing}`; `tipsy_web::web_audio::{play, silence, forget}` (wasm only).

- [ ] **Step 1: Read Task 3.1's verdict, and stop if it was FAIL**

Read `.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md` and find the line the Task 3.1 implementer appended with the spike's verdict, the backend it chose and the font its page used. Spec §8.1 makes that verdict a gate, not a formality: "**If it fails, Phase 3 stops there**, the remaining Phase 3 tasks are abandoned, and the delivered answer to 'the browser' is the web terminal" (`docs/superpowers/specs/2026-10-11-web-tier-design.md:384-388`).

- Verdict **FAIL**: do not run this task. Append `Task 3.4: abandoned (Task 3.1 FAIL)` to the ledger, report it to the operator, and stop.
- Verdict **PASS**: continue, and keep the backend name — Step 6 needs it (`WebGl2Backend` is the one the published table recommends; note §2b, note §8 item 3).

Confirm the wasm target exists before any step that compiles for wasm (Task 3.1 installed it; this request cannot be assumed green):

```
rustup target list --installed
```

Expected: a line `wasm32-unknown-unknown`. If it is absent, Task 3.1's gate did not actually run: stop and report, do not run `rustup target add` here (it is Task 3.1's step and its own record).

- [ ] **Step 2: Write the failing tests, and watch them fail**

Create `crates/tipsy-web/src/playback.rs` with the module documentation, the public shapes and `todo!()` bodies, and the complete test module below. The tests are the deliverable's specification: the timeline arithmetic, the duplicate and gap decision, the envelope and the barge-in rule are all pure and all run on this host — no browser is involved anywhere in this file.

```rust
//! Browser playback of the server's speech, as a timeline the tests can read.
//!
//! The wire is one binary frame per spoken **sentence**, announced by the `SpeechChunk` that precedes
//! it (`crates/tipsy-protocol/src/lib.rs:151`, `:178`; `brain.rs:454-455` sends the pair). `seq` counts
//! those sentences inside one turn and restarts at 0 for the next one (`brain.rs:392-393`, `:198`), so
//! this module owns a timeline rather than a queue: every chunk is placed after the previous one, which
//! is what rodio's `append` does on the native side (`audio/device.rs:53-54`).
//!
//! **A pure core and a thin presenter.** This file is the core: the anchor, the cursor arithmetic, the
//! gap decision, the clamp that keeps every chunk out of the past, the envelope and the barge-in reset,
//! driven by
//! `(seq, samples, now)` and nothing else. The `AudioBufferSourceNode`s are `web_audio.rs`'s and only
//! present the PCM this file schedules. That split is the shape the prior-art review names, quoting the
//! working browser audio it read (`codewhale-audio-prior-art.md` §3, from
//! `pet/src/core/pet-audio.ts:10-12`):
//!
//! > Core emits score events; WebAudio / AVAudioEngine only present their PCM. Calling at any display
//! > cadence produces the same score when all world ticks are supplied. Calling twice for a world tick
//! > cannot retrigger a voice.
//!
//! Both of those sentences are requirements here, and both have a test: a chunk sequence scores the
//! same however often the presenter asks (`the_same_chunks_score_the_same_at_any_cadence`) and a frame
//! the socket delivers twice is scheduled once (`a_frame_delivered_twice_cannot_schedule_twice`).
//!
//! **The four scheduling details the prior-art review pins, and what this wire does with each**
//! (`codewhale-audio-prior-art.md` §3, which quotes `pet/src/ui/pet.ts`):
//!
//! 1. *"Skip a buffer that has entirely passed (`:43`) instead of scheduling into the past."* **Taken,
//!    as a property rather than a branch**: nothing this file returns is ever in the past — every chunk
//!    is placed at `max(cursor, now + CUSHION_S)` — so there is no past chunk and nothing to skip. The
//!    `:43` guard exists because the pet's anchor is fixed at sound-on and a late frame lands behind it;
//!    this timeline is anchored per turn and clamped, which is the same guarantee with no dead branch.
//!    `a_late_chunk_is_placed_now_plus_the_cushion_and_is_never_truncated` pins it.
//! 2. *"A partly-past buffer starts at `currentTime` with a non-zero offset into itself (`:45`)."*
//!    **Not applied, deliberately, and this is the one place this wire parts company with the idiom.**
//!    The pet's voices are notes at absolute positions in a score, with other voices to stay in time
//!    with, so entering a note mid-way is right. A reply here is a single stream of whole sentences
//!    synthesized one after another, and `cursor < now` has exactly one cause: the sentence took longer
//!    to arrive than the previous one takes to play. Starting it at an offset would silently eat its
//!    first words, and skipping it would eat the sentence — a slow speech provider must never cost the
//!    user words. What the offset protects against (hearing something twice) is already impossible:
//!    a chunk is scheduled at most once (`:last_seq`), a slot on the timeline is claimed once
//!    (`:cursor`), and the two things that do replay audio are deliberate (`Replay`) or a new turn
//!    (`end_of_turn`, whose `seq` restarts at 0 by construction). The test above
//!    (`a_late_chunk_is_placed_now_plus_the_cushion_and_is_never_truncated`) is the one that asserts the
//!    whole sentence is still there, and the admission is written down here rather than left for a
//!    reviewer to find: if a future plan gives this client a fixed anchor and several synchronised
//!    streams, this is the line to revisit.
//! 3. *"A single `Set` of live nodes with `onended` cleanup (`:26`, `:44`) and one `silence()` that stops
//!    them all (`:33`)."* **Taken**, with `onended`'s cleanup moved out of the browser — see
//!    `web_audio.rs`, and [`Playback::finished`] for what replaces it and why.
//! 4. *"An explicit lookahead anchor (`:71`) rather than 'now'."* **Taken verbatim**: [`CUSHION_S`] is
//!    the prior art's own 80 ms, and a turn's first chunk is placed that far ahead of the context's
//!    clock.
//!
//! Nothing here touches a browser type: this file compiles and its tests run on this host.

use tipsy_protocol::Command;

/// How far in front of `now` a turn's first chunk is placed.
///
/// Scheduling at "now" on every frame is what makes a jitter buffer stutter, and this is the one number
/// the prior art sets explicitly: `anchor = audio.currentTime - seconds + .08`
/// (`pet/src/ui/pet.ts:71`, quoted in `codewhale-audio-prior-art.md:97-98`). 80 ms is comfortably more
/// than a frame's own jitter and far too little to be heard as a delay before the first word.
pub const CUSHION_S: f64 = 0.08;

/// One chunk the presenter must hand to a fresh `AudioBufferSourceNode`.
#[derive(Debug, Clone, PartialEq)]
pub struct Scheduled {
    /// Identifies the node for the presenter's live-node set and for `barge_in`.
    pub id: u64,
    /// Where it starts, in `AudioContext.currentTime` seconds. Never in the past.
    pub at: f64,
    /// `Some(seq)` for a chunk off the wire; `None` for a `Replay`.
    pub seq: Option<u32>,
    /// The rate in force when it was scheduled, which is the rate the buffer must be created at.
    pub rate: u32,
    pub samples: Vec<i16>,
}

/// A chunk on the timeline that has not finished yet: what `barge_in` has to stop.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Live {
    id: u64,
    ends_at: f64,
}

pub struct Playback {
    /// The rate announced by the last `SpeechChunk`; `24_000` until one arrives, the native default
    /// (`client.rs:101`).
    rate: u32,
    /// The next start time on this turn's timeline. `None` until a turn's first chunk anchors it, and
    /// `None` again after every turn boundary and every barge-in.
    cursor: Option<f64>,
    /// The `seq` of the last chunk accepted this turn: the duplicate guard.
    last_seq: Option<u32>,
    /// Sentences the server never delivered this turn (a forward sequence gap).
    gaps: u32,
    next_id: u64,
    live: Vec<Live>,
    /// Everything received this turn, which is what `Replay` plays and what the envelope is built from
    /// (`client.rs:257`, `:271`).
    reply: Vec<i16>,
}

impl Playback {
    pub fn new() -> Self { todo!() }
    pub fn rate(&self) -> u32 { todo!() }
    pub fn set_rate(&mut self, rate: u32) { todo!() }
    pub fn chunk(&mut self, seq: u32, samples: Vec<i16>, now: f64) -> Option<Scheduled> { todo!() }
    pub fn end_of_turn(&mut self) { todo!() }
    pub fn barge_in(&mut self, now: f64) -> Vec<u64> { todo!() }
    pub fn finished(&mut self, now: f64) -> Vec<u64> { todo!() }
    pub fn replay(&mut self, now: f64) -> Option<Scheduled> { todo!() }
    pub fn forget_reply(&mut self) -> bool { todo!() }
    pub fn envelope(&self) -> Vec<f32> { todo!() }
    pub fn reply(&self) -> &[i16] { todo!() }
    pub fn gaps(&self) -> u32 { todo!() }
    pub fn is_playing(&self, now: f64) -> bool { todo!() }
}

/// Whether a command starts the next turn, which is the native client's rule (`client.rs:619-640`) and
/// the reason a new reply's audio may not be queued behind the previous one.
pub fn starts_turn(command: &Command) -> bool { todo!() }

/// Whether a command quiets the device: the user is taking the floor (`client.rs:658-660`).
pub fn interrupts_playback(command: &Command) -> bool { todo!() }

#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::{AssistantState, Capability, ClientKind, Event, ListeningMode};

    /// 24 000 samples at 24 kHz is one second: the arithmetic every test below relies on.
    fn samples(ms: u64, rate: u32) -> Vec<i16> {
        vec![0i16; (rate as u64 * ms / 1000) as usize]
    }

    fn sine(freq: f32, rate: u32, ms: u32) -> Vec<i16> {
        let n = (rate as u64 * ms as u64 / 1000) as usize;
        (0..n)
            .map(|i| {
                let s = (i as f32 * freq * std::f32::consts::TAU / rate as f32).sin() * 0.5;
                (s * 32767.0) as i16
            })
            .collect()
    }

    #[test]
    fn the_first_chunk_is_anchored_a_cushion_ahead_and_the_rest_follow_it_gaplessly() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let first = p.chunk(0, samples(1000, 24_000), 10.0).expect("the first chunk plays");
        assert_eq!(first.at, 10.0 + CUSHION_S, "the anchor is `now` plus the cushion, never `now` itself");
        assert_eq!(first.seq, Some(0));
        let second = p.chunk(1, samples(500, 24_000), 10.1).expect("the second chunk plays");
        assert_eq!(second.at, 11.0 + CUSHION_S, "the second starts exactly where the first ends: no gap, no overlap");
        let third = p.chunk(2, samples(500, 24_000), 10.2).expect("the third chunk plays");
        assert_eq!(third.at, 11.5 + CUSHION_S);
    }

    /// The prior art's `:43` guard, as a property of this timeline rather than a branch in it: no chunk
    /// is ever placed in the past, however late it arrives, and a late one is not truncated either.
    #[test]
    fn a_late_chunk_is_placed_now_plus_the_cushion_and_is_never_truncated() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        p.chunk(0, samples(500, 24_000), 10.0).expect("plays"); // 10.08 .. 10.58
        // The frame loop stalled and this chunk arrives long after its slot would have been: a sentence
        // whose synthesis took longer than the previous one takes to play. Its audio is whole and its
        // first words matter, so it is placed ahead of `now` and played from its start.
        let late = p.chunk(1, samples(1000, 24_000), 12.0).expect("plays");
        assert_eq!(late.at, 12.0 + CUSHION_S, "in the future, never in the past: `start(when)` with a past `when` plays at once");
        assert_eq!(late.samples.len(), 24_000, "and not one sample of it is dropped (the offset the prior art uses is rejected above)");
        let next = p.chunk(2, samples(500, 24_000), 12.1).expect("plays");
        assert_eq!(next.at, 13.0 + CUSHION_S, "the timeline continues from the new anchor, gaplessly");
        assert_eq!(p.gaps(), 0, "a late chunk is not a gap: nothing was missing from the wire");
    }

    /// The prior art's second invariant, in this wire's terms: a frame the socket delivers twice (a
    /// re-sent batch, a caller that drains its queue twice) must not schedule a second buffer. A
    /// replayed buffer is audible corruption, and this is the test that pins it.
    #[test]
    fn a_frame_delivered_twice_cannot_schedule_twice() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let first = p.chunk(0, samples(1000, 24_000), 10.0).expect("plays");
        assert!(p.chunk(0, samples(1000, 24_000), 10.05).is_none(), "the same sentence twice is scheduled once");
        assert_eq!(p.reply().len(), 24_000, "and it was not appended a second time either");
        assert_eq!(p.gaps(), 0, "a repeat is not a gap");
        let next = p.chunk(1, samples(500, 24_000), 10.06).expect("the turn continues");
        assert_eq!(next.at, 11.0 + CUSHION_S, "the repeat left the timeline untouched");
        assert_ne!(next.id, first.id);
    }

    /// The prior art's first invariant: "Calling at any display cadence produces the same score when all
    /// world ticks are supplied" (`pet-audio.ts:10-12`). The score here is `at`, and the only things that
    /// may change it are the order of the chunks and their arrival times — never how often the presenter
    /// asks. A *stall* is a different case and is the test above.
    #[test]
    fn the_same_chunks_score_the_same_at_any_cadence() {
        let run = |nows: [f64; 4]| {
            let mut p = Playback::new();
            p.set_rate(24_000);
            let mut out = Vec::new();
            for (i, now) in nows.into_iter().enumerate() {
                out.push(p.chunk(i as u32, samples(1000, 24_000), now).expect("plays").at);
            }
            out
        };
        let fast = run([0.0, 0.010, 0.020, 0.030]);
        let slow = run([0.0, 0.050, 0.060, 0.070]);
        assert_eq!(fast, slow, "one frame loop drains at 30 Hz and another at 10 Hz; the score is identical");
        assert_eq!(fast, [0.08, 1.08, 2.08, 3.08]);
    }

    #[test]
    fn a_forward_gap_keeps_the_timeline_gapless_and_counts_the_missing_sentences() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        p.chunk(0, samples(1000, 24_000), 10.0).expect("plays");
        let after_gap = p.chunk(3, samples(1000, 24_000), 10.1).expect("the next sentence that did arrive plays");
        assert_eq!(after_gap.at, 11.0 + CUSHION_S, "no silence is invented for the two sentences that never arrived");
        assert_eq!(p.gaps(), 2, "the missing sentences are counted, and that is all a gap can do here");
    }

    #[test]
    fn the_rate_is_the_one_the_speech_chunk_announced() {
        let mut p = Playback::new();
        assert_eq!(p.rate(), 24_000, "the native default (`client.rs:101`)");
        // 16 000 samples last one second at 16 kHz and two thirds of a second at 24 kHz: playing the
        // reply at the microphone's own 16 kHz (device.rs:18) is what makes it audibly slow.
        p.set_rate(16_000);
        assert_eq!(p.chunk(0, samples(1000, 16_000), 0.0).expect("plays").at, CUSHION_S);
        let one_second = p.chunk(1, samples(1000, 16_000), 0.0).expect("plays");
        assert_eq!(one_second.at, 1.0 + CUSHION_S);
        assert_eq!(one_second.rate, 16_000, "the chunk carries the rate its buffer must be created at");
        // The same 16 000 samples, announced at 24 kHz, are two thirds of a second: the cursor moves by
        // the announced rate and not by the microphone's.
        p.set_rate(24_000);
        let two_thirds = p.chunk(2, samples(1000, 16_000), 1.0).expect("plays");
        assert!((two_thirds.at - 2.08).abs() < 1e-9, "{}", two_thirds.at);
        assert_eq!(two_thirds.rate, 24_000, "and a later rate change is what the *next* chunk gets");
        let after = p.chunk(3, samples(1000, 24_000), 2.0).expect("plays");
        assert!((after.at - (2.08 + 2.0 / 3.0)).abs() < 1e-9, "16 000 samples at 24 kHz is 0.667 s: {}", after.at);
    }

    #[test]
    fn a_turn_boundary_forgets_the_timeline_and_leaves_the_audio_playing() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let first = p.chunk(0, samples(1000, 24_000), 10.0).expect("plays");
        p.chunk(1, samples(1000, 24_000), 10.0).expect("plays");
        assert_eq!(p.gaps(), 0);
        p.end_of_turn();
        // `Done` and `Idle` arrive while the device is still speaking (`client.rs:217-219`): the
        // scheduled chunks are still live, and only the next turn's numbering is new.
        assert!(p.is_playing(10.5), "the audio of the turn that just ended is still live");
        let next_turn = p.chunk(0, samples(500, 24_000), 20.0).expect("seq restarts at 0, and it plays");
        assert_eq!(next_turn.at, 20.0 + CUSHION_S, "a fresh turn is anchored at `now`, not at the old cursor");
        assert_eq!(p.gaps(), 0, "and the gap count is per turn");
        assert_eq!(p.finished(11.5), vec![first.id], "the first turn's finished chunk is still reported");
    }

    #[test]
    fn barge_in_hands_back_every_live_node_and_starts_a_new_epoch() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let a = p.chunk(0, samples(1000, 24_000), 10.0).expect("plays");
        let b = p.chunk(1, samples(1000, 24_000), 10.0).expect("plays");
        assert!(p.is_playing(10.0));
        assert_eq!(p.barge_in(10.0), vec![a.id, b.id], "every node that is playing is the caller's to stop");
        assert!(!p.is_playing(10.0), "and the timeline holds nothing");
        let after = p.chunk(0, samples(500, 24_000), 10.2).expect("plays");
        assert_eq!(after.at, 10.2 + CUSHION_S, "the next turn's first chunk is anchored at `now`");
    }

    /// A chunk that has finished is not handed back by a barge-in: stopping it is meaningless, and
    /// handing its id back would let the presenter try to stop a node it has already dropped.
    #[test]
    fn barge_in_hands_back_only_what_has_not_finished() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let past = p.chunk(0, samples(500, 24_000), 10.0).expect("plays"); // 10.08 .. 10.58
        let live = p.chunk(1, samples(1000, 24_000), 12.0).expect("plays"); // 12.08 .. 13.08
        assert_eq!(p.barge_in(12.0), vec![live.id], "the finished chunk is not the caller's to stop");
        assert!(!p.is_playing(12.0));
        let _ = past;
    }

    #[test]
    fn finished_reports_the_nodes_whose_audio_has_ended() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let a = p.chunk(0, samples(1000, 24_000), 10.0).expect("plays"); // 10.08 .. 11.08
        let b = p.chunk(1, samples(500, 24_000), 10.0).expect("plays"); // 11.08 .. 11.58
        assert!(p.finished(10.5).is_empty(), "nothing has finished yet");
        assert_eq!(p.finished(11.2), vec![a.id]);
        assert_eq!(p.finished(11.6), vec![b.id]);
        assert!(p.finished(12.0).is_empty(), "and a finished node is never reported twice");
    }

    #[test]
    fn replay_plays_the_kept_reply_without_disturbing_the_sequence() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        assert!(p.replay(10.0).is_none(), "nothing has been heard, so there is nothing to replay");
        p.chunk(0, samples(1000, 24_000), 10.0).expect("plays"); // 10.08 .. 11.08
        p.chunk(1, samples(1000, 24_000), 10.0).expect("plays"); // 11.08 .. 12.08
        // The cue is behind the cursor, so it queues after what is already scheduled: rodio's `append`.
        let again = p.replay(11.5).expect("the reply is kept until the next turn starts");
        assert!((again.at - 12.08).abs() < 1e-9, "{}", again.at);
        assert_eq!(again.seq, None, "a replay is not a sentence off the wire");
        assert_eq!(p.reply().len(), 48_000);
        // The sequence position is untouched: the turn that is still arriving continues where it was.
        let next = p.chunk(2, samples(500, 24_000), 11.6).expect("plays");
        assert!((next.at - (12.08 + 48_000.0 / 24_000.0)).abs() < 1e-9, "the cursor moved by the replay, exactly as rodio's queue does: {}", next.at);
        assert_eq!(p.gaps(), 0);
        // A replay asked for after the timeline went stale is still heard *now*: the user asked for it.
        let stale = p.replay(40.0).expect("the reply is still kept");
        assert_eq!(stale.at, 40.0 + CUSHION_S);
        assert!(p.forget_reply());
        assert!(!p.forget_reply(), "and forgetting an empty reply reports nothing to forget");
        assert!(p.replay(41.0).is_none());
    }

    #[test]
    fn the_envelope_is_the_native_one_value_per_50_ms_normalised_to_the_peak() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        // The same fixture as `crates/tipsy-compadre/src/audio/pcm.rs:157-166`: 50 ms of silence and
        // 100 ms of tone, which is three hops at 24 kHz.
        p.chunk(0, vec![0i16; 24_000 / 20], 0.0).expect("plays");
        p.chunk(1, sine(220.0, 24_000, 100), 0.0).expect("plays");
        let env = p.envelope();
        assert_eq!(env.len(), 3, "one value per 50 ms (`state.rs:14`, `render.rs:32`)");
        assert_eq!(env[0], 0.0);
        assert!((env.iter().cloned().fold(0.0, f32::max) - 1.0).abs() < 1e-6, "normalised to the peak, as `pcm.rs:58-66` does");
        // Silence is silence, and not a row of ones.
        let mut quiet = Playback::new();
        quiet.set_rate(24_000);
        quiet.chunk(0, vec![0i16; 2400], 0.0).expect("plays");
        assert!(quiet.envelope().iter().all(|v| *v == 0.0));
    }

    #[test]
    fn the_barge_in_rule_is_the_native_one() {
        let yes = [
            Command::Ask { text: "¿qué pasa?".into() },
            Command::Say { text: "hola".into() },
            Command::ListenStop,
            Command::Reset,
            Command::Cancel,
        ];
        for command in yes {
            assert!(starts_turn(&command) || command == Command::Cancel, "{command:?}");
            assert!(interrupts_playback(&command), "{command:?} takes the floor");
        }
        // The commands that must NOT cut a sentence in half: an empty question starts nothing
        // (`session.rs:233-238` ignores it), `ListenStart` opens the microphone while the reply is
        // still being spoken, and the probe says nothing out loud (`client.rs:619-640`, `:650-652`).
        let no = [
            Command::Ask { text: "   ".into() },
            Command::Say { text: String::new() },
            Command::ListenStart,
            Command::Hello { token: None, client: ClientKind::Web, protocol_version: 1 },
            Command::TestProvider { capability: Capability::Speech },
            Command::SetProfile { id: "compae".into() },
            Command::SetListeningMode { mode: ListeningMode::Push },
            Command::LoginStart,
            Command::LoginStop,
            Command::LoginAnswer { text: "compadre".into() },
        ];
        for command in no {
            assert!(!starts_turn(&command), "{command:?} starts no turn");
            assert!(!interrupts_playback(&command), "{command:?} must not quiet the device");
        }
        assert!(interrupts_playback(&Command::Cancel), "`Cancel` ends a turn without starting one (`session.rs:266-270`)");
    }

    #[test]
    fn an_event_that_ends_a_turn_is_the_one_that_resets_it() {
        // The three boundaries the caller watches, named here so the wiring in Step 5 cannot drift:
        // `Heard` starts a turn (a call's own sequence counter restarts there, `realtime/mod.rs:611-613`),
        // `Done` ends one that completed (`brain.rs:115-118`, `:166-169`) and `Idle` ends one that was
        // cancelled and gets no `Done` (`brain.rs:110`, `:162-163`; `state.rs:315-317`).
        let boundaries = [
            Event::Heard { text: "¿y el ticket?".into() },
            Event::Done { first_audio_ms: Some(300), total_ms: 1200 },
            Event::Status { state: AssistantState::Idle, detail: None },
        ];
        assert_eq!(boundaries.len(), 3);
        assert!(matches!(boundaries[2], Event::Status { state: AssistantState::Idle, .. }));
    }
}
```

Add the two lines to `crates/tipsy-web/src/lib.rs` (Task 3.2's file) so the module is part of the crate:

```rust
pub mod playback;
```

Then run:

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-web playback
```

Expected: FAIL — the crate compiles and every test panics in the `todo!()` skeleton, e.g.

```
running 14 tests
test playback::tests::the_first_chunk_is_anchored_a_cushion_ahead ... FAILED
...
---- playback::tests::the_first_chunk_is_anchored_a_cushion_ahead_and_the_rest_follow_it_gaplessly stdout ----
thread 'playback::tests::the_first_chunk_is_anchored_a_cushion_ahead_and_the_rest_follow_it_gaplessly' panicked at crates/tipsy-web/src/playback.rs:NN:5:
not yet implemented

failures:
    playback::tests::the_first_chunk_is_anchored_a_cushion_ahead_and_the_rest_follow_it_gaplessly
    ...
test result: FAILED. 0 passed; 14 failed; 0 ignored; 0 measured; 0 filtered out
```

If the run instead fails at the compiler, the crate's wasm-only dependencies are being built for this host target: do Step 4 before continuing. `cargo test -p tipsy-web` must never need `wasm32-unknown-unknown` or a browser.

- [ ] **Step 3: Implement the timeline**

Replace the `todo!()` bodies in `crates/tipsy-web/src/playback.rs`. Every decision below is one of the tests above; the comments name the native line each one mirrors.

```rust
impl Default for Playback {
    /// `clippy::new_without_default` is part of the workspace's `-D warnings` gate: a public `new`
    /// with no `Default` is a finding, and `Playback::new()` has a fixed default rate, so a derive is
    /// not available.
    fn default() -> Self {
        Self::new()
    }
}

impl Playback {
    pub fn new() -> Self {
        Self {
            // `client.rs:101`: the native client starts at 24 kHz and takes the real rate from the
            // first `SpeechChunk` (`:199-201`).
            rate: 24_000,
            cursor: None,
            last_seq: None,
            gaps: 0,
            next_id: 1,
            live: Vec::new(),
            reply: Vec::new(),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn set_rate(&mut self, rate: u32) {
        if rate > 0 {
            self.rate = rate;
        }
    }

    /// One whole sentence off the wire, or `None` when it must not be played.
    ///
    /// `now` is `AudioContext.currentTime`. Every chunk is placed at `max(cursor, now + CUSHION_S)`: a
    /// start time in the past would make `start(when)` play at once (MDN `AudioBufferSourceNode.start()`:
    /// "If `when` is less than `AudioContext.currentTime`, or if it's 0, the sound begins to play at
    /// once"), which is how a stalled reply collapses into one simultaneous burst — that is the prior
    /// art's `:43` guard, as a property of this function instead of a branch in it (module doc, item 1).
    pub fn chunk(&mut self, seq: u32, samples: Vec<i16>, now: f64) -> Option<Scheduled> {
        // An empty chunk is not audio: the native client drops it (`client.rs:254-256`).
        if samples.is_empty() {
            return None;
        }
        if let Some(last) = self.last_seq {
            if seq <= last {
                // A repeat inside the turn — the retrigger guard, and the reason its own test exists.
                // The transport is TCP-ordered, so a *backwards* jump is a server bug rather than a
                // reordering (research note §5.3 marks that guarantee UNVERIFIED); either way this is not
                // audio the user may hear twice or out of order.
                return None;
            }
            // A forward jump: the sentences the server never delivered. Their length is not knowable
            // from their sequence numbers, so nothing is invented for them — the note's
            // `(seq - last_played - 1) * 0.020` sketch belongs to a 20 ms chunk shape this wire does
            // not use (`lib.rs:178`: one frame is one sentence). The count is the whole record.
            self.gaps += seq - last - 1;
        }
        let duration = samples.len() as f64 / f64::from(self.rate);
        let at = self.cursor.map_or(now + CUSHION_S, |cursor| cursor.max(now + CUSHION_S));
        self.cursor = Some(at + duration);
        self.last_seq = Some(seq);
        self.reply.extend_from_slice(&samples);
        Some(self.schedule(Some(seq), at, duration, samples))
    }

    fn schedule(&mut self, seq: Option<u32>, at: f64, duration: f64, samples: Vec<i16>) -> Scheduled {
        let id = self.next_id;
        self.next_id += 1;
        self.live.push(Live { id, ends_at: at + duration });
        Scheduled { id, at, seq, rate: self.rate, samples }
    }

    /// A turn is over: the next `SpeechChunk` carries `seq` 0 again (`brain.rs:198`), so the cursor and
    /// the numbering start from nothing. The audio already scheduled is **not** touched — `Done` and
    /// `Idle` arrive while the device is still speaking the last sentence (`client.rs:217-219`).
    pub fn end_of_turn(&mut self) {
        self.cursor = None;
        self.last_seq = None;
        self.gaps = 0;
    }

    /// Barge-in: everything that is still to be heard goes, and the timeline starts over. Returns the
    /// ids of the nodes the caller must `stop()` and `disconnect()` — the prior art's `silence()`
    /// (`pet/src/ui/pet.ts:33`), with the ids this core owns instead of a set the presenter owns. Only
    /// the chunks whose audio has not ended by `now` come back: stopping a finished node is meaningless,
    /// and the ids of the finished ones are `finished`'s to report.
    pub fn barge_in(&mut self, now: f64) -> Vec<u64> {
        self.cursor = None;
        self.last_seq = None;
        self.gaps = 0;
        self.live.drain(..).filter(|live| live.ends_at > now).map(|live| live.id).collect()
    }

    /// The ids whose audio has ended by `now`: the presenter drops those node references.
    ///
    /// The prior art does this from the node's own `onended` (`pet/src/ui/pet.ts:44`). From Rust that
    /// means a `Closure` per node, and its lifetime is the hazard `wasm-bindgen`'s own documentation
    /// warns about: `Closure::forget` leaks it for the life of the page, and storing it in the entry the
    /// callback removes drops the closure while it is running. The end time was known when the chunk was
    /// scheduled, so the core reports it instead — the same bookkeeping, no leak, and `ended` stays what
    /// MDN says it is (it fires after an explicit `stop()` too, so it could not prove that a sentence was
    /// heard; research note §5.1, §5.4).
    pub fn finished(&mut self, now: f64) -> Vec<u64> {
        let (done, live): (Vec<Live>, Vec<Live>) = self.live.drain(..).partition(|live| live.ends_at <= now);
        self.live = live;
        done.into_iter().map(|live| live.id).collect()
    }

    /// `Replay`: the reply kept for the selected entry, played again from `now`. The sequence position
    /// is untouched and the reply is not extended — the native client's `Replay` only queues the audio
    /// it kept (`client.rs:371-377`). A replay the user just asked for is heard now, whatever the
    /// timeline's own position is, which is why a stale cursor is not used here.
    pub fn replay(&mut self, now: f64) -> Option<Scheduled> {
        if self.reply.is_empty() {
            return None;
        }
        let samples = self.reply.clone();
        let duration = samples.len() as f64 / f64::from(self.rate);
        let at = self.cursor.filter(|cursor| *cursor > now).unwrap_or(now + CUSHION_S);
        self.cursor = Some(at + duration);
        Some(self.schedule(None, at, duration, samples))
    }

    /// The next turn starts: what `Replay` would replay is the previous turn's audio (`client.rs:364-367`).
    pub fn forget_reply(&mut self) -> bool {
        let had = !self.reply.is_empty();
        self.reply.clear();
        had
    }

    /// The envelope of everything heard this turn: one value per 50 ms, peak-normalised.
    ///
    /// The arithmetic is `crates/tipsy-compadre/src/audio/pcm.rs:58-66` (`hop = rate / 20`, `rms` per
    /// hop, divided by the peak) with `i16_to_f32` from `:24-26`; `tipsy-compadre` is the native binary
    /// and is not a dependency of this crate, so the two implementations are the same twelve lines
    /// rather than one function — and the test above reproduces `pcm.rs:157-166`'s own fixture so a
    /// drift between them shows up as a failing test here. This is the *same observation* the native
    /// client feeds its oscilloscope with (`client.rs:271`), which is what keeps the two HUDs looking
    /// alike: building it per chunk instead of over the accumulated reply would normalise each sentence
    /// to its own peak and draw a different picture.
    pub fn envelope(&self) -> Vec<f32> {
        let floats: Vec<f32> = self.reply.iter().map(|s| *s as f32 / 32768.0).collect();
        let hop = (self.rate / 20).max(1) as usize;
        let env: Vec<f32> = floats
            .chunks(hop)
            .map(|window| {
                if window.is_empty() {
                    0.0
                } else {
                    (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt()
                }
            })
            .collect();
        let peak = env.iter().cloned().fold(0.0, f32::max);
        if peak <= 0.0 {
            return env;
        }
        env.into_iter().map(|value| value / peak).collect()
    }

    pub fn reply(&self) -> &[i16] {
        &self.reply
    }

    pub fn gaps(&self) -> u32 {
        self.gaps
    }

    pub fn is_playing(&self, now: f64) -> bool {
        self.live.iter().any(|live| live.ends_at > now)
    }
}

/// `client.rs:619-640`, verbatim in behaviour: an empty question and a bare `!` start nothing
/// (`session.rs:233-238`, `:242-249`), `ListenStop`/`Reset` start the next turn, and the probe, the
/// profile change, the mode change and the login commands start none. `Hello` is never sent by this
/// client (Task 3.2 sends it once on open) and `Cancel` ends a turn without starting one, which is why
/// it is `interrupts_playback`'s second half and not this function's.
pub fn starts_turn(command: &Command) -> bool {
    match command {
        Command::Ask { text } | Command::Say { text } => !text.trim().is_empty(),
        Command::ListenStop | Command::Reset => true,
        Command::Hello { .. }
        | Command::TestProvider { .. }
        | Command::ListenStart
        | Command::Cancel
        | Command::SetProfile { .. }
        | Command::SetListeningMode { .. }
        | Command::LoginStart
        | Command::LoginStop
        | Command::LoginAnswer { .. } => false,
        // Phase 1's three configuration commands are additive and start no turn: a save is not a
        // question, and it must not cut a sentence in half. `lib.rs`'s `Command` after Task 1.1 has
        // them; if the compiler says this arm is unreachable, Task 1.1 named them differently and this
        // is the one line to fix.
        Command::GetConfig | Command::UpdateConfig { .. } | Command::SetKey { .. } => false,
    }
}

/// `client.rs:658-660`: everything that starts a turn, plus the one command that ends one without
/// starting another. The v1 spec's Tab sentence (`spec:166-171`: "presses `Tab` while the assistant is
/// talking, the client sends `Cancel` and drops its audio queue") is **not** what the code does —
/// `Tab` with the microphone closed raises `ListenStart` (`state.rs:412-422`) and `starts_turn`
/// answers `false` for it (`client.rs:632`) — and the native client is the authority (`CLAUDE.md`:
/// terminal first). This task mirrors the code; widening the set is a change to `client.rs` and belongs
/// to a task that owns it.
pub fn interrupts_playback(command: &Command) -> bool {
    starts_turn(command) || matches!(command, Command::Cancel)
}
```

Run the same command as Step 2.

Expected: PASS — `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`.

One thing this task does **not** do: the note's §5.5 finding is that `AnalyserNode` is unnecessary because the client already holds the PCM16 it is playing (§5.5 of the research note, and the v1 spec agrees at `spec:140`: "Clients compute the audio envelope for the visualisers locally from what they record and play"). Do not add an analyser node, a second `Float32Array` and a second clock.

- [ ] **Step 4: Keep the host build free of wasm-only dependencies**

`cargo test -p tipsy-web` on this machine compiles for `x86_64-pc-windows-msvc`, and the pure module above is the only thing it should need. In `crates/tipsy-web/Cargo.toml`, Task 3.2's wasm-only dependencies (`ratzilla`, `web-sys`, `js-sys`, `wasm-bindgen`, `wasm-bindgen-futures`) belong under a target-specific section, and every module that names one carries `#[cfg(target_arch = "wasm32")]`:

```toml
# Task 3.2's socket, Task 3.3's microphone and this task's `web_audio` are the only modules that may
# name a browser type; they are compiled for wasm only, which is what lets `cargo test -p tipsy-web`
# run the pure half on this host (`playback::tests`, `hud::tests`). RESEARCH NOTE: no page about
# per-target dependency sections was read; the Cargo Book's "Target specific dependencies" section is
# the authority for the syntax below.
[target.'cfg(target_arch = "wasm32")'.dependencies]
ratzilla = "=0.3.1"
wasm-bindgen = "0.2.129"
wasm-bindgen-futures = "0.4.79"
js-sys = "0.3.106"

[target.'cfg(target_arch = "wasm32")'.dependencies.web-sys]
version = "0.3.106"
features = ["AudioContext", "AudioBuffer", "AudioBufferSourceNode", "AudioNode", "Document", "Element", "Window", "console"]
```

The version numbers are the ones the plan's Tech Stack paragraph fixes and the research note read (`ratzilla` 0.3.1, `wasm-bindgen` 0.2.129, `js-sys`/`web-sys` 0.3.106, `wasm-bindgen-futures` 0.4.79 — note §4.6, §6.3) **except the feature list**, which is this task's own and marked INFERRED: note §4.6 records `AudioBuffer`, `AudioContext` and `AudioWorklet` as verified feature *names* but says "the exact minimal set has to be found by compiling, which this machine cannot do". Take 3.2's and 3.3's lists as the base and add exactly the four this task needs (`AudioBuffer`, `AudioBufferSourceNode`, `AudioNode`, and whatever 3.2's socket already has).

Run:

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-web playback
```

Expected: still `14 passed` — and the build must not need the wasm target. If the crate had no `AudioBufferSourceNode` feature before this task, the only place it can fail is the wasm build of Step 6, which is where the feature list gets its real test.

- [ ] **Step 5: Route the socket's messages into the timeline**

In `crates/tipsy-web/src/lib.rs`, inside the frame that Task 3.2 already runs (the `draw_web` callback or the queue drain it put there — read its chunk and use its shape, do not add a second loop), drain the socket's decoded values through the timeline. The native client's `on_message` (`client.rs:196-225`) is the order to mirror: the rate first, then the audio, and the turn boundaries where the native client's own bookkeeping sits.

```rust
// `now` is the AudioContext's own timeline in seconds (MDN `BaseAudioContext.currentTime`: "the
// elapsed time in the context's audio timeline in seconds"; it stops advancing while suspended).
let now = ctx.current_time();

for message in socket.drain() {
    match message {
        ToClient::Event(event) => {
            // `client.rs:199-201`, the two lines that make the reply's rate the announced one.
            if let Event::SpeechChunk { sample_rate, .. } = &event {
                playback.set_rate(*sample_rate);
            }
            // The three turn boundaries: `Heard` opens one (a call's counter restarts there,
            // `realtime/mod.rs:611-613`), `Done` closes one that completed (`brain.rs:115-118`,
            // `:166-169`), `Idle` closes one that was cancelled and gets no `Done` (`brain.rs:110`,
            // `:162-163`). `end_of_turn` touches no node: the audio of the ending turn is still playing.
            if matches!(event, Event::Done { .. })
                || matches!(event, Event::Heard { .. })
                || matches!(&event, Event::Status { state: AssistantState::Idle, .. })
            {
                playback.end_of_turn();
            }
            shared.borrow_mut().state.apply(&event);
        }
        ToClient::Audio(Frame::Speech { seq, samples }) => {
            if let Some(scheduled) = playback.chunk(seq, samples, now)
                && let Err(e) = web_audio::play(&ctx, &scheduled)
            {
                // The frame callback cannot propagate an error, and a silent client is the worst
                // outcome this task has: put it where the browser console can be read.
                web_sys::console::error_1(&e);
            }
            // `client.rs:270-271`: the oscilloscope gets the envelope of the whole reply, rebuilt as it
            // grows, and `start_voice` restarts the playhead at the first hop.
            shared.borrow_mut().state.start_voice(playback.envelope());
        }
        // A microphone frame coming back from the server is not a thing (`lib.rs:223` ignores it too).
        ToClient::Audio(Frame::Mic(_)) => {}
    }
}

// Nodes whose audio has ended are forgotten so the map cannot grow for the length of a reply. The
// prior art does this from each node's `onended`; `web_audio.rs` says why this does it from the core.
for id in playback.finished(now) {
    web_audio::forget(id);
}
```

Barge-in is the action loop's, and it is the same rule the native client applies to the same commands — `client.rs:361-367`, `:658-660`, including the reply's lifetime. Put these five lines where the loop dispatches an action (Task 3.2's frame loop, which Task 3.5 completes; Task 3.5's `dispatch` repeats them so the two cannot drift):

```rust
let now = ctx.current_time();
if let UiAction::Send(command) = &action {
    if interrupts_playback(command) && playback.is_playing(now) {
        // The browser's half of "the client sends `Cancel` and drops its audio queue" (v1 spec §5.3),
        // and the prior art's one `silence()`: stop everything that is playing, in one call.
        web_audio::silence(&playback.barge_in(now));
    }
    if starts_turn(command) {
        playback.forget_reply();
    }
}
```

The native client's `flush_on_interrupt(interrupting, playing)` guard (`client.rs:578-580`) is kept here even though it is not load-bearing in a browser: the guard exists because rodio's `clear()` pauses the sink and the client had to resume it (`audio/device.rs:57-68`), and WebAudio has no such trap. Keeping the two clients reading the same is worth one `&&`.

- [ ] **Step 6: Write the `AudioContext` half**

Create `crates/tipsy-web/src/web_audio.rs`, whole file behind `#[cfg(target_arch = "wasm32")]`, and add `#[cfg(target_arch = "wasm32")] mod web_audio;` to `lib.rs`. **Every `web_sys` method name below is INFERRED** — the note read MDN's API descriptions and `web-sys`'s *feature names*, not the method pages (§5.1–§5.4, §4.6) — so the implementer opens `https://docs.rs/web-sys/0.3.106/web_sys/struct.AudioBufferSourceNode.html` and `…/struct.AudioContext.html` and corrects the spelling before compiling. The MDN facts that fix the *shape* are quoted beside each call.

```rust
//! The browser's half of playback: one `AudioBufferSourceNode` per chunk.
//!
//! MDN, verbatim: "An `AudioBufferSourceNode` can only be played once; after each call to `start()`,
//! you have to create a new node if you want to play the same sound again. Fortunately, these nodes are
//! very inexpensive to create" — so one node per sentence is the design and not a compromise, and
//! holding the live ones is what makes barge-in possible (the same page's "don't even bother to hold a
//! reference" is about nodes nobody has to stop).

use std::cell::RefCell;
use std::collections::HashMap;

use wasm_bindgen::prelude::*;
use web_sys::{AudioBufferSourceNode, AudioContext};

use crate::playback;

thread_local! {
    /// The nodes still playing, by the id `playback::Playback` gave them. A map rather than the prior
    /// art's bare `Set` only because the ids are what `barge_in` and `finished` name; the shape is its
    /// shape (`pet/src/ui/pet.ts:26`). Thread-local is enough: wasm is single-threaded and `draw_web`'s
    /// callbacks all run on it.
    static NODES: RefCell<HashMap<u64, AudioBufferSourceNode>> = RefCell::new(HashMap::new());
}

/// Schedules one chunk. `scheduled.at` is never in the past — `Playback::chunk` places it at least a
/// cushion ahead — so the node is always scheduled for a future moment and `start(when)`'s
/// play-at-once-on-a-past-`when` behaviour is never what makes this audible.
pub fn play(ctx: &AudioContext, scheduled: &playback::Scheduled) -> Result<(), JsValue> {
    // `createBuffer(numOfChannels, length, sampleRate)`: `length` is "the size of the buffer in
    // sample-frames", and "All browsers must support sample rates in at least the range 8,000 Hz to
    // 96,000 Hz" (MDN `BaseAudioContext.createBuffer()`), so the announced rate is always accepted.
    let buffer = ctx.create_buffer(1, scheduled.samples.len() as u32, scheduled.rate as f32)?;
    // `copyToChannel(source, channelNumber, startInChannel)` takes "A `Float32Array` that the channel
    // data will be copied from" (MDN `AudioBuffer.copyToChannel()`). The conversion is `pcm.rs:24-26`'s.
    let floats: Vec<f32> = scheduled.samples.iter().map(|s| *s as f32 / 32768.0).collect();
    buffer.copy_to_channel(&floats, 0)?;
    let node = ctx.create_buffer_source()?;
    node.set_buffer(Some(&buffer));
    node.connect_with_audio_node(&ctx.destination())?;
    node.start_with_when(scheduled.at)?;
    NODES.with_borrow_mut(|nodes| nodes.insert(scheduled.id, node));
    Ok(())
}

/// Barge-in: the prior art's `silence()` (`pet/src/ui/pet.ts:33`) — `stop()` on every node that is
/// playing and forget them all. `stop()` with no argument or 0 "causes the sound to stop playback
/// immediately" and `disconnect()` with no arguments disconnects "all outgoing connections" (MDN
/// `AudioScheduledSourceNode.stop()`, `AudioNode.disconnect()`). The ids come from
/// `Playback::barge_in`, so what is stopped is exactly what the core has not already finished.
pub fn silence(ids: &[u64]) {
    NODES.with_borrow_mut(|nodes| {
        for id in ids {
            if let Some(node) = nodes.remove(id) {
                // A node that was never started throws `InvalidStateError` on `stop()`; swallowing it
                // is what the prior art's `try {} catch {}` does at `:33`, and there is nothing to
                // report — the node is gone either way.
                let _ = node.stop();
                let _ = node.disconnect();
            }
        }
    });
}

/// A node whose audio has ended (`Playback::finished` reported its id): it is never started again, so
/// dropping the reference is all there is to do.
///
/// The prior art hangs this on the node's own `onended` (`pet/src/ui/pet.ts:44`). From Rust that is a
/// `Closure` per node and a lifetime problem `wasm-bindgen` documents rather than solves (`forget()`
/// leaks it for the life of the page; storing it in the entry the callback removes drops it while it is
/// running), so the end time the core already knows is what drives this instead. `ended` is also not
/// evidence that a sentence was heard — MDN documents that `stop()` fires it too — and the v1 spec
/// §5.3's history rule (`spec:166-171`) is about exactly that distinction.
pub fn forget(id: u64) {
    NODES.with_borrow_mut(|nodes| {
        nodes.remove(&id);
    });
}
```

The node a `play` returns is never kept beyond `NODES`: the map is the only owner, so a dropped node is a dropped node — this is what keeps a long reply from holding hundreds of finished nodes.

**Two patterns from the same prior art that this task deliberately does not take, and why** — both are right for an ambient, continuous pet with several simultaneous hosts, and neither is right here (`codewhale-audio-prior-art.md` §3, last paragraph):

1. **Muting on `visibilitychange`** (`pet/public/shared.html:50`): the page must **not** mute, suspend or stop playback when the document becomes hidden, and no task in this plan may add it. The web client exists to be reached from another device — `docs/architecture.md:59` gives its purpose as "Talking to the assistant from a phone or another PC" — and a phone that goes silent when its screen locks breaks exactly that. The hidden-tab case is handled where it belongs instead: `draw_web`'s `requestAnimationFrame` stops in a background tab (research note §6.4), so the timeline can go stale, and Step 3's cushion plus the `max(cursor, now + CUSHION_S)` clamp are what keep the reply from collapsing into a burst when the tab comes back. `Playback::finished` and `barge_in` also keep running on whatever frames do arrive.
2. **An audio-output lease between clients** (`POST /v1/audio` with `{client, enabled}` → `granted`, re-leased every 500 ms; `pet/SHARED.md:187`): nothing here needs one. A pet plays continuous ambient audio, so four hosts would talk over each other; this client plays the reply to a question a *specific* client asked, and each connected client hearing its own reply is the intended behaviour (spec §5.2: separate sessions, separate histories). There is no shared resource to arbitrate and this task adds no lease, no lock and no server-side audio ownership.

- [ ] **Step 7: Run the crate's whole suite and clippy**

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-web
```

Expected: all green — this task's 14 tests plus whatever Tasks 3.2 and 3.3 added, and nothing else in the workspace is touched by this task, so `tipsy-ui`'s and `tipsy-protocol`'s existing tests must still be green if the operator runs `--workspace`.

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo clippy -j 1 -p tipsy-web --all-targets
```

Expected: the finding set of `.superpowers/sdd/2026-10-11-plan-3-web-tier/global-constraints.md:26-33` and nothing new — that list has no `tipsy-web` entry, so **any** finding here is this task's. `clippy::new_without_default` is the one to expect and the `Default` impl in Step 3 is why it is not there. Never run `cargo fmt`.

- [ ] **Step 8: Manual check — the audible half (recorded here, run after Task 3.6)**

There is no browser in this harness and no test above pretends there is one. This check needs the built bundle, which is Task 3.6's `cargo xtask dist`; it is written here because the playback path is this task's deliverable. Run it once 3.6's first `dist` has passed, and record the outcome in the ledger.

1. `cargo xtask dist`, then start the server: `cargo run --release -p tipsy-compadre -- serve` (or the release binary `target/release/tipsy-compadre.exe serve`).
2. Open `http://127.0.0.1:7070/` in Chrome or Firefox on this machine, click **press to start**, grant the microphone.
3. Type `hola, ¿qué tal?` and press `Enter`.
   **Expected:** the reply is audible and **gapless** — sentences follow one another with no silence between them and no stutter at the join, which is what the cursor is for. If the sentences overlap or the reply sounds like one burst, `at` collapsed to `now` (Step 3's `max`); if the *first* word is clipped, the cushion is not being applied.
4. With the reply still being spoken, type another question and press `Enter`.
   **Expected:** the audio stops *immediately* — mid-syllable, not at the end of the sentence — and the new reply starts after its own latency. In the browser console, `performance.now()` around the click and the last audible sample should not be separated by a whole sentence.
5. Press `F4` (clear/`Reset`) while a reply is playing.
   **Expected:** the same immediate stop.
6. Press `F2` (replay) after a reply has finished.
   **Expected:** the whole of the last reply is played again from the beginning, the rings keep animating, and the oscilloscope follows it. Pressing `F2` again queues another copy (the native client does the same: `client.rs:371-377` appends to rodio's queue).
7. In the server's log (or a debugger) watch for the rate: with the default providers the sentences arrive at 24 000 Hz, and a 16 kHz provider must sound *correct*, not slow — that is Step 5's `set_rate`.
8. Optional, to see a gap: stop the server mid-sentence, watch the client's audio end, and confirm nothing is invented after it — a gap adds silence nowhere.
9. **The stall, and the one pattern this client does not take.** Ask a question whose reply is long, let it start speaking, then switch to another tab or window for about 15 seconds and come back while the reply is still arriving.
   **Expected:** the audio is still there when the tab comes back — it is **not** muted, suspended or stopped by the page (`visibilitychange` is deliberately not handled; see the two rejected patterns in Step 6), and the sentences that arrived while the tab was hidden are played from their own beginnings, in order, none of them swallowed and none of them all at once. Roughly one sentence is late, not the whole rest of the reply: if everything after the return plays in a single burst, the `max(cursor, now + CUSHION_S)` clamp is not in place; if a sentence or two never plays at all, a skip decision (prior-art item 1) has been added back and its reason in the module doc has to be answered before it stays.

- [ ] **Step 9: Commit**

```bash
git add crates/tipsy-web/Cargo.toml crates/tipsy-web/src/playback.rs crates/tipsy-web/src/web_audio.rs crates/tipsy-web/src/lib.rs
git commit -m "feat(web): play the server's speech on a gapless cursor

One AudioBufferSourceNode per spoken sentence, placed at max(cursor, now) plus
the prior art's 80 ms cushion so jitter cannot collapse the reply into one burst
and no chunk is ever scheduled into the past. A duplicate sequence number is
dropped, a forward gap is counted rather than filled with silence the wire cannot
size, and a late sentence plays whole instead of being skipped or entered
mid-word. Barge-in stops and disconnects every live node in one silence(), and
the envelope is the native client's own observation over the accumulated reply.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 3.5: The HUD glue, Ratzilla's input and the host page `[mid]`

**Files:**
- Create: `crates/tipsy-web/src/hud.rs` (the host-runnable half: the clock mapping, the key translation and the click dispatch), with its own `#[cfg(test)] mod tests`
- Modify: `crates/tipsy-web/src/lib.rs` (the render loop inside Task 3.2's `start()`: register the two event callbacks, call `draw_web`, dispatch the actions)
- Create: `web/client/index.html` (the host page)
- Modify: `crates/tipsy-web/Cargo.toml` (`ratatui` by name, for the tests' `Rect`; the workspace dependency, whose features carry no crossterm, `Cargo.toml:14`)
- Test: `crates/tipsy-web/src/hud.rs` (its own `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `tipsy_ui::{ClockView, Hit, KeyInput, UiAction, UiState, draw}` — `draw(frame: &mut Frame, state: &UiState, clock: &ClockView, title: &str) -> Vec<Hit>` (`crates/tipsy-ui/src/render.rs:65`), `Hit { area: Rect, key: KeyInput, action: Option<ConfigAction> }` (`render.rs:51-63`), `KeyInput` (`state.rs:70-81`), `UiAction` (`state.rs:83-98`), `UiState::{on_key, on_hit, apply, start_voice}` (`state.rs:389`, `:202`, `:289`, `:511`); `tipsy_protocol::{Command, Event, ToServer}` — `Command::GetConfig`/`UpdateConfig`/`TestProvider` (Phase 1's, Task 1.1) and `Event::Config` (`:171`); `tipsy_ui::ConfigForm` as the view of `tipsy_protocol::ConfigView` after Task 1.1 (spec §4.3); Task 3.1's recorded verdict, **its backend choice, its glyph result and its header result**; Task 3.2's `tipsy_web::start()`, its socket and its sender; Task 3.3's gate (its handler owns `getUserMedia` and the `AudioContext`); Task 3.4's `playback::{Playback, interrupts_playback, starts_turn}` and `web_audio::{play, silence, forget}`.
- Produces: `tipsy_web::hud::{RawKey, Input, key_input, hit_at, drain, clock_from}`; the render loop and its `dispatch`; `web/client/index.html`.

- [ ] **Step 1: Re-read Task 3.1's verdict and stop if it was FAIL**

Same gate as Task 3.4: read the verdict line in `.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md`. **FAIL → this task does not run**: append `Task 3.5: abandoned (Task 3.1 FAIL)` and stop, exactly as spec §8.1 requires ("If it fails, Phase 3 stops there", `docs/superpowers/specs/2026-10-11-web-tier-design.md:386-388`).

From the same record take the four things the rest of this task depends on, and write them into the ledger's task line if they are not there already:

1. **The backend.** `WebGl2Backend` is the one the published table gives "60fps on large terminals" to and the one whose keyboard is the thinnest — "Some backends (e.g. `WebGl2Backend`) do not support key events and **will silently succeed without registering any handlers**" (note §2b, quoting the `WebEventHandler` page). Step 4 and the manual check below are written for that failure mode. `DomBackend` and `CanvasBackend` register keys but carry four open DOM layout bugs, two of them filed on 2026-10-10/11 (note §0, §2b). Whatever 3.1 chose, the code below takes the backend it named; do not re-decide.
2. **The font** its page rendered the Braille canvases with (note §8 item 2 makes a real font part of the spike). The spike now carries the *header* and its three ambiguous-width glyphs in its evidence — `◉`, `⚙` and `·` — so read the verdict for that glyph result too, not only for the backend: it is the answer to whether the page's font agrees with the tables `tipsy-ui` measures with (Step 5, and the manual check's header item). If 3.1's record has no glyph result, the manual check is where it gets settled, and the outcome goes in the ledger.
3. **Whether keys actually arrived** — 3.1's verdict should say, because this is the one thing no public source answers.
4. **Whether the header lined up on the spike's page**: the badge, the profile's title, the mode word and the gear in the columns `drawn_width` promises (`render.rs:218-244`). Same rule as the glyphs: recorded in the ledger either way, and re-tested by the check in Step 7.

- [ ] **Step 2: Write the failing tests, and watch them fail**

Create `crates/tipsy-web/src/hud.rs` with the shapes and `todo!()` bodies below, plus the tests. Everything in this file runs on this host: no Ratzilla type appears in it, because the translation is written over this crate's own key vocabulary and the wasm glue (Step 4) is a dumb adapter.

The tests build `tipsy_ui::Hit`, whose `area` is a `ratatui::layout::Rect`, so `crates/tipsy-web/Cargo.toml` needs `ratatui` as a direct dependency — the **workspace** entry, which is `{ version = "0.30", default-features = false, features = ["std", "layout-cache", "all-widgets"] }` (`Cargo.toml:14`) and therefore **no crossterm**: `codebase-recon.md` §8.3 records that this exact property is what keeps the wasm build plausible, so do not name a feature or a backend here.

```toml
[dependencies]
ratatui.workspace = true
```

It must resolve to the same copy `tipsy-ui` uses (one `ratatui 0.30.2` in the lock, `codebase-recon.md` §8.2), since `Hit.area` is that crate's `Rect`.

```rust
//! The HUD's host-independent half: the clock the client injects, the keys it translates and the
//! clicks it dispatches.
//!
//! `tipsy_ui::draw` is called verbatim on both clients (`render.rs:65`), and the translation from a
//! terminal-shaped event to `KeyInput`/`UiAction` is what has to exist twice — once over crossterm's
//! types (`client.rs:683-707`, `:676-680`) and once over Ratzilla's. This module is the second one's
//! decisions, spelled over this crate's own `RawKey`: the Ratzilla types are `#[cfg]`-gated to wasm
//! (Step 4) and never reach a test.

use tipsy_ui::{ClockView, Hit, KeyInput, UiAction, UiState};

/// The subset of a key event the HUD has an opinion about. The wasm adapter fills it from Ratzilla's
/// `KeyEvent` — the same crossterm-*shaped* vocabulary, a separate type (research note §2b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKey {
    Char { c: char, ctrl: bool },
    Enter,
    Backspace,
    Tab,
    Esc,
    Up,
    Down,
    Left,
    Right,
    F(u8),
    /// Anything the HUD does not use: `Home`, `PageDown`, a media key, a bare modifier.
    Other,
}

/// One thing a handler received, waiting for the frame that acts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Key(KeyInput),
    /// The cell the pointer went down on, in the grid's own coordinates.
    Click { col: u16, row: u16 },
}

pub fn key_input(raw: RawKey) -> Option<KeyInput> { todo!() }

/// The hit whose clickable rectangle contains the cell, if any. The native client's `hit_at`
/// (`client.rs:676-680`), and the rectangles are `tipsy_ui`'s (`render.rs:51-63`).
pub fn hit_at(hits: &[Hit], col: u16, row: u16) -> Option<Hit> { todo!() }

/// Applies everything the handlers queued since the last frame, in the order it arrived.
pub fn drain(state: &mut UiState, hits: &[Hit], queued: &mut Vec<Input>) -> Vec<UiAction> { todo!() }

/// The clock the HUD shows, from the browser's own local time.
pub fn clock_from(hours: u32, minutes: u32, seconds: u32, js_day: u32, day: u32, month: u32) -> ClockView {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use tipsy_protocol::ProfileView;

    fn welcome() -> tipsy_protocol::Event {
        tipsy_protocol::Event::Welcome {
            session_id: 1,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: tipsy_protocol::UiLanguage::Es,
            listening_mode: tipsy_protocol::ListeningMode::Push,
            web_bundle: true,
            login: None,
        }
    }

    #[test]
    fn the_key_vocabulary_is_the_native_clients() {
        assert_eq!(key_input(RawKey::Enter), Some(KeyInput::Enter));
        assert_eq!(key_input(RawKey::Backspace), Some(KeyInput::Backspace));
        assert_eq!(key_input(RawKey::Tab), Some(KeyInput::Tab));
        assert_eq!(key_input(RawKey::Esc), Some(KeyInput::Esc));
        assert_eq!(key_input(RawKey::Up), Some(KeyInput::Up));
        assert_eq!(key_input(RawKey::Down), Some(KeyInput::Down));
        assert_eq!(key_input(RawKey::Left), Some(KeyInput::Left));
        assert_eq!(key_input(RawKey::Right), Some(KeyInput::Right));
        assert_eq!(key_input(RawKey::F(9)), Some(KeyInput::F(9)));
        assert_eq!(key_input(RawKey::Char { c: 'ñ', ctrl: false }), Some(KeyInput::Char('ñ')));
        assert_eq!(key_input(RawKey::Other), None, "a key the HUD has no opinion about is not an action");
    }

    #[test]
    fn a_control_chord_arrives_as_the_control_character_it_stands_for() {
        // `client.rs:694-703`: `KeyInput` has no modifier, so Ctrl+S and Ctrl+R are the control
        // characters the screen's save and regenerate rows are bound to
        // (`ConfigForm::SAVE_KEY`, `ConfigForm::REGENERATE_KEY`).
        assert_eq!(key_input(RawKey::Char { c: 's', ctrl: true }), Some(KeyInput::Char('\u{13}')));
        assert_eq!(key_input(RawKey::Char { c: 'S', ctrl: true }), Some(KeyInput::Char('\u{13}')), "the case folds first");
        assert_eq!(key_input(RawKey::Char { c: 'r', ctrl: true }), Some(KeyInput::Char('\u{12}')));
        // A chord that names no ASCII letter is not a key this HUD has.
        assert_eq!(key_input(RawKey::Char { c: '1', ctrl: true }), None);
        assert_eq!(key_input(RawKey::Char { c: 'ñ', ctrl: true }), None);
    }

    #[test]
    fn a_click_lands_on_the_rectangle_that_contains_it_and_on_nothing_else() {
        let hits = vec![
            Hit { area: Rect::new(10, 2, 6, 1), key: KeyInput::F(7), action: None },
            Hit { area: Rect::new(20, 2, 4, 1), key: KeyInput::F(8), action: None },
        ];
        assert_eq!(hit_at(&hits, 10, 2).map(|h| h.key), Some(KeyInput::F(7)), "the first column is inside");
        assert_eq!(hit_at(&hits, 15, 2).map(|h| h.key), Some(KeyInput::F(7)), "and so is the last: right() is exclusive");
        assert!(hit_at(&hits, 16, 2).is_none(), "one column past the end is not a hit");
        assert!(hit_at(&hits, 9, 2).is_none());
        assert!(hit_at(&hits, 10, 3).is_none(), "one row below is not a hit either");
        assert_eq!(hit_at(&hits, 21, 2).map(|h| h.key), Some(KeyInput::F(8)), "overlapping rectangles report the first");
    }

    #[test]
    fn the_actions_come_out_in_the_order_the_input_arrived() {
        let mut state = UiState::new();
        state.apply(&welcome());
        let hits = vec![Hit { area: Rect::new(20, 2, 4, 1), key: KeyInput::F(8), action: None }];
        let mut queued = vec![
            Input::Key(KeyInput::Char('h')),
            Input::Click { col: 20, row: 2 },
            Input::Key(KeyInput::Char('i')),
        ];
        let actions = drain(&mut state, &hits, &mut queued);
        assert!(actions.is_empty(), "none of these three asks the server for anything: {actions:?}");
        assert_eq!(state.input, "hi", "the two keys reached the question box in order, around the click");
        assert_eq!(state.tab, tipsy_ui::Tab::Network, "and the click switched the tab");
        assert!(queued.is_empty(), "the queue is drained, not copied");
    }

    #[test]
    fn a_click_that_lands_on_nothing_is_not_an_action() {
        let mut state = UiState::new();
        state.apply(&welcome());
        let hits = vec![Hit { area: Rect::new(20, 2, 4, 1), key: KeyInput::F(8), action: None }];
        let mut queued = vec![Input::Click { col: 0, row: 0 }];
        assert!(drain(&mut state, &hits, &mut queued).is_empty());
        assert_eq!(state.tab, tipsy_ui::Tab::Chat, "and nothing on screen changed");
        assert!(state.input.is_empty());
    }

    #[test]
    fn a_click_on_a_control_raises_the_action_its_key_raises() {
        let mut state = UiState::new();
        state.apply(&welcome());
        // The header's gear is `render.rs:194-198`: the key it answers is F9, and the click opens the
        // screen through `UiState::on_hit` exactly as F9 does (`state.rs:202-234`).
        let hits = vec![Hit { area: Rect::new(78, 0, 2, 1), key: KeyInput::F(9), action: None }];
        let mut queued = vec![Input::Click { col: 78, row: 0 }];
        assert!(drain(&mut state, &hits, &mut queued).is_empty(), "opening the screen asks the client for nothing yet");
        assert!(state.config_form.is_some(), "the screen is open");
    }

    #[test]
    fn the_clock_is_the_browsers_local_time_in_the_huds_numbering() {
        // A Sunday: JavaScript numbers the week from Sunday (`Date.getDay()` 0..6) and the HUD from
        // Monday (`client.rs:716`, `now.weekday().num_days_from_monday()`), and JavaScript's month is
        // already 0-based like `now.month0()`.
        let sunday = clock_from(9, 5, 7, 0, 4, 9);
        assert_eq!(sunday.time, "09:05:07");
        assert_eq!(sunday.weekday, 6, "Sunday is the seventh day of the HUD's week");
        assert_eq!(sunday.day, 4);
        assert_eq!(sunday.month, 9);
        assert_eq!(clock_from(23, 59, 59, 1, 31, 0).weekday, 0, "Monday is 0");
        assert_eq!(clock_from(0, 0, 0, 6, 1, 11).weekday, 5, "Saturday is 5");
    }
}
```

Add `pub mod hud;` to `lib.rs` and run:

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-web hud
```

Expected: FAIL — the crate compiles and every test panics in the `todo!()` skeleton:

```
running 7 tests
test hud::tests::the_key_vocabulary_is_the_native_clients ... FAILED
...
thread 'hud::tests::the_key_vocabulary_is_the_native_clients' panicked at crates/tipsy-web/src/hud.rs:NN:5:
not yet implemented

test result: FAILED. 0 passed; 7 failed; 0 ignored; 0 measured; 0 filtered out
```

- [ ] **Step 3: Implement the four decisions**

```rust
/// `client.rs:683-707` in this crate's vocabulary. The only two decisions a browser adds are the ones
/// the native function already makes: a control chord becomes the control character no text row wants,
/// and everything the HUD does not name becomes `None`.
pub fn key_input(raw: RawKey) -> Option<KeyInput> {
    match raw {
        RawKey::Enter => Some(KeyInput::Enter),
        RawKey::Backspace => Some(KeyInput::Backspace),
        RawKey::Tab => Some(KeyInput::Tab),
        RawKey::Esc => Some(KeyInput::Esc),
        RawKey::Up => Some(KeyInput::Up),
        RawKey::Down => Some(KeyInput::Down),
        RawKey::Left => Some(KeyInput::Left),
        RawKey::Right => Some(KeyInput::Right),
        RawKey::F(n) => Some(KeyInput::F(n)),
        RawKey::Char { c, ctrl: true } => {
            let lower = c.to_ascii_lowercase();
            lower.is_ascii_lowercase().then_some(KeyInput::Char((lower as u8 & 0x1f) as char))
        }
        RawKey::Char { c, ctrl: false } => Some(KeyInput::Char(c)),
        RawKey::Other => None,
    }
}

/// `client.rs:676-680`, unchanged: the rectangles are half-open, so the cell at `area.right()` and the
/// row at `area.bottom()` are outside.
pub fn hit_at(hits: &[Hit], col: u16, row: u16) -> Option<Hit> {
    hits.iter().copied().find(|hit| {
        col >= hit.area.x && col < hit.area.right() && row >= hit.area.y && row < hit.area.bottom()
    })
}

/// The frame's worth of input. The clicks are tested against the `hits` the *previous* frame drew —
/// which is the frame the user was looking at, and the native client does exactly this
/// (`client.rs:148-151` tests the click against `hits` from the draw above it).
pub fn drain(state: &mut UiState, hits: &[Hit], queued: &mut Vec<Input>) -> Vec<UiAction> {
    let mut actions = Vec::new();
    for input in queued.drain(..) {
        match input {
            Input::Key(key) => actions.extend(state.on_key(key)),
            Input::Click { col, row } => {
                if let Some(hit) = hit_at(hits, col, row) {
                    // The click names its own target where the control has one: `on_hit` dispatches
                    // `hit.action` and falls back to `hit.key` (`state.rs:202-234`), which is the whole
                    // of what keeps a click on "Profiles" from arriving as F9.
                    actions.extend(state.on_hit(&hit));
                }
                // A click on nothing is nothing. `client.rs:148-151` ignores it too, and inventing a
                // focus change or a "miss" notice here would make a stray click change the screen.
            }
        }
    }
    actions
}

/// The browser's local time, in the HUD's numbering. `client.rs:710-720` is the native twin and the
/// same numbers come out of it; the mapping that is not obvious is the weekday, because
/// `Date.getDay()` numbers from Sunday and the HUD numbers from Monday.
pub fn clock_from(hours: u32, minutes: u32, seconds: u32, js_day: u32, day: u32, month: u32) -> ClockView {
    ClockView {
        time: format!("{hours:02}:{minutes:02}:{seconds:02}"),
        weekday: ((js_day + 6) % 7) as u8,
        day: day as u8,
        month: month as u8,
    }
}
```

Run the same command as Step 2.

Expected: PASS — `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`.

- [ ] **Step 4: Fill in the render loop and the two event callbacks**

In `crates/tipsy-web/src/lib.rs`, inside Task 3.2's `start()` — the contract's name (`tipsy_web::start()`, `#[wasm_bindgen(start)]`) — add the HUD half. Task 3.2's socket and Task 3.3's gate stay where they are; this step is the loop that draws and the two handlers that feed it. Read 3.2's file first: it owns the module list, the socket handle and the `UiState`.

```rust
use ratzilla::event::{KeyEvent, MouseEvent, MouseEventKind};
use ratzilla::ratatui::Terminal;
use ratzilla::{WebGl2Backend, WebRenderer};   // the backend Task 3.1's verdict named

/// Everything the render loop and the two callbacks share. `Rc<RefCell<..>>` because the callbacks are
/// `'static` closures and `draw_web` takes the terminal by value (research note §1, §6.4).
struct Shared {
    state: UiState,
    /// The hits of the frame that is on screen, which is what a click is tested against.
    hits: Vec<Hit>,
    /// Input since the last frame, in arrival order.
    queued: Vec<hud::Input>,
    /// The clock, refreshed once per frame from the browser's own time.
    clock: ClockView,
    title: String,
    /// Task 3.2's sender for everything the HUD asks the server for.
    to_server: tokio::sync::mpsc::UnboundedSender<ToServer>,
    /// Task 3.2's socket, for the one action that closes it.
    socket: web_sys::WebSocket,
    /// Task 3.3's context, created and resumed inside the press-to-start click.
    ctx: AudioContext,
    /// Task 3.4's timeline, which `Replay` replays from.
    playback: Playback,
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    // Task 3.3's gate created and resumed the AudioContext inside the press-to-start click (research
    // note §5.2: "Create or resume context from inside a user gesture") and Task 3.2's socket is open
    // with its `to_server` sender in hand; this half is the HUD.
    let backend = WebGl2Backend::new()?;
    let mut terminal = Terminal::new(backend)?;
    let shared = Rc::new(RefCell::new(Shared {
        state: UiState::new(),
        hits: Vec::new(),
        queued: Vec::new(),
        clock: ClockView { time: String::new(), weekday: 0, day: 0, month: 0 },
        title: String::new(),
        // The three handles Tasks 3.2 and 3.3 already hold, under the names they gave them.
        to_server,
        socket,
        ctx,
        playback: Playback::new(),
    }));

    // INFERRED (not read in a primary source): WebGL2's own docs say the canvas must be focusable and
    // that a backend may register no key handler and still return `Ok(())`. Setting the attribute is
    // harmless when the backend already did it, and the manual check below is what proves the keyboard.
    if let Some(canvas) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector("canvas").ok().flatten())
    {
        canvas.set_attribute("tabindex", "0")?;
    }

    // Registered BEFORE `draw_web`: it consumes the `Terminal` by value, so a callback added
    // afterwards has nothing to attach to (research note §1).
    terminal.on_key_event({
        let shared = Rc::clone(&shared);
        move |event: KeyEvent| {
            // The adapter from Ratzilla's `KeyEvent` to `hud::RawKey`. Its exact variant list is not in
            // the research note: open https://docs.rs/ratzilla/0.3.1/src/ratzilla/event.rs.html — the
            // page §2b read for the mouse types — and fill this match from the real `KeyCode` and
            // `KeyModifiers`. `key_event.code` is the field the README's example uses.
            if let Some(raw) = raw_key(&event)
                && let Some(key) = hud::key_input(raw)
            {
                shared.borrow_mut().queued.push(hud::Input::Key(key));
            }
        }
    })?;
    terminal.on_mouse_event({
        let shared = Rc::clone(&shared);
        move |event: MouseEvent| {
            // Ratzilla 0.3.1's mouse event, quoted from `event.rs` in the research note §2b:
            //     pub struct MouseEvent { pub kind: MouseEventKind, pub col: u16, pub row: u16,
            //                             pub ctrl: bool, pub alt: bool, pub shift: bool }
            // and the coordinates are grid cells: "Coordinates are reported as terminal cell positions
            // (`col`, `row`), not raw pixel coordinates. The origin (0, 0) is the top-left cell".
            //
            // `ButtonDown` only, on purpose: the two 0.3.1 pages disagree about what `WebGl2Backend`
            // reports for `SingleClick`/`Entered`/`Exited` (note §2b), and `ButtonDown` is the one
            // variant both pages agree on — it is also the only one the native client matches
            // (`client.rs:148`). 0.3.1 has no wheel variants at all, which costs this client nothing.
            if matches!(event.kind, MouseEventKind::ButtonDown(_)) {
                shared.borrow_mut().queued.push(hud::Input::Click { col: event.col, row: event.row });
            }
        }
    })?;

    terminal.draw_web(move |frame| {
        // ONE borrow for the whole frame: the callbacks above cannot run while it is held, which is
        // what keeps a keystroke from landing in the middle of a draw.
        //
        // This closure is the whole loop, and it runs when the browser gives us a frame: `requestAnimationFrame`
        // is paused in a background tab (research note §6.4), so a hidden page draws nothing — which is
        // fine and must stay the only effect of being hidden. Do NOT add a `visibilitychange` handler
        // that mutes the audio, suspends the context or stops the socket: Task 3.4's Step 6 records why
        // (the web client exists for a phone in another room), and the reply's audio keeps playing while
        // the page is hidden precisely because the nodes are already scheduled.
        let mut shared = shared.borrow_mut();
        shared.clock = browser_clock();
        let actions = hud::drain(&mut shared.state, &shared.hits, &mut shared.queued);
        shared.hits = draw(frame, &shared.state, &shared.clock, &shared.title);
        for action in actions {
            dispatch(action, &mut shared);
        }
    });
    Ok(())
}

/// The clock the HUD shows, read from the browser once per frame. `client.rs:710-720` is the native
/// twin; `js_sys::Date`'s getters are INFERRED (the note read `js_sys::Uint8Array`, not `Date`), so the
/// implementer checks https://docs.rs/js-sys/0.3.106/js_sys/struct.Date.html and maps the six numbers
/// through `hud::clock_from`.
fn browser_clock() -> ClockView {
    let now = js_sys::Date::new_0();
    hud::clock_from(
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds(),
        now.get_day(),
        now.get_date(),
        now.get_month(),
    )
}
```

`dispatch` is the action loop's other half — the native client's `on_action` (`client.rs:333-414`) with a socket instead of a file. The five `UiAction` variants and their browser meanings are the whole of it:

```rust
fn dispatch(action: UiAction, shared: &mut Shared) {
    match action {
        // Everything the HUD asks the server for: `Ask`, `Say`, `ListenStart`, `ListenStop`, `Reset`,
        // and the screen's `GetConfig`/`UpdateConfig`/`TestProvider`. Task 3.2 owns the sender and the
        // `Hello` that must have gone first; a closed socket drops the message, which is what the native
        // client does with a closed channel (`client.rs:368` ignores the send's result).
        UiAction::Send(command) => {
            // The barge-in, in the order the native client does it (`client.rs:361-367`, `:578-580`,
            // `:658-660`) and with Task 3.4's own rule and timeline: the user taking the floor silences
            // the device *before* the command that starts the next turn is sent, so no syllable of the
            // old reply is heard after the keystroke. Five lines, and the guard is the native one.
            let now = shared.ctx.current_time();
            if interrupts_playback(&command) && shared.playback.is_playing(now) {
                web_audio::silence(&shared.playback.barge_in(now));
            }
            if starts_turn(&command) {
                shared.playback.forget_reply();
            }
            let _ = shared.to_server.send(ToServer::Command(command));
        }
        // There is no terminal to restore in a browser: the socket closes and the next frame is the last
        // one drawn. MDN `WebSocket.close()`; the binding's exact name joins Step 4's INFERRED list.
        UiAction::Quit => {
            let _ = shared.socket.close();
        }
        // `client.rs:371-377`: what `Replay` replays is the reply the client kept, whatever entry the
        // HUD selected — the native client ignores the index too. Task 3.4 owns the audio and the
        // timeline, and a `Replay` that is audible does not touch the sequence position.
        UiAction::Replay(_) => {
            if let Some(scheduled) = shared.playback.replay(shared.ctx.current_time()) {
                if let Err(e) = web_audio::play(&shared.ctx, &scheduled) {
                    web_sys::console::error_1(&e);
                }
            }
        }
        // F9 and the gear. There is no `config.toml` in a browser, so the screen is read over the wire
        // — Phase 1's `GetConfig` (spec §4.2, §6.2: "Reading: `GetConfig` on open"). The screen is
        // already up, on the placeholder form `toggle_config_form` put there (`state.rs:171-181`); the
        // answer fills it through `Event::Config` (Task 1.1's arm) and `show_config_form`
        // (`state.rs:187-191`), which is also what marks it filled and so saveable.
        UiAction::OpenConfig => {
            let _ = shared.to_server.send(ToServer::Command(Command::GetConfig));
        }
        // Every other screen action goes through the form's own reducer, exactly as it does natively
        // (`client.rs:392-411`), and the effects it returns are what the browser has to do about it:
        UiAction::Config(action) => {
            let effects = match shared.state.config_form.as_mut() {
                Some(form) => form.apply(action),
                None => Vec::new(),
            };
            for effect in effects {
                match effect {
                    // The save, over the wire. TASK 1.1'S CONVERSION, BY THE NAME ITS BODY PINS:
                    // `ConfigView` moves the form's shape into `tipsy-protocol` and `tipsy-ui`'s form
                    // becomes a view of it (spec §4.3), so this is either a `From` impl or the view's own
                    // accessor. The native `config_from` (`crates/tipsy-compadre/src/config/form.rs:17-102`)
                    // is out of reach: `tipsy-compadre` is the server binary, not a wasm dependency.
                    ConfigEffect::Save => {
                        if let Some(form) = shared.state.config_form.as_ref() {
                            let config = ConfigView::from(form);
                            let _ = shared.to_server.send(ToServer::Command(Command::UpdateConfig { config }));
                        }
                    }
                    // `ConfigEffect::Test(capability)`: the probe runs against the server's providers
                    // (spec §5.2) and its answer is `Event::TestResult`, which `UiState` already routes
                    // to the screen's notice line (`state.rs:343-345`, `client.rs:205-209`). It says
                    // nothing out loud, so it must not quiet a reply being spoken — the native client
                    // decides that from `test_interrupts_playback` (`client.rs:642-652`) and this arm
                    // never goes near the playback.
                    ConfigEffect::Test(capability) => {
                        let _ = shared.to_server.send(ToServer::Command(Command::TestProvider { capability }));
                    }
                    // No command in the protocol this plan fixes mints a token: §4.2 lists `GetConfig`,
                    // `UpdateConfig` and `SetKey`, and `store::new_token` (`config/store.rs:28-48`) lives
                    // in the server process. The browser therefore cannot regenerate one, and this arm
                    // must NOT invent a command or mint a token client-side. Leave the row inert and
                    // report it: the Server tab's Regenerate row is native-only until a plan defines a
                    // command for it.
                    ConfigEffect::RegenerateToken => {}
                    ConfigEffect::None => {}
                }
            }
        }
    }
}
```

`Event::Config` is Phase 1's and `UiState::apply` already has its arm (Task 1.1); the browser only has to make sure the event reaches `apply`, which Step 5 of Task 3.4 is where the socket's messages are drained. If that arm is not filling the form when this task runs, it is a Task 1.1/1.5 finding: report it, do not add an arm in `tipsy-ui` from here.

- [ ] **Step 5: Write the host page**

Create `web/client/index.html`:

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>tipsy-compadre</title>
<!--
  The web client's host page (v1 spec §10, `spec:379-381`): a minimal page that loads the bundle and a
  monospace font with Braille glyphs. It fetches nothing at run time — the server embeds it and serves
  it with `Referrer-Policy: no-referrer` and `Cache-Control: no-store` (spec §5.3, `server::assets`).
-->
<style>
  html, body { margin: 0; height: 100%; background: #000; color: #d0d0d0; }
  /* The font stack is the one Task 3.1's spike rendered the Braille canvases with, first. It is part
     of the LAYOUT CONTRACT, not a cosmetic choice, which is why the family is named rather than left to
     a generic `monospace`.

     `tipsy-ui` measures its header with `drawn_width`, which draws a span and reads the column the
     *next* one lands in, because "A span is laid out by one width table and drawn by another, and the
     two disagree on `◉`, `⚙` and `·` — each one column to `Span::width` and two as drawn"
     (`render.rs:221-222`, the whole method at `:218-244`). The browser adds a third table: the font's
     own advance widths. A font that draws one of those three glyphs two cells wide spills into the
     cell the header measured for the span after it — the badge, the profile's title, the mode word and
     the gear all shift, and the work `drawn_width` does is defeated by the font rather than by ratatui.

     The glyphs that must be one cell each: `◉` (the badge's first character, `render.rs:167`, and
     printed inside the rings canvas, `render.rs:545`), `⚙` (the gear, `render.rs:174`), `·` (the help
     row's separator, `strings.rs:90`) and U+2800..U+28FF (the two Braille canvases, `render.rs:527`,
     `:577`; the ambiguous-width class is still open in Ratzilla issue #135, note §2a). The first family
     in the stack that agrees with all of them is the one to keep, and a candidate that fails any of
     them is removed rather than left in as a fallback: a fallback chain that disagrees per glyph is
     worse than one font that agrees. A box or a doubled cell is a FAILED check, not a cosmetic detail.
     Task 3.1's verdict says whether its page passed this; the manual check below re-tests it on the
     header. */
  #hud, #boot { font-family: "Fira Code", "Cascadia Mono", "DejaVu Sans Mono", Consolas, monospace; }
  #hud { position: absolute; inset: 0; }
  #boot { position: absolute; inset: 0; display: flex; flex-direction: column; gap: 1rem;
          align-items: center; justify-content: center; background: #000; }
  #boot.hidden { display: none; }
  #boot .sample { font-size: 20px; color: #00d2ff; }
  #boot button { font: inherit; font-size: 16px; padding: .5rem 1rem; color: #000;
                 background: #00d2ff; border: 0; cursor: pointer; }
  #boot p { max-width: 32rem; text-align: center; line-height: 1.4; }
</style>
</head>
<body>
  <div id="hud"></div>

  <!-- The gate. Task 3.3 owns its behaviour and its copy: its handler calls getUserMedia AND creates or
       resumes the AudioContext inside this click (research note §5.2 — a context created outside a user
       gesture comes up suspended, and a client that looks started would be silent). This markup is the
       host its states write into; the ids are `boot` and `start`, and if Task 3.3's code names others,
       they are the one thing to change here. -->
  <div id="boot">
    <div class="sample">⠿ ◉ ⠿</div>
    <h1>tipsy-compadre</h1>
    <button id="start" type="button">press to start</button>
    <p>The browser needs your permission to use the microphone and to play the reply. Nothing leaves
      this machine except what your own tipsy-compadre server sends to the providers you configured.</p>
  </div>

  <script type="module">
    // The bundle's JS entry: `wasm-bindgen --target web` output (Task 3.6, `--out-dir web/dist`), served
    // from `/assets/dist/`. INFERRED (note §3.4 marks it UNVERIFIED): the generated `init()` resolves
    // `tipsy_web_bg.wasm` relative to the module's own URL, so both files sitting in one directory is
    // what makes `init()` work with no argument. If the check below sees a 404 for the `.wasm`, read the
    // generated file's header and pass the path its `init` accepts.
    import init from "/assets/dist/tipsy_web.js";
    await init();
    // `tipsy_web::start()` is `#[wasm_bindgen(start)]` (Task 3.2's name): it has run by the time
    // `init()` resolves.
  </script>
</body>
</html>
```

The font is a decision with two branches, and which one applies is Task 3.1's recorded glyph result (Step 1):

- **3.1 recorded that these glyphs agreed** (each one cell wide on its page): the stack above is kept as it is, and the ledger's task line says the check was performed and passed, naming the browser and the family that won.
- **3.1 recorded a mismatch** (a box, a doubled cell, or the header's spans shifted): the mismatching family is **removed from the stack**, and the family that agreed — or the first of the remaining candidates that does — is put first, with the manual check below confirming it on the header. If no available family agrees, that is a `spec:389-391`-class finding: record it in the ledger with the evidence and report it, because an ambiguous-width glyph (`◉`) or a Braille cell that is not one column wide breaks the HUD's own geometry, and the only remaining lever is inside `tipsy-ui` (`render.rs`'s glyphs), which this task does not own. Do not paper over it with CSS (`letter-spacing`, a per-glyph `font-feature-settings` or a forced `font-variant-east-asian`) — the header's columns are computed in Rust, before the browser sees a glyph, so the browser can only be made to agree, never compensated for.

The two paths in this file (`/assets/dist/tipsy_web.js` and the wasm next to it) are the join between this page and Task 3.6's `--out-dir`. Task 3.6 re-reads this file after `wasm-bindgen` has actually produced its output and fixes that one line if the names differ; whoever runs it first, the paths must agree in exactly these two places.

- [ ] **Step 6: Run the crate's whole suite and clippy**

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-web
```

Expected: all green — this task's 7 tests, Task 3.4's 14, and whatever Tasks 3.2 and 3.3 added. `tipsy-ui` and `tipsy-protocol` are untouched by this task, so their suites must be unchanged.

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo clippy -j 1 -p tipsy-web --all-targets
```

Expected: nothing new against `.superpowers/sdd/2026-10-11-plan-3-web-tier/global-constraints.md:26-33`, which has no `tipsy-web` entry. Never run `cargo fmt`.

- [ ] **Step 7: Manual check — the page, the HUD and the keys (recorded here, run after Task 3.6)**

There is no browser in this harness, so this is a documented check with exact observations, not a test. The full version needs the built bundle (Task 3.6) and must happen on a machine with a browser; the first two steps work before 3.6 exists and are worth doing now.

**Now (no bundle needed):**

1. Serve the page statically: `npx --yes serve -l 8080 web` from the repository root (or `python -m http.server 8080 --directory web`).
2. Open `http://127.0.0.1:8080/client/index.html`.
   **Expected (the font):** black page, the sample line `⠿ ◉ ⠿` drawn as three *dots, a circle, three dots*, the title, and the button. The console reports a 404 for `/assets/dist/tipsy_web.js` — that is the expected state before Task 3.6, and it proves the page is not reaching for anything else. If the sample renders as boxes the family has no Braille; if any glyph is not one cell wide the family disagrees with the table `drawn_width` measures with (`render.rs:221-222`), and the ring canvases and the header will both show it. This is where a candidate family is accepted or struck out of the stack, family by family, before the bundle exists.
   To check the same three glyphs the header uses, add them to the sample line while checking — `⠿ ◉ ⚙ · ⠿` — and put the file back to `⠿ ◉ ⠿` before the commit if the extra glyphs are not wanted on screen. `⚙` and `·` are what the header's gear and the help row are made of (`render.rs:174`, `strings.rs:90`).

**After Task 3.6's `dist`:**

3. `cargo xtask dist`, then `target/release/tipsy-compadre.exe serve`, then open `http://127.0.0.1:7070/`.
   **Expected:** the same gate screen; the console has no errors and one `tipsy_web_bg.wasm` request answering 200 with `application/wasm`.
4. Click **press to start** and grant the microphone.
   **Expected:** the overlay disappears; the header shows `◉ EL COMPAE` (the profile's title), the mode word (`push` by default) and the red `ESCUCHANDO` status once the mic is open; the three rings rotate and pulse, and the oscilloscope draws a cyan wave.
   **Acceptance (the header's geometry — this is a separate pass/fail from the rest of this item):** the badge's `◉`, the title, the status text, the mode word and the `⚙` sit in the columns the header measured, i.e. the `⚙` is flush against the right edge of the frame and the mode word ends exactly where the gear's rectangle begins (`render.rs:186-198`). Zoom the browser to 125 % and 150 % and check it again: the three glyphs `tipsy-ui` cannot agree on (`◉`, `⚙`, `·`, `render.rs:221-222`) must still be one cell each, because the header's columns are computed in Rust and the font is the third width table the browser adds. `◉` pushing the spans after it, a gap before the `⚙`, or a `⚙` that no longer sits under the cell the click test in item 8 uses are all failures of **this** item, and they are the font's, not the HUD's. Record the family and the zoom levels that passed.
5. Press `Tab`.
   **Expected:** the microphone opens — the header's mode word and status turn red, and the red mic envelope draws in the oscilloscope. Press `Tab` again: the mic closes.
6. Type `hola` and press `Enter`.
   **Expected:** the letters appear in the question box as they are typed (this is the check for the note's worst failure mode: `on_key_event` on `WebGl2Backend` "will silently succeed without registering any handlers", so **nothing appearing is the failure**, and the fix is the `tabindex="0"` step above — verify the canvas actually carries the attribute in the inspector).
7. Click `NETWORK` in the left panel.
   **Expected:** the panel switches to the network chart (`render.rs:311-313` maps that rectangle to `F8`).
8. Click the ⚙ in the header.
   **Expected:** the full-screen configuration screen opens, exactly as `F9` opens it. Then click one row below the header at the same column.
   **Expected:** nothing happens. The gear's rectangle is one row tall (`render.rs:194-198`); a click that opens the screen from outside it is the off-by-one `DomBackend` bugs #179/#188 describe (note §0) showing on the chosen backend.
9. Click the gear again (or `F9`) to close the screen, then check the browser's reserved keys, one at a time, because a key the browser eats is a control with no keyboard path and every action must have both (v1 spec §9, `spec:373-374`): `Tab`, `F2` (replay), `F3` (mic), `F4` (clear), `F6`, `F7`, `F8`, `F9`, `Esc`, the four arrows, and `Ctrl+S`/`Ctrl+R` inside the configuration screen.
   **Expected:** each has its documented effect and **none** moves the browser's focus away from the canvas. `Esc` closes the client (its socket closes; the last frame stays on screen).
10. Resize the window.
    **Expected:** the HUD re-lays out and the rings stay round. A blank screen after a resize is `DomBackend` #190 (note §0); on `WebGl2Backend` it is a failure worth recording.

Record the outcome of every step in the ledger, including the backend, the font family that won and the ones struck out, the glyph result for `◉`/`⚙`/`·` at each zoom level, the header-alignment verdict of item 4, and the browser (name and version). If Task 3.1's verdict already answered the glyph question for its own page, say here whether this page reproduced it — a spike's font result that the shipped page does not reproduce is a finding, since the spike's page and this one are supposed to be the same experiment.

- [ ] **Step 8: Commit**

```bash
git add crates/tipsy-web/src/hud.rs crates/tipsy-web/src/lib.rs web/client/index.html
git commit -m "feat(web): render the HUD in the browser and translate its input

tipsy_ui::draw is called verbatim through draw_web, with the state in a shared
UiState and the clock read from the browser once a frame. Keys go through the
native client's own vocabulary and clicks through its own hit test, so no click
lands on anything its key would not and a click on nothing is nothing. The host
page carries the font the header's glyph widths depend on (`render.rs:221-222`),
the press-to-start gate and the bundle's entry.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 3.6: `cargo xtask dist` and the flag that turns true `[small]`

**Files:**
- Create: `xtask/src/dist.rs` (the four steps, the version pin and the asset check), with its own `#[cfg(test)] mod tests`
- Modify: `xtask/src/main.rs` (Task 2.3's dispatch: add the `dist` subcommand)
- Modify: `.gitignore` (ignore the generated bundle directory)
- Modify: `web/client/index.html` (only the bundle's import path, and only if `wasm-bindgen`'s output names differ from the ones this body pins)
- Modify: `README.md` (the full-distribution build, and what a plain `cargo build` produces)
- Modify: `docs/architecture.md` (the crate table: `xtask` as a real crate, `tipsy-web` as a real crate)
- Test: `xtask/src/dist.rs` (its own `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: Task 2.3's `xtask` crate, its subcommand dispatch in `xtask/src/main.rs` and its xterm.js fetch-and-verify entry point (this task calls it — the call site below is where it lands); `crates/tipsy-web` (Task 3.2) as the wasm crate, whose locked `wasm-bindgen` version is the pin; `crates/tipsy-compadre/src/server/assets.rs` (Task 1.5): the embed folder it covers, `asset(path)`, `PAGE_CLIENT`, `PAGE_TERMINAL`, `page_missing()`; `Welcome.web_bundle`'s truth (Task 1.5, spec §5.4).
- Produces: `xtask::dist::{run, steps, Step, target_dir, wasm_artifact, locked_wasm_bindgen, missing_assets, ensure_wasm_built}`; the `dist` subcommand.

- [ ] **Step 1: Read Task 2.3's xtask and Task 1.5's assets before writing anything**

1. `xtask/src/main.rs` (Task 2.3's): the subcommand dispatch and the name of its fetch-and-verify entry point. This task calls that function first and does not re-implement the fetch or the hash: the pinned xterm.js version and its sha256 are Task 2.3's and are never invented here.
2. `crates/tipsy-compadre/src/server/assets.rs` (Task 1.5's): the folder its `#[derive(Embed)]` covers, and the path `asset()` takes. **This task's output directory, the page's import path and the embed must be one directory.** The body below writes `web/dist/` and `/assets/dist/…`; if 1.5 chose another name, substitute it in exactly three places — the `--out-dir`, `missing_assets`' required list, and `web/client/index.html`'s import.
3. `.cargo/config.toml` (read today, 5 lines: `[build] target-dir = "G:/cargo-targets/tipsy-compadre/target"`, and its own comment says it "stays out of git"): there is **no** `[alias]` section, so `cargo xtask dist` cannot work yet. Add the alias locally and do not commit it:

```toml
[alias]
xtask = "run --package xtask --"
```

The README below therefore documents `cargo xtask dist` (the spec's own spelling: `spec:76-77`, `architecture.md:84`) **and** `cargo run -p xtask -- dist` as the form that works without the alias. `.cargo/config.toml` is machine-local here (`.git/info/exclude`), so `git add` must not name it.

- [ ] **Step 2: Write the failing tests and watch them fail**

Create `xtask/src/dist.rs` with the shapes, `todo!()` bodies and this test module, and add `mod dist;` plus the `dist` arm to Task 2.3's dispatch in `xtask/src/main.rs`.

```rust
//! `cargo xtask dist`: the WebAssembly bundle first, then the binary that embeds it.
//!
//! The order is spec §8.3's, verbatim: "fetch and verify xterm.js (W5), `cargo build --target
//! wasm32-unknown-unknown -p tipsy-web --release`, `wasm-bindgen --target web` (CLI version pinned to
//! the locked crate, because a schema mismatch is enforced), then the host binary with `web/`
//! embedded." The four steps are named by [`Step`] so the order is one test and not four comments.

use std::path::{Path, PathBuf};

/// The four steps, in the order they must run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Task 2.3's fetch-and-verify of the pinned xterm.js (W5).
    Fetch,
    /// `cargo build --target wasm32-unknown-unknown -p tipsy-web --release`.
    BuildWasm,
    /// `wasm-bindgen --target web --out-dir <bundle>`.
    Bindgen,
    /// `cargo build -p tipsy-compadre --release`, which embeds the bundle Task 1.5's `assets.rs` names.
    BuildHost,
}

pub fn steps() -> [Step; 4] { todo!() }

/// Cargo's target directory for this run: the environment's own `CARGO_TARGET_DIR` when it names one,
/// and `target/` under the workspace root otherwise. The same value is passed to both `cargo`
/// invocations as `--target-dir`, so the artifact's path is derived once and never guessed twice.
///
/// The environment is read in the caller and passed in, because a test that sets an environment
/// variable is a test that races every other test in the binary.
pub fn target_dir(root: &Path, from_env: Option<std::ffi::OsString>) -> PathBuf { todo!() }

/// Where cargo puts the wasm build: `<target-dir>/wasm32-unknown-unknown/release/tipsy_web.wasm` —
/// cargo's documented layout for a package named `tipsy-web`. INFERRED (`<crate-name>.wasm` from the
/// crate name, with `-` written as `_`); Step 5 checks it against the file system before `wasm-bindgen`
/// is asked to read the file.
pub fn wasm_artifact(target_dir: &Path) -> PathBuf { todo!() }

/// The `wasm-bindgen` version in `Cargo.lock`, which is what pins the CLI: wasm-bindgen and
/// wasm-bindgen-cli "are required to have the exact same SCHEMA_VERSION in order to work together" and
/// the 0.2.129 release notes say "The CLI's schema version mismatch error now names the input Wasm file
/// and the path of the running `wasm-bindgen` binary" (research note §3.3) — so the pin is not optional.
pub fn locked_wasm_bindgen(lock: &str) -> Option<String> { todo!() }

/// The files the two pages load, missing from the bundle directory. Empty when the bundle is complete.
pub fn missing_assets(root: &Path, bundle: &Path) -> Vec<String> { todo!() }

/// The bundle is not built: the error names the file and the command that produces it.
pub fn ensure_wasm_built(artifact: &Path) -> Result<(), String> { todo!() }

pub fn run(root: &Path) -> Result<(), String> { todo!() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_steps_are_in_spec_8_3s_order() {
        assert_eq!(steps(), [Step::Fetch, Step::BuildWasm, Step::Bindgen, Step::BuildHost]);
    }

    #[test]
    fn the_target_directory_is_the_environments_or_the_workspaces_own() {
        let root = Path::new("E:/w/tipsy-compadre");
        assert_eq!(target_dir(root, None), root.join("target"));
        assert_eq!(target_dir(root, Some("E:/w/.target-sandbox".into())), PathBuf::from("E:/w/.target-sandbox"));
        assert_eq!(target_dir(root, Some("".into())), root.join("target"), "an empty value is not a directory");
    }

    #[test]
    fn the_wasm_artifact_is_cargos_path_for_the_crate() {
        let target = Path::new("E:/w/.target-sandbox");
        assert_eq!(
            wasm_artifact(target),
            target.join("wasm32-unknown-unknown").join("release").join("tipsy_web.wasm")
        );
    }

    #[test]
    fn a_missing_wasm_artifact_fails_readably_instead_of_building_a_bundleless_binary() {
        let missing = Path::new("E:/w/.target-sandbox/wasm32-unknown-unknown/release/tipsy_web.wasm");
        let error = ensure_wasm_built(missing).expect_err("a binary without a bundle is not a distribution");
        assert!(error.contains("tipsy_web.wasm"), "the error names the file: {error}");
        assert!(error.contains("wasm32-unknown-unknown"), "and the target: {error}");
        assert!(error.contains("rustup target add wasm32-unknown-unknown"), "and the step that installs it: {error}");
        assert!(error.contains("cargo build --target wasm32-unknown-unknown -p tipsy-web --release"), "and the build: {error}");
    }

    #[test]
    fn the_wasm_bindgen_pin_is_read_from_the_lock() {
        // The real entry, quoted from `Cargo.lock:4166-4170`.
        let lock = r#"
[[package]]
name = "wasm-bindgen"
version = "0.2.129"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "9bb54f33acc68fd454578d9820b0bde1a1a3d17aa17bb7b6595806d02886d409"
"#;
        assert_eq!(locked_wasm_bindgen(lock).as_deref(), Some("0.2.129"));
        // `wasm-bindgen-shared` and `wasm-bindgen-macro` come first in the real file and must not be
        // mistaken for the crate: the name is matched whole.
        let neighbours = r#"
[[package]]
name = "wasm-bindgen-macro"
version = "0.2.129"

[[package]]
name = "wasm-bindgen-shared"
version = "0.2.129"
"#;
        assert_eq!(locked_wasm_bindgen(neighbours), None, "a sibling is not the crate");
        assert_eq!(locked_wasm_bindgen(""), None);
    }

    #[test]
    fn the_embed_check_reports_every_asset_the_pages_need() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bundle = root.join("web").join("dist");
        std::fs::create_dir_all(bundle.join("vendor")).unwrap();
        std::fs::write(root.join("web").join("client").join("index.html"), b"").ok();
        std::fs::create_dir_all(root.join("web").join("client")).unwrap();
        std::fs::write(root.join("web").join("client").join("index.html"), b"").unwrap();
        std::fs::create_dir_all(root.join("web").join("terminal")).unwrap();
        std::fs::write(root.join("web").join("terminal").join("index.html"), b"").unwrap();
        std::fs::write(bundle.join("tipsy_web.js"), b"").unwrap();
        std::fs::write(bundle.join("tipsy_web_bg.wasm"), b"").unwrap();

        let missing = missing_assets(root, &bundle);
        assert_eq!(missing, vec!["vendor/xterm.js".to_string()], "the fetched asset is the one that is absent");

        std::fs::write(root.join("web").join("vendor").join("xterm.js"), b"").unwrap();
        assert!(missing_assets(root, &bundle).is_empty(), "nothing is missing once the fetch has run");
    }
}
```

Run:

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p xtask dist
```

Expected: FAIL — the crate compiles and every test panics in the `todo!()` skeleton:

```
running 6 tests
test dist::tests::the_four_steps_are_in_spec_8_3s_order ... FAILED
...
thread 'dist::tests::the_four_steps_are_in_spec_8_3s_order' panicked at xtask/src/dist.rs:NN:5:
not yet implemented

test result: FAILED. 0 passed; 6 failed; 0 ignored; 0 measured; 0 filtered out
```

If the `tests` module needs `tempfile`, add it as a dev-dependency of `xtask` **only if** a workspace member already carries `tempfile = "3.27"` (`crates/tipsy-compadre/Cargo.toml:36`, per `codebase-recon.md` §9.1): reusing a crate the workspace already has is the rule; a new crate version is not this task's to add. If the operator would rather not add it, write the fixture with `std::env::temp_dir().join(...)` and remove it at the end of the test — the `$env:TEMP` override the test command sets makes that path writable.

- [ ] **Step 3: Implement the helpers**

```rust
pub fn steps() -> [Step; 4] {
    // Spec §8.3's order. The host build is last because `rust-embed` reads the folder when the crate
    // is compiled: "In `release` … the folder path is resolved relative to where `Cargo.toml` is"
    // (rust-embed README, quoted in the research note §2.1), so the bundle must exist before it.
    [Step::Fetch, Step::BuildWasm, Step::Bindgen, Step::BuildHost]
}

pub fn target_dir(root: &Path, from_env: Option<std::ffi::OsString>) -> PathBuf {
    match from_env {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        // `.cargo/config.toml`'s `build.target-dir` is deliberately not read: both `cargo` calls below
        // pass `--target-dir` explicitly, and a `G:` directory in this checkout is the trap the
        // constraints file records (artifacts there are low-integrity). The plan's own commands set
        // `CARGO_TARGET_DIR`, which is what the operator will have.
        _ => root.join("target"),
    }
}

pub fn wasm_artifact(target_dir: &Path) -> PathBuf {
    target_dir.join("wasm32-unknown-unknown").join("release").join("tipsy_web.wasm")
}

pub fn locked_wasm_bindgen(lock: &str) -> Option<String> {
    let mut in_package = false;
    let mut name_matches = false;
    for line in lock.lines().map(str::trim) {
        if line == "[[package]]" {
            in_package = true;
            name_matches = false;
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(value) = line.strip_prefix("name = ") {
            name_matches = value.trim_matches('"') == "wasm-bindgen";
        } else if name_matches
            && let Some(value) = line.strip_prefix("version = ")
        {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

/// The pages' own assets plus the bundle's two files. `tipsy_web.js` and `tipsy_web_bg.wasm` are the
/// names `wasm-bindgen` derives from the input file's stem — INFERRED, and `run` re-reads the directory
/// after the command and reports a mismatch rather than trusting this list.
pub const REQUIRED: [&str; 5] = [
    "web/client/index.html",
    "web/terminal/index.html",
    "web/dist/tipsy_web.js",
    "web/dist/tipsy_web_bg.wasm",
    "web/vendor/xterm.js",
];

pub fn missing_assets(root: &Path, _bundle: &Path) -> Vec<String> {
    REQUIRED
        .iter()
        .filter(|relative| !root.join(relative).is_file())
        .map(|relative| relative.to_string())
        .collect()
}

pub fn ensure_wasm_built(artifact: &Path) -> Result<(), String> {
    if artifact.is_file() {
        return Ok(());
    }
    Err(format!(
        "{} does not exist: the wasm bundle was not built, and a binary built without it would serve \
         `page_missing()` instead of the web client. Install the target with `rustup target add \
         wasm32-unknown-unknown` (Task 3.1) and build it with `cargo build --target \
         wasm32-unknown-unknown -p tipsy-web --release` (the bundle's own path: {}).",
        artifact.display(),
        artifact.parent().map(|p| p.display().to_string()).unwrap_or_default(),
    ))
}
```

- [ ] **Step 4: Implement `run`, the four steps**

```rust
pub fn run(root: &Path) -> Result<(), String> {
    let target = target_dir(root, std::env::var_os("CARGO_TARGET_DIR"));
    let bundle = root.join("web").join("dist");
    let web = root.join("web");

    for step in steps() {
        match step {
            // Task 2.3's entry point, called by the name `xtask/src/main.rs` gives it: this task adds
            // no fetch and no hash. Its `xtask/vendor/README.md` records what was fetched, from where
            // and under what licence (spec §12).
            Step::Fetch => vendor::fetch_and_verify(root)?,
            Step::BuildWasm => {
                // `--target-dir` explicitly, so the artifact's path is the one `wasm_artifact` computes
                // rather than whatever `.cargo/config.toml` says. This is the step that needs Task 3.1's
                // `rustup target add wasm32-unknown-unknown`.
                cargo(&["build", "--target", "wasm32-unknown-unknown", "-p", "tipsy-web", "--release"], &target)?;
                ensure_wasm_built(&wasm_artifact(&target))?;
            }
            Step::Bindgen => {
                // The CLI's version must be the locked crate's: the schema check is enforced and its
                // error is what a mismatch looks like (research note §3.3). The documented install is
                // `cargo install -f wasm-bindgen-cli` (the guide, quoted in the note); the version pin
                // is the note's INFERRED recommendation.
                let locked = locked_wasm_bindgen(&std::fs::read_to_string(root.join("Cargo.lock")).map_err(|e| e.to_string())?)
                    .ok_or("wasm-bindgen is not in Cargo.lock: add it to tipsy-web before running dist")?;
                check_bindgen_version(&locked)?;
                std::fs::create_dir_all(&bundle).map_err(|e| format!("{}: {e}", bundle.display()))?;
                run_tool(
                    "wasm-bindgen",
                    &[
                        "--target",
                        "web",
                        "--out-dir",
                        &bundle.display().to_string(),
                        &wasm_artifact(&target).display().to_string(),
                    ],
                )?;
                // The names are INFERRED, so they are checked rather than assumed: if `wasm-bindgen`
                // wrote something else, `missing_assets` names it and `web/client/index.html`'s import
                // is the one line to correct (see the note on `REQUIRED`).
            }
            Step::BuildHost => {
                let missing = missing_assets(root, &bundle);
                if !missing.is_empty() {
                    return Err(format!(
                        "the bundle is incomplete; missing: {}. Build it before the host binary, or the \
                         embed picks up a directory that cannot serve the web client.",
                        missing.join(", ")
                    ));
                }
                cargo(&["build", "-p", "tipsy-compadre", "--release"], &target)?;
            }
        }
    }
    Ok(())
}
```

`cargo`, `run_tool` and `check_bindgen_version` are thin `std::process::Command` wrappers: inherit stdio, return `Err` with the command line and the exit status when it fails, and print every command before running it — a distribution build that needs the network and several minutes must say where it is. `check_bindgen_version` runs `wasm-bindgen --version` and fails with the readable instruction when the output is not `wasm-bindgen <locked>`:

```
wasm-bindgen 0.2.128 is installed and Cargo.lock pins 0.2.129. Install the matching CLI:
    cargo install -f wasm-bindgen-cli --version 0.2.129
```

Two decisions this step records rather than re-argues, both from the research note §3.4's recommendation (which spec §8.3 makes the plan's):

- **`trunk` is not used.** It "cannot be driven as a library" — `has_lib: false`, quoted from crates.io in note §3.2 — so an xtask could only spawn a binary that every machine that builds would have to install, and everything trunk adds (hashed assets, the `index.html` asset graph, SRI, `modulepreload`, a dev server) is discarded because the page is embedded in the server binary and served by it. Its newest version is also a pre-release (note §3.2).
- **`wasm-pack` is not used.** It *does* have a library API (`has_lib: true`, `run_wasm_pack`, `BuildOptions`, note §3.1/§3.4), but it is a heavy build dependency, its registry commands require npm ("Currently, `wasm-pack` requires that you have npm installed to pack and publish your package", note §3.1), and its `--mode no-install`/`--no-opt` flags exist precisely because it otherwise installs tools. The direct `wasm-bindgen` route is the note's recommendation and has "the smallest surface: no npm anywhere in the documented install or invocation", with `--target web` needing "no further postprocessing" (note §3.4).
- **`wasm-opt` is skipped for this first cut** (note §3.4: "skip `wasm-opt` for the first cut"). Nothing in the direct `wasm-bindgen` path requires it (note §3.3).

- [ ] **Step 5: Run it once by hand and pin the names**

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo build --target wasm32-unknown-unknown -p tipsy-web --release
```

Expected, and this is the step that proves Task 3.1's gate and Tasks 3.2–3.5's code compile for wasm: `Finished \`release\` profile`. The artifact is then at

```
$env:CARGO_TARGET_DIR\wasm32-unknown-unknown\release\tipsy_web.wasm
```

If this fails at the linker or on a missing target, stop and report it: `wasm32-unknown-unknown` not installed is Task 3.1's, and a compile error is the owning task's.

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo run -p xtask -- dist
```

Expected: the four steps print their commands in order; `web/dist/` ends up with the JS glue and the processed `.wasm`. Then:

```
Get-ChildItem web\dist | Select-Object Name, Length
```

Expected: two files. **If they are not `tipsy_web.js` and `tipsy_web_bg.wasm`**, correct the one line in `web/client/index.html` (the `import init from …` line) and `REQUIRED` in `xtask/src/dist.rs` to the names that were actually produced, and say so in the ledger. This is the only place the two files' names are pinned, and the note marks the naming rule INFERRED.

- [ ] **Step 6: Add the generated directory to `.gitignore`**

`.gitignore` is 6 lines (read today: `.env`, `target/`, `out/`, `.venv/`, `__pycache__/`, `*.log`). The generated bundle is a build artifact and the fetched third-party files are Task 2.3's business — add the one line this task owns, after `out/`:

```
# The wasm-bindgen output `cargo xtask dist` writes: a build artifact, never committed.
web/dist/
```

Do not touch `web/vendor/`'s rules: Task 2.3 owns them and its `README.md` is the one file under `web/vendor/` that is committed (spec §12).

- [ ] **Step 7: Run the whole test suite and clippy**

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p xtask
```

Expected: all green — this task's 6 tests plus Task 2.3's, with the `dist` tests passing.

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 --workspace
```

Expected: the ledger's baseline plus every task's own additions, 0 failures. The `$env:TEMP` override is mandatory (22 `tempfile::tempdir()` tests fail without it) and `-j 1` is mandatory on this machine.

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo clippy -j 1 -p xtask --all-targets
```

Expected: nothing new against `.superpowers/sdd/2026-10-11-plan-3-web-tier/global-constraints.md:26-33`, which has no `xtask` finding. Never run `cargo fmt`.

- [ ] **Step 8: Manual check — the full distribution, and the flag that turns true**

This needs the network once (the xterm.js fetch) and several minutes; there is no test that can stand in for it. Everything is a real command with a real expected output.

1. Clean up and build the distribution end to end:

```
Remove-Item -Recurse -Force web\dist -ErrorAction SilentlyContinue
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo xtask dist
```

**Expected:** four steps, in this order: the xterm.js fetch with its hash verified (Task 2.3's own output), `cargo build --target wasm32-unknown-unknown -p tipsy-web --release`, `wasm-bindgen --target web --out-dir …`, `cargo build -p tipsy-compadre --release`. Then `web/dist/tipsy_web.js` and `web/dist/tipsy_web_bg.wasm` exist, and so does `web/vendor/xterm.js`.

2. **The failure this step exists for.** Delete the artifact and run the *host* build alone:

```
Remove-Item web\dist\tipsy_web.wasm, web\dist\tipsy_web_bg.wasm -ErrorAction SilentlyContinue
$env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo build -p tipsy-compadre --release
```

**Expected:** this *succeeds* — `#[allow_missing = true]` is what makes the embed optional and no `build.rs` gate is needed ("In that case, RustEmbed will be empty", research note §2.2) — and the binary it produces is the honest no-bundle build, which is the state criterion 4 of Step 3's check describes. `dist` is the command that must refuse to get there: rerun `cargo xtask dist` with the artifact missing and confirm `ensure_wasm_built`'s message appears instead of a bundleless binary.

3. Build the plain binary and look at the page it serves:

```
$env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo build -p tipsy-compadre
$env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo run -p tipsy-compadre -- serve
```

Open `http://127.0.0.1:7070/`.
**Expected:** the page that says which command builds the bundle (`page_missing()`, spec §5.4, `architecture.md:85-86`). Note that a **debug** build reads `web/` from the directory the binary is run from — rust-embed's documented debug behaviour, "the folder path is resolved relative to where the binary is run from" (note §2.1) — so run it from the repository root, and after a `dist` the same debug binary will serve the real bundle. That is the development path and the README says so.

4. The loop the plan was written to close:

```
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo xtask dist
& "$env:CARGO_TARGET_DIR\release\tipsy-compadre.exe" serve
```

Open `http://127.0.0.1:7070/` in the browser and watch the first WebSocket frame in DevTools → Network → WS → Messages.
**Expected:** `{"type":"welcome", …, "web_bundle":true, …}` and **no** "the web UI is not available in this build" anywhere in the HUD. That is the v1 spec §10 sentence ("Both bundles are embedded in the executable … when built with `cargo xtask dist`", `spec:386`) and `architecture.md:83-86` made true, and it is exactly the assertion Task 1.5 made `Welcome.web_bundle` capable of carrying (spec §5.4). If it reads `false`, stop and report: Task 1.5's plumb to `session.rs:249` is the missing half and it is not this task's to add.

5. `http://127.0.0.1:7070/terminal` — the other bundle, unchanged by this task but built by the same command: the xterm.js page loads and the TUI runs in the PTY (Task 2.4's check).

- [ ] **Step 9: The documents this phase owes**

`README.md`. The **Building** section (`:243-259`) currently says only:

> The toolchain is pinned to Rust 1.96.1 and every crate uses `edition = "2024"`.

and shows `cargo build` / `cargo test`, with no mention of the wasm target or the xtask (`codebase-recon.md` §10.2). Add the full-distribution section after it, and leave the Git Bash `PATH` note and everything else where it is:

````markdown
### Building the full distribution

Two steps, wrapped in one command: the WebAssembly bundle first, then the binary that embeds it.

```sh
cargo xtask dist
```

It needs the network once per clean checkout (it fetches the pinned xterm.js for the web terminal) and
the wasm target, which the pinned toolchain does not install by itself:

```sh
rustup target add wasm32-unknown-unknown
```

The command runs `cargo build --target wasm32-unknown-unknown -p tipsy-web --release`,
`wasm-bindgen --target web` (the CLI's version must be the one `Cargo.lock` pins — `cargo install -f
wasm-bindgen-cli --version <that version>`), and finally `cargo build -p tipsy-compadre --release` with
the bundle embedded. If `cargo xtask` is not set up on your machine, `cargo run -p xtask -- dist` is the
same command; the alias lives in `.cargo/config.toml`:

```toml
[alias]
xtask = "run --package xtask --"
```

**A plain `cargo build` produces a binary without the web client.** The server still runs and `/` answers
with a page that says how to build the bundle. A *debug* build reads `web/` from the directory you run
the binary from, so `cargo run -p tipsy-compadre` started at the repository root serves a bundle that is
already built, and a binary copied elsewhere does not.
````

Then correct the two statements that this tier makes false:

- `:10-16` — the status paragraph's "Still to come: `hands_free` …, **the WebSocket server, the web client and the web terminal**, plus CI and releases." loses the server and the two browser routes; `hands_free` and CI stay. Phase 1's documentation task (1.9) has already edited this paragraph: re-read it and keep its wording, removing only what now ships.
- `:213-214` — "The Server tab — `bind`, the LAN switch and the token — is for the WebSocket server, which does not exist yet: the three values are written to the file and nothing listens on them today." That sentence is false from Phase 1 on; 1.9 owns the correction. If it is still there when this task runs, report it as a Task 1.9 finding rather than editing it twice.

`docs/architecture.md`. The crate table (`:73-78`) needs `xtask` and a truthful `tipsy-web` row. It reads today:

```
| Crate | Kind | Contents | Compiles to WASM |
|---|---|---|---|
| `tipsy-protocol` | library | `Command` / `Event` messages and the audio frame format | yes |
| `tipsy-ui` | library | The ratatui HUD: widgets, state and es/en strings, backend-agnostic | yes |
| `tipsy-web` | `.wasm` bundle | Web client: Ratzilla backend, browser audio, WebSocket transport | yes (target) |
| `tipsy-compadre` | **binary** | The server (brain, providers, config, axum) plus the native TUI client | no |
```

Replace the `tipsy-web` row and add the `xtask` row (Task 1.9 has already touched this file for Phase 1 — re-read it and keep the row order it left; these two lines are this task's):

```
| `tipsy-web` | cdylib | Web client: the Ratzilla backend, the browser's microphone and speakers, the WebSocket transport | yes |
| `xtask` | **binary** | The build's own tool: `cargo xtask dist` fetches and verifies the terminal assets, builds the wasm bundle, then builds the binary that embeds it | no |
```

The paragraph below the table (`:80-86`) already describes the two-step build and the bundle-less binary correctly — it was written as a promise and this task makes it true, so it needs no edit. The `session` row's "either through an in-process channel or a WebSocket" (`:122`) and the transport line in the diagram (`:37`) become true in Phase 1; that is 1.9's.

- [ ] **Step 10: Commit**

```bash
git add xtask/src/main.rs xtask/src/dist.rs .gitignore web/client/index.html README.md docs/architecture.md
git commit -m "feat(xtask): build the wasm bundle and the binary that embeds it

cargo xtask dist in spec 8.3's order: the xterm.js fetch Task 2.3 owns, the
tipsy-web release build for wasm32-unknown-unknown, wasm-bindgen --target web
with the CLI pinned to the locked crate, then the host binary with web/ embedded.
trunk and wasm-pack are deliberately not used, and a missing wasm artifact is a
readable failure instead of a binary that serves the no-bundle page.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

### Task 3.7: The login door in the browser `[mid]`

**The brief this body answers.** The login spec's closing block gives Plan 3 two things
(`docs/superpowers/specs/2026-10-11-login-screen-design.md:491-511`): "**1. The browser wiring.** §3
hands the web client the same screen and says the server needs no change for it, but the *wiring* is
per client: `tipsy-web` must show the screen on `Welcome.login`, drive its own capture (`getUserMedia`
→ `Frame::Mic`) in step with `LoginState::open`, send the same three commands and honour the
once-per-run rule." and "**2. Two sentences of this spec that Plan 3 supersedes**", whose amendment is
Step 5 below. This task is the *engine*, not the screen: `tipsy-ui`'s `LoginState`, its key table and
`render_login.rs`'s veil are already on `master` (`28c2fb8`) and **not one line of them is
reimplemented here**.

**No `Transport` trait is added**, and none exists (`session.rs:46-52`; the front matter's
"the seam is `SessionEnds` and bridges"). The browser's socket is Task 3.2's `Outbox`, its microphone
is Task 3.3's `Capture`, its action loop is Task 3.5's `dispatch`, and this task adds the one thing
none of them owns: **when the microphone's frames may leave the page.**

**The three defects this task exists to prevent**, each of them something a plausible implementation
does:

1. **Audio out of a microphone that is not open.** The login plan's whole-branch review already had to
   fix this on the native side — the client could forward login audio before `Mic::start` had opened
   the device (`crates/tipsy-compadre/src/client.rs:183-190`'s "the capture first, then the device,
   then the frames", `:385-387`'s "a queue that emptied before `Mic::start` succeeded used to send
   audio the spec promises never leaves the machine", `mic_route`'s `(false, _, _) => MicRoute::Drop`
   at `:795-802`). The browser has the same shape of hole unless it is closed on purpose: a frame
   handler that posts to the socket unconditionally puts room tone on the wire from the moment the
   page's gate was pressed, which breaks §9's promise ("Nothing reaches the provider until the local
   detector confirms speech") at its first hop.
2. **A second `Welcome` reopening a door that was passed.** The server re-sends `Welcome` on
   `SetProfile` and `SetListeningMode` — `Hub::resend_welcome` exists for exactly that
   (`docs/superpowers/plans/2026-10-11-plan-3-web-tier.md:2425-2435`) — so a client that treats every
   `Welcome` as a fresh one puts the veil back over a screen the user is already inside.
3. **A frame kept when the capture was not running.** The capture path runs on the audio thread; the
   state may not be borrowed from there. A queue that keeps what it collected while the door was shut
   sends it the moment the door opens, and the server transcribes an old sentence as the answer.

**The once-per-run rule lives in the shared state, and this body says where.** The client's screen is
`UiState::login: Option<LoginState>` (`crates/tipsy-ui/src/state.rs:198-200`), and `UiState::apply`'s
`Welcome` arm deliberately ignores the `login` field — it destructures with `..`
(`state.rs:454-475`), so **applying a `Welcome` can never put a screen up**. The only way a screen
appears is an explicit `UiState::show_login(voice)`, and `show_login` is also the rule itself: it
returns `false` without touching the screen when this run has already been asked, and a new
`pub login_seen: bool` remembers that (`state.rs:201-207` for the field, `:278-298` for the rule).
Commit `b9f2402` on `fix-login-once-per-run` is what put it there, and the native client's `Welcome`
arm is now that return value and nothing else
(`crates/tipsy-compadre/src/client.rs:241-258`):

```rust
                if let Event::Welcome { login, .. } = &event
                    && let Some(view) = login
                    && self.state.show_login(view.voice)
                    && let Some(login) = self.state.login.as_mut()
                {
                    // With the voice on, the capture opens *with* the screen: the reconciliation
                    // below sees the state on the very first frame and opens the stream then. The
                    // phase follows it, because the screen's own key table reads the phase to
                    // decide what `Tab` and `Enter` mean.
                    login.open = view.voice;
                    login.phase = if view.voice { LoginPhase::Listening } else { LoginPhase::Waiting };
                }
```

So **`tipsy-ui` owns the state machine, including this rule, and this task does not patch around it**:
`state.rs` gains no field for the browser's benefit, `LoginState` gains no variant, and nothing in
`tipsy-web` second-guesses a phase. The browser does not carry a second copy of the bit either, and
that is the correction this body makes: `login_seen` is per-run state about *the client's own screen*,
but "the client" is not only the native one. A page reconnects, the session re-sends `Welcome` on
every `SetProfile` and `SetListeningMode`, and the native client asks the same question of the same
`UiState` — so a browser-side `passed: bool` would be a second home for a rule that already has one,
and the two would drift the first time either client changed. The rule is asked once, where it lives:
`state.show_login(view.voice)` returns whether *that call* was the one that showed the screen
(`state.rs:284-298`), and a `false` is what keeps defect 2 from happening. What arrives with it is the
second half of the same fix: a pass now closes the capture in the reducer (`state.rs:561-572`,
`login.open = false`; `show_login` overrides it to `true` only for the first frame of a *new* screen at
`:289-293`, which is the one caller that may), so the browser's frame gate follows `open` and the
`LoginStop` spec §6.2 requires falls out of the reconciliation that was already there.

**Files:**
- Create: `crates/tipsy-web/src/login_door.rs` — **this task's own file**: the first-show call into
  `UiState::show_login`, the capture reconciliation and the one routing predicate, all pure, with its
  own `#[cfg(test)] mod tests`
- Modify: `crates/tipsy-web/src/lib.rs` — **Task 3.2 creates this file** (the `#[wasm_bindgen(start)]`
  entry, the `mod` list, the render loop) and **Task 3.5 modifies it again** (the `Shared` struct, the
  two Ratzilla callbacks, `hud::drain`, `dispatch`); this task adds `pub mod login_door;`, the
  `Door::note_event` call beside `state.apply`, the one `sync` call after `hud::drain` that pushes its
  `Command` into the same action path, `drain_capture` where Task 3.4's `ToClient::Audio` routing is,
  and one `UiAction::LoginDone` arm in `dispatch` whose whole body is `state.login = None`
- Modify: `crates/tipsy-web/src/mic.rs` — **Task 3.3 creates this file** (the pure half — `MicState`,
  `MicAvailability`, `FrameAssembler`, `resample_linear`, `f32_to_i16`, `rms` — plus `Capture`,
  `Gate`, `press_to_start` and `open_capture`); this task adds the queued-capture frames, their
  drain, and the frame path `Door`'s predicate gates — which is the state's own `voice && open`, so no
  pass/fail bookkeeping lands in this file. **No test in this file changes**
- Modify: `docs/superpowers/specs/2026-10-11-login-screen-design.md` — the two sentences Plan 3
  supersedes, and the record that this tier made the section editable (§13's closing block, item 2)
- Modify: `README.md` — the login section's "no Login tab" sentence and one release-checklist line for
  the manual check of Step 6
- Test: `crates/tipsy-web/src/login_door.rs` (its own `#[cfg(test)] mod tests`, run on the host)

**A note on the plan's own front matter, and why this task does not edit it.** By the time this body
was spliced in, the plan's front matter already carried the task: the index row is at
`docs/superpowers/plans/2026-10-11-plan-3-web-tier.md:286` — `- Task 3.7: The login door in the browser
— the engine, not the screen `[mid]`` under "Phase 3 — the web client" — and the phase table at `:62`
reads `3.1 – 3.7`. Whoever splices a later revision of this chunk into the document should confirm
those two lines are still there rather than adding them again; the body is written so that it needs
no edit to the front matter of its own. It is listed here rather than done silently because a task
index that does not name this task is a reader's dead end.

**Interfaces:**
- Consumes:
  - `tipsy_protocol::{Command, Event, Frame, LoginView, ToServer}` — and **`LoginView` here is
    `Welcome`'s login-*screen* payload** (`crates/tipsy-protocol/src/lib.rs:99-105`, `{ voice: bool }`),
    which is the type this task's engine is handed and the only field it reads. It is **not** the
    `[login]` configuration section: Task 1.1 added that one as a *separate* type, `LoginSettingsView`,
    because the name `LoginView` was already taken by the screen payload — an earlier draft of this
    bullet treated the two as one type and told the implementer to expect the section's three fields
    here, which would not compile. **Read the file before writing a literal.**
  - `tipsy_protocol::Frame::Mic(Vec<i16>)` — `crates/tipsy-protocol/src/lib.rs:174-177`
  - `Event::Welcome { .., login: Option<LoginView> }` — `lib.rs:135-145`; `Event::LoginVerdict
    { ok, name }` — `:170-171`; `Event::LoginHeard { text }` — `:167-169`
  - `Command::{LoginStart, LoginStop, LoginAnswer { text }}` — `lib.rs:122-129`
  - `tipsy_ui::{UiState, UiAction, LoginPhase, LoginLine}` and the login's own state:
    `UiState.login: Option<LoginState>` (`state.rs:198-200`) and **`UiState.login_seen: bool`**
    (`:201-207`, the run-scoped bit), `UiState::show_login(voice) -> bool` (`:278-298`; `true` only
    for the call that showed the screen, and it is what sets `login_seen`, `open: false`,
    `phase: Waiting`, `line: Prompt`, `refusals: 0`), `LoginState { input, voice, open, phase, line,
    name, refusals }` (`:148-159`), `LoginPhase` (`:114-124`), the login's key table
    (`state.rs:362-410` — `:383` `LoginAnswer`, `:391` and `:405` `LoginStart`/`LoginStop`, `:379`
    `LoginDone` for `Enter` on a `Passed` screen), the `LoginVerdict` arm (`:561-591`, whose `ok`
    branch closes the capture), the `Event::Welcome` arm that ignores `login` (`:454-475`),
    `push_mic_level` (`:736`) and `start_voice` (`:744`)
  - Task 3.2's `tipsy_web::socket::Outbox` (`Outbox::send(ToServer) -> Result<(), WireError>`) and
    Task 3.2's decoded `ToClient` queue; Task 3.3's `Capture`/`press_to_start`;
    Task 3.4's `playback::starts_turn`/`interrupts_playback`; Task 3.5's `hud::drain` and `dispatch`
  - The v1 spec's gesture rule, which is why nothing here calls `getUserMedia`:
    "In the browser, audio capture requires a user gesture; the web client shows a 'press to start'
    screen until then" (`docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md:193-194`);
    "loopback over plain HTTP **is already a secure context** for `getUserMedia`"
    (`docs/superpowers/specs/2026-10-11-web-tier-design.md:483-489`, §9.5)
- Produces:
  - `tipsy_web::login_door::{Door, MicStep}` — `Door` holds the request in flight and nothing else,
    because the once-per-run rule and the capture gate are both the state's — with
    `Door::new()`, `Door::note_event(&mut self, state: &mut UiState, event: &Event) -> bool`,
    `Door::open_screen(&mut self, state: &mut UiState, login: Option<&LoginView>) -> bool`,
    `Door::admitted(&mut self, state: &mut UiState)`,
    `Door::sends_frames(&self, state: &UiState) -> bool`,
    `Door::capture_open(&self, state: &UiState) -> bool`,
    `Door::sync(&mut self, state: &UiState) -> MicStep`, and
    `MicStep::{Start, Stop, None}`
  - `tipsy_web::mic::{levels_from_samples, drain_capture, LOGIN_ENVELOPE_POINTS}` — the level
    conversion, and the one place that turns the capture's queued frames into an `Outbox::send` per
    frame behind `Door::sends_frames`, or drops them
  - the `UiAction::LoginDone` arm in Task 3.5's `dispatch`
  - the amended login spec and the README's manual check

- [ ] **Step 1: Write the failing test**

**One thing to record in the ledger before starting, and it is not this task's to fix.**
`ProfileView.language` is a `String` — `crates/tipsy-protocol/src/lib.rs:92-97` — and Task 3.2's
`every_event()` fixture sets it to `UiLanguage::Es`
(`docs/superpowers/plans/2026-10-11-plan-3-web-tier.md:10651`), which will not compile. The compiler
will say so the first time Task 3.2's own suite runs; the fix is `language: "es-ES".into()`, and it
belongs to Task 3.2's commit, not to this one. Say so in the ledger's Task 3.2 line and leave the file
alone.

Create `crates/tipsy-web/src/login_door.rs` holding the module doc, the two shapes and the tests only,
then add `pub mod login_door;` to `lib.rs`'s module list. Nothing else is written yet, which is what
makes Step 2 fail for the right reason — the same order Task 3.2's Step 2 and Task 3.3's Step 1 use.

The tests drive the **real** `tipsy_ui` reducer (`UiState::apply`, `UiState::show_login`,
`UiState::on_key`), so the phase transitions, the `LoginVerdict` handling and the `LoginDone` action
are the shared state machine's answers and not this module's opinion of them.

```rust
//! The login door, from the browser's side: the first-show call into the shared state, the capture's
//! reconciliation and the one predicate that decides whether a frame may leave the page.
//!
//! The screen itself is `tipsy-ui`'s and is *not* reimplemented here: `UiState::login`,
//! `LoginState`'s phases, the key table that raises `LoginStart` / `LoginStop` / `LoginAnswer` and
//! `render_login.rs`'s veil all arrive from `master` and are driven by `UiState::apply` and
//! `UiState::on_key`. What this module owns is the engine around them, which is per client:
//!
//! 1. **`Welcome.login` is the only authority, once per run — and `UiState` is where that rule
//!    lives.** `UiState::apply`'s `Welcome` arm deliberately ignores the `login` field
//!    (`crates/tipsy-ui/src/state.rs:454-475`, the destructuring `..`), so a mere `Welcome` never puts
//!    a screen up; only `show_login` does — and `show_login` itself refuses once the run has been
//!    asked (`:278-298`, `login_seen` at `:201-207`). The server re-sends `Welcome` on `SetProfile`
//!    and `SetListeningMode` (`Hub::resend_welcome`), and the native client's arm is nothing but that
//!    return value (`crates/tipsy-compadre/src/client.rs:241-258`). This module carries **no second
//!    copy of the bit**: a reconnect, a re-sent announcement and a native client all ask the same
//!    state, so a browser-side flag would be a second home for a rule that already has one; and
//!    `state.login.is_none()`, the guard this replaced, cannot answer it at all, because `LoginDone`
//!    puts `login` back to `None` (`client.rs:568-576`), which is exactly what it holds before the
//!    first screen is ever shown. That is what commit `b9f2402` removed, and this module does not
//!    bring it back.
//! 2. **The microphone follows `LoginState::open`, and nothing else.** The native client reads
//!    `login.voice && login.open` once a frame and sends `LoginStart` / `LoginStop` **on the edges**
//!    (`client.rs:406-435`). The browser's `Tab`, its third refusal and a pass move exactly the same
//!    field (`state.rs:396-405`, `:561-572`, `:577-579`), so the same edge detector applies.
//! 3. **A frame leaves the page only while that capture is open.** This is the login plan's review
//!    finding 1, on the other client: the native rule is "the capture first, then the device, then the
//!    frames" (`client.rs:183-190`) and "a frame belongs to a *stream*"
//!    (`mic_route`, `:789-802`). Here the device is Task 3.3's gate and the stream is this predicate.
//!    It reads the state's `voice && open` and keeps no copy of either: a pass closes the capture in
//!    the reducer (`state.rs:561-572`), so the gate shuts with the door and the reconciliation sends
//!    the `LoginStop` spec §6.2 asks for. The browser does not decide that a capture is over; the
//!    state says so, and this module is only the reader.
//!
//! Everything in this file is pure and runs on the host target (Task 3.2 Step 1): no `web_sys` value
//! is constructed, no socket is touched, and no test needs a browser.

use tipsy_protocol::{Command, Event, LoginView, ToServer};
use tipsy_ui::{LoginPhase, UiState};

/// The browser's read of one screen. `UiState` stays the state machine, and it holds the answer to
/// every question this type asks: what is left here is the request in flight, and nothing else.
#[derive(Debug, Default)]
pub struct Door {
    /// Whether this run has told the server a capture is open. This is the native client's
    /// `login_mic` (`crates/tipsy-compadre/src/client.rs:82-85`): **the request in flight, not the
    /// capture**, so a `LoginStart` that went out is always answered by a `LoginStop` — and a screen
    /// that opens without a voice switch never sends a `LoginStart` at all.
    started: bool,
}

/// What the client still owes the server about the capture, at one frame's edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicStep {
    /// Send `Command::LoginStart`.
    Start,
    /// Send `Command::LoginStop`.
    Stop,
    /// Nothing changed.
    None,
}

impl Door {
    pub fn new() -> Self {
        Self::default()
    }

    /// The whole of `Welcome`'s login half: the screen goes up on the **first** `Welcome` that
    /// carries a `LoginView`, and on none after it. `UiState::show_login` is both the call and the
    /// rule — it returns whether *this* call was the one that showed the screen — so there is no
    /// guard to write here and no bit to keep (`crates/tipsy-ui/src/state.rs:284-298`, and the native
    /// arm that does the same thing at `crates/tipsy-compadre/src/client.rs:247-251`).
    ///
    /// Returns whether a screen went up, which is `show_login`'s own answer and is not re-derived.
    pub fn open_screen(&mut self, state: &mut UiState, login: Option<&LoginView>) -> bool {
        let Some(view) = login else {
            // `Welcome.login == None`: there is no screen, now or later (`Welcome` is the only
            // authority, spec §3).
            return false;
        };
        // `show_login` is the whole of the once-per-run rule: it returns `false` for a second view
        // while the screen is up — the half-typed answer, the phase and the refusal streak stay put —
        // and `false` again after the door was passed, which is the case `login.is_none()` cannot see
        // (`state.rs:284-298`).
        if state.show_login(view.voice)
            && let Some(screen) = state.login.as_mut()
        {
            screen.open = view.voice;
            screen.phase = if view.voice { LoginPhase::Listening } else { LoginPhase::Waiting };
            return true;
        }
        false
    }

    /// `UiAction::LoginDone`: the door was passed. This is the **only** place the shared state may
    /// lose its screen, and it is enough on its own — the run-scoped bit was set when `show_login`
    /// first showed the screen (`crates/tipsy-ui/src/state.rs:288`), so there is nothing to remember
    /// here. The capture is already down by now: a pass closed it in the reducer (`:567-572`), which
    /// is why the frame loop's reconciliation sends the `LoginStop` spec §6.2 requires.
    pub fn admitted(&mut self, state: &mut UiState) {
        state.login = None;
    }

    /// **The predicate that keeps audio inside the machine.** True exactly while the screen is up,
    /// its voice switch is on and its capture is open — the native client's
    /// `login.voice && login.open` (`crates/tipsy-compadre/src/client.rs:411-413`) — and false at
    /// every other moment, including a screen that is up but shut (`Tab` off, a pass, the third
    /// refusal, a failed device) and the whole of the time after the door.
    pub fn sends_frames(&self, state: &UiState) -> bool {
        state.login.as_ref().is_some_and(|screen| screen.voice && screen.open)
    }

    /// The screen's capture state, whether or not it may send: the `● rec` badge and `Tab` read the
    /// same field (`crates/tipsy-ui/src/render_login.rs`, spec §6).
    pub fn capture_open(&self, state: &UiState) -> bool {
        state.login.as_ref().is_some_and(|screen| screen.voice && screen.open)
    }

    /// Reconciles `Command::LoginStart` / `Command::LoginStop` with the screen's capture state, **on
    /// the edges**: the native client's `sync_login_mic` (`crates/tipsy-compadre/src/client.rs:415-435`),
    /// which exists because `UiState::apply` cannot raise an action — a refusal that stops listening is
    /// expressed as `open == false` and nothing else — so the client reads the state once a frame and
    /// tells the server what it is doing.
    ///
    /// The returned command goes through the **same** path every other HUD command does
    /// (`ToServer::Command` into Task 3.2's `Outbox`), which is why this returns a protocol value
    /// rather than an action of its own.
    pub fn sync(&mut self, state: &UiState) -> MicStep {
        let wanted = self.capture_open(state);
        match (self.started, wanted) {
            (false, true) => {
                self.started = true;
                MicStep::Start
            }
            (true, false) => {
                self.started = false;
                MicStep::Stop
            }
            _ => MicStep::None,
        }
    }

    /// One message from the socket, as far as the door is concerned. Only `Welcome` reaches it; a
    /// `LoginHeard` or a `LoginVerdict` belongs to `UiState::apply`
    /// (`crates/tipsy-ui/src/state.rs:553-591`) and an `Error` to its own arm (`:592-613`), so this
    /// never second-guesses a phase.
    pub fn note_event(&mut self, state: &mut UiState, event: &Event) -> bool {
        match event {
            Event::Welcome { login, .. } => self.open_screen(state, login.as_ref()),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::{
        AllowedPersonView, ClientKind, ListeningMode, ProfileView, UiLanguage, PROTOCOL_VERSION,
    };
    use tipsy_ui::KeyInput;

    /// The `Welcome` the browser acts on. Task 1.1's `LoginSettingsView` has three fields
    /// (`enabled`, `voice`, `allowed`); `allowed` is carried for the configuration screen's sake and
    /// is never read here — the door is `Some` or `None` (spec §3), which is the point.
    fn welcome(login: Option<LoginView>) -> Event {
        Event::Welcome {
            session_id: 7,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: UiLanguage::Es,
            listening_mode: ListeningMode::Push,
            web_bundle: true,
            login,
        }
    }

    fn login_view(voice: bool) -> LoginView {
        LoginView {
            enabled: true,
            voice,
            allowed: vec![AllowedPersonView { name: "Andrés".into(), keywords: vec!["andres".into()] }],
        }
    }

    /// The `Hello` Task 3.2's socket sends first, as the JSON the `Outbox` puts on the wire. The
    /// login's three commands go down that same path, so the test asserts on the text and not on a
    /// private side-channel.
    fn hello_json() -> String {
        serde_json::to_string(&Command::Hello {
            token: None,
            client: ClientKind::Web,
            protocol_version: PROTOCOL_VERSION,
        })
        .expect("a command serialises")
    }

    /// A 320-sample frame, the shape Task 3.3's `FrameAssembler` emits.
    fn mic_frame() -> Vec<i16> {
        vec![1_000i16; 320]
    }

    /// The verdict that opens the door. A pass closes the capture in the reducer
    /// (`crates/tipsy-ui/src/state.rs:561-572`), so this is the event that shuts the browser's frame
    /// gate without `Door` remembering anything.
    fn pass() -> Event {
        Event::LoginVerdict { ok: true, name: Some("Andrés".into()) }
    }

    #[test]
    fn a_welcome_with_a_login_view_puts_the_screen_up_and_starts_the_capture() {
        let mut state = UiState::new();
        let mut door = Door::new();

        assert!(door.open_screen(&mut state, Some(&login_view(true))), "the first Welcome opens the screen");
        let screen = state.login.as_ref().expect("the screen is up");
        assert!(screen.voice, "the voice switch came from the server");
        assert!(screen.open, "and the capture opens with the screen, without a key");
        assert_eq!(screen.phase, LoginPhase::Listening, "which is what the phase says");
        assert!(door.sends_frames(&state), "so the frames may leave");

        assert_eq!(door.sync(&state), MicStep::Start, "and the client tells the server once");
        assert_eq!(door.sync(&state), MicStep::None, "and never twice");
    }

    /// **What the browser still owns: asking the state, once, and never asking on its own.** The rule
    /// itself is tested where it lives — `state::tests::the_screen_is_shown_once_per_run`
    /// (`crates/tipsy-ui/src/state.rs:1354-1366`) is the reference, and this does not restate it; it
    /// proves only that `tipsy-web`'s wiring is one `show_login` behind `Welcome` and no bit of its
    /// own. A re-sent `Welcome` — the one `SetProfile` and `SetListeningMode` make the session send —
    /// must leave the half-typed answer, the phase and the refusal streak exactly where the user left
    /// them, and an announcement after the door opened must never put the veil back.
    #[test]
    fn a_second_welcome_never_re_enters_the_login_screen() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(true)));

        // While the screen is up: the same view again moves nothing, and the type-into-'andres' is
        // the work that would be lost if this re-entered.
        state.on_key(KeyInput::Char('a'));
        assert!(
            !door.note_event(&mut state, &welcome(Some(login_view(true)))),
            "a second Welcome does not re-enter the screen"
        );
        let screen = state.login.as_ref().expect("the screen is still up");
        assert_eq!(screen.input, "a", "and the half-typed answer is still there");
        assert_eq!(screen.phase, LoginPhase::Listening);
        assert_eq!(screen.refusals, 0);

        // The door opens and the user presses `Enter`: `UiAction::LoginDone`, and the screen goes.
        state.apply(&pass());
        assert_eq!(state.on_key(KeyInput::Enter), vec![tipsy_ui::UiAction::LoginDone]);
        door.admitted(&mut state);
        assert!(state.login.is_none(), "the screen is down");
        assert!(state.login_seen, "and the run remembers that it was asked");

        assert!(
            !door.note_event(&mut state, &welcome(Some(login_view(true)))),
            "the announcement after the door never puts the screen back up"
        );
        assert!(state.login.is_none(), "the HUD stays the HUD");
    }

    /// **A pass closes the capture, and the gate follows the state rather than a flag of its own.** A
    /// pass ends `LoginState::open` in the reducer (`crates/tipsy-ui/src/state.rs:561-572`), and that
    /// is what turns spec §6.2's `LoginStop` — and the end of `Frame::Mic` — into a consequence of the
    /// state instead of a second copy of "is the capture open" kept beside it.
    /// `state::tests::a_pass_closes_the_capture_it_found_open` (`state.rs:1372-1383`) is the reference
    /// for the state half; this is the browser's half: the predicate and the edge the frame loop reads.
    #[test]
    fn a_pass_takes_the_frames_away_because_the_state_says_so() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(true)));
        assert_eq!(door.sync(&state), MicStep::Start, "the capture opens with the screen");

        state.apply(&pass());

        assert_eq!(state.login.as_ref().unwrap().phase, LoginPhase::Passed);
        assert!(
            !state.login.as_ref().unwrap().open,
            "the state closed the capture the moment the door opened"
        );
        assert!(!door.sends_frames(&state), "so no frame may leave the page any more");
        assert!(!door.capture_open(&state), "and there is no capture to draw");
        assert_eq!(door.sync(&state), MicStep::Stop, "which is the LoginStop spec §6.2 asks for");
        assert_eq!(door.sync(&state), MicStep::None, "once, on the edge");

        // The screen stays up until `Enter`: the door is open, the capture is not.
        assert_eq!(state.on_key(KeyInput::Enter), vec![tipsy_ui::UiAction::LoginDone]);
        door.admitted(&mut state);
        assert!(state.login.is_none());
        assert_eq!(door.sync(&state), MicStep::None, "the `LoginStop` went out at the pass");
    }

    /// Spec §9: a denied or unavailable microphone means `mic_open == false`, **no `LoginStart` and
    /// no stream**, and the screen stays typing-only. The `voice = false` case is the same path
    /// without the failure: the switch is an explicit choice, and typing always works (spec §6.2).
    #[test]
    fn no_capture_no_login_start_and_no_frame_leaves() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(false)));

        let screen = state.login.as_ref().unwrap();
        assert!(!screen.voice);
        assert!(!screen.open, "no stream is opened at all");
        assert_eq!(screen.phase, LoginPhase::Waiting, "so the phase waits for a key");
        assert!(!door.sends_frames(&state), "and no frame may leave the page");
        assert_eq!(door.sync(&state), MicStep::None, "a typing-only client sends no `LoginStart`");
    }

    /// **The capture's edges, including the failed device.** `Tab` and the third refusal move
    /// `LoginState::open` (`crates/tipsy-ui/src/state.rs:396-405`, `:577-579`), and a microphone that
    /// will not open clears it too (Task 3.3's gate, and the native `sync_mic`'s own branch at
    /// `crates/tipsy-compadre/src/client.rs:462-478`). Every one of those is a `LoginStop` here, and
    /// none of them is a frame. The pass's own edge is the test above this one.
    #[test]
    fn the_capture_opens_and_closes_with_the_state_and_only_an_open_capture_sends() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(true)));
        assert_eq!(door.sync(&state), MicStep::Start);

        // `Tab`: capture on → off → on, the native transitions of §6.2.
        assert_eq!(state.on_key(KeyInput::Tab), vec!(tipsy_ui::UiAction::Send(Command::LoginStop)));
        assert!(!door.sends_frames(&state), "a shut capture sends nothing");
        assert_eq!(door.sync(&state), MicStep::Stop);
        assert_eq!(state.on_key(KeyInput::Tab), vec!(tipsy_ui::UiAction::Send(Command::LoginStart)));
        assert_eq!(door.sync(&state), MicStep::Start);

        // The device that will not open: the client's own error path lowers the capture, and the
        // frame's reconciliation must follow it without a second `LoginStart`.
        state.login.as_mut().unwrap().open = false;
        assert!(!door.sends_frames(&state), "a failed device is not a capture");
        assert_eq!(door.sync(&state), MicStep::Stop, "and the server is told the capture is over");
        assert_eq!(door.sync(&state), MicStep::None);
    }

    /// The door's own wiring against the shared reducer: a typed answer leaves as
    /// `LoginAnswer { text }` — the text trimmed, the box cleared, the phase `Judging` — which is the
    /// **same** `UiAction::Send` every other HUD command uses (spec §6.2). No second path exists.
    #[test]
    fn the_typed_answer_is_the_state_machines_own_action() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(true)));

        for c in " andres  ".chars() {
            state.on_key(KeyInput::Char(c));
        }
        assert_eq!(
            state.on_key(KeyInput::Enter),
            vec!(tipsy_ui::UiAction::Send(Command::LoginAnswer { text: "andres".into() }))
        );
        let screen = state.login.as_ref().unwrap();
        assert_eq!(screen.input, "", "the box is cleared");
        assert_eq!(screen.phase, LoginPhase::Judging, "and the answer is with the server");
        // The capture stays open while a verdict is in flight: the next thing said is the next try
        // with no key at all, which is §6.2's auto re-arm.
        assert!(door.sends_frames(&state));
    }

    /// Nothing but `Welcome` reaches the door at all: the verdicts and the utterances are the shared
    /// reducer's, and this must not "help" with them.
    #[test]
    fn a_verdict_is_the_shared_states_business_not_the_doors() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(true)));

        assert!(!door.note_event(&mut state, &Event::LoginHeard { text: "soy andres".into() }));
        assert_eq!(
            state.login.as_ref().unwrap().line,
            tipsy_ui::LoginLine::Heard("soy andres".into())
        );
        assert!(!door.note_event(&mut state, &Event::LoginVerdict { ok: false, name: None }));
        assert_eq!(state.login.as_ref().unwrap().phase, LoginPhase::Refused);
        assert!(door.sends_frames(&state), "attempt 1 leaves the capture open, as the state says");
    }

    /// The frame the predicate has to keep inside: Task 3.2's `Hello` and the login's own three
    /// commands are the same `ToServer::Command` path, and a frame is only ever
    /// `ToServer::Audio(Frame::Mic(..))` behind the predicate (Task 3.3's `drain_capture` takes the
    /// predicate and never calls `Outbox::send` without it).
    #[test]
    fn the_commands_that_leave_are_the_protocols_own_and_the_hello_is_first() {
        assert_eq!(
            hello_json(),
            r#"{"type":"hello","token":null,"client":"web","protocol_version":1}"#,
            "Task 3.2's socket sends this before anything else"
        );

        // The three login commands, as the wire spells them (login spec §4).
        assert_eq!(
            serde_json::to_string(&Command::LoginStart).unwrap(),
            r#"{"type":"login_start"}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::LoginStop).unwrap(),
            r#"{"type":"login_stop"}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::LoginAnswer { text: "andres".into() }).unwrap(),
            r#"{"type":"login_answer","text":"andres"}"#
        );

        // …and a 320-sample microphone frame is the protocol's own `0x01` binary frame, which is what
        // the predicate guards (`crates/tipsy-protocol/src/lib.rs:174-177`, `:209-225`).
        let bytes = tipsy_protocol::Frame::Mic(mic_frame()).encode();
        assert_eq!(bytes[0], 0x01, "the frame kind byte");
        assert_eq!(bytes.len(), 1 + 640, "one kind byte and 320 little-endian samples");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-web login_door
```

Expected: FAIL — the crate does not compile, because `login_door.rs` has the tests and no items:

```
error[E0425]: cannot find type `Door` in this scope
  --> crates\tipsy-web\src\login_door.rs:NN:NN
error[E0425]: cannot find type `MicStep` in this scope
  --> crates\tipsy-web\src\login_door.rs:NN:NN
```

with `error[E0433]: failed to resolve: use of undeclared crate or module` for the test module's
`tipsy_ui::UiAction` / `tipsy_ui::LoginLine` imports behind it if `tipsy-web`'s `Cargo.toml` is
missing them. **A failure that is not about `Door` and `MicStep` is a different failure**: fix the
manifest or the import before reading anything else. `serde_json` is already a direct dependency
(Task 3.2's manifest) and `tipsy-protocol` and `tipsy-ui` are too.

- [ ] **Step 3: Implement the pure half**

Above the `#[cfg(test)] mod tests`, write the module doc, the two shapes and the six methods exactly
as Step 1 spells them: `Door { started: bool }`, `MicStep`, `Door::new`, `Door::open_screen`,
`Door::admitted`, `Door::sends_frames`, `Door::capture_open`, `Door::sync`, `Door::note_event`. Three
things in that code are load-bearing and are explained where they sit:

1. **`open_screen` asks one question of the state and obeys the answer.** It is
   `if state.show_login(view.voice) && let Some(screen) = state.login.as_mut() { … }` and nothing
   more: no `is_some()`, no `is_none()`, no field of its own. `show_login` is what a reconnect, a
   second `Welcome` and the native client all ask, so the rule is asked there and the browser's answer
   is not a second opinion — the `passed: bool` this body used to build is exactly the duplicate home
   `b9f2402` removed.
2. **`sends_frames` and `capture_open` are the same expression today, deliberately.** They are two
   questions and they diverge the moment a second reason to capture exists (the HUD's own
   push-to-talk is a native-only path in this tier, spec §8.2); naming both keeps a later reader from
   widening the frame guard to cover the badge. Do **not** merge them, and do not make one call the
   other if the two answers ever differ.
3. **`sync` returns a `ToServer`, not an action of its own.** The login's commands are the same three
   `Command`s every HUD command is, and Task 3.5's `dispatch` already sends a `ToServer::Command` to
   Task 3.2's `Outbox`; a second way to reach the socket is the drift the front matter's contract
   forbids.

`Door`'s only field is the request in flight, and that is deliberate: a browser-side "is the capture
open" would be the same duplicate `passed` was. The capture is `LoginState::open` — which `Tab`, the
third refusal, a failed device and now a pass all move (`state.rs:396-405` for `Tab`, `:577-579` for
the third refusal, `:600` for a failure, `:567-572` for the pass) — and the frame gate reads it on
every frame through `door.sends_frames`, so nothing is remembered twice.

- [ ] **Step 4: Run the test to verify it passes**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-web login_door
```

Expected: PASS — `9` tests, `0` failed:

```
running 9 tests
test login_door::tests::a_welcome_with_a_login_view_puts_the_screen_up_and_starts_the_capture ... ok
test login_door::tests::a_second_welcome_never_re_enters_the_login_screen ... ok
test login_door::tests::a_pass_takes_the_frames_away_because_the_state_says_so ... ok
test login_door::tests::no_capture_no_login_start_and_no_frame_leaves ... ok
test login_door::tests::the_capture_opens_and_closes_with_the_state_and_only_an_open_capture_sends ... ok
test login_door::tests::the_typed_answer_is_the_state_machines_own_action ... ok
test login_door::tests::a_verdict_is_the_shared_states_business_not_the_doors ... ok
test login_door::tests::the_commands_that_leave_are_the_protocols_own_and_the_hello_is_first ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

— `9` is `7` plus the two cases Step 1 adds: the once-per-run assertion carried over into the
browser's own wiring, and the pass that closes the capture. Both are assertions about `tipsy-web`, and
both name the shared test they are not duplicating rather than restating it.

- [ ] **Step 5: Wire it into the page — the wasm half, and the two documents**

**5a. `mic.rs`: the frames wait for the frame loop.** Task 3.3's port handler converts samples and
calls `outbox.send` from the **audio thread**. It must not: `outbox.send` is a DOM call, and
`state.borrow_mut().push_mic_level(..)` would panic against the render loop's own borrow
(Task 3.5's `start()` holds `shared.borrow_mut()` for the whole frame, "ONE borrow for the whole
frame: the callbacks above cannot run while it is held",
`docs/superpowers/plans/2026-10-11-plan-3-web-tier.md:13512-13522`). So the port handler appends, and
the frame loop drains. Add to `Capture`:

```rust
    /// The frames the port has delivered since the last frame, as 16 kHz mono `i16`. A queue and not
    /// a call into `Outbox` or `UiState`, for the two reasons the frame loop above already writes
    /// down: the port runs on the audio thread, so a DOM call from it is not ours to make, and the
    /// render loop holds its `borrow_mut()` on the shared state for the whole frame (Task 3.5,
    /// `lib.rs`'s "ONE borrow for the whole frame").
    frames: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<Vec<i16>>>>,
```

The port's closure (Task 3.3's step 5 of "the mechanical remainder", the `set_onmessage` handler)
becomes: downmix, resample to `MIC_RATE` when the context's `sample_rate()` differs, push each
completed frame through the `FrameAssembler`, and **append** each result to `self.frames`. It sends
nothing itself.

Then, in `mic.rs`, beside Task 3.3's other pure functions:

```rust
/// The loudness the HUD's oscilloscope and the widget's core show, from one frame's samples. The
/// twin of `crates/tipsy-compadre/src/audio/pcm.rs:28-33` reached the way the native client reaches
/// it (`crates/tipsy-compadre/src/client.rs:403`: `rms(&i16_to_f32(&samples)) * self.mic_gain`). The
/// **gain is not applied here**: `[listening] mic_gain` arrives with the configuration over the wire
/// (W3), and Task 3.5 owns applying it with the rest of the HUD's settings.
pub fn levels_from_samples(samples: &[i16]) -> f32 {
    rms(&samples.iter().map(|s| *s as f32 / 32767.0).collect::<Vec<f32>>())
}

/// The one place a captured frame is allowed to reach the socket. Called once per frame from the
/// render loop, after `hud::drain` (so `Tab` has already moved the state) and after `Door::sync` (so
/// a capture that just opened has already sent its `LoginStart`).
///
/// **It takes `&mut UiState`, not the `Rc<RefCell<UiState>>`**, and that is not a style choice:
/// Task 3.5's `draw_web` closure already holds `shared.borrow_mut()` for the whole frame ("ONE
/// borrow for the whole frame: the callbacks above cannot run while it is held", Task 3.5's `start`),
/// so a second `borrow_mut()` from inside it is a panic — and the queue has to be drained inside
/// that closure, because the frame is the only moment the state is legitimately ours.
///
/// `door.sends_frames` is the whole guard, and a frame it refuses is **dropped, not kept**: with the
/// door shut — the screen down, the capture closed by a pass, `Tab` off, the third refusal, a failed
/// device — room tone is discarded as it arrives. Keeping it would send it the moment the door opened
/// and the server would transcribe a sentence said to a closed door as the user's answer, which is the
/// login plan's review finding 1 on this client. The pass is why the guard is read and not cached: the
/// frame the user said just after "Pasa, Andrés." is refused on the very next frame, because the state
/// closed `open` in the reducer (`crates/tipsy-ui/src/state.rs:561-572`) and this reads it.
pub fn drain_capture(
    capture: &Capture,
    door: &Door,
    state: &mut tipsy_ui::UiState,
    outbox: &crate::socket::Outbox,
) {
    let mut frames = capture.frames.borrow_mut();
    let sending = door.sends_frames(state);
    let mut levels = Vec::new();
    while let Some(samples) = frames.pop_front() {
        if sending {
            let _ = outbox.send(ToServer::Audio(Frame::Mic(samples.clone())));
        }
        levels.push(levels_from_samples(&samples));
    }
    drop(frames);
    // The core's pulse, for as long as the capture runs. The login spec's §6 wants the core to pulse
    // "with the mic" and to follow the voice "while the door is shut", so the level is taken whether
    // or not the frame was sent — and it is taken from a capture that is open, so a frame the guard
    // refused (the screen is down) never reaches the visualiser either.
    for level in levels {
        state.push_mic_level(level);
    }
    if door.capture_open(state) {
        let points = state.mic_levels.len().min(LOGIN_ENVELOPE_POINTS);
        let envelope = state.mic_levels[state.mic_levels.len() - points..].to_vec();
        // `voice_level()` reads `voice_envelope`, not `mic_levels` (`crates/tipsy-ui/src/state.rs:749-755`),
        // and the login widget's core is drawn from `voice_level()` (login spec §6), so the live
        // capture is what feeds it; without this the core stands still for the whole of the door,
        // which is exactly the feedback §6 says the screen needs. A reply's own envelope, if Task 3.4
        // set one, is left alone: nothing is being spoken behind the veil.
        state.start_voice(envelope);
    }
}
```

with `pub const LOGIN_ENVELOPE_POINTS: usize = 12;` beside them (twelve 320-sample frames ≈ 240 ms of
history — three of the HUD's `ENVELOPE_HOP_MS = 50` windows, `crates/tipsy-ui/src/state.rs:14`), and
`tipsy_protocol::{Frame, ToServer}` added to `mic.rs`'s imports.

`open_capture` takes the door so the port's closure can hold its own handle — `Capture` keeps it and
the closure clones the `Rc`, which is how every other shared cell in this crate is passed:

```rust
async fn open_capture(
    state: &std::rc::Rc<std::cell::RefCell<tipsy_ui::UiState>>,
    outbox: &crate::socket::Outbox,
    door: std::rc::Rc<std::rc::RefCell<crate::login_door::Door>>,
) -> Result<Capture, String> {
```

and the click handler that calls it (`press_to_start`, Task 3.3's `on_click` closure) clones the door
into it exactly as it clones the state and the outbox. If Task 3.3's `open_capture` has a different
name or argument order, this is the one call site to match — the *decision* above is unaffected.

**5b. `lib.rs`: three call sites and one arm.** Inside Task 3.2's `start()` and Task 3.5's render
loop, all in the same `Rc<RefCell<..>>` world:

```rust
    // One per `start()`. It carries a `Door { started: bool }` and no run-scoped bit at all: the
    // once-per-run rule is `UiState::login_seen`, asked of the state by `show_login`
    // (`crates/tipsy-ui/src/state.rs:201-207`, `:284-298`).
    let door = Rc::new(RefCell::new(login_door::Door::new()));
    // Task 3.3's gate takes the same handle: the port's closure needs it to know whether a frame may
    // leave the page, and the click handler needs it to hand it over.
    let door_for_gate = Rc::clone(&door);
```

In the message loop where Task 3.4 routes `ToClient` (inside `draw_web`, the
`while let Some(message) = queue.pop_front()` loop):

```rust
                        ToClient::Event(event) => {
                            // `apply` first, then the door: the reducer leaves the login alone (its
                            // `Welcome` arm ignores the field), and a screen the door just put up
                            // must not be touched again by the same event.
                            state.apply(&event);
                            // The screen, on `Welcome.login`. `state.apply` cannot do it: its
                            // `Welcome` arm destructures with `..` and ignores `login`
                            // (`crates/tipsy-ui/src/state.rs:454-475`), so this call is the only way a
                            // screen goes up — and `show_login` is itself the once-per-run rule
                            // (`:284-298`), so a re-sent `Welcome` is refused by the state and not by
                            // a flag out here.
                            door.borrow_mut().note_event(&mut state, &event);
                        }
```

Immediately after `hud::drain` — **before** the actions are dispatched, which is what makes
`LoginStart` reach the socket in the same frame the screen opened. `Door::sync` is a state machine
and its edge is consumed by the call, so it is called **once** per frame and its answer is mapped:

```rust
        let actions = hud::drain(&mut shared.state, &shared.hits, &mut shared.queued);
        // The capture's edges: a screen that just opened with `voice` sends `LoginStart`; `Tab`, the
        // third refusal, a failed device and a **pass** all close the capture and send `LoginStop`; and
        // the command goes down the same socket path every other HUD command does (Task 3.5's
        // `UiAction::Send` arm reaches the same `Outbox`). One call, because `sync` consumes the edge:
        // calling it twice would send nothing the second time.
        let step = door.borrow_mut().sync(&shared.state);
        let capture_command = match step {
            MicStep::Start => Some(Command::LoginStart),
            MicStep::Stop => Some(Command::LoginStop),
            MicStep::None => None,
        };
        if let Some(command) = capture_command {
            let _ = shared.to_server.send(ToServer::Command(command));
        }
```

`lib.rs`'s imports grow by `tipsy_protocol::{Command, ToServer}` (Task 3.2's `use
tipsy_protocol::{Frame, ToClient};` line) and by `login_door::MicStep`; `mic` and `login_door` are
already in the module list. The login's other two commands need no new code at all: `Tab` and the
third refusal raise `UiAction::Send(Command::LoginStart | LoginStop)` from `UiState::on_key`
(`crates/tipsy-ui/src/state.rs:391` and `:405`) and a typed answer raises
`UiAction::Send(Command::LoginAnswer { text })` (`:383`), so they arrive through `hud::drain` and
leave through Task 3.5's existing `UiAction::Send` arm — **the same path as `Ask`**, which is the
whole of "wire the login into the same paths rather than adding a second path".

One new arm in `dispatch`, beside Task 3.5's `UiAction::Quit`:

```rust
        // The door opened and the user pressed `Enter`: the screen goes down. The capture went down
        // before it — a pass closes `LoginState::open` in the reducer (`crates/tipsy-ui/src/state.rs:567-572`),
        // so the `sync` above already sent the `LoginStop`, and a `Door` that tracked the capture
        // itself would be the second copy this branch removed.
        UiAction::LoginDone => {
            door.borrow_mut().admitted(&mut shared.state);
        }
```

`dispatch` needs the door in `Shared` (Task 3.5's struct) or as a parameter; either is fine, and the
one rule is that **there is exactly one `Door` per `start()`** — a second one would be a second
`started` bit, so the page could send two `LoginStart`s for one capture or miss the `LoginStop` that
belongs to the one it opened, which is the defect this task exists to prevent.

The frames are drained in the same closure, **in the same frame and after the queue was drained**:
the capture may not touch the state from the audio thread, and the closure is the only place that
already holds it. Task 3.5's closure holds `shared.borrow_mut()`, so the state is passed as the
`&mut UiState` it already is:

```rust
        // Task 3.3's frames, drained once a frame, behind the door's predicate. `shared.state` is
        // handed over as the `&mut UiState` the closure already holds: a second `borrow_mut()` in
        // here is the panic Task 3.5's own comment warns about. The capture is what the gate opened;
        // a run that never pressed it has nothing to drain. The two borrows are hoisted into bindings
        // so the call reads as the one call it is.
        let capture = capture.borrow();
        let door = door.borrow();
        let outbox = outbox.borrow();
        if let Some(capture) = capture.as_ref() {
            mic::drain_capture(capture, &door, &mut shared.state, &outbox);
        }
```

`capture` and `outbox` are the two handles Task 3.3 and Task 3.2 leave for exactly this: `Capture`
owns the port and the queue, `Outbox` owns the socket, and each lives behind an `Rc<RefCell<..>>`
that the gate fills in. If the names the earlier tasks chose differ, this is the one line to match —
the guard and the drop are unaffected, because both are `drain_capture`'s.

**5c. The login spec's two untrue sentences, and the record.** Edit
`docs/superpowers/specs/2026-10-11-login-screen-design.md` in four places, keeping every *decision*
exactly as it is (the server still judges, `server.token` is still the only door):

- **§1, the non-goals list** — the bullet that reads "**A Login tab** in the configuration screen.
  The section is hand-edited in `config.toml`, like `[[profiles]]`, and the configuration screen must
  carry it through untouched (§5)." (`:47-48`). Replace its first sentence with "**Hand-editing
  `config.toml` as the only way to change the section.** Plan 3 added a Login tab: Task 1.1 extended
  `LoginSettingsView` with `enabled` and `allowed`, so the configuration screen shows and edits `[login]`
  over the wire (§13)." and leave the second sentence's requirement standing — the file round trip of
  §5.3 is still what keeps a hand-written allowlist alive.
- **§2, the decisions log, the L2 row** (`:62`) — "the allowlist never leaves it" becomes "the
  allowlist travels only inside `ConfigView`, which a client receives when it is entitled to the
  token (§4.3, Task 1.1's extension)". The *decision* is the row's first half and does not move:
  the **server** judges, and `tipsy-ui` still never matches a keyword.
- **§5.3, "The section survives a save"** (`:245-249`) — it opens "The configuration screen has no
  Login tab"; make it "The configuration screen's Login tab is Plan 3's (Task 1.1's `LoginSettingsView`
  gained `enabled` and `allowed`), and until a client has that view the section is hand-edited; what
  must not change is the file round trip" and keep the rest of the paragraph verbatim.
- **§13's closing block, item 2** (`:500-505`) — append the landing record, in the spec's own voice:
  "**Landed in Plan 3's Task 1.1** (2026-10-11): `LoginSettingsView` carries `enabled`, `voice` and
  `allowed`, `ConfigView` carries the section, the configuration screen edits it, and `Welcome.login`
  is still the only authority a login screen is shown from — a client that never fetches the
  configuration never sees the allowlist. The judging stays server-side (`login::judge`) and
  `server.token` stays the only door." Also append the manual check of Step 6 to the same closing
  block, which is where the spec already keeps its release checks.

**5d. `README.md`.** In "The login screen" section, the paragraph at `README.md:246-255` ends "The
section is hand-edited — there is no Login tab — but saving anything else on the configuration
screen carries it through untouched." Replace that clause with "The section is editable from the
configuration screen's Login tab (the client reads and writes `[login]` over the same wire it reads
the rest of the configuration on), and saving anything else carries it through untouched." Then add
one line to the **Release checklist** (`README.md:351-383`), which is where a test cannot reach:

```
- [ ] **The login door in the browser, with the system microphone disabled.** Turn the microphone off
      in the OS (Windows: Settings → Privacy → Microphone → "Let desktop apps access your
      microphone"), open `http://127.0.0.1:7070/` with `[login]` configured and `voice = true`, and
      press the gate. **Expected:** the screen opens *typing-only* — no `● rec` on its top border, no
      microphone in the OS' own in-use indicator — and its one line reads `Sin micro: escribe, que
      también vale`. Type an allowed answer and press `Enter`: the door opens and the HUD appears.
      Then turn the microphone back on and repeat: the same screen opens with the red `● rec`, and
      while the door is shut a human must look at two things no test can see — that the veil is
      unreadable (the HUD is visibly there and visibly out of focus; `Veil::Dim` is the one-constant
      escape hatch if `▒`/`░` land badly in the browser's font, login spec §7 and §13), and that the
      core inside the widget pulses while you speak. Record which it was.
```

- [ ] **Step 6: Run the whole crate, lint it, and record the manual check**

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo test -j 1 -p tipsy-web
```

Expected: PASS — every test in the crate: Task 3.2's `7`, Task 3.3's `10`, Task 3.4's `14`, Task 3.5's
`7` and this task's `9`, `0` failed. `tipsy-ui` and `tipsy-protocol` are untouched by this task — the
once-per-run rule and the capture a pass closes are already there, each with its own test
(`state::tests::the_screen_is_shown_once_per_run`, `state::tests::a_pass_closes_the_capture_it_found_open`)
— so their suites and their counts are unchanged, and this task's two new cases are the browser's own
half of those two rules rather than a re-test of them.

```powershell
$env:TEMP="$PWD\.target-sandbox\tmp"; $env:TMP=$env:TEMP; $env:CARGO_TARGET_DIR="$PWD\.target-sandbox"; cargo clippy -j 1 -p tipsy-web --all-targets
```

Expected: the finding set is still exactly the seven pre-existing findings pinned in
`.superpowers/sdd/2026-10-11-plan-3-web-tier/global-constraints.md:26-37` — which have no
`tipsy-web` entry — and **nothing new**. A new finding in `crates/tipsy-web/` is this task's to fix
before the commit. `cargo fmt` is never run (`global-constraints.md:39-40`).

**The browser half is a documented manual check, not a test, because no browser exists in this
harness.** Step 5d's checklist line is the deliverable; the ordering that matters is that the
typing-only path is checked **first**, with the device really taken away, because that is the one
branch the shared state cannot prove (the login spec's own note: "The *decision* is tested — a failed
`Mic::start` leaves no capture and the line above — but the *device* is not",
`docs/superpowers/specs/2026-10-11-login-screen-design.md:506-511`). Record in the ledger's task line:
the browser and its version, whether the typing-only line appeared, whether the veil was readable,
and whether the core pulsed — and, if `Veil::Dim` had to be switched on, that it was, because both
strengths are tested and the constant is the only thing that changes.

- [ ] **Step 7: Commit**

```bash
git add crates/tipsy-web/src/login_door.rs crates/tipsy-web/src/lib.rs crates/tipsy-web/src/mic.rs docs/superpowers/specs/2026-10-11-login-screen-design.md README.md
git commit -m "feat(web): open the login door in the browser and keep its audio inside

tipsy-web gets the engine around tipsy-ui's login screen, which is reused
verbatim: Welcome.login shows it once per run, its capture opens in step with
LoginState::open through the same LoginStart/LoginStop edges the native client
sends, and a typed answer leaves as the UiAction::Send the state machine already
raises. The once-per-run rule stays where b9f2402 put it, in UiState: the
browser asks show_login and obeys its answer instead of keeping a passed bit of
its own, because a reconnect, a re-sent Welcome and the native client all share
that state, and login.is_none() cannot tell "asked and let in" from "never
asked". A pass now closes the capture in the reducer, so Door carries only the
request in flight and the sends_frames predicate is the only thing between Task
3.3's frames and the socket: a frame is drained and dropped unless the screen is
up, its voice switch is on and its capture is open — the state's own fields, read
once a frame, which is the login plan's review finding 1 on this client. Capture
queues its frames instead of calling the socket from the audio thread, and the
state's voice envelope follows the live capture so the widget's core pulses
without a reply being spoken. The login spec's no-Login-tab
non-goal and its allowlist sentence are amended as Plan 3's Task 1.1 makes them
untrue, with the decisions themselves unchanged: the server still judges and
server.token is still the only door.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

The checkout holds other plans' uncommitted work (`global-constraints.md:42-48`): never `git add -A`
here. The plan document itself carries this task's index line and `3.1 – 3.7` in its phase table
already (`docs/superpowers/plans/2026-10-11-plan-3-web-tier.md:286`, `:62`) — see the note in `Files`
— so this commit adds no document beyond the two Step 5 edits named above.
