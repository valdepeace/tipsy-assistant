//! One connection: the socket on one side, a session's channels on the other, and no way out that
//! skips the teardown.
//!
//! Every `Message` here is **axum's**: `Text(Utf8Bytes) | Binary(Bytes) | Ping(Bytes) | Pong(Bytes)
//! | Close(Option<CloseFrame>)`, with no `Frame` variant and with ping and the close handshake
//! handled by axum itself (`web-transport-and-terminal.md:203-228`). It is not
//! `tokio_tungstenite::tungstenite::Message`, which the client side uses and which has a `Frame`
//! variant: axum 0.8.9 pins `tokio-tungstenite ^0.29` while the native client keeps 0.30, so both
//! enums are compiled into this binary (W1, `web-transport-and-terminal.md:161-167`).
//!
//! **And every command from a socket goes to the session, `UpdateConfig` included.** The hub is the
//! only writer of the configuration (Shared interface contract), but the *route* to it is the
//! session's own `Command::UpdateConfig` arm, because the session is the only thing that knows
//! whether this client was admitted: a bridge that wrote the hub itself let a socket that had not
//! said `Hello` persist a configuration change while its session was still refusing it. The arm is
//! behind the handshake, so no config-writing command can run unadmitted. The hub's own fan-out is
//! `ToServer::Reconfigure`, a different message with no payload, which is what keeps the client route
//! and the fan-out from calling each other — Task 1.3's spin — and keeps a client's view (no provider
//! address, no key name, no token) from ever being taken for the whole configuration.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::stream::StreamExt;
use tipsy_protocol::{ClientKind, Command, ErrorCode, Event, Frame, ToClient, ToServer};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

use crate::client::pty::{PtySession, PtySpawner};
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
    /// The largest inbound message the socket will assemble, in bytes — and, because the upgrade sets
    /// `max_frame_size` from the same number, the largest single frame too. axum's default is **64
    /// MiB** and `Frame::decode` caps nothing (`tipsy-protocol/src/lib.rs:465-480`), so without this a
    /// single binary message could hand a session a payload the size of a small file. See
    /// [`READ_CEILING`] for the arithmetic behind the value.
    pub max_message_size: usize,
}

/// The largest message a client may send, and why it is this number.
///
/// A microphone frame is 16 kHz mono i16 over 20 ms: 320 samples, 640 bytes, 641 with its kind byte
/// (`audio/device.rs:20`, `tipsy-protocol/src/lib.rs:446-463`). 256 KiB is **400 frames** — over eight
/// seconds of audio in one message, when a client sends one frame per 20 ms — so a legitimate client
/// cannot come near it even if it batches; and it is 256 times below axum's 64 MiB default, so the
/// most a peer can make a session hold in one step is a quarter of a megabyte. The largest text
/// message in this protocol is a `ClientConfigView`: a whole configuration, with the providers' key
/// *names* and none of their addresses. The default `config.toml` is a few KiB, and even a hand-written
/// one with several long prompts is an order of magnitude below this ceiling.
pub const READ_CEILING: usize = 256 * 1024;

impl Default for ConnOptions {
    fn default() -> Self {
        Self {
            write_timeout: Duration::from_secs(5),
            queue: WRITE_QUEUE,
            write_buffer_size: 128 * 1024,
            max_write_buffer_size: 1024 * 1024,
            max_message_size: READ_CEILING,
        }
    }
}

/// Applies [`ConnOptions`]' socket bounds to an upgrade, in **one** place: the production `/ws`
/// handler (`server::ws`) and this file's own test handler both call it, so a bound cannot be dropped
/// from one of them and stay covered by the other's test (M1, the read ceiling's lost coverage).
///
/// `max_frame_size` is set from `max_message_size` deliberately: a *message* ceiling that left single
/// frames unbounded would let one framing choice — a fragmented message — carry a frame past it.
pub fn with_limits(upgrade: WebSocketUpgrade, options: ConnOptions) -> WebSocketUpgrade {
    upgrade
        .write_buffer_size(options.write_buffer_size)
        .max_write_buffer_size(options.max_write_buffer_size)
        .max_message_size(options.max_message_size)
        .max_frame_size(options.max_message_size)
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
///
/// `peer` is the address the socket arrived from and `tls` is whether the listener terminates TLS.
/// Together they are §9.4's whole rule ([`crate::server::tls::accepts_a_secret`]): they are passed in
/// rather than read from the configuration because a `SetKey`'s safety is a fact about *this*
/// connection, not about what the file says the server intends.
pub async fn serve(
    socket: WebSocket,
    ends: SessionEnds,
    hub: Hub,
    client: ClientKind,
    peer: SocketAddr,
    tls: bool,
) -> Ending {
    serve_with(socket, ends, hub, client, ConnOptions::default(), peer, tls).await
}

/// The same bridge with the bounds chosen — the shape the tests drive, and the shape Task 2.2's PTY
/// pump can reuse.
pub async fn serve_with(
    socket: WebSocket,
    ends: SessionEnds,
    hub: Hub,
    client: ClientKind,
    options: ConnOptions,
    peer: SocketAddr,
    tls: bool,
) -> Ending {
    // `session_id` is the id the hub allocated and `Welcome` carries. `Hub::spawn` already attached
    // this session, before anything was sent to the client, so nothing here attaches it again — the
    // id is bound here only so the one exit can hand it back to `detach`, and so a test can attribute
    // what this connection is holding to it.
    let SessionEnds { to_server, mut from_server, session_id, .. } = ends;
    tracing::debug!(?client, "a client connected to /ws");

    let (queue_tx, queue_rx) = mpsc::channel::<Message>(options.queue.max(1));
    let (sink, mut stream) = socket.split();
    let mut writer = tokio::spawn(write_side(session_id, sink, queue_rx, options.write_timeout));

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
                Some(Ok(message)) => match inbound(message, &to_server, peer, tls).await {
                    Inbound::Nothing => {}
                    // The session's end of the channel is gone: the session is over. That is the
                    // server stopping, not this peer saturating anything — `Saturated` is for the
                    // peer that stopped reading, and a session that ended must not be reported as one.
                    Inbound::Gone => ending = Some(Ending::ServerStopped),
                    Inbound::Send(message) => {
                        if enqueue(session_id, &queue_tx, encode(message)).await.is_err() {
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
                    let stopping = matches!(&event, Event::Error { recoverable: false, .. });
                    match enqueue(session_id, &queue_tx, encode(ToClient::Event(event))).await {
                        Ok(()) if stopping => ending = Some(Ending::ServerStopped),
                        Ok(()) => {}
                        Err(_) => ending = Some(Ending::Saturated),
                    }
                }
                Some(ToClient::Audio(frame)) => {
                    if enqueue(session_id, &queue_tx, encode(ToClient::Audio(frame))).await.is_err() {
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

/// Decodes one inbound frame and hands it to the session — **except for the one command the bridge
/// answers itself.**
///
/// `UpdateConfig` is handed to the session and **not** written to the hub here. The hub is still the
/// only writer of the configuration (Shared interface contract), but the session is the only place
/// that knows whether this client was admitted, and the bridge does not: a connection that has not
/// said `Hello` yet has a session that is still refusing it, and the earlier shape of this function
/// let exactly that socket persist a configuration change while its session was refusing it. The
/// session's own `Command::UpdateConfig` arm is behind the handshake, so it is the one route that
/// cannot run unadmitted; it calls `Hub::update_config_from_view`, which merges the fields a client's
/// view does not carry, writes the file and fans the change out as `ToServer::Reconfigure`.
///
/// **And this is where §9.4's rule is enforced**, one step before the same place anything else is
/// handed over: a `SetKey` on a connection that is neither loopback nor TLS is answered with
/// `Event::Error { code: insecure_transport, recoverable: true }` and **never pushed into
/// `to_server`**. The code is the machine meaning and the message is a sentence for a person; the
/// value that was refused appears in neither. The socket stays open: the same screen can save the
/// same key once the connection is TLS. This is the guard rather than the session's own `SetKey` arm
/// because the session cannot see the transport — its `SessionPolicy.loopback` is a fact about the
/// listener, and "the listener is TLS" is not in it at all.
///
/// A view the hub refuses writes nothing and answers nothing, exactly as the session's arm was
/// silent for one: the client's own screen is what names the row it could not parse.
///
/// A malformed command or audio frame is an `Error` event on the wire and **not** a panic and
/// **not** a dropped connection (spec §10).
async fn inbound(message: Message, to_server: &mpsc::UnboundedSender<ToServer>, peer: SocketAddr, tls: bool) -> Inbound {
    match message {
        // axum answers ping, pong and the close handshake itself; the close is handled by the loop.
        Message::Ping(_) | Message::Pong(_) | Message::Close(_) => Inbound::Nothing,
        Message::Text(text) => match serde_json::from_str::<Command>(&text) {
            Ok(command) => {
                let message = ToServer::Command(command);
                if let Some(refusal) = crate::server::tls::refuse_set_key(&message, peer, tls) {
                    // The socket stays open: the screen can try again once the connection is TLS.
                    return Inbound::Send(refusal);
                }
                if to_server.send(message).is_err() {
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

/// Spec §10's malformed-message row: `code: "bad_message", recoverable: true`. The code is the
/// machine half and `ErrorCode::BadMessage` is the variant Task 1.1 added for exactly this row — the
/// plan's draft sent `ErrorCode::Internal` because that variant did not exist yet, and it does now.
/// The message repeats the row's own spelling because the sentence a client *shows* is the client's
/// business: this bridge has no profile language to localize with.
fn bad_message(what: &'static str) -> Event {
    Event::Error { code: ErrorCode::BadMessage, message: format!("bad_message: {what}"), recoverable: true }
}

/// The one place a `ToClient` becomes a socket frame: events are JSON text, audio is binary
/// (`tipsy-protocol/src/lib.rs:1-3`).
fn encode(message: ToClient) -> Message {
    match message {
        ToClient::Event(event) => Message::Text(serde_json::to_string(&event).expect("an event serializes").into()),
        ToClient::Audio(frame) => Message::Binary(frame.encode().into()),
    }
}

/// Hands one message to the writer's queue, counted as held for this connection until the writer
/// takes it. `Err` means the writer is gone — it gave up on a peer that stopped reading, or the
/// socket failed — and the queue's receiver was dropped with it, which is the whole of how a write
/// that gave up reaches the bridge. The undelivered message comes back with the error, which is
/// what `mpsc::Sender::send` does; nothing here wants it back.
async fn enqueue(
    session_id: u64,
    queue: &mpsc::Sender<Message>,
    message: Message,
) -> Result<(), mpsc::error::SendError<Message>> {
    hold(session_id);
    match queue.send(message).await {
        Ok(()) => Ok(()),
        Err(error) => {
            // The message never entered the queue, so it is not held by anyone.
            release(session_id);
            Err(error)
        }
    }
}

/// The writer half: one socket write at a time, each with a deadline. A send that does not finish in
/// `write_timeout` ends this task; that drops the queue's receiver, which is what stops the drain on
/// the other side and so carries the pressure back to the session.
async fn write_side<S>(session_id: u64, mut sink: S, mut queue: mpsc::Receiver<Message>, write_timeout: Duration)
where
    S: futures::Sink<Message> + Unpin,
    // Spelled out because the body logs it: a generic `Sink`'s `Error` has no bounds at all, and
    // `tracing`'s `%e` needs `Display`. Every caller's sink is axum's, whose error is `axum::Error`.
    S::Error: std::fmt::Display,
{
    while let Some(message) = queue.recv().await {
        // The writer has it now: it is off the queue, and from here it is the socket's own buffer
        // that holds it, under `max_write_buffer_size`.
        release(session_id);
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

// ---------------------------------------------------------------------------------------------
// The web terminal: `/terminal/ws`, and the pump behind it (spec §5.3, §7.3).
//
// This is **not** the protocol socket above. Nothing here is JSON that a session reads, no `Command`
// or `Event` crosses it, and it shares nothing with the bridge but the crate: the bytes on the wire
// are the bytes a terminal would have shown, and the session never sees this socket. The child it
// starts is this same binary in `--connect` mode, so it *is* a client of the protocol socket — one
// hop away, on the server's own loopback, which is what keeps a browser from needing a token in a
// URL to reach a session (spec §7.2, W10).
//
// **Nothing is filtered, dropped, reordered or coalesced in either direction, and that is the
// contract rather than a style.** ConPTY's first byte is a device status report request (`ESC[6n`)
// and it delivers nothing else until the terminal answers with a cursor position report (measured by
// Task 2.1: four bytes and an empty configuration directory for 9.3 seconds, with the child alive past
// when it should have exited). xterm.js answers it natively, so the pump must carry it — and must
// carry every other byte — through unmodified.
// ---------------------------------------------------------------------------------------------

/// The ceiling this phase accepts on live PTY children: one process and two pipe pairs each, and two
/// browsers on `/terminal` are two TUI clients fighting for the same microphone anyway (research note
/// §7, trap 4). One more than this is answered `503 Service Unavailable` **before the upgrade**, so a
/// refused page fails its handshake instead of getting a socket that closes under it, and nothing is
/// spawned for it.
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

/// The socket's write-buffer ceiling: 256 KiB, or two of axum's default 128 KiB buffers. axum's
/// `max_write_buffer_size` is **unlimited** until it is set (research note §7, trap 2), and the only
/// thing past `write_buffer_size` that grows it is writes that are failing — which is exactly the
/// browser that stopped reading.
const TERMINAL_WRITE_CEILING: usize = 256 * 1024;

/// The size a terminal child is spawned with, before the page's first `resize` arrives. xterm.js's
/// `fit` addon sends one as soon as it has measured, so this is only what the very first paint of a
/// child that has not been resized yet sees.
const TERMINAL_COLS: u16 = 80;
const TERMINAL_ROWS: u16 = 24;

/// How long teardown waits for the OS to confirm that the kill landed. It is a bound rather than a
/// wait, because the socket must not be held open by a child that will not go; a `wait` is the only
/// thing that turns "we called kill" into "the OS says it is gone", and `ChildKiller::kill` cannot be
/// trusted to report (it inverts `TerminateProcess`'s result on Windows — see
/// [`crate::client::pty::PtySession::killer`]).
const TERMINAL_REAP_TIMEOUT: Duration = Duration::from_secs(10);

/// How often the pump asks the OS whether the child is still there.
///
/// **The pump cannot learn it from the reader, and that is ConPTY's doing**: the pseudoconsole keeps
/// the console output pipe open after its client exits, so EOF arrives only when the master is
/// dropped — after the loop. Measured here with a PowerShell child that printed one line and exited
/// with 7: the reader stayed parked for the twenty seconds the pump waited, and no exit frame was
/// ever sent. A quarter of a second is a cheap `GetExitCodeProcess`, and the page's "the terminal
/// ended" line is worth more than that.
const TERMINAL_EXIT_POLL: Duration = Duration::from_millis(250);

/// The query `/terminal/ws` is reached with. The token is the one `/terminal` itself takes, because a
/// browser has nowhere else to put it (spec §5.3, §9.2): a WebSocket handshake carries no body and a
/// browser cannot set a header on it.
#[derive(serde::Deserialize)]
pub struct TerminalQuery {
    token: Option<String>,
}

/// What the terminal route needs from whatever state the router was built with.
///
/// **It is a trait because the route has two callers with different states**: `server::run` passes its
/// own `ServeState`, and this module's tests pass a two-field value of their own, so the pump is
/// tested against a real listener and real children without a server, a hub, a provider stack or a
/// credential store. `server::run`'s state implements it in `server/mod.rs`.
pub trait TerminalRouteState: Clone + Send + Sync + 'static {
    /// The token this server requires on both terminal routes, or `None` when it requires none.
    fn websocket_token(&self) -> Option<String>;

    /// What a connection spawns its child from.
    fn terminal_spawner(&self) -> Arc<dyn PtySpawner>;

    /// **The cap, and it belongs to a listener rather than to the process.** One `Arc<Semaphore>` per
    /// server, acquired before the upgrade and held for the connection: a process-wide counter would
    /// need a check and an increment that two simultaneous upgrades could interleave, and it would
    /// make one listener's load refuse another's connection.
    fn terminal_gate(&self) -> Arc<Semaphore>;
}

/// Registers `/terminal/ws` on a router whose state implements [`TerminalRouteState`]. This is the
/// one place the route is spelled: `server::mount` merges it into the real router, and this module's
/// tests build a router of their own with it.
pub fn router<S: TerminalRouteState>(router: axum::Router<S>) -> axum::Router<S> {
    router.route("/terminal/ws", axum::routing::get(terminal_ws::<S>))
}

/// `Ok(())` when the query carries the token this server requires, or when the server requires none.
/// `Err` is a 401's body and names the rule, never the value — the token never reaches a log line, and
/// a refusal that echoed it would be a refusal that leaked it (spec §9.6).
///
/// **`presented` is the token as the query decoded it**, which is what `Query<TerminalQuery>` hands
/// the handler; comparing the raw `token=…` text here was the plan's spelling and would never have
/// matched its own call site.
pub fn terminal_access(presented: Option<&str>, expected: Option<&str>) -> Result<(), String> {
    match expected {
        None => Ok(()),
        Some(expected) if presented == Some(expected) => Ok(()),
        Some(_) => Err("this server requires its token on /terminal/ws".to_string()),
    }
}

/// `GET /terminal/ws?token=…`, upgrade. The token rule is §5.3's, the same one `/terminal` uses; the
/// `Origin` rule is the one the fix wave added to `/ws` and the ledger ruled this route needs too.
pub async fn terminal_ws<S: TerminalRouteState>(
    State(state): State<S>,
    Query(query): Query<TerminalQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // **The same check `/ws` makes, on the same reasoning** (`.superpowers/sdd/2026-10-11-plan-3-web-tier/progress.md`,
    // the scoped re-review of the second fix wave): a WebSocket handshake is not subject to the
    // same-origin policy, so any page in the user's browser can open this route against a loopback
    // listener, and the terminal it would get spawns a child that is itself a client of `/ws` — with
    // `Command::UpdateConfig` in reach, which the hub persists. `403` for the same reason it is `403`
    // there: no credential would change the answer. Absent is admitted, which is the CLI.
    if !crate::server::origin_is_this_host(&headers) {
        return forbidden_origin();
    }
    if let Err(message) = terminal_access(query.token.as_deref(), state.websocket_token().as_deref()) {
        return (StatusCode::UNAUTHORIZED, message).into_response();
    }
    // Acquired before the upgrade, so a refusal is an HTTP status the page can read rather than a
    // socket that closes on its first frame, and so the `503` costs nothing but a comparison.
    let Ok(permit) = state.terminal_gate().try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("this server already has {MAX_TERMINAL_CONNECTIONS} terminals open"),
        )
            .into_response();
    };
    let ws = ws.max_write_buffer_size(TERMINAL_WRITE_CEILING);
    ws.on_upgrade(move |socket| handle_terminal(socket, state, permit))
}

/// A foreign `Origin`, refused before the upgrade. The body names no address, no header and no token,
/// exactly as `/ws`'s refusal does; `server/mod.rs`'s [`crate::server::origin_is_this_host`] is the
/// whole rule and is shared rather than copied.
fn forbidden_origin() -> Response {
    (StatusCode::FORBIDDEN, "the Origin of this request is not this server's own host").into_response()
}

/// One terminal's whole life: spawn the child, pump until either side ends, kill and reap, then say
/// what the child exited with.
///
/// `permit` is held for the duration of this function and nothing else, so the connection's slot is
/// released by the same scope that owned the child.
async fn handle_terminal<S: TerminalRouteState>(mut socket: WebSocket, state: S, permit: OwnedSemaphorePermit) {
    let spawner = state.terminal_spawner();
    // `openpty` and `spawn_command` are both synchronous and can take a moment on this platform, so
    // they run on the blocking pool rather than on a runtime worker (research note §7, trap 7).
    let spawned = tokio::task::spawn_blocking(move || spawner.spawn(TERMINAL_COLS, TERMINAL_ROWS)).await;
    let mut session = match spawned {
        Ok(Ok(session)) => session,
        Ok(Err(message)) => {
            // The one thing a page can act on: no pseudo-terminal is available on this machine (a
            // Windows older than 10 1809, most likely). The message names the platform's own reason.
            let _ = socket.send(Message::text(format!("terminal unavailable: {message}"))).await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
        // The blocking pool refused the job, or the spawn panicked. Nothing to report but the end.
        Err(_) => {
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };

    let observed = pump(&mut socket, &mut session).await;
    // The code the pump already read off the OS, if the child ended on its own; otherwise the pump
    // has killed it and this is where that kill is *confirmed*. `kill` reports neither success nor
    // failure honestly on Windows, so the only answer that means anything is the OS's, and `wait` is
    // how it is read. It blocks, so it runs on the blocking pool, and it is bounded so a child that
    // will not go cannot hold the socket.
    let exit = reap(session, observed).await;
    let _ = socket
        .send(Message::text(serde_json::json!({ "type": "exit", "code": exit }).to_string()))
        .await;
    let _ = socket.send(Message::Close(None)).await;
    drop(permit);
}

/// The child's own exit code.
///
/// `observed` is what the pump's own poll of the OS already saw: when it is `Some`, the child ended by
/// itself and the code is that answer — there is nothing left to kill, and reaping it twice would be
/// a second `wait` on a handle that is gone. When it is `None` the socket died first, so this is where
/// the pump's kill is confirmed by a bounded, blocking `kill_and_wait`. A child that cannot be reaped
/// inside [`TERMINAL_REAP_TIMEOUT`] is reported as 0 rather than guessed at; the page prints a line
/// either way.
async fn reap(mut session: PtySession, observed: Option<u32>) -> u32 {
    if let Some(code) = observed {
        return code;
    }
    let reaped =
        tokio::time::timeout(TERMINAL_REAP_TIMEOUT, tokio::task::spawn_blocking(move || session.kill_and_wait())).await;
    match reaped {
        Ok(Ok(Ok(status))) => status.exit_code(),
        Ok(Ok(Err(message))) => {
            tracing::debug!(%message, "the terminal child could not be reaped");
            0
        }
        Ok(Err(_)) => 0,
        Err(_) => {
            tracing::debug!("the terminal child did not go inside the reap window");
            0
        }
    }
}

/// The pump. Both directions are bounded and both ends of the PTY are serviced on threads of their
/// own — ConPTY's own guidance is to service each channel on a separate thread, "to prevent race
/// conditions and deadlocks" (research note §5.3). Every byte goes through untouched.
///
/// It returns the exit code the OS reported for a child that ended by itself, or `None` when the
/// socket ended first. **Only [`PtySession::try_wait`] can produce the `Some`**: the blocking reader
/// cannot, because ConPTY keeps the console's output pipe open after its client exits, which is
/// measured and recorded on that method.
async fn pump(socket: &mut WebSocket, session: &mut PtySession) -> Option<u32> {
    let mut resize = ResizeDebounce::new(TERMINAL_RESIZE_DEBOUNCE_MS);
    // An interval rather than a `sleep` inside the branch: a TUI that repaints continuously would make
    // a re-created `sleep` future start again on every frame, and the exit of its child would never be
    // noticed. `Interval::tick` is cancel safe, so no tick is lost to another branch winning.
    let mut exit_poll = tokio::time::interval_at(tokio::time::Instant::now() + TERMINAL_EXIT_POLL, TERMINAL_EXIT_POLL);

    // One bounded queue, two threads: the blocking reader fills it, the socket drains it. `send`
    // awaits when it is full, which is exactly the backpressure a WebSocket does not have.
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(TERMINAL_WRITE_QUEUE);
    let reader_task = session.take_reader().map(|mut reader| {
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
                    // A read error is this console ending; there is no second reader to retry with.
                    Err(_) => break,
                }
            }
        })
    });

    // The other direction is a blocking writer fed by a task of its own, so no runtime worker ever
    // sits in `Box<dyn Write>` (research note §7, trap 7).
    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(TERMINAL_WRITE_QUEUE);
    let writer_task = session.take_writer().map(|mut writer| {
        tokio::task::spawn_blocking(move || {
            while let Some(bytes) = in_rx.blocking_recv() {
                if std::io::Write::write_all(&mut writer, &bytes).is_err() {
                    break;
                }
                let _ = std::io::Write::flush(&mut writer);
            }
        })
    });

    // The child's own end, and the only exit this loop can be told about on purpose.
    let mut observed: Option<u32> = None;
    loop {
        // The debounce's deadline is read as a plain value here, so the `select!` holds no borrow of
        // `resize` while the branches below mutate it. `None` is "no resize is pending", where the
        // branch's future never completes and can never win.
        let wake = resize.deadline;
        tokio::select! {
            chunk = out_rx.recv() => match chunk {
                Some(bytes) => {
                    if socket.send(Message::binary(bytes)).await.is_err() {
                        break;
                    }
                }
                // The reader reached EOF: this console ended.
                None => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let (bytes, request) = match serde_json::from_str::<TerminalClientMessage>(&text) {
                        Ok(message) => (message.bytes(), message.resize()),
                        // A malformed message is a bug in the page, not a reason to end a session
                        // someone may be talking to: ignore it and keep the socket open.
                        Err(_) => (None, None),
                    };
                    if let Some(bytes) = bytes {
                        // A closed writer task is the PTY's input being gone, not the socket's.
                        let _ = in_tx.send(bytes).await;
                    }
                    if let Some((cols, rows)) = request {
                        resize.request(cols, rows);
                    }
                }
                // The page's `tipsterm-hello-pad` frame, and nothing else, is binary. It exists so a
                // browser can observe that its own socket is really open at the server; it is not data
                // for the child's stdin, and writing it there would type 256 characters into the TUI.
                Some(Ok(Message::Binary(_padding))) => {}
                // A close frame or a transport error: the tab is gone.
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                // Ping and Pong, which axum answers itself.
                Some(Ok(_)) => {}
            },
            () = sleep_until_or_never(wake) => {
                // A let chain, because a nested `if let` is the shape clippy's own `collapsible_if`
                // rejects and the eight-finding baseline is a set this task may not grow.
                if let Some((cols, rows)) = resize.take()
                    && let Err(message) = session.resize(cols, rows)
                {
                    tracing::debug!(%message, "terminal resize ignored");
                }
            },
            _ = exit_poll.tick() => {
                if let Ok(Some(status)) = session.try_wait() {
                    observed = Some(status.exit_code());
                    // Everything the console already handed the reader goes out before the exit
                    // frame, so a page's last line is not overtaken by the notice that it ended.
                    while let Ok(bytes) = out_rx.try_recv() {
                        if socket.send(Message::binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                    break;
                }
            }
        }
    }

    // Teardown, in this order and for these reasons (spec §7.3, Review Focus 4): the child is killed,
    // the writer is dropped — "Dropping the writer will send EOF to the slave end" (research note
    // §5.1) — and then the master goes, so the blocking reader observes EOF and exits. A TUI that
    // ignores EOF on stdin would otherwise linger, which is why killing is the first step and not the
    // last, and why every step here is explicit rather than left to a `Drop`.
    //
    // `PtySession::kill` and not `killer().kill()`: the handle `clone_killer` returns inverts
    // `TerminateProcess`'s result on Windows (Task 2.1 measured `Err(Os { code: 6 })` *after* the
    // process had died), so it reports the opposite of the truth and is not evidence of anything. The
    // reaping that confirms this kill is `reap`, after this function returns — and it is skipped
    // altogether when `observed` is `Some`, because then the OS has already answered.
    if let Err(message) = session.kill() {
        tracing::debug!(%message, "the terminal child could not be killed");
    }
    // Nothing more is written. Dropping the sender ends the writer task, and with it the writer —
    // which is what sends EOF to the slave end.
    drop(in_tx);
    // Both handles are dropped rather than awaited. Awaited, they would be the one place this socket
    // could stall: the reader is parked in a blocking `read` until the master goes, and `await`ing a
    // `spawn_blocking` task does not help a task that is not due to finish. They end on their own —
    // the writer when its channel closes, the reader when the master is dropped by `reap`.
    drop(writer_task);
    drop(reader_task);
    observed
}

/// A future that completes when the debounce window ends, or never when no size is pending.
async fn sleep_until_or_never(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// The debounced resize: xterm.js's own doc is the reason (research note §6.3). The last size asked
/// for inside the window is the one applied, and only when the window has elapsed.
struct ResizeDebounce {
    window: Duration,
    deadline: Option<tokio::time::Instant>,
    requested: Option<(u16, u16)>,
}

impl ResizeDebounce {
    fn new(debounce_ms: u64) -> Self {
        Self { window: Duration::from_millis(debounce_ms), deadline: None, requested: None }
    }

    /// A resize arrived: remember it and push the deadline back.
    fn request(&mut self, cols: u16, rows: u16) {
        self.requested = Some((cols, rows));
        self.deadline = Some(tokio::time::Instant::now() + self.window);
    }

    /// Take the pending size, if the window has elapsed. The `select!` in [`pump`] reads `deadline`
    /// itself rather than through a future-returning method, so nothing borrows this struct across an
    /// await.
    fn take(&mut self) -> Option<(u16, u16)> {
        self.deadline = None;
        self.requested.take()
    }
}

/// The three control messages the page sends, spelled exactly as `web/terminal/index.html` writes
/// them (Task 2.4). Everything else on this socket is binary and comes from the PTY.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalClientMessage {
    /// The page's opening frame, sent as the 256-byte binary padding message rather than as JSON; the
    /// variant exists so an older page that spells it in text is read rather than treated as garbage.
    Hello,
    /// `onData`: escape sequences and UTF-8 text, written to the PTY as UTF-8 bytes.
    Data { data: String },
    /// `onBinary`: one byte per JS `charCode`, mouse reports among them. Research note §6.4 is
    /// explicit that sending these as text is "exactly how mouse reports get corrupted" — a mouse
    /// report is `0xFF`-prefixed and not UTF-8 — so the page turns them into a latin-1 string and each
    /// char is truncated back to its low byte here, which is what the page did in reverse.
    Mouse { data: String },
    /// A debounced resize, in cells.
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

    /// The size this message asks for, if it asks for one.
    fn resize(&self) -> Option<(u16, u16)> {
        match self {
            Self::Resize { cols, rows } => Some((*cols, *rows)),
            _ => None,
        }
    }
}

/// How many messages each connection is holding for its socket right now: the writer's queue plus
/// the message being handed over. **Keyed by session id on purpose** — every test in this binary
/// shares this process, and a bare count would let one test's traffic fail another's assertion.
///
/// This exists for one test and one line: `the_write_side_queue_is_bounded` is the assertion that the
/// bound engages, and Review Focus 2 is the requirement it answers. Every message is counted exactly
/// once — up before the handover, down when the writer takes it — so a queue of `n` can never read
/// more than `n + 1` here, however fast the session produces and however little the peer reads.
#[cfg(test)]
static HELD_FOR_THE_SOCKET: std::sync::Mutex<std::collections::BTreeMap<u64, usize>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// One more message this connection is holding for its socket.
#[cfg(test)]
fn hold(session_id: u64) {
    let mut held = HELD_FOR_THE_SOCKET.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *held.entry(session_id).or_default() += 1;
}

/// One fewer: the writer has taken it off the queue.
///
/// A writer that gives up while messages are still queued drops them without this ever running, so a
/// count can be left behind for a session id that is already over. Nothing reads such an id again:
/// session ids are unique for the life of the process, and no build but a test build counts at all.
#[cfg(test)]
fn release(session_id: u64) {
    let mut held = HELD_FOR_THE_SOCKET.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(count) = held.get_mut(&session_id) {
        *count = count.saturating_sub(1);
    }
}

/// What this connection is holding for its socket right now.
#[cfg(test)]
fn queued_on_the_write_side(session_id: u64) -> usize {
    HELD_FOR_THE_SOCKET
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&session_id)
        .copied()
        .unwrap_or(0)
}

/// The no-op twins every other build uses, so the bridge's own code carries no `#[cfg(test)]` in the
/// middle of a function and costs a release build nothing.
#[cfg(not(test))]
fn hold(_session_id: u64) {}

#[cfg(not(test))]
fn release(_session_id: u64) {}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use futures::{SinkExt, StreamExt};
    use tipsy_protocol::{ClientConfigView, ClientKind, Command, ErrorCode, Event, Frame, PROTOCOL_VERSION};
    use tokio::net::TcpStream;
    use tokio_tungstenite::tungstenite::{Message as ClientMessage, Utf8Bytes};
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

    use super::*;
    use crate::audio::pcm::Pcm;
    use crate::config::store;
    // `SecretStore` is the trait `MemoryStore::set` comes from, which is why the plan's copy of this
    // helper — taken from `server/mod.rs`'s tests — does not compile without it.
    use crate::keys::{KeyResolver, MemoryStore, SecretStore};
    use crate::providers::{
        ChatProvider, ChatRequest, ProviderError, ProviderSlot, Providers, SpeechProvider, SpeechRequest, TextStream,
        TranscribeProvider,
    };
    use crate::session::SessionPolicy;

    /// How long a test lets the server do its work before calling it dead.
    const PATIENCE: Duration = Duration::from_secs(5);

    /// How many samples the speech fake returns for one sentence.
    ///
    /// **This is the lever Task 1.6's Step 4 note points at** ("raise the amount of greeting text
    /// rather than the test's patience"): a client that stops reading must meet a write that cannot
    /// fit in a socket buffer, and the greeting is only a sentence long. The number is chosen to sit
    /// just *under* the 1 MiB `max_write_buffer_size` the default `ConnOptions` sets, so the frame is
    /// buffered and then blocks the socket write, rather than being refused by the ceiling — that
    /// refusal is what `a_peer_that_stops_reading_ends_the_connection_instead_of_growing` tests, and
    /// the two tests must not be the same test.
    const SPEECH_SAMPLES: usize = 500_000;

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
        async fn synthesize(&self, _req: SpeechRequest) -> Result<Pcm, ProviderError> {
            Ok(Pcm { samples: vec![0.25; SPEECH_SAMPLES], sample_rate: 24000 })
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
    ///
    /// `keep` rather than a dropped `TempDir`, because the directory has to outlive this function:
    /// the routing test reads back the very file `Hub::apply_config` wrote.
    fn config() -> (crate::config::Config, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.keep().join("config.toml");
        let config = store::load_or_create(&path).unwrap().config;
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
    /// is the connection's: a test that needs the handshake to be the *only* gate passes
    /// `SessionPolicy::exposed`, and every other test passes the permissive one.
    #[derive(Clone)]
    struct ServeTest {
        hub: Hub,
        options: ConnOptions,
        policy: SessionPolicy,
    }

    /// The `/ws` route's own handler. The `WebSocketUpgrade` extractor takes the last slot and
    /// `State` the one before it, which is how Task 1.4's `ws` handler is written too.
    ///
    /// The peer and the TLS flag are the two facts Task 1.4's own handler reads off the connection;
    /// this one's listener is a plain loopback one, so it says so — which is also what makes the
    /// loopback half of the §9.4 test below the *real* behaviour of a local connection rather than a
    /// special case invented for a test.
    async fn handler_with_state(
        axum::extract::State(state): axum::extract::State<ServeTest>,
        upgrade: axum::extract::ws::WebSocketUpgrade,
    ) -> axum::response::Response {
        let hub = state.hub.clone();
        let options = state.options;
        let policy = state.policy.clone();
        let peer: SocketAddr = "127.0.0.1:54123".parse().expect("a literal loopback address parses");
        // The production handler calls the same function, which is what makes one test of the
        // ceiling a test of both routes (M1).
        super::with_limits(upgrade, options)
            .on_upgrade(move |socket| async move {
                // The hub's entry point: it allocates the session id, attaches the session before
                // `Welcome` can be missed, and clones the fakes in its slot into the session.
                let ends = hub.spawn(policy);
                serve_with(socket, ends, hub.clone(), ClientKind::Web, options, peer, false).await;
            })
    }

    /// Starts the listener and returns its address **and a handle to the hub behind it**. The
    /// listener is bound before this returns, so a connect from the test cannot race it.
    ///
    /// The handle is a second name for the same hub — `Hub` is `Clone` and every one of its fields is
    /// a handle — which is what lets the routing test look at the configuration the hub *wrote*
    /// rather than only at what the socket carried.
    async fn server_with_policy(chat: Arc<FakeChat>, options: ConnOptions, policy: SessionPolicy) -> (SocketAddr, Hub) {
        let (config, config_path) = config();
        let hub = Hub::new(
            config,
            config_path,
            keys(),
            ProviderSlot::new(Providers {
                chat,
                speech: Arc::new(FakeSpeech),
                transcribe: Arc::new(FakeTranscribe),
            }),
        );
        let state = ServeTest { hub: hub.clone(), options, policy };
        let app = axum::Router::new()
            .route("/ws", axum::routing::get(handler_with_state))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (addr, hub)
    }

    /// The same listener with the permissive policy every test but the admission one needs.
    async fn server_with_hub(chat: Arc<FakeChat>, options: ConnOptions) -> (SocketAddr, Hub) {
        server_with_policy(chat, options, SessionPolicy::local()).await
    }

    /// The same listener, for the tests that only need somewhere to connect to.
    async fn server(chat: Arc<FakeChat>, options: ConnOptions) -> SocketAddr {
        server_with_hub(chat, options).await.0
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
    /// every test waits it out before asking for the floor. The session id comes back because one
    /// test has to attribute what the bridge is holding to **its own** connection: the depth it reads
    /// is shared with every other test in this process.
    async fn wait_for_the_floor(socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>) -> u64 {
        let welcome = event_until(socket, is_welcome).await;
        let Event::Welcome { session_id, .. } = welcome else { panic!("`is_welcome` matched another event") };
        event_until(socket, is_done).await;
        session_id
    }

    /// The one session id this listener's hub has registered, as soon as it is there. `Hub::spawn`
    /// attaches the session before `Welcome` goes out, so nothing is raced — but this test must not
    /// read the socket for it: the whole point is that nobody is reading the socket.
    async fn wait_for_a_session(hub: &Hub) -> u64 {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            if let Some(id) = hub.session_ids().first() {
                return *id;
            }
            assert!(tokio::time::Instant::now() < deadline, "no session was attached to the hub");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// True once the hub has no live session left, which is what the one exit's `detach` leaves
    /// behind. Polled rather than asserted outright: the bridge notices a dropped socket when the
    /// kernel tells it, and the instant that happens is nobody's contract.
    async fn the_hub_forgets(hub: &Hub) -> bool {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            if hub.session_ids().is_empty() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
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
        // a frame is not JSON. The `compae` greeting is three sentences that the splitter makes two
        // of (`MIN_SENTENCE_CHARS`, `text.rs:5`), so two is what the greeting speaks.
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
                assert_eq!(code, ErrorCode::BadMessage, "spec §10's own code, not a class in its place");
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
            Event::Error { code, message, recoverable } => {
                assert!(message.contains("bad_message"), "{message}");
                assert_eq!(code, ErrorCode::BadMessage, "{message}");
                assert!(recoverable, "not a panic, and not a dropped connection (spec §10)");
            }
            other => panic!("an unknown frame kind is an error event, got {other:?}"),
        }
    }

    /// **F5, the socket half of the read side.** axum's `max_message_size` default is **64 MiB** and
    /// `Frame::decode` caps nothing, so one inbound message could hand a session tens of megabytes in
    /// a single step. The ceiling is explicit now (`READ_CEILING`) and it bites: a frame over it ends
    /// the connection, while a frame under it is answered normally. This test's ceiling is small so
    /// the frame is cheap; the two lines that apply the value are in `server::ws` and in this file's
    /// test handler.
    #[tokio::test]
    async fn a_message_over_the_read_ceiling_ends_the_connection_and_a_smaller_one_does_not() {
        let options = ConnOptions { max_message_size: 4096, ..ConnOptions::default() };
        // The chat fake is never asked: nothing here is a turn, and the greeting is a `Say`.
        let addr = server(FakeChat::new(vec![]), options).await;
        let mut socket = connect(addr).await;
        wait_for_the_floor(&mut socket).await;

        // Under the ceiling: a frame with an unknown kind is a recoverable error, so this is proof
        // that the connection really was alive and being read a moment before the next assertion.
        socket.send(ClientMessage::Binary(vec![0x7f, 0x00, 0x00].into())).await.unwrap();
        event_until(&mut socket, |e| matches!(e, Event::Error { .. })).await;

        // Over it: eight thousand samples are 16 KB of perfectly well-formed `Frame::Mic` payload, and
        // the ceiling is what refuses it.
        let oversized = Frame::Mic(vec![0i16; 8192]).encode();
        assert!(oversized.len() > options.max_message_size, "the frame has to be over the ceiling to prove it");
        socket.send(ClientMessage::Binary(oversized.into())).await.unwrap();

        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(!left.is_zero(), "a message over the read ceiling must end the connection");
            match tokio::time::timeout(left, socket.next()).await {
                Ok(Some(Ok(_))) => continue,
                Ok(Some(Err(_))) | Ok(None) => break,
                Err(_) => panic!("the connection survived a message over the read ceiling"),
            }
        }
    }

    /// Review Focus line 2, first half: the bridge's own queue is **bounded**. The client stops
    /// reading the moment it has connected, the greeting is a megabyte per sentence, and the writer
    /// therefore takes one frame, blocks inside the socket write and stays blocked. What is asserted
    /// is the depth the bridge may hold while that lasts: it may fill its queue and hold one message
    /// over the handover, and no more, however fast the session upstream produces.
    ///
    /// **`write_timeout` is two seconds here and not the plan's `Duration::ZERO`.** A zero deadline
    /// kills the writer on its first poll, which drops the queue's receiver, so the depth reads 0
    /// whatever the queue's capacity is and the assertion cannot fail. Two seconds keeps the writer
    /// *blocked* — the state the bound is about — when the depth is read at 400 ms. The socket is
    /// then left alone until that deadline has passed, because reading it is exactly what unblocks
    /// the writer; only then is the close read, which still lands inside the five the second half
    /// waits.
    ///
    /// **The greeting's size is the lever.** [`SPEECH_SAMPLES`] is what makes one write exceed a
    /// socket buffer, so this is the test of the queue and the next one is the test of the ceiling.
    #[tokio::test]
    async fn the_write_side_queue_is_bounded() {
        let options = ConnOptions { queue: 2, write_timeout: Duration::from_secs(2), ..ConnOptions::default() };
        let chat = FakeChat::slow(
            vec![Ok(vec!["uno ", "dos ", "tres ", "cuatro ", "cinco ", "seis ", "siete ", "ocho ", "nueve ", "diez "])],
            Duration::from_millis(2),
        );
        let (addr, hub) = server_with_hub(chat, options).await;
        let mut socket = connect(addr).await;
        let session_id = wait_for_a_session(&hub).await;

        // The greeting alone is more than the queue holds, so the bound is engaged before anything
        // else is asked for. The client is never polled while the writer is blocked: polling it is
        // what would let the write finish, and the bound is about what happens when nobody reads.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let queued = queued_on_the_write_side(session_id);
        eprintln!("the write side held {queued} message(s) of a queue of {}", options.queue);
        assert!(
            queued <= options.queue + 1,
            "the write side must hold at most its capacity plus the message in flight, held {queued}"
        );
        assert!(
            queued > 0,
            "the bound is only proved while the bridge is holding something, and it held {queued}: \
             this machine's socket buffers swallowed the greeting"
        );

        // Still without reading, for longer than the writer's deadline: the give-up has to happen
        // while the peer really is silent, or the read below is what lets the write finish and the
        // peer has stopped reading for 400 ms instead of long enough to matter.
        tokio::time::sleep(options.write_timeout + Duration::from_millis(200)).await;

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
            max_message_size: READ_CEILING,
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
    /// (`session.rs:203`) — the hub forgets the session, and the listener goes on serving.
    #[tokio::test]
    async fn a_disconnect_cancels_the_turn_and_the_listener_answers_the_next_connection() {
        // Two chunks 150 ms apart: a turn cancelled after the first never produces the second, and a
        // turn left running would make a second chat call.
        let chat = FakeChat::slow(vec![Ok(vec!["primera ", "segunda"])], Duration::from_millis(150));
        let (addr, hub) = server_with_hub(chat.clone(), ConnOptions::default()).await;
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
        // And Review Focus line 1's third clause: the hub forgets it. `detach` is on the one exit, so
        // a session still in the registry is a connection that leaked one.
        assert!(the_hub_forgets(&hub).await, "the hub must forget a session whose socket died");

        // And the listener still answers the next connection, which is the half of Review Focus
        // line 1 that a leaked or panicking session would fail.
        let mut second = connect(addr).await;
        wait_for_the_floor(&mut second).await;
        send_command(&mut second, Command::Ask { text: "¿estás?".into() }).await;
        event_until(&mut second, |e| matches!(e, Event::Reply { .. })).await;
    }

    /// **The hole this task closes.** The shared contract's last bullet: the hub is the only writer
    /// of the configuration, and a configuration command that arrives *from a socket* is the
    /// bridge's to route to it. `UpdateConfig` is the one that had lost its route — the session's
    /// own arm is the hub's fan-out half and calling back from there is what made Task 1.3's first
    /// implementation spin for ever — so a save over a socket used to reach one session and nothing
    /// else, while spec §4.2 asks the server to replace the configuration, persist it, re-derive
    /// every session and broadcast `ConfigChanged`.
    ///
    /// Both halves of "the hub did it" are observable from the socket, and neither can be produced
    /// by a session that merely applied the view to itself: the file on disk changes, and **both**
    /// connections are told `ConfigChanged` — the one that saved and the one that did not.
    #[tokio::test]
    async fn a_socket_update_config_reaches_the_hub_persists_and_reaches_every_client() {
        let chat = FakeChat::new(vec![]);
        let (addr, hub) = server_with_hub(chat, ConnOptions::default()).await;
        let mut first = connect(addr).await;
        let mut second = connect(addr).await;
        wait_for_the_floor(&mut first).await;
        wait_for_the_floor(&mut second).await;

        // What a screen holds, exactly as its save sends it back: the full view it was told, turned
        // into the shape a client may send (`ClientConfigView`), which is the conversion
        // `Client::save_remote` makes.
        send_command(&mut first, Command::GetConfig).await;
        let mut view = match event_until(&mut first, |e| matches!(e, Event::Config { .. })).await {
            Event::Config { config } => *config,
            other => panic!("`GetConfig` answers with the configuration, got {other:?}"),
        };
        assert_eq!(view.server.bind, "127.0.0.1:7070", "the editable configuration, as it stands");
        view.server.bind = "127.0.0.1:7171".into();
        let client_view = ClientConfigView::from(&view);
        send_command(&mut first, Command::UpdateConfig { config: Box::new(client_view) }).await;

        // The hub's own announcement, to the client that saved and to the one that did not. Nothing
        // else in the process sends this event: the session's arm calls the hub, and the hub's own
        // fan-out is `ToServer::Reconfigure`, which is not an event at all.
        event_until(&mut first, |e| matches!(e, Event::ConfigChanged)).await;
        event_until(&mut second, |e| matches!(e, Event::ConfigChanged)).await;

        // "persists it" (spec §4.2): the announcement is what `Hub::update_config` does *after* the
        // file was written, and the file is where the value has to be.
        assert_eq!(hub.config().server.bind, "127.0.0.1:7171", "the hub remembered the new value");
        let on_disk = store::load_or_create(hub.path()).unwrap().config;
        assert_eq!(on_disk.server.bind, "127.0.0.1:7171", "and it is in the file, not only in memory");
    }

    /// **F3, over a real socket.** A client's save may move the editable configuration and may not
    /// move where a provider is reached, which key name it spends, or the server's token. The *shape*
    /// makes sending them impossible (`ClientConfigView` has no such field); what this pins is the
    /// other half — the server's own values are still there afterwards, on disk and in the hub,
    /// because the merge takes them from the server rather than from the sender.
    ///
    /// The server's values are deliberately not the defaults: a `base_url` of its own, a key name of
    /// its own and a token, so a merge that quietly dropped or reset one of them is visible.
    #[tokio::test]
    async fn a_clients_save_cannot_move_a_provider_address_a_key_name_or_the_token() {
        let (addr, hub) = server_with_hub(FakeChat::new(vec![]), ConnOptions::default()).await;
        // The server's own configuration, written through the hub the way an operator's file is.
        let mut owned = hub.config();
        owned.server.token = "the-servers-own-token".into();
        owned.providers.chat.base_url = "https://the-servers-own-host.test/v1".into();
        owned.providers.chat.key = "the-servers-own-key-name".into();
        assert!(hub.apply_config(owned.clone()), "the server's configuration is written");

        let mut socket = connect(addr).await;
        wait_for_the_floor(&mut socket).await;
        send_command(&mut socket, Command::GetConfig).await;
        let view = match event_until(&mut socket, |e| matches!(e, Event::Config { .. })).await {
            Event::Config { config } => *config,
            other => panic!("`GetConfig` answers with the configuration, got {other:?}"),
        };
        assert_eq!(view.server.token, "the-servers-own-token", "an admitted client is told the token");

        // The save a screen would send: the same view, with the fields a client *may* move edited —
        // and the three it may not move are not in this shape to edit.
        let mut edited = ClientConfigView::from(&view);
        edited.listening.silence_ms = "4321".into();
        send_command(&mut socket, Command::UpdateConfig { config: Box::new(edited) }).await;
        event_until(&mut socket, |e| matches!(e, Event::ConfigChanged)).await;

        let on_disk = store::load_or_create(hub.path()).unwrap().config;
        assert_eq!(on_disk.listening.silence_ms, 4321, "the field a client may move did move");
        assert_eq!(on_disk.server.token, "the-servers-own-token", "the door password is the server's");
        assert_eq!(
            on_disk.providers.chat.base_url, "https://the-servers-own-host.test/v1",
            "the address the server sends its key to is the server's"
        );
        assert_eq!(on_disk.providers.chat.key, "the-servers-own-key-name", "and so is which key it spends");
    }

    /// **F4.** No config-writing command may run before the client was admitted. The bridge used to
    /// hand `UpdateConfig` straight to the hub, which is *before* the session's handshake — so a
    /// socket that never said `Hello` persisted a configuration change while its own session was
    /// refusing it. The route is the session's now, and its command loop is behind the prelude, so
    /// the write cannot happen: the socket is refused exactly as any other ungreeted first message is.
    #[tokio::test]
    async fn an_unadmitted_socket_cannot_write_the_configuration() {
        let (addr, hub) = server_with_policy(
            FakeChat::new(vec![]),
            ConnOptions::default(),
            SessionPolicy::exposed("the-token"),
        )
        .await;
        let before = std::fs::read(hub.path()).expect("the hub's file exists");

        // No `Hello`: the very first frame is the save an attacker would send.
        let mut socket = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.unwrap().0;
        let mut view = ClientConfigView::from(&crate::config::form::view_of(&hub.config()));
        view.server.bind = "0.0.0.0:7070".into();
        view.server.lan = true;
        send_command(&mut socket, Command::UpdateConfig { config: Box::new(view) }).await;

        // The session refuses the frame as a first message that is not a `Hello`…
        match event_until(&mut socket, |e| matches!(e, Event::Error { .. })).await {
            Event::Error { code, recoverable, .. } => {
                assert_eq!(code, ErrorCode::ProtocolVersion, "an ungreeted first message is not a handshake");
                assert!(!recoverable, "and the session ends rather than serving it");
            }
            other => panic!("the refusal is an error event, got {other:?}"),
        }
        // …and nothing was written: the hub still holds what it held, and so does the file.
        assert_eq!(hub.config().server.bind, "127.0.0.1:7070", "the hub must not have taken the write");
        assert_eq!(
            std::fs::read(hub.path()).expect("the hub's file is still there"),
            before,
            "an unadmitted socket's save must not reach config.toml"
        );
    }

    /// Review Focus line 3's third leak, and §9.4's whole rule **wired in** rather than merely
    /// correct: a `SetKey` from off this machine over plaintext never reaches the session and the
    /// refusal comes back on the session's own event channel, while the same command from loopback —
    /// or over TLS — is forwarded.
    ///
    /// The guard is driven at [`inbound`], which is the one place a socket message becomes a
    /// `ToServer`: that is what makes this a test of the wiring and not only of
    /// `tls::refuse_set_key`, whose own test lives beside it.
    #[tokio::test]
    async fn a_set_key_off_a_plaintext_lan_never_reaches_the_session() {
        // A fixture, not a key: the workspace rule is that keys appear by name and never in a file,
        // a log or a fixture, and this string only proves that the value never crosses.
        const FIXTURE: &str = "sk-or-v1-fixture0000000000000000000000000000";
        let lan: SocketAddr = "192.168.1.30:54123".parse().unwrap();
        let loopback: SocketAddr = "127.0.0.1:54123".parse().unwrap();
        let command = Command::SetKey { name: "openrouter".into(), value: FIXTURE.into() };
        let text = Message::Text(serde_json::to_string(&command).unwrap().into());

        // Off this machine, over plaintext: the session must never see it, and the screen must be
        // told why on the same channel every other server answer uses.
        let (to_server, mut reached) = mpsc::unbounded_channel();
        match inbound(text.clone(), &to_server, lan, false).await {
            Inbound::Send(ToClient::Event(Event::Error { code, message, recoverable })) => {
                assert_eq!(code, ErrorCode::InsecureTransport, "the machine meaning a client branches on");
                assert!(recoverable, "the same save works once the connection is TLS");
                assert!(!message.contains(FIXTURE), "the refusal must not repeat the key: {message}");
            }
            Inbound::Send(other) => panic!("a refusal is an error event, got {other:?}"),
            Inbound::Nothing | Inbound::Gone => panic!("a key must not cross a plaintext LAN"),
        }
        assert!(reached.try_recv().is_err(), "and the session must never see it");

        // The same command on loopback: forwarded, not refused — a client on this machine is this
        // machine, however it reached us.
        let (to_server, mut reached) = mpsc::unbounded_channel();
        assert!(matches!(inbound(text.clone(), &to_server, loopback, false).await, Inbound::Nothing));
        assert!(
            matches!(reached.try_recv(), Ok(ToServer::Command(Command::SetKey { .. }))),
            "a client on this machine may set a key"
        );

        // And over TLS, from anywhere: the other half of the same rule.
        let (to_server, mut reached) = mpsc::unbounded_channel();
        assert!(matches!(inbound(text, &to_server, lan, true).await, Inbound::Nothing));
        assert!(matches!(reached.try_recv(), Ok(ToServer::Command(Command::SetKey { .. }))));
    }

    /// The page's two data shapes become the bytes the PTY has to receive, and nothing else does.
    /// The `Mouse` half is the one worth pinning: a mouse report is a `0xFF`-prefixed byte sequence
    /// that is not UTF-8, so the page sends it as a latin-1 string and each char has to come back out
    /// as its low byte rather than as UTF-8 — sending it as text is "exactly how mouse reports get
    /// corrupted" (research note §6.4).
    #[test]
    fn the_terminal_clients_bytes_are_the_pages_own() {
        let data: TerminalClientMessage = serde_json::from_str(r#"{"type":"data","data":"ls -la\r"}"#).unwrap();
        assert_eq!(data.bytes().as_deref(), Some(b"ls -la\r".as_slice()), "`onData` is UTF-8 bytes");
        assert_eq!(data.resize(), None);

        // `\u{fffb}` and `\u{fffd}` are the two chars a mouse report produces, and both truncate to
        // `0xFB` and `0xFD` — not to a multi-byte UTF-8 encoding of the char code.
        let mouse: TerminalClientMessage = serde_json::from_str(r#"{"type":"mouse","data":"\u00ff\u00fb\u00fd"}"#).unwrap();
        assert_eq!(mouse.bytes().as_deref(), Some([0xffu8, 0xfb, 0xfd].as_slice()), "one byte per char code");
        assert_eq!(mouse.resize(), None);

        let resize: TerminalClientMessage = serde_json::from_str(r#"{"type":"resize","cols":100,"rows":24}"#).unwrap();
        assert_eq!(resize.bytes(), None, "a resize writes nothing to the PTY");
        assert_eq!(resize.resize(), Some((100, 24)));

        let hello: TerminalClientMessage = serde_json::from_str(r#"{"type":"hello"}"#).unwrap();
        assert_eq!(hello.bytes(), None);
        assert_eq!(hello.resize(), None);

        // Anything else is the page's bug, and the pump's answer to it is to ignore it rather than to
        // end a session somebody may be talking to.
        assert!(serde_json::from_str::<TerminalClientMessage>(r#"{"type":"nonsense"}"#).is_err());
        assert!(serde_json::from_str::<TerminalClientMessage>(r#"not json at all"#).is_err());
    }

    /// The debounce's own arithmetic: a size requested is not taken before its window ends, the last
    /// request inside the window is the one that survives, and a taken size leaves nothing pending.
    #[tokio::test]
    async fn the_resize_debounce_holds_a_size_until_its_window_ends() {
        let mut debounce = ResizeDebounce::new(TERMINAL_RESIZE_DEBOUNCE_MS);
        assert!(debounce.take().is_none(), "nothing was requested, so nothing is pending");

        debounce.request(100, 24);
        assert!(debounce.deadline.is_some(), "a request arms the window");
        // The second request inside the window is the one that counts, and it pushes the deadline on.
        let first_deadline = debounce.deadline.expect("armed");
        tokio::time::sleep(Duration::from_millis(TERMINAL_RESIZE_DEBOUNCE_MS / 2)).await;
        debounce.request(120, 30);
        assert!(debounce.deadline.expect("still armed") > first_deadline, "the window restarts");

        // Waiting the rest of the window out and then the pump's own `sleep_until` is what applies it.
        tokio::time::sleep(Duration::from_millis(TERMINAL_RESIZE_DEBOUNCE_MS)).await;
        assert_eq!(debounce.take(), Some((120, 30)), "only the last size inside the window survives");
        assert!(debounce.deadline.is_none(), "and taking it disarms the window");
        assert!(debounce.take().is_none(), "a taken size is not handed out twice");
    }
}
