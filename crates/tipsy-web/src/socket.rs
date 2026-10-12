//! The browser's socket, and the three ends the browser's equivalent of `SessionEnds` has.
//!
//! The native client's `SessionEnds` (`crates/tipsy-compadre/src/session.rs:46-52`) is
//! `{ to_server, from_server, events }` over `tokio::sync::mpsc`. There is no trait here either
//! (spec W6, §3.3): [`Outbox`] is that struct's `to_server`, the queue `spawn_connection` is handed
//! is its `from_server` and its `events`, and the payloads are the same `ToServer`/`ToClient`, so the
//! server needs no change for a web client.
//!
//! **The pure half is at the top and the `web-sys` glue at the bottom, and the split is deliberate.**
//! There is no browser on the machine that runs this suite, so everything that can be a decision
//! rather than a DOM call — the reconnect schedule and when each of the two notices is due
//! ([`Connection`]), the socket URL and the token out of `location` ([`socket_url`],
//! [`token_from_query`]) — is a plain function over plain values with a test under it. What is left
//! for `web-sys` is opening a socket, installing four handlers and resolving one promise, none of
//! which has a branch a host test could reach.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use tipsy_protocol::{ClientKind, Command, ToClient, ToServer, PROTOCOL_VERSION};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::wire::{self, WireError};

/// Reconnect delays, in milliseconds, after each close; the last value is used for every attempt
/// after it. The v1 spec §12 promises "the client reconnects with backoff" without a schedule, so
/// this is the plan's choice, and 500 ms doubling to 16 s is small enough for a server restart to be
/// invisible and slow enough not to hammer a server that is down.
pub const RECONNECT_MS: [u32; 6] = [500, 1_000, 2_000, 4_000, 8_000, 16_000];

/// How long to wait before the connection attempt numbered `attempt` (0 being the one right after
/// the first close). The last entry of [`RECONNECT_MS`] is the ceiling, so a long outage cannot
/// overflow the schedule and cannot grow it without bound.
pub fn reconnect_delay(attempt: usize) -> u32 {
    RECONNECT_MS[attempt.min(RECONNECT_MS.len() - 1)]
}

/// What this client has to remember between two connections: how many attempts have failed since the
/// last `Welcome`, and whether the user is currently being told the connection is down.
///
/// **One type owns both notices, and neither edge is the other's business.** A socket that closes
/// without ever having opened is just as offline as one that opened and dropped, so both go through
/// [`Connection::closed`]; the only thing that clears the state is a `Welcome`, which is the one
/// message that proves a *session* and not merely a TCP connection ([`Connection::welcomed`]).
///
/// **The backoff does not reset when a socket opens, only when a `Welcome` arrives.** A server that
/// accepts the upgrade and then refuses the handshake — a protocol mismatch, a loopback listener with
/// a token and a `Hello` that does not carry it — would otherwise be retried every 500 ms for ever,
/// which is the one shape of this loop that can hammer a server that is actually up.
#[derive(Debug, Default)]
pub struct Connection {
    /// Failed attempts since the last `Welcome`: the index into [`RECONNECT_MS`].
    attempt: usize,
    /// Whether the user has been told the connection is down and has not been told it is back.
    offline: bool,
}

impl Connection {
    pub fn new() -> Self {
        Self::default()
    }

    /// The attempt this connection is on, counted from zero. Exposed for the tests and for a caller
    /// that wants to say how many times it has tried.
    pub fn attempt(&self) -> usize {
        self.attempt
    }

    /// A connection ended — a close, or a URL the browser refused to open at all. Answers the delay
    /// before the next attempt and whether the user has to be told: `true` exactly once per outage,
    /// however many attempts that outage takes.
    pub fn closed(&mut self) -> (u32, bool) {
        let delay = reconnect_delay(self.attempt);
        self.attempt += 1;
        let say = !self.offline;
        self.offline = true;
        (delay, say)
    }

    /// A `Welcome` arrived, which is the one message that means a session is really up. Answers
    /// whether the user has to be told the connection is back: `true` only if it had been told it
    /// was gone.
    pub fn welcomed(&mut self) -> bool {
        self.attempt = 0;
        std::mem::take(&mut self.offline)
    }
}

/// The socket URL, from the scheme and the authority the page was served under: `http`→`ws`,
/// `https`→`wss`, same host and port, path `/ws` (spec §5.3).
///
/// **The host is passed through untouched, and that is a security-shaped rule rather than a
/// convenience.** `/ws` refuses an upgrade whose `Origin` is not the host the request arrived on
/// (`crates/tipsy-compadre/src/server/mod.rs`'s `origin_is_this_host`), and a browser sets `Origin`
/// from the document's own authority, so a page served from `http://localhost:7070` must open
/// `ws://localhost:7070/ws` and not a rewritten `127.0.0.1` — the two spellings are different
/// authorities by design, with a test each on the server side.
pub fn socket_url(scheme: &str, host: &str) -> String {
    let scheme = if scheme == "https:" { "wss" } else { "ws" };
    format!("{scheme}://{host}/ws")
}

/// The token out of a location's query string (`?token=…`), which is the one shape a browser can send
/// on a navigation (spec §9.2).
///
/// **An empty or blank value is no token at all**, and that is the honest reading rather than a
/// convenience: `SessionPolicy::admits` compares `Some("")` against the configured token and refuses
/// it as `token_invalid`, so a user who opened `/?token=` would be told their token is wrong when
/// what happened is that they presented none. `None` gets them `token_required`, which is true.
///
/// **Nothing is percent-decoded**, because nothing has to be: `store::new_token` emits 32 lowercase
/// hex characters, and a value that needed escaping is not one of this project's tokens.
pub fn token_from_query(search: &str) -> Option<String> {
    let query = search.strip_prefix('?').unwrap_or(search);
    // A fragment is not part of the query string. `location.search` never carries one, but the test
    // below feeds whole strings, and a `#` that reached the token would make it a wrong token rather
    // than a missing one.
    let query = query.split('#').next().unwrap_or(query);
    query.split('&').find_map(|pair| {
        let value = pair.strip_prefix("token=")?;
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// The socket URL for the page this is running in. `web_sys::Location` is a DOM call, so the mapping
/// itself is [`socket_url`]'s and this is the two lines that read the browser.
pub fn url_from_location() -> Result<String, JsValue> {
    let location = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?.location();
    Ok(socket_url(&location.protocol().unwrap_or_default(), &location.host()?))
}

/// The token the page was opened with (`?token=…`). The server puts it there because a browser cannot
/// set a header on a navigation (spec §9.2), and this client sends it back in `Hello` — never in a
/// log line (spec §9.6), never in the socket's own URL.
pub fn token_from_location() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    token_from_query(&search)
}

/// The browser's `UnboundedSender<ToServer>`, and the whole of the live connection with it.
///
/// `send` takes the same two payloads the native client passes and puts them on the wire as a text or
/// a binary frame (spec §3.3). **It is `Clone`, and a clone is a second handle to the same live socket
/// rather than a copy of it** — the socket behind it is swapped by [`spawn_connection`] on every
/// reconnect, so a handle taken once at startup (the microphone's, the HUD's) keeps working across a
/// server restart. That is also why the reconnect state lives here and not in the loop's local
/// variables: the two facts a clone has to follow are the same two facts.
#[derive(Clone)]
pub struct Outbox {
    /// The live socket; `None` before the first attempt and while a connection is down.
    socket: Rc<RefCell<Option<web_sys::WebSocket>>>,
    /// The schedule and the two notices.
    connection: Rc<RefCell<Connection>>,
    /// The user asked to quit. Set by [`Outbox::close`] and read by the reconnect loop, which is the
    /// only thing that can end the run with a socket still trying to come back.
    quit: Rc<std::cell::Cell<bool>>,
}

impl Outbox {
    fn new() -> Self {
        Self {
            socket: Rc::new(RefCell::new(None)),
            connection: Rc::new(RefCell::new(Connection::new())),
            quit: Rc::new(std::cell::Cell::new(false)),
        }
    }

    /// A handle with **no socket behind it**, for a test that needs an `Outbox` without a browser.
    ///
    /// [`spawn_connection`] is the only production constructor and it opens a real `WebSocket` through
    /// `spawn_local` — neither of which exists on the host — so this seam is what lets
    /// [`crate::dispatch`]'s `UiAction::LoginDone` arm (Task 3.7's) be tested at all. It is
    /// `#[cfg(test)]` and it is the same shape `close`/`is_quitting` already have here: a small,
    /// explicit door for the tests rather than a public constructor.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self::new()
    }

    /// Closes the socket for good: the user quit, so nothing reconnects and nothing may be sent again.
    ///
    /// **This is Task 3.5's one addition to this file, and it exists because the socket's cell is
    /// private.** Task 3.5's `Shared` wants `UiAction::Quit` to close the connection, and the plan's
    /// own sketch gave that struct a `web_sys::WebSocket` field to do it with — a field no caller can
    /// build, because `spawn_connection` keeps the socket in an `Outbox` and a clone shares it across
    /// a reconnect. Closing through the handle that owns the cell is the only version of that which
    /// compiles.
    ///
    /// The flag matters more than the `close()`: without it the loop below would see the close as one
    /// more outage, say `web_offline` and dial again on the backoff schedule, for ever. A socket closed
    /// while still `CONNECTING` may never fire `close` at all (MDN: the connection is failed rather
    /// than closed), and that is harmless here — the quit flag is already set, so the attempt that
    /// resolves next still ends the loop.
    pub fn close(&self) {
        self.quit.set(true);
        if let Some(ws) = self.socket.borrow().as_ref() {
            // The result is not acted on: a socket that is already closing or closed refuses this,
            // and the flag above is what makes the outcome the same either way.
            let _ = ws.close();
        }
    }

    /// Whether the user asked to quit. Exposed for the render loop, which draws nothing more once it
    /// is true, and for the tests.
    pub fn is_quitting(&self) -> bool {
        self.quit.get()
    }

    /// Whether a socket is open right now. `false` while one is still connecting, which is why `send`
    /// asks this rather than letting the browser throw.
    pub fn is_open(&self) -> bool {
        self.socket.borrow().as_ref().is_some_and(|ws| ws.ready_state() == web_sys::WebSocket::OPEN)
    }

    /// Forwards [`Connection::welcomed`]: the render loop reports every `Welcome` it applies, and the
    /// `true` answer is when the user has to be told the connection is back. It is called from there
    /// and not from the socket's own message handler because `UiState::apply` is what sets the
    /// interface language, and the notice has to be in the language the profile selected.
    pub fn welcomed(&self) -> bool {
        self.connection.borrow_mut().welcomed()
    }

    /// How many attempts this connection is on, since the last `Welcome`.
    pub fn attempt(&self) -> usize {
        self.connection.borrow().attempt()
    }

    pub fn send(&self, message: ToServer) -> Result<(), WireError> {
        match message {
            ToServer::Command(command) => {
                let text = wire::command_to_text(&command)?;
                self.with_open_socket(|ws| ws.send_with_str(&text))
            }
            ToServer::Audio(frame) => {
                let bytes = wire::frame_to_bytes(&frame);
                self.with_open_socket(|ws| ws.send_with_u8_array(&bytes))
            }
            // The third variant is the hub talking to its own sessions and never crosses a socket
            // (`tipsy-protocol/src/lib.rs:520-531`). Refusing it by name is what keeps the browser's
            // `ToServer` from looking like a channel anything may push on.
            ToServer::Reconfigure => Err(WireError::ServerOnly),
        }
    }

    /// Runs `send` on the live socket, or answers [`WireError::NotOpen`] without calling the browser
    /// at all.
    ///
    /// **This check is the `send` half of the two MDN facts the research note records**: `send()`
    /// while `CONNECTING` throws `InvalidStateError`, and anything sent while `CLOSING`/`CLOSED` is
    /// silently discarded (`ratzilla-and-wasm-audio.md` §6.1). Asking first turns both of those into
    /// one honest error a caller can act on.
    fn with_open_socket(&self, send: impl FnOnce(&web_sys::WebSocket) -> Result<(), JsValue>) -> Result<(), WireError> {
        let socket = self.socket.borrow();
        let Some(ws) = socket.as_ref().filter(|ws| ws.ready_state() == web_sys::WebSocket::OPEN) else {
            return Err(WireError::NotOpen);
        };
        send(ws).map_err(|error| WireError::Message(error_text(&error)))
    }
}

/// A JS exception's readable text, for a [`WireError`]. `Reflect::get` is used rather than
/// `js_sys::Error`'s accessors because the thrown value is not always an `Error`.
pub fn error_text(value: &JsValue) -> String {
    js_sys::Reflect::get(value, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .or_else(|| value.as_string())
        .unwrap_or_else(|| format!("{value:?}"))
}

/// Opens the socket and keeps it open: one connection at a time, `Hello` first, and another attempt
/// after [`RECONNECT_MS`] whenever one ends.
///
/// `state` is where the one notice this side owns goes — [`Connection::closed`]'s `web_offline` — and
/// the answer is the sender every other part of the client shares. Nothing here blocks: each attempt
/// is a `Promise` the close handler resolves, awaited with `JsFuture`, so the browser's own event loop
/// drives the whole thing and no async runtime is needed (`ratzilla-and-wasm-audio.md` §6.4 point 5).
///
/// **The return type is this task's one deviation from the plan's own `Produces` line**, which wrote
/// `spawn_connection(…)` with no return at all. A caller has to be able to send — Task 3.3's
/// microphone and Task 3.5's keys both do — and the plan's Step 6 sketch has no other way to reach the
/// socket, which is why its own Task 3.5 then invents a `to_server` sender that nothing builds.
pub fn spawn_connection(
    url: String,
    token: Option<String>,
    queue: Rc<RefCell<VecDeque<ToClient>>>,
    state: Rc<RefCell<tipsy_ui::UiState>>,
) -> Outbox {
    let outbox = Outbox::new();
    let wiring = outbox.clone();
    spawn_local(async move {
        loop {
            let closed = Closed::new();
            match connect(&url, token.as_deref(), &wiring, &queue, &closed) {
                // The whole life of one connection: the close handler is what ends this await, and an
                // error before the upgrade ends it the same way, because the browser fires `error` and
                // then `close` for a connection it could not make.
                Ok(()) => {
                    let _ = wasm_bindgen_futures::JsFuture::from(closed.promise.clone()).await;
                }
                // `WebSocket::new` refused the URL outright — a malformed one, or a scheme this page
                // may not open. There is no socket to wait on, so the delay below is the whole of this
                // attempt.
                Err(error) => {
                    web_sys::console::warn_1(&JsValue::from_str(&format!("tipsy-web: {}", error_text(&error))));
                }
            }
            // The user quit: the run is over, and this is what stops the loop from dialling again on
            // the backoff schedule. It is asked *before* `closed()`, so a deliberate quit is not
            // counted as one more failed attempt and says nothing — a `web_offline` notice over a
            // quit the user asked for would be a lie.
            if wiring.quit.get() {
                return;
            }
            let (delay, say_offline) = wiring.connection.borrow_mut().closed();
            if say_offline {
                // The language is the one the last `Welcome` set. Before any `Welcome` there is no
                // profile to take a language from, and `UiState::new`'s default — Spanish — is the
                // honest answer rather than a guess.
                let text = state.borrow().strings().web_offline;
                state.borrow_mut().notify(text);
            }
            sleep(delay).await;
        }
    });
    outbox
}

/// One connection's close signal: the `Promise` the loop awaits and the `resolve` the socket's own
/// `close` handler calls. This is the plan's shape (a) — the `Rc<RefCell<Option<Function>>>` — with no
/// `futures` crate and nothing polled.
struct Closed {
    promise: js_sys::Promise,
    resolve: Rc<RefCell<Option<js_sys::Function>>>,
}

impl Closed {
    fn new() -> Self {
        let resolve: Rc<RefCell<Option<js_sys::Function>>> = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&resolve);
        // `Promise::new`'s executor runs before the constructor returns, so the resolver is in the
        // cell by the time this returns. It is taken out again when it is called, so a socket that
        // somehow closed twice cannot resolve a promise that has already been resolved once.
        let promise = js_sys::Promise::new(&mut move |yes, _no| *slot.borrow_mut() = Some(yes));
        Self { promise, resolve }
    }
}

/// Opens one socket into `outbox` and installs the four handlers. `Ok` means the browser accepted the
/// URL and the socket is connecting: **the outcome arrives through the handlers**, never here, which
/// is what [`Outbox::is_open`] is for.
fn connect(
    url: &str,
    token: Option<&str>,
    outbox: &Outbox,
    queue: &Rc<RefCell<VecDeque<ToClient>>>,
    closed: &Closed,
) -> Result<(), JsValue> {
    let ws = web_sys::WebSocket::new(url)?;
    // Required for the binary branch: the documented default is "blob" (MDN, §6.1).
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
    *outbox.socket.borrow_mut() = Some(ws.clone());

    // `Hello` is the first message (spec §4.1) and it goes out on `open`, because `send()` before that
    // throws. It is built here, once per connection, so a reconnect says hello again exactly as the
    // first attempt did.
    let hello = ToServer::Command(Command::Hello {
        token: token.map(str::to_string),
        client: ClientKind::Web,
        protocol_version: PROTOCOL_VERSION,
    });
    ws.set_onopen(Some(
        Closure::once_into_js({
            let outbox = outbox.clone();
            move || {
                if let Err(error) = outbox.send(hello) {
                    web_sys::console::warn_1(&JsValue::from_str(&format!("tipsy-web: {error}")));
                }
            }
        })
        .unchecked_ref(),
    ));

    // The inbound side is a queue the render loop drains once per frame, never a callback that draws:
    // a browser `WebSocket` has no backpressure at all (`web-transport-and-terminal.md` §7.1), so a
    // draw inside this handler would let a fast server decide the frame rate.
    //
    // **Leaked deliberately.** `Closure::into_js_value` hands ownership to JS because the socket
    // outlives this call; a `Closure` kept in a local would be dropped at the end of `connect` and the
    // handler would never fire again.
    ws.set_onmessage(Some(
        Closure::wrap(Box::new({
            let queue = Rc::clone(queue);
            move |event: web_sys::MessageEvent| {
                let data = event.data();
                let message = if let Some(buffer) = data.dyn_ref::<js_sys::ArrayBuffer>() {
                    // Binary: the only binary traffic is audio (spec §4.5).
                    wire::frame_from_bytes(&js_sys::Uint8Array::new(buffer).to_vec()).map(ToClient::Audio)
                } else if let Some(text) = data.as_string() {
                    wire::event_from_text(&text).map(ToClient::Event)
                } else {
                    Err(WireError::Message("neither a string nor an ArrayBuffer".into()))
                };
                match message {
                    Ok(message) => queue.borrow_mut().push_back(message),
                    // A malformed message NEVER kills the render loop. An unknown event type is
                    // silent, by design: a newer server may send more than this build understands, and
                    // that must not break a running HUD (v1 spec §12; Plan 2C Task 3's "an unknown
                    // `type` is data, not death").
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

    // The close signal, once per connection: it ends the loop's await above, which is what makes the
    // next attempt happen at all.
    ws.set_onclose(Some(
        Closure::once_into_js({
            let resolve = Rc::clone(&closed.resolve);
            move || {
                if let Some(resolve) = resolve.borrow_mut().take() {
                    let _ = resolve.call0(&JsValue::UNDEFINED);
                }
            }
        })
        .unchecked_ref(),
    ));

    // Reported and nothing else: `close` always follows an `error` on a failed connection, so
    // resolving here as well would end one attempt twice and start two reconnect loops.
    ws.set_onerror(Some(
        Closure::wrap(Box::new(move |event: web_sys::Event| {
            let text = error_text(&event.into());
            web_sys::console::warn_1(&JsValue::from_str(&format!("tipsy-web: the socket reported an error: {text}")));
        }) as Box<dyn FnMut(web_sys::Event)>)
        .into_js_value()
        .unchecked_ref(),
    ));

    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The schedule, read end to end: half a second, doubling, and 16 s for every attempt after. The
    /// last rows are the ceiling, and the row after the last entry is what "the last value is used for
    /// every attempt after it" means.
    #[test]
    fn the_reconnect_schedule_doubles_from_half_a_second_and_then_stops_growing() {
        let delays: Vec<u32> = (0..9).map(reconnect_delay).collect();
        assert_eq!(delays, vec![500, 1_000, 2_000, 4_000, 8_000, 16_000, 16_000, 16_000, 16_000]);
    }

    /// What an outage says, in order: one notice, growing delays, and then one "back".
    #[test]
    fn an_outage_is_announced_once_and_the_backoff_grows_until_a_welcome() {
        let mut connection = Connection::new();
        assert_eq!(connection.closed(), (500, true), "the first close is what the user has to be told about");
        assert_eq!(connection.closed(), (1_000, false), "and every attempt after it is silent");
        assert_eq!(connection.closed(), (2_000, false));
        assert_eq!(connection.attempt(), 3);

        assert!(connection.welcomed(), "the `Welcome` is what says the connection is back");
        assert_eq!(connection.attempt(), 0, "and it is the only thing that resets the schedule");
        assert!(!connection.welcomed(), "a second `Welcome` on a live connection says nothing");
        assert_eq!(connection.closed(), (500, true), "the next outage starts the schedule over");
    }

    /// A `Welcome` on a connection that never went away is not news.
    #[test]
    fn the_first_welcome_of_a_run_says_nothing() {
        let mut connection = Connection::new();
        assert!(!connection.welcomed());
    }

    /// Twenty attempts into one outage: the delay is at its ceiling and the user has still been told
    /// exactly once. Nothing here resets anything on its own — a socket that *opens* and is then
    /// refused is one more `closed()` and no `welcomed()`, which is why a server that accepts the
    /// upgrade and then refuses the handshake cannot be hammered at 500 ms for ever.
    #[test]
    fn a_long_outage_says_offline_once_and_the_schedule_saturates() {
        let mut connection = Connection::new();
        let mut announcements = 0;
        for _ in 0..20 {
            let (delay, say) = connection.closed();
            announcements += usize::from(say);
            assert!(delay <= 16_000, "the schedule has a ceiling: {delay}");
        }
        assert_eq!(announcements, 1, "twenty failed attempts are one outage");
        assert_eq!(connection.attempt(), 20);
        assert_eq!(connection.closed().0, 16_000, "and the delay is at the ceiling by then");
    }

    /// The host the page was served from is the host the socket goes to. The two spellings are
    /// different authorities on purpose (`origin_is_this_host`), so neither is rewritten into the
    /// other: rewriting one would make the browser send an `Origin` the server refuses.
    #[test]
    fn the_socket_is_opened_under_the_host_the_page_was_served_from() {
        assert_eq!(socket_url("http:", "127.0.0.1:7070"), "ws://127.0.0.1:7070/ws");
        assert_eq!(socket_url("https:", "127.0.0.1:7070"), "wss://127.0.0.1:7070/ws");
        assert_eq!(socket_url("http:", "localhost:7070"), "ws://localhost:7070/ws");
        assert_ne!(socket_url("http:", "localhost:7070"), socket_url("http:", "127.0.0.1:7070"));
    }

    #[test]
    fn the_token_comes_from_the_query_and_nowhere_else() {
        assert_eq!(token_from_query("?token=abc").as_deref(), Some("abc"));
        assert_eq!(token_from_query("token=abc").as_deref(), Some("abc"), "a bare query string is still a query");
        assert_eq!(token_from_query("?x=1&token=abc").as_deref(), Some("abc"));
        assert_eq!(token_from_query("?token=abc#frag").as_deref(), Some("abc"), "the fragment is not the token");
        assert_eq!(token_from_query("?token=abc&x=1").as_deref(), Some("abc"));
        assert_eq!(token_from_query("?x=1"), None);
        assert_eq!(token_from_query(""), None);
        assert_eq!(token_from_query("?tokenother=abc"), None, "a parameter that merely starts the same way");
        // An empty value is no credential at all: `SessionPolicy::admits` would call `Some("")` a
        // *wrong* token, and "you presented none" is what actually happened.
        assert_eq!(token_from_query("?token="), None);
        assert_eq!(token_from_query("?token=   "), None, "a blank value is no value");
        // Nothing is percent-decoded, so a percent-escaped value stays as it was typed and the session
        // refuses it as a wrong token. `store::new_token` is 32 lowercase hex characters, so no token
        // this project can generate needs escaping.
        assert_eq!(token_from_query("?token=%20").as_deref(), Some("%20"));
    }

    /// The three refusals a socket that is not open produces — **and none of them touches the
    /// browser**. `Outbox::new` leaves the cell empty and [`Outbox::with_open_socket`] answers before
    /// it dereferences anything, so this is the `send` half of the two MDN facts the plan records
    /// (`send()` while `CONNECTING` throws `InvalidStateError`; anything sent while `CLOSING`/`CLOSED`
    /// is silently discarded) made checkable on a machine with no browser at all.
    #[test]
    fn an_outbox_that_is_not_open_refuses_without_calling_the_browser() {
        let outbox = Outbox::new();
        assert!(!outbox.is_open(), "a client that has not connected has no open socket");
        assert_eq!(
            outbox.send(ToServer::Command(Command::Hello {
                token: None,
                client: ClientKind::Web,
                protocol_version: PROTOCOL_VERSION,
            })),
            Err(WireError::NotOpen)
        );
        assert_eq!(
            outbox.send(ToServer::Audio(tipsy_protocol::Frame::Mic(vec![1, 2]))),
            Err(WireError::NotOpen)
        );
        assert_eq!(
            outbox.send(ToServer::Reconfigure),
            Err(WireError::ServerOnly),
            "the hub's own message is refused by name, which is not the same as a closed socket"
        );
    }

    /// **The quit is a flag before it is a `close()`**, and that half is testable here: a clone sees
    /// the flag its sibling set, because it is the same `Cell` through the same `Rc` — which is also
    /// what makes a `Shared` held across the whole run able to end the socket the loop owns. The
    /// `close()` call itself needs a socket, so it is the browser half; the flag is the half that
    /// decides whether the loop dials again, and it is this test's.
    #[test]
    fn quitting_is_one_flag_that_every_clone_of_the_outbox_sees() {
        let outbox = Outbox::new();
        let handle = outbox.clone();
        assert!(!outbox.is_quitting(), "a fresh client has not quit");
        assert!(!handle.is_quitting(), "and neither has a clone of it");
        handle.close();
        assert!(outbox.is_quitting() && handle.is_quitting(), "one quit, seen by every handle to the socket");
    }
}
