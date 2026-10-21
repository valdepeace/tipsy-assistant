//! The client-side socket bridge: the `--connect` path's three channel ends.
//!
//! There is no `Transport` trait (W6). `client.rs` polls an `UnboundedReceiver<ToClient>` and sends
//! into an `UnboundedSender<ToServer>`; this module hands it exactly that pair plus the `events`
//! sender, made here, so nothing in the client's frame loop can tell a socket from `spawn_with_events`.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tipsy_protocol::{ErrorCode, Event, Frame, ToClient, ToServer};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

use crate::session::SessionEnds;

/// The first wait after a lost connection, and the ceiling the doubling stops at. The v1 spec §12
/// says "the client reconnects with backoff" and names no numbers; these are the plan's choice, and
/// `the_reconnect_backoff_doubles_and_then_stops_growing` is what pins them.
const BACKOFF_START: Duration = Duration::from_millis(100);
const BACKOFF_MAX: Duration = Duration::from_millis(3_200);

/// A URL as it may be printed: the scheme, the authority and the path, with the query and the
/// fragment taken off.
///
/// **It exists because a URL here can carry a secret, and the sinks that print one do not pass
/// through the log's scrubber.** `?token=…` is the spelling a browser has to use on the two pages
/// (spec §9.2), and this client used to need it on `/ws` as well — which is how the token reached the
/// `--verbose` banner, a `BridgeError`'s sentence and the configuration screen's notice line, none of
/// which is a `ScrubWriter`. Nothing a reader needs is in a query: the address, the port and the path
/// are all before it. Every value that reaches a user is built from this type, so "nothing may print a
/// URL that could carry a secret" is a property of the shape rather than a rule to remember.
#[derive(Clone, PartialEq, Eq)]
pub struct DisplayUrl(String);

impl DisplayUrl {
    /// The safe form of `url`: everything up to the first `?` or `#`.
    pub fn new(url: &str) -> Self {
        let cut = url.find(['?', '#']).unwrap_or(url.len());
        Self(url[..cut].to_string())
    }
}

impl std::fmt::Display for DisplayUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The same string as `Display`: a type whose whole purpose is to be safe to print must not grow a
/// second, unguarded spelling the day somebody writes `{url:?}`.
impl std::fmt::Debug for DisplayUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

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
    /// The URL in this sentence is a [`DisplayUrl`]: the address and the path, and never the query a
    /// token could have travelled in. This is the line a user copies into a bug report.
    #[error("cannot reach {url}: {reason}")]
    Unreachable { url: DisplayUrl, reason: String },
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
    let (mut socket, _response) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|error| BridgeError::Unreachable { url: DisplayUrl::new(url), reason: error.to_string() })?;
    let welcome = say_hello(&mut socket, token, url).await?;

    let (to_server_tx, to_server_rx) = mpsc::unbounded_channel();
    let (view_tx, view_rx) = mpsc::unbounded_channel();
    // The `Welcome` the handshake consumed is *handed on*, never swallowed: it is the only authority
    // on whether there is a login screen and which profile is speaking (spec §3), so a bridge that
    // kept it to itself would leave the HUD on its defaults for the whole run. Nothing has read the
    // channel yet, so this is still the first thing it will ever deliver.
    let _ = view_tx.send(ToClient::Event(welcome));
    tokio::spawn(pump(url.to_string(), token.map(str::to_string), socket, to_server_rx, view_tx.clone()));
    // A socket has no session id of its own: the server's own `Welcome.session_id` is what names this
    // connection, and this field is only ever read by the in-process path's own bookkeeping. Zero
    // means "the server has not said yet", which is exactly true here.
    Ok(SessionEnds { session_id: 0, to_server: to_server_tx, from_server: view_rx, events: view_tx })
}

/// Where a `--connect` socket lives while it is open.
type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Sends `Hello` and waits for the first answer. `Welcome` is the only acceptable one and is
/// returned rather than dropped; an `Error` is the server's refusal, quoted as it came.
///
/// The command itself comes from [`super::hello`], the one place it is built: the in-process client
/// sends the same one, and a second copy here is exactly where the two paths would drift apart.
async fn say_hello(socket: &mut Socket, token: Option<&str>, url: &str) -> Result<Event, BridgeError> {
    let text = serde_json::to_string(&super::hello(token.map(str::to_string))).expect("a `Command` always serialises");
    socket
        .send(Message::text(text))
        .await
        .map_err(|error| BridgeError::Unreachable { url: DisplayUrl::new(url), reason: error.to_string() })?;
    wait_for_welcome(socket, url).await
}

/// The one frame that says the server accepted this client.
async fn wait_for_welcome(socket: &mut Socket, url: &str) -> Result<Event, BridgeError> {
    let unreachable = |error: tokio_tungstenite::tungstenite::Error| BridgeError::Unreachable {
        url: DisplayUrl::new(url),
        reason: error.to_string(),
    };
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<Event>(text.as_str()) {
                Ok(welcome @ Event::Welcome { .. }) => return Ok(welcome),
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
    // The address as every line in this task may print it: the real URL is kept for reconnecting and
    // for nothing else (see [`DisplayUrl`]).
    let shown = DisplayUrl::new(&url);
    loop {
        if matches!(shuttling(&mut socket, &mut to_server, &view).await, Ending::ClientGone) {
            return;
        }
        // The socket is gone. One line to the HUD, a wait, and `Hello` again.
        let _ = view.send(ToClient::Event(Event::Error {
            code: ErrorCode::Network,
            message: format!("{shown}: connection lost; reconnecting"),
            recoverable: true,
        }));
        tokio::time::sleep(backoff(attempt)).await;
        attempt = attempt.saturating_add(1);
        if let Some((fresh, welcome)) = reconnect(&url, token.as_deref()).await {
            socket = fresh;
            attempt = 0;
            // The new session announces itself to the HUD exactly as the first one did. It is sent
            // before `shuttling` resumes, so it stays ahead of everything else this connection says.
            let _ = view.send(ToClient::Event(welcome));
        }
        // While it is detached, `shuttling`'s `to_server.recv()` is not being polled, so anything
        // the HUD sends waits in the channel and is drained and dropped by the next `shuttling` call:
        // the peer that comes back is a new session (W9), and a `Mic` frame or an `Ask` for the one
        // that is gone is not something to replay. The `Event::Error` above is what tells the user.
    }
}

/// One connection's lifetime: `select!` between the socket and the client's commands, until either
/// side goes.
///
/// The wire is the one `server::conn`'s `encode` writes and its `inbound` reads: a **bare** `Event`
/// as a JSON text frame and a bare `Frame::encode` as a binary one. There is deliberately no
/// `ToClient` on the wire — `ToClient` is a channel payload, not a protocol message, and it carries
/// no serde implementation (the plan's draft of this module called `from_str::<ToClient>`, which does
/// not compile) — so each frame is decoded as the type it actually is.
async fn shuttling(
    socket: &mut Socket,
    to_server: &mut UnboundedReceiver<ToServer>,
    view: &UnboundedSender<ToClient>,
) -> Ending {
    loop {
        tokio::select! {
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(event) = serde_json::from_str::<Event>(text.as_str())
                        && view.send(ToClient::Event(event)).is_err()
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
                // Server-internal, and unreachable from here: the hub's fan-out is on the other
                // machine, and a client's own save is `Command::UpdateConfig` on the wire. Nothing is
                // sent for it — there is no wire message this could mean.
                Some(ToServer::Reconfigure) => {}
            },
        }
    }
}

/// One attempt at getting back: the fresh socket and the `Welcome` its own handshake answered with.
/// `None` means "keep trying": the peer is not there yet.
async fn reconnect(url: &str, token: Option<&str>) -> Option<(Socket, Event)> {
    let (mut socket, _response) = tokio_tungstenite::connect_async(url).await.ok()?;
    let welcome = say_hello(&mut socket, token, url).await.ok()?;
    Some((socket, welcome))
}

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

    /// F1's shape rule: what this program may print is a URL without its query and without its
    /// fragment, in both of the ways a value can be formatted.
    #[test]
    fn a_displayed_url_carries_no_query_and_no_fragment() {
        assert_eq!(DisplayUrl::new("ws://192.168.1.5:7070/ws?token=abc").to_string(), "ws://192.168.1.5:7070/ws");
        assert_eq!(DisplayUrl::new("ws://192.168.1.5:7070/ws?a=1&b=2").to_string(), "ws://192.168.1.5:7070/ws");
        assert_eq!(DisplayUrl::new("ws://host/ws#secret").to_string(), "ws://host/ws");
        assert_eq!(DisplayUrl::new("ws://host/ws?").to_string(), "ws://host/ws");
        assert_eq!(
            DisplayUrl::new("ws://host:7070/ws").to_string(),
            "ws://host:7070/ws",
            "an address and a path survive"
        );
        assert_eq!(
            format!("{:?}", DisplayUrl::new("ws://host/ws?token=abc")),
            "ws://host/ws",
            "`Debug` is the same string: a type that exists to be safe to print may not have a second spelling"
        );
    }

    /// F1 on the path a user actually hits: the address in shell history carries the token, nothing
    /// listens on it, and the sentence they paste into a bug report is what must not carry it.
    ///
    /// A real connection to a port that was just released, so the failure is the transport's and
    /// happens before any handshake — the refused-attach path rather than a formatted value.
    #[tokio::test]
    async fn an_unreachable_url_never_carries_a_token_into_its_message() {
        const SECRET: &str = "9f8e7d6c5b4a39281706f5e4d3c2b1a0";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let url = format!("ws://{addr}/ws?token={SECRET}");
        let error = match connect(&url, Some(SECRET)).await {
            Err(error) => error,
            Ok(_) => panic!("nothing listens on {addr} any more"),
        };
        assert!(matches!(error, BridgeError::Unreachable { .. }), "{error:?}");
        let text = error.to_string();
        assert!(!text.contains(SECRET), "the token reached the line a user copies: {text}");
        assert!(text.contains(&format!("ws://{addr}/ws")), "and the address they need is still there: {text}");
        assert!(!text.contains('?'), "the query is what carried it, and it is gone: {text}");
    }
}
