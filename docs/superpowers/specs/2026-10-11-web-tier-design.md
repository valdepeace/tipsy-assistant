# tipsy-compadre — Web tier (Plan 3) — Design

- **Date:** 2026-10-11
- **Status:** awaiting review
- **Companion documents:** [`docs/architecture.md`](../../architecture.md) (diagrams, the security
  model and the run modes this spec makes true), the v1 spec
  [`2026-10-10-tipsy-compadre-design.md`](2026-10-10-tipsy-compadre-design.md) (§3.2 run modes, §4
  protocol, §5.4 sessions, §9 interface, §10 web client and web terminal, §11 security, §12 errors,
  §14 milestones 7–9) and the login spec
  [`2026-10-11-login-screen-design.md`](2026-10-11-login-screen-design.md) (§4 the wire, whose
  `LoginView` this tier carries unchanged).
- **Research notes** (git-ignored, quoted by the plan and never recalled):
  [`.superpowers/sdd/plan-3-research/codebase-recon.md`](../../../.superpowers/sdd/plan-3-research/codebase-recon.md)
  (every current signature and line number this spec cites),
  [`.superpowers/sdd/plan-3-research/web-transport-and-terminal.md`](../../../.superpowers/sdd/plan-3-research/web-transport-and-terminal.md)
  (axum, `rust-embed`, `rcgen`, `portable-pty`, xterm.js — every version read off a primary source)
  and
  [`.superpowers/sdd/plan-3-research/ratzilla-and-wasm-audio.md`](../../../.superpowers/sdd/plan-3-research/ratzilla-and-wasm-audio.md)
  (Ratzilla, the wasm toolchain, the MDN-cited microphone and playback pipelines).
- **Approval:** the shape of this tier — a server with a WebSocket door, a remote native client, a
  web terminal and a WASM web client — was accepted in conversation on 2026-10-11. Five choices
  were made **against** the planning agent's recommendation and are recorded in §2 with the cost
  each one carries, so that no implementer mistakes them for oversights: `axum` as the HTTP/WS
  stack, a generated self-signed certificate, the configuration travelling over the wire, the
  `ProfileChanged`/`ConfigChanged` events, and xterm.js fetched at build time.

## 1. Purpose

The assistant stops being a program that only one terminal on one machine can reach. `tipsy-compadre
serve` becomes a server with a token-guarded door: the native TUI can attach to it from another
machine with `--connect`, a browser can drive the same TUI through a real pseudo-terminal at
`/terminal`, and a browser can render the HUD itself at `/` — the same `tipsy-ui` widgets, compiled
to WebAssembly — using the browser's own microphone and speakers.

Nothing about the conversation changes. The protocol of the v1 spec is already the wire: this tier
makes the channel that already carries `Command`, `Event` and audio frames reachable from a socket,
and adds only the messages a *remote* client needs and cannot get today.

### Success criteria

1. `tipsy-compadre serve` starts headless, prints where it is listening, and serves `/` and
   `/terminal`. On a fresh install with `lan = false` it requires no token and answers on
   `127.0.0.1` only.
2. `tipsy-compadre --connect ws://127.0.0.1:7070/ws` opens the **same** TUI a bare run opens — the
   core rings, the microphone, the reply audio — with every byte of provider traffic happening in
   the server process. `--connect` to a server bound to the LAN without the token, or with the
   wrong one, ends with a readable error and no partial connection.
3. Two clients connected at once have **separate** conversation histories and separate
   `session_id`s, and switching the profile in one of them changes the profile in the other.
4. `http://127.0.0.1:7070/terminal` gives a working full-screen TUI in the browser: keys, mouse,
   and the browser window's size are respected, and the audio comes out of the server machine.
5. `http://127.0.0.1:7070/` renders the HUD from WebAssembly, captures the browser microphone into
   16 kHz frames, plays the reply, and never sees an API key.
6. A server exposed with `lan = true` and **no** token refuses to start, naming `lan` in the error.
7. Every failure the v1 spec §12 lists for a disconnected client has a defined ending here: the
   in-flight reply is cancelled, the socket is closed, and the server keeps running.

### Non-goals

- **Local speech, always.** Still v2 (v1 spec §1 non-goals).
- **`hands_free` and the wake word.** Plan 4; this tier does not touch `vad.rs` or Rustpotter.
- **Two-way configuration without a connection.** §4.2's `GetConfig`/`UpdateConfig`/`SetKey` are
  implemented in this tier, but with the transport rules of §9.4: `SetKey` is refused on a
  connection that is neither loopback nor TLS.
- **An account system.** There is one machine owner, one token, and no users, roles or sessions
  that outlive a connection.
- **CI, release packaging and the full `cargo xtask dist` of milestone 11.** Phase 3 builds only the
  `xtask` needed to produce a bundle-embedded binary; `.github/`, artefacts and the README's
  release section stay out.
- **A ratatui upgrade.** Ratzilla 0.3.1 needs `ratatui ^0.30.1` and the workspace is on `0.30`
  (locked `0.30.2`), so one resolved copy exists today; moving to 0.31 is not this tier's business.

## 2. Decisions log

Every line here is either a choice made in this design or one of the five the user took against the
planning agent's advice. The **cost** column is the price, not an objection: it is what a reviewer
should check has been paid.

| # | Decision | Alternatives considered | Cost |
|---|---|---|---|
| W1 | **`axum` 0.8.9 with its `ws` feature is the server's HTTP and WebSocket stack**, together with `axum-server` 0.8.0 (`tls-rustls`) for TLS | A hand-written HTTP responder over `tokio-tungstenite` 0.30, which is already a direct dependency and whose `accept_hdr_async` the current tests already use | `axum` 0.8.9 depends on `tokio-tungstenite ^0.29`, while the native client keeps `0.30`, so **two tungstenite versions and two incompatible `Message` types** are compiled into one binary. The server needs a conversion layer between `axum::extract::ws::Message` and this project's wire types, and every task that touches the socket must say which `Message` it means. `tokio-tungstenite` 0.30 stays a dependency for `--connect` and for the existing fake-server tests |
| W2 | **The server generates a self-signed certificate on first use** (`rcgen`), stored in the config directory, and `--tls-cert`/`--tls-key` override it with a pair the user supplies | Generating nothing and accepting only a user-supplied pair (works with `mkcert`) | The browser will not treat an untrusted self-signed certificate as a secure context, so **generating one does not by itself unlock `getUserMedia`** for a browser on another machine (§9.5 records the evidence and the unverified part). The feature is therefore documented as *confidentiality* — it encrypts the token and the API keys a remote client sends — and the microphone-from-another-device case is documented as needing a certificate the browser already trusts |
| W3 | **The configuration travels over the wire**: `GetConfig`, `UpdateConfig`, `SetKey` and `Event::Config`, with a `ConfigView` that redacts API keys and carries the real `server.token` only to a client that already presented it | Keeping the configuration screen native-only, so `--connect` and the web client hide `F9` and point at the server machine | The configuration screen stops being a native-only feature and its save path moves behind the protocol, which is the largest single chunk of Phase 1. It also needs §9.4's rule — `SetKey` over loopback or TLS only — or an API key would cross a plaintext LAN socket, breaking the v1 spec §11 promise |
| W4 | **`Event::ProfileChanged` and `Event::ConfigChanged` exist**, broadcast to every connected session, as the v1 spec §4.2 and §5.4 promise | Re-sending `Welcome` to every session on a change, which needs no new variants | Two more protocol variants and their HUD handling; every session that receives `ConfigChanged` must re-derive what it shows, and the browser clients must not drop the event on the floor |
| W5 | **xterm.js is fetched at build time** by `cargo xtask dist` from the npm registry, at a pinned version, with a **sha256 pinned in the source** and verified before the files enter the bundle | Vendoring the prebuilt files into the repository, or loading them from a CDN | The build needs the network once per clean checkout, and the plan must obtain and record the real hash rather than invent one. In exchange the executable stays self-contained and the committed tree grows by no third-party JavaScript |
| W6 | **There is no new `Transport` trait.** The in-process channel is the protocol, so a socket peer is a *bridge* that produces or consumes the same `SessionEnds` the client already uses | Introducing a trait the client drives, and refactoring `client.rs`'s loop onto it | Two bridge modules instead of one abstraction, and `SessionEnds`' fields stay `pub` as they are. The gain is that `client.rs`'s 33 ms poll loop is untouched and the native path is unchanged by construction |
| W7 | **`Hello` becomes real**: every client sends it first, and the session validates the protocol version and, when the server is exposed, the token | Leaving `Hello` ignored, as `session.rs:353` does today | `session.rs:353`'s `Command::Hello { .. } => {}` arm becomes the only place a connection can be refused, so it needs its own tests for the three refusals (bad version, bad token, missing token on a LAN bind) |
| W8 | **The token is 32 bytes (64 hex characters) and is generated on first run**, generated again only by the Server tab's Regenerate row | Leaving it 16 bytes and generating it only from the screen, which is what the code does today | The v1 spec §11 and `architecture.md:158` both promise "generated on first run" while the example file's comment (`spec:250`) says "empty until the Server tab generates one"; this spec rules for §11, which means fixing the existing tests that pin a 32-character token and correcting that comment |
| W9 | **One session per connection**, with a unique `session_id`, and a server-side registry (`Hub`) that owns every session's event sender | One shared session for all connections | Session ids stop being the constant `1` (`session.rs:23`) and the two concurrent sessions the ledger's tests already spawn stop reporting the same id. The registry is what makes W4 possible |
| W10 | **The web terminal's PTY child is the project's own binary in `--connect` mode**, pointed at the parent's own loopback `/ws` | Giving the child the server's in-process channel, as `architecture.md:63-64`'s wording says | One code path instead of two, and the child needs no new mode. The deviation is real and `architecture.md` is corrected by Phase 2's documentation task: the child is *not* in-process, it is a loopback socket client, which is why it can also be moved to another machine later |
| W11 | **The server's stats for a remote peer are produced by the server.** `sysmon` is already spawned by the client today (`client.rs:125`), so in `--connect` mode the client keeps its own and the server's connection bridge does **not** spawn one | Both sides sending `Stats` | Two sources of `Stats` on the same channel would double the traffic and fight in the HUD. The rule is: whoever owns the process the user is looking at owns `Stats` — the client in-process, the server for a remote or web peer |
| W12 | **`--verbose` and `tracing` arrive with the server**, not with milestone 11 | No logging until the release milestone | A headless server with no log is undebuggable, and the v1 spec §11 promise "logs never contain keys or audio" is cheaper to honour from the first line than to retrofit. The redaction rule is part of the task, not a follow-up |

## 3. Where the tier lives

### 3.1 Crates and directories

| Path | Kind | What it is |
|---|---|---|
| `crates/tipsy-compadre/src/server/` | module | `mod.rs` (boot, bind, routes), `hub.rs` (the session registry), `conn.rs` (the per-connection bridge), `assets.rs` (the embedded bundle), `tls.rs` (the certificate) |
| `crates/tipsy-compadre/src/client/bridge.rs` | module | The client-side socket bridge that produces a `SessionEnds` for `--connect` |
| `crates/tipsy-compadre/src/client/pty.rs` | module | The PTY that owns a web terminal's child process |
| `crates/tipsy-protocol/src/lib.rs` | modify | W3's commands and `ConfigView`, W4's two events. Nothing else changes shape |
| `crates/tipsy-web/` | new cdylib | The WASM client: Ratzilla backend, `web-sys` WebSocket, browser audio |
| `xtask/` | new bin | `cargo xtask dist`: fetch xterm.js, build `tipsy-web`, build the binary that embeds `web/` |
| `web/` | assets | `client/index.html`, `terminal/index.html`, `vendor/` (fetched, git-ignored except its README) |

`tipsy-protocol` and `tipsy-ui` keep their rules: no `tokio`, `cpal`, `rodio`, `keyring`,
`sysinfo`, `reqwest` or `crossterm`, and both keep compiling to `wasm32-unknown-unknown`. A `wasm`
target that needs `tokio` lives only in `tipsy-web`.

### 3.2 Run modes

| Command | What runs |
|---|---|
| `tipsy-compadre` | Server and native TUI in one process, over the in-process channel (unchanged) |
| `tipsy-compadre serve [--bind <addr>] [--tls] [--tls-cert <pem> --tls-key <pem>] [--verbose]` | Headless server. `/` the web client, `/terminal` the web terminal, `/ws` the protocol and `/terminal/ws` the raw PTY bytes |
| `tipsy-compadre --connect <ws-url> [--token <t>] [--verbose]` | Native TUI only, attached to a remote server. The configuration screen is disabled with a message that names the server (W3 does *not* lift this: a remote client edits the configuration through the same screen, but the screen's Providers tab cannot resolve a key locally, so §9.4's rules apply line by line) |
| `tipsy-compadre key …` / `config path` | Unchanged |

The CLI is `clap` 4.6 derive, as `main.rs:16-59` already is: subcommands `key`, `config`, `serve`,
and top-level `--connect`, `--token`, `--tls`, `--tls-cert`, `--tls-key`, `--verbose`.

### 3.3 The bridge, in one paragraph

`spawn_with_events` (`session.rs:56`) hands back `SessionEnds { to_server, from_server, events }`
(`session.rs:46-52`) over unbounded `mpsc` channels. A **server-side bridge** owns the tail of a
socket and: turns `Message::Text` into `Command` and `Message::Binary` into `Frame`, pushing them
into `to_server`; drains `from_server` into the socket; and hands `events`' clones to whatever else
belongs to that session. A **client-side bridge** does the same in the other direction and returns
a `SessionEnds` **identical in shape** to the one `spawn_with_events` returns, so `client.rs` cannot
tell the difference. Nothing in this tier adds a trait, an enum of transports or a `cfg` gate to
`client.rs`'s loop.

## 4. Protocol

`PROTOCOL_VERSION` stays `1` (`lib.rs:7`). Both new commands and both new events are additive and
every existing variant keeps its shape byte for byte — the rule Plan 2B (`lib.rs`'s `TestProvider`)
and Plan 2C (`Event::Live`) both followed. The `Command` and `Event` enums are
`#[serde(tag = "type", rename_all = "snake_case")]` (`lib.rs:108`, `:133`), so the JSON below is
exactly what the derive produces.

### 4.1 The handshake

`Hello` already exists (`lib.rs:110`) with `token: Option<String>`, `client: ClientKind`,
`protocol_version: u32`, and `ClientKind` already has `Tui | Web | WebTerminal` (`lib.rs:11`). It is
sent by **every** client as its first message, and it is the only message that may precede it.

The server's answers, all of them already expressible with `Event::Error { code, message,
recoverable }` (`lib.rs:158` region):

| Condition | Answer | Then |
|---|---|---|
| `protocol_version != PROTOCOL_VERSION` | `Error { code: "protocol_version", recoverable: false }` | Close the socket |
| The server is exposed (its bound address is not loopback) and `token` is `None` | `Error { code: "token_required", recoverable: false }` | Close |
| `token != Some(server.token)` | `Error { code: "token_invalid", recoverable: false }` | Close |
| Anything else | `Welcome` | Serve |

The native in-process client does not need a token, but it **does** send `Hello`: the same
validation path then covers both transports, which is what makes W7's refusals testable without a
socket.

### 4.2 Commands this tier adds

| Command | JSON | Fields | Effect |
|---|---|---|---|
| `GetConfig` | `{"type":"get_config"}` | — | The session answers `Event::Config` |
| `UpdateConfig` | `{"type":"update_config","config":{…}}` | `config: ConfigView` | The server replaces the editable configuration, persists it, re-derives every session's brain and broadcasts `ConfigChanged` |
| `SetKey` | `{"type":"set_key","name":"openrouter","value":"…"}` | `name: String`, `value: String` | Stores the key in the **server's** credential store under `keys::SERVICE` and broadcasts `ConfigChanged`. **Refused unless the connection is loopback or TLS** (§9.4) |

`Event::Config { config: ConfigView }` is the answer to `GetConfig`.

### 4.3 `ConfigView`, and what it redacts

`ConfigView` is the editable configuration as the screen edits it. It **moves the shape of
`tipsy_ui::ConfigForm` into `tipsy-protocol`** — the form's sections, field by field, exactly as
Plan 2B built them (`crates/tipsy-ui/src/config_form.rs`), because the screen is the only consumer
and a second shape would be a conversion that can drift. `tipsy-compadre/src/config/form.rs`'s
`form_of`/`config_from` keep their job and convert `Config` ↔ `ConfigView`.

**`ConfigForm` itself stays where it is and gains the conversions**, rather than being deleted and
moved into `tipsy-protocol`: it is the shape the screen draws with, it is covered by Plan 2B's tests,
and moving it would touch the whole configuration screen to buy nothing. A round-trip test over a
fully populated value is what keeps the two from drifting, and that test is the point — not the
move.

Two fields do not cross unchanged, and both are load-bearing:

- **API keys never cross in any form.** The configuration stores key *names* (`key = "openrouter"`),
  never values, so `ConfigView` carries the names it always did and there is nothing to redact. The
  spec's `sk-or-…a1b2` display is about the *credential store's* contents, which the screen never
  shows today either; `GetConfig` does not change that, and no field of `ConfigView` may hold a
  secret.
- **`server.token` crosses as itself**, but only to a client that has already presented it: a
  loopback connection, or one that authenticated with that exact token. It is not a secret *from
  that client* — it is what the client sent — and the Server tab's whole purpose is to display and
  regenerate it. A client that presented no token on a loopback connection receives the token as
  the file holds it. No other connection ever receives it.

### 4.4 Events this tier adds

| Event | JSON | Meaning |
|---|---|---|
| `Config` | `{"type":"config","config":{…}}` | The redacted configuration, answering `GetConfig` |
| `ProfileChanged` | `{"type":"profile_changed","profile":{…}}` | The active profile changed; carries the same `ProfileView { id, name, language }` (`lib.rs:92-97`) `Welcome` carries |
| `ConfigChanged` | `{"type":"config_changed"}` | The configuration changed; a client that is showing it asks again with `GetConfig` |

`ConfigChanged` deliberately carries no payload: the v1 spec §4.2 defines it as a signal, and
sending the whole configuration to every session on every save would put the token on the wire more
often than it needs to be. A HUD that is not showing the configuration ignores it.

### 4.5 Audio frames

Unchanged. Binary frames are the existing `0x01` (mic, client → server) and `0x02` (speech, server →
client) formats (`tipsy-protocol`'s frame helpers), carried as WebSocket binary messages. A WebSocket
has no frame-size contract we can rely on across browsers, so the pump in §6.3 and the PTY pump in
§7.3 chunk their writes (8–64 KiB) and bound their queues; the exact number is a choice recorded in
the plan, not a protocol constant.

## 5. The server

### 5.1 Bind, and the rule that protects a LAN

`[server]` is `ServerConfig { bind: "127.0.0.1:7070", lan: false, token: "" }`
(`config/model.rs:46-68`), and `has_token()` already exists (`model.rs:59-61`), referenced only by
tests today.

The rule, enforced **before the bind and again after it**:

1. Parse `bind` as a `SocketAddr`. A hostname is refused with a readable error: `TcpListener::bind`
   takes `ToSocketAddrs`, and an unqualified name can resolve to something off this machine, so
   parsing first is what makes the check meaningful.
2. If the address is not loopback — and `0.0.0.0` is *unspecified*, not loopback, so it is already
   caught — and the configuration has no token, refuse to start, naming `lan` and the token in the
   error.
3. After binding, re-check `listener.local_addr()`; if it is not loopback and there is no token,
   drop the listener and refuse. This catches a `bind` value that resolved wider than it looked.

`serve`'s `--bind` overrides the file for this run only and never writes it back.

### 5.2 Sessions, the `Hub`, and what a change does

`Hub` owns every live session's `UnboundedSender<ToClient>`, keyed by session id, and is the only
thing that outlives a connection. `SESSION_ID` becomes an atomic counter
(`session.rs:23`'s constant and its "one session per process for now" comment both go), and the
counter is what a client sees as `Welcome.session_id`.

The three writes that used to be one session's business become the server's:

| Message | What happens |
|---|---|
| `SetProfile` | Persist the active profile; every session re-derives its `Brain` from the new configuration (`session.rs:371-385`'s logic, now applied per session through the `Hub`) and receives `Event::ProfileChanged` |
| `SetListeningMode` | Persist the mode; every session receives a fresh `Welcome`, which is exactly what `session.rs:389-392` already does for one — the mode is not a `ConfigChanged` because it is not configuration |
| `UpdateConfig` | Persist; every session re-derives its providers and `Brain`; every session receives `Event::ConfigChanged` |

`Login*` (`lib.rs`, Plan login-screen) and `TestProvider` (`lib.rs`) keep their current meanings; the
login capture stays per session, and `TestProvider` runs against the server's providers.

### 5.3 Routes

| Route | Method | Purpose |
|---|---|---|
| `/` | GET | The web client's host page, or §5.4's "not built" page |
| `/assets/*` | GET | The embedded bundle: the wasm module, its JS glue, `xterm.js`, `xterm.css` |
| `/ws` | GET, upgrade | The protocol socket |
| `/terminal` | GET | The web terminal's host page |
| `/terminal/ws` | GET, upgrade | The raw PTY byte socket |

The pages answer with `Referrer-Policy: no-referrer` and `Cache-Control: no-store`, because the
token travels as a query parameter and the default `strict-origin-when-cross-origin` policy puts a
query string into `Referer` on same-origin requests. Hashed asset filenames may be cached; the two
pages may not.

### 5.4 The bundle, and the honest page

`rust-embed`'s `#[allow_missing = true]` is what makes the optional bundle work: with the directory
absent the embed is empty and the lookup returns `None`, so **no `build.rs` gate is needed** and
§5.3's two page routes fall back to a page that says which command builds it (`cargo xtask dist`).

`Welcome.web_bundle` stops being the hard-coded `false` of `session.rs:249` and becomes the truth for
the running binary. `UiState` gains the flag — today `state.rs:291` destructures `Welcome` with `..`
and never reads it — and the HUD needs somewhere to say "the web UI is not available in this build",
which is what `architecture.md:85-86` promises.

### 5.5 TLS

`--tls` turns on the listener's TLS; the certificate comes from `--tls-cert`/`--tls-key` when both
are given, and otherwise from a self-signed pair generated once with `rcgen` and stored beside
`config.toml` (the config directory the `config path` subcommand already prints). The generated
pair is written with restrictive permissions where the platform has them, is never committed, and
its fingerprint is printed once so a user can compare it.

`axum-server`'s `tls_rustls::RustlsConfig::from_pem_file` + `bind_rustls` is the pair the research
note read end to end; `rcgen`'s `generate_simple_self_signed` returns a `CertifiedKey` whose
`cert.pem()` and `signing_key.serialize_pem()` are what that pair wants. The plan quotes the note
rather than recalling either.

## 6. The remote native client (`--connect`)

### 6.1 What it is

A TUI whose two channels come from a socket instead of from `spawn_with_events`. The bridge owns a
`tokio-tungstenite` 0.30 client (the dependency Plan 2C added and whose fake-server tests already
exercise `accept_hdr_async`), sends `Hello` first, and then shuttles `Command`/`Frame` up and
`Event`/`Frame` down. Reconnection follows the v1 spec §12: the client reconnects with backoff and
tells the user in the log.

### 6.2 What is disabled, and why it is not a gap

The Providers tab of the configuration screen resolves keys and builds connectors **locally**, and a
remote client holds neither the keys nor the providers. W3 puts the configuration on the wire, so the
screen keeps working — but its save path now sends `UpdateConfig` and its Test button sends
`TestProvider` instead of calling `build_providers`/`build_one` in-process. The audited rule is:

- Reading: `GetConfig` on open, `ConfigChanged` to know when to ask again.
- Writing: `UpdateConfig`; `SetKey` only over loopback or TLS (§9.4), with the screen saying so when
  the connection is neither.

### 6.3 Backpressure

A WebSocket has no backpressure (the research note quotes MDN saying exactly that), and the two
channels between the bridge and the session are `mpsc::unbounded`. The bridge therefore owns a
**bounded** queue on the socket's write side, and `axum`'s server-side socket gets an explicit
`max_write_buffer_size` (default: unlimited). A client that stops reading must slow the session
down, not grow the server's memory until it dies.

## 7. The web terminal (`/terminal`)

### 7.1 The screen

An xterm.js page that opens `/terminal/ws` and does nothing else: the bytes it receives are the
bytes a terminal would have shown. The application is the project's own TUI, in a real
pseudo-terminal, with `portable-pty` (`native_pty_system().openpty(...)`, ConPTY on Windows 10 1809
and later).

### 7.2 The child (W10)

The child process is **`tipsy-compadre --connect ws://127.0.0.1:<port>/ws --token <token>`**, run
inside the PTY. It is a client, exactly like a user on another machine, so the TUI reaches the
parent server over one loopback socket and there is one code path for "a TUI that is not in the
server process". Keys, providers, history and the configuration all stay in the parent.

Consequences to write down rather than discover:

- **The child is spawned as `std::env::current_exe()`, never by name.** A `PATH` lookup, a relative
  path or a bare `tipsy-compadre` would execute whatever a checked-out working directory or a
  planted directory puts there, with the user's authority, and the blast radius here is worse than
  in a local helper: what that child runs is driven by whatever the browser sends. The prior art
  reviewed on 2026-10-11 makes the same argument for a *cue player* and refuses `PATH` outright
  ([`codewhale-audio-prior-art.md`](../../../.superpowers/sdd/plan-3-research/codewhale-audio-prior-art.md) §5,
  quoting `crates/tui/src/notify/audio.rs:32-41`), so a server feature has no excuse. Task 2.1 owns
  the test: a fake executable planted in a directory prepended to `PATH` must never be the one that
  runs.
- The child must not start a server, must not build providers and must not read the credential
  store; its `--connect` path is what makes that true, and the plan's task proves it by asserting
  the child process makes no provider call.
- The child's audio is the **server machine's** (v1 spec §10, `architecture.md:59`): cpal and rodio
  run in the child, which runs on the server machine.
- Resize reaches the child through `MasterPty::resize`, which is the only resize call needed — it
  generates the child's own resize signal (`SIGWINCH` on Unix, `ResizePseudoConsole` on Windows), so
  there is no `libc::kill` anywhere in this tier.

### 7.3 The pump, and what happens when the socket dies

`portable-pty` hands back **std-blocking** readers and writers (`Box<dyn Read + Send>`,
`Box<dyn Write + Send>`), and does not depend on tokio; one `spawn_blocking` reader per connection
is the shape, with the writes going through a bounded channel.

Teardown is explicit and is a review focus, because a TUI that ignores a closed stdin lingers: kill
the child, drop the writer, drop the master, and let the blocking reader observe EOF. A PTY per
connection is a real resource; the plan states the ceiling it accepts and the test that proves the
child is gone.

## 8. The web client (`/`)

### 8.1 The gate the v1 spec wrote, and how this tier honours it

`spec:389-391` and `architecture.md:88-90` both say a spike must confirm Braille canvas rendering,
mouse support and ratatui version compatibility **before the plan is finalised**, and that if
Ratzilla falls short the web client is postponed and the web terminal remains the browser option.

The research note answers all three from primary sources, and the answers are *not* a clean yes:

- **Braille:** a `Canvas` with the `Braille` marker writes glyphs into the buffer, so there is no
  canvas-specific path to break; a real DOM-backend Braille bug was fixed in Ratzilla 0.2.0. **The
  1-vs-2-column glyph disagreement this project's own code describes does not reproduce on the pinned
  build, and the spike measured that rather than reasoning about it.** `render.rs:221-224`'s comment
  on `drawn_width` says "a span is laid out by one width table and drawn by another, and the two
  disagree on `◉`, `⚙` and `·`" — ratatui 0.30.2 with this machine's font disagrees with that comment:
  natively `Span::width() == drawn_width` for every span (gear 3/3, badge 18/18, `·` 3/3), and in the
  browser each of the three glyphs advances exactly one monospace cell (9.64063 px at a 16 px font,
  14.45313 px at 24 px — a ratio of 1.0008 against plain `M`). The header is safe for a *structural*
  reason rather than by luck: Ratzilla's DOM backend sizes every cell box in
  `{Cell::symbol().width()}ch`, the browser's own width table. **So this plan does not carry the glyph
  hazard as a live risk** — and the real width-related fact is a different one: that backend's
  `measure_cell_size` reads its `1ch` from a font the grid does not use (9.625 px against the grid's
  true 8.796875 px, a 9.2 % error), which is about cell metrics and not about any glyph.
- **Mouse:** yes, `MouseEvent { kind, col, row, … }` in grid cells, crossterm-*shaped* but a
  separate type, so a small translation is needed; there are no wheel variants, which costs nothing
  because the native client only matches `Down`.
- **ratatui:** Ratzilla 0.3.1 requires `ratatui ^0.30.1`; the workspace requires `0.30` (locked
  `0.30.2`), so one resolved copy exists.
- **Maturity, stated plainly:** four open **DOM-backend** layout bugs, two of them filed on
  2026-10-10/11 — including a panic when a cell is larger than 10×20 px and a blank screen after a
  resize. Only `WebGl2Backend` is described as reaching 60 fps on a large terminal, and its key
  events need `tabindex="0"` and otherwise fail *silently*.

Therefore **Phase 3's first task is the spike itself**, and it is a gate, not a formality: it
installs `wasm32-unknown-unknown`, renders `tipsy_ui::draw` through Ratzilla, and records a go/no-go
on the four points above with the backend choice. **If it fails, Phase 3 stops there**, the remaining
Phase 3 tasks are abandoned, and the delivered answer to "the browser" is the web terminal — which is
what the v1 spec asks for. The ledger records the verdict and the evidence.

### 8.2 The three jobs a browser must do that the native client does in Rust

| Job | Native | Browser |
|---|---|---|
| Capture | cpal, downmix, resample to 16 kHz, 320-sample frames (`audio/device.rs:104-206`) | `getUserMedia` + `AudioWorklet`, `Int16Array` frames of the same size, resampled to 16 kHz |
| Playback | rodio `Player` (`audio/device.rs:24-98`) | `AudioBufferSourceNode`s scheduled on a cursor, one per frame, with barge-in stopping everything |
| Transport | `tokio-tungstenite` | `web-sys` `WebSocket` with `binaryType = "arraybuffer"` |

The HUD itself is not redone: `draw_web` hands over a `&mut ratatui::Frame`, so `tipsy_ui::draw` is
reused verbatim. Input is the translation §8.1 describes.

**The two browser jobs follow one shape: a pure core and a thin presenter.** The prior art reviewed
on 2026-10-11 states the invariant in exactly these terms — "Core emits score events; WebAudio /
AVAudioEngine only present their PCM. Calling at any display cadence produces the same score when all
world ticks are supplied. Calling twice for a world tick cannot retrigger a voice"
([`codewhale-audio-pet-…`](../../../.superpowers/sdd/plan-3-research/codewhale-audio-prior-art.md) §3,
quoting `pet/src/core/pet-audio.ts:10-12`). Applied here:

- **Capture.** The float-samples-to-`Int16Array` conversion, the 320-sample boundary and the carry of
  a partial render quantum are pure and host-tested; the `AudioWorklet` only hands samples over.
- **Playback.** The cursor arithmetic, the gap decision (a sequence number that skips), the envelope
  and the barge-in reset are pure and host-tested; `AudioBufferSourceNode`s only present the frames.
  The one invariant worth its own test: **a frame that arrives twice must not be scheduled twice**,
  because the wire can repeat a `ReplyDelta` and a replayed buffer is audible corruption.
- Three details the prior art fixes in working code and naive schedulers get wrong:
  a buffer that has **entirely** passed is skipped rather than scheduled into the past; a buffer that
  is **partly** past starts at `currentTime` **with a non-zero offset into itself**
  (`node.start(when, offset)`), because for speech, restarting from the beginning repeats words the
  user already heard; and the scheduler keeps a lookahead anchor with an explicit cushion instead of
  scheduling "now" per frame. A `Set` of live source nodes with an `onended` cleanup is the barge-in
  primitive and the leak bookkeeping.

### 8.3 The build

`cargo xtask dist`, in this order: fetch and verify xterm.js (W5), `cargo build --target
wasm32-unknown-unknown -p tipsy-web --release`, `wasm-bindgen --target web` (CLI version pinned to
the locked crate, because a schema mismatch is enforced), then the host binary with `web/` embedded.
`wasm-pack` and `trunk` are not used: `trunk` has no library API and could not be driven from an
`xtask`, and neither is needed for a bundle this shape.

## 9. Security

### 9.1 The door is the token

Loopback needs nothing. Anything else needs the token, on the upgrade request for the pages and in
`Hello` for the protocol. The comparand is the configuration's `server.token` (W8: 32 bytes), and a
mismatch is a refusal, never a retry.

### 9.2 The token in a query string, and what that costs

The pages receive the token as `?token=…` because a browser cannot set a header on a navigation. RFC
6750 §2.3 says a client `SHOULD NOT` put a token in a URL, for the reason the research note quotes:
URLs are logged. This tier therefore: keeps the pages out of every cache (`no-store`), sets
`Referrer-Policy: no-referrer`, never logs a token or a URL containing one, and prefers the `Hello`
message for anything that can send one.

### 9.3 What never leaves the server

The v1 spec §11 rule stands with W3's precision: API keys, configuration and conversation history
never leave the server. `GetConfig` answers with key *names*; the credential store's values are
read only by the provider connectors, in the server process, and no protocol type has a field that
can hold one.

### 9.4 The one case where a secret travels towards the server

`SetKey` sends a *new* key into the server's credential store, so on a plaintext LAN connection the
key would cross the wire in the clear. The rule: **`SetKey` is refused with `Error { code:
"insecure_transport", recoverable: true }` unless the connection is loopback or TLS.** A screen that
offers the row over such a connection says why it cannot be used. This is what keeps §9.3 true
without giving up W3.

### 9.5 TLS, and the microphone claim this tier does not make

The research note establishes, with sources, that loopback over plain HTTP **is already a secure
context** for `getUserMedia` (`127.0.0.0/8`, `::1/128`, `localhost`), so the microphone works at
`http://127.0.0.1:7070/` with no certificate at all. What it could **not** establish is whether a
page reached past a browser's self-signed-certificate interstitial counts as secure; it marks the
guess "probably not".

This design therefore does not claim that `--tls` unlocks the browser microphone. It claims:

- `--tls` encrypts the connection, which is what makes the token and `SetKey` safe on a LAN.
- The microphone from another machine requires a certificate **the browser already trusts** —
  `--tls-cert`/`--tls-key` are the door for a `mkcert` or real certificate.
- The self-signed default is convenient for the first case and insufficient for the second, and the
  plan says so where a user would read it, not only here.

### 9.6 Logging

`tracing` to a file in the user data directory, `--verbose` also to stderr, and three redaction
rules that the implementing task tests: the token never appears (including as a query parameter and
including in a request line), no key value ever appears, and no audio is ever logged. The
`tokio-tungstenite` `trace!` of a request line that Plan 2C carried to its review is the same class
of hazard on the client side, and the plan's logging task covers both.

## 10. Errors and edge cases

The v1 spec §12 is the authority; these are the cases this tier introduces or sharpens.

| Case | Behaviour |
|---|---|
| Wrong `protocol_version` | `Error { code: "protocol_version", recoverable: false }`, then close |
| Missing or wrong token on an exposed server | `Error { code: "token_required" / "token_invalid", recoverable: false }`, then close |
| Non-loopback bind with no token | Refuse to start, naming `lan` and the token |
| `bind` is a hostname | Refuse to start, naming the address |
| A client disconnects mid-reply | The session's in-flight turn is cancelled (`cancel_running`, `session.rs:494`), the session exits, the `Hub` forgets it. The server keeps running |
| A WebSocket write fails | The connection ends like a disconnect; no panic |
| A malformed `Command` on the wire | `Error { code: "bad_message", recoverable: true }`; the connection stays open |
| A binary frame that is not audio | Same as a malformed command: an error event, not a panic |
| The PTY child dies | The socket closes, the HUD's page shows a message, the PTY is reaped |
| The browser socket dies | Kill the child, drop the writer, drop the master |
| The bundle is not embedded | `/` and `/terminal` answer with the page that says how to build it; `web_bundle` is `false` |
| TLS is requested but the certificate cannot be read or parsed | Refuse to start with the path in the message |
| `SetKey` on a plaintext non-loopback connection | `Error { code: "insecure_transport", recoverable: true }` |

## 11. Testing

| Level | Scope | Technique |
|---|---|---|
| Protocol | The four new variants and `ConfigView`'s round trip; `ConfigView` carries no secret | The existing `roundtrip_command`/`roundtrip_event` helpers (`crates/tipsy-protocol/tests/roundtrip.rs:3`, `:9`) plus a redaction test |
| Handshake | The three refusals and the acceptance, over the in-process channel and over a socket | `session.rs`'s own tests for the logic; a real socket for the transport |
| Server | Boot, bind, the loopback rule (before and after bind), the "not built" page, the token on `/ws`, the wrong token, two clients with separate histories, a profile change reaching both | `axum`'s router driven by a real listener on `127.0.0.1:0`, with the fake providers `tests/session.rs:27-105` already uses. No test hard-codes a port |
| Remote client | `--connect` end to end against a real server: a command in, an event and a reply out, and a reconnect after the server drops the socket | The fake-server technique of `tests/realtime.rs` (`FakeServer` `:85`, `start` `:114-140`) is the precedent for the server side of a socket test |
| Configuration over the wire | `GetConfig` answers the token to a client that presented it and to a loopback client, and nothing secret to anyone; `UpdateConfig` persists and broadcasts; `SetKey` is refused on plaintext non-loopback | Server tests with a real socket, plus the existing `store.rs` tempdir tests for the persistence half |
| Web terminal | A real PTY: spawn the child, write a keystroke, read the terminal bytes, resize, kill the socket and assert the child is gone; the token on `/terminal/ws` | `portable-pty` on this machine's Windows; the child being a `--connect` client means the test can also assert no provider call |
| Web client | The spike's four questions; then the wasm build; browser behaviour (permission, playback, clicks) | The spike is a real build with a recorded verdict; the browser half is a documented manual check in the README's "how to verify" section, because there is no browser in this harness |
| Assets | The xtask's fetch verifies the pinned hash and rejects a mismatch; a missing bundle yields the explanation page | A unit test of the verifier against a fixture, and a router test with the embed absent |

The workspace's test count is the ledger's baseline and each task's own suite is what it adds; the
existing 352 tests must stay green. Every task runs with `-j 1`, `TEMP`/`TMP` inside the workspace
and `CARGO_TARGET_DIR` off the `G:` default, for the reasons the ledger records.

## 12. What changes, where

| File | Change |
|---|---|
| `crates/tipsy-protocol/src/lib.rs` | `ConfigView` and its sections; `GetConfig`, `UpdateConfig`, `SetKey`; `Event::Config`, `Event::ProfileChanged`, `Event::ConfigChanged` |
| `crates/tipsy-protocol/tests/roundtrip.rs` | Round trips for the four new variants; the redaction test |
| `crates/tipsy-compadre/src/session.rs` | `Hello` validation; the session-id counter; `GetConfig`/`UpdateConfig`/`SetKey` handling; `web_bundle` truthful; `SESSION_ID`'s constant and comment go |
| `crates/tipsy-compadre/src/server/` | New: the server, the `Hub`, the connection bridge, the embedded assets, TLS |
| `crates/tipsy-compadre/src/client.rs` | The `--connect` path choosing the bridge instead of `spawn_with_events`; the configuration screen's save and Test paths going through the protocol when remote; the "insecure transport" refusal; the HUD's `web_bundle` message |
| `crates/tipsy-compadre/src/client/bridge.rs`, `pty.rs` | New: the client-side socket bridge and the PTY owner |
| `crates/tipsy-compadre/src/main.rs` | `serve`, `--connect`, `--token`, `--tls`, `--tls-cert`, `--tls-key`, `--verbose` |
| `crates/tipsy-compadre/src/config/model.rs`, `store.rs`, `form.rs` | The token generated on first run (32 bytes); `ConfigView` conversions; the token's redaction rule |
| `crates/tipsy-compadre/src/logging.rs` | New: the `tracing` bootstrap and the redaction rules |
| `crates/tipsy-ui/src/config_form.rs` | `ConfigForm` gains the conversions to and from `ConfigView` (§4.3); the insecure-transport state; `web_bundle` in the HUD |
| `crates/tipsy-ui/src/state.rs` | `web_bundle`, `ProfileChanged`, `ConfigChanged` |
| `crates/tipsy-ui/src/strings.rs` | The new copy, es and en |
| `crates/tipsy-web/` | New crate: the Ratzilla backend, `web-sys` WebSocket, browser audio |
| `xtask/` | New crate: `cargo xtask dist` |
| `web/client/index.html`, `web/terminal/index.html`, `web/vendor/README.md` | New assets |
| `docs/architecture.md` | W1's `axum` claim becomes true and stays; W2's certificate paragraph; W8's token; W10's PTY child; the crate table gains `tipsy-web` and `xtask` |
| `README.md` | `serve` and `--connect` instructions; the TLS and microphone truth; what to do without a bundle |
| `CLAUDE.md` | The current-work section: Plan 2C's real state, the login plan, this plan |

## 13. Risks and follow-ups

1. **The Ratzilla gate is unresolved until Phase 3's first task runs.** §8.1's four findings are
   requirement-level, not build-level: the `wasm32-unknown-unknown` target is not installed here and
   nothing was compiled. If the spike fails, the web client is postponed, exactly as `spec:389-391`
   says, and this design's §8 becomes the record of why.
2. **W2's honest limitation.** Generated self-signed certificates do not make a browser trust them;
   §9.5 records the evidence and the case it is not verified for. Anyone who reads the TLS flag as
   "the microphone now works from my phone" has been misled by a promise this design refuses to make.
3. **W3's security edge.** `SetKey` on a plaintext non-loopback connection is refused (§9.4). If a
   future plan lifts that refusal without adding TLS enforcement, an API key crosses the LAN in the
   clear, and the v1 spec §11 promise silently stops being true.
4. **Two tungstenite versions in one binary (W1).** Not a correctness problem, but every socket task
   must say which `Message` it means, and a future reader who greps for `Message::Binary` will find
   two unrelated enums.
5. **The spec-vs-code discrepancies this tier does not fix**, recorded so nobody re-discovers them:
   `Event::SpeechChunk` has no `last` field (`spec:125`); the v1 spec §5.3 says pressing `Tab` while
   the assistant is speaking sends `Cancel` and drops the queue (`spec:166-171`), but `Tab` raises
   `ListenStart` (`tipsy-ui/src/state.rs:412-422`) and the client's `starts_turn` answers `false` for
   it (`client.rs:632`), so **the native client does not flush on that key** — the browser mirrors the
   code, because terminal first means the code is the truth and the spec is the aspiration; the
   `tracing`/`--verbose` pair arrives here rather than in milestone 11 (W12); `cargo xtask dist`
   arrives here in the minimal form of §8.3 rather than the release form of milestone 11; and the
   glyph-width hazard §8.1 used to carry was **withdrawn after measurement** — the spike found one
   column per glyph and a structural reason for it — while Ratzilla's other DOM-backend issues (#186,
   #190) stay untested. The rest of the recon's §12 list — the
   missing `GetConfig`/`UpdateConfig`/`SetKey`, `ConfigView`, `ProfileChanged`/`ConfigChanged` — is
   what W3 and W4 close.
6. **The Server tab's Regenerate-token row stays inert over the wire, by design, and this is a
   decision rather than an omission.** §4.2 adds three commands and none of them mints a token;
   `store::new_token` (`config/store.rs:28-48`) is server-side and `ConfigEffect::RegenerateToken`
   (`tipsy-ui/src/config_form.rs:55`) has no remote path. **No spec ever promised one** — the v1
   spec's §4.1 command list does not include it either — and a remote client rotating the token it
   authenticated with is a self-lockout footgun: the new token would have to come back through
   `GetConfig`, which §4.3 permits only to a client that presented the old one, so a client that
   rotated and then reconnected with a stale value is locked out of its own server. Rotation stays a
   server-machine act (the native screen, or `tipsy-compadre config path` and an editor). **If a
   future plan wants the row live, the shape is one command** — `RegenerateToken`, owned by the wire
   task, refused on §9.4's insecure-transport rule for the same reason `SetKey` is (the new token
   would otherwise cross a plaintext socket), with a test that the old token no longer authenticates
   and the new one does.
6. **The login plan's wire contract is honoured unchanged, and its *wiring* is this tier's — a
   correction to an earlier draft of this very section.** `LoginView` and the `Login*` commands travel
   as the login spec's §4 defines them, and this tier adds nothing to them: what `tipsy-ui` already
   does with `Welcome.login` — the `LoginState` transitions, the veil, the key handling — comes to the
   browser through the same crate, unmodified. But **the engine around it does not**: `tipsy-web` must
   open its own `getUserMedia` in step with the login capture, stream `Frame::Mic` only while that
   capture is open, forward the three commands, dismiss the veil on `UiAction::LoginDone`, and honour
   the once-per-run rule when the server re-sends `Welcome` on `SetProfile`/`SetListeningMode`. The
   login spec's §13 names exactly this and is right to; the earlier draft's "a web client inherits the
   login screen for free" was the half-truth of a shared *state* and a per-client *engine*. **Phase 3
   carries it as Task 3.7**, and that task also amends the two sentences in the login spec this tier
   makes untrue — §1's "no Login tab" non-goal and L2's "the allowlist never leaves it" — because
   Task 1.1 extends `LoginView` with `enabled` and `allowed` so the configuration screen can show and
   edit the section over the wire. The *decision* survives both: the server still judges, and
   `server.token` is still the only door.
