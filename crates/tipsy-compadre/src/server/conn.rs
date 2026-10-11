//! The per-connection bridge — what turns a socket into a session and back again.
//!
//! **This file is a placeholder whose subject belongs to Task 1.6.** Task 1.4 registers `/ws` and
//! upgrades it, which means the upgrade callback needs a callee; the alternatives were a `todo!()`
//! (which would make a lint gate finding out of a panic) or leaving the socket unhandled in the
//! handler (which would put Task 1.6's subject inside `server/mod.rs`). So the socket is handed to
//! this function, which today ends the connection.
//!
//! Task 1.6 replaces the body: a `Hello`-first handshake, the frame loop in both directions, a
//! bounded write path, and the `Hub::detach` on every way out. The signature is the shape the
//! shared contract pins for that call — `serve(socket, session_ends, hub)` — so the router does not
//! change when the body does. The `ClientKind` the upgrade handler knows is deliberately **not** a
//! parameter: `/terminal/ws` (Task 2.2) is a different route with a different socket, and the HUD
//! protocol's own front door is `Command::Hello`, which the session already judges.

use axum::extract::ws::WebSocket;
use tipsy_protocol::ClientKind;

use crate::session::SessionEnds;
use crate::server::hub::Hub;

/// What a socket is opened with, and what the bridge is allowed to queue on one.
///
/// The two numbers are a pair on purpose: axum's `write_buffer_size` is where the buffer *starts*
/// and `max_write_buffer_size` is where it stops being allowed to grow, and its own doc says the
/// ceiling "should always be at least `write_buffer_size + 1 message`". The ceiling is said here
/// rather than left at axum's default of **unlimited**, because a browser tab that stops draining
/// must not become the server's memory bill (research note
/// `web-transport-and-terminal.md:1339-1346`, Review Focus 2).
pub struct ConnOptions {
    /// axum's own default, named so a reader does not have to look it up.
    pub write_buffer_size: usize,
    /// Four times that: one large frame plus headroom fits, and a stalled peer is cut off rather
    /// than queued for ever.
    pub max_write_buffer_size: usize,
}

impl Default for ConnOptions {
    fn default() -> Self {
        Self { write_buffer_size: 128 * 1024, max_write_buffer_size: 512 * 1024 }
    }
}

/// Serves one connection until it ends.
///
/// **Today: immediately** — this body is Task 1.6's subject and is a placeholder, which is why it
/// is worth saying exactly what it does. The session is detached and the socket dropped, so a client
/// that connects gets a socket that closes at once. That is the honest state of this commit: the
/// route exists, the upgrade happens, the buffer ceiling is set, and what to *do* with the socket is
/// the next task's. Nothing here panics, and nothing here is reachable from this task's tests.
pub(crate) async fn serve(socket: WebSocket, ends: SessionEnds, hub: Hub, _client: ClientKind) {
    hub.detach(ends.session_id);
    // Both receivers and the socket go out of scope here: the session task sees its channel close
    // and ends, which is the same ending a finished connection has, reached early.
    drop((socket, ends.to_server, ends.from_server, ends.events));
}
