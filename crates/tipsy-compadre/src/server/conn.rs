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
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use futures::stream::StreamExt;
use tipsy_protocol::{ClientKind, Command, ErrorCode, Event, Frame, ToClient, ToServer};
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
}
