//! `--connect`: the client-side bridge, against a real socket server on `127.0.0.1:0`.
//!
//! The socket side is the technique `tests/realtime.rs` established — a `TcpListener` plus
//! `accept_hdr_async` — because `wiremock` speaks HTTP and this is a duplex WebSocket. The server
//! half of the protocol is Tasks 1.4 and 1.6; what is under test here is the client.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tipsy_assistant::client::bridge;
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
/// `tests/realtime.rs` already records.
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

    /// The polling idiom `tests/realtime.rs` uses: no fixed sleeps, no port races.
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

    assert_eq!(server.target(0).await, "/ws", "the token travels in `Hello`, never as a query parameter (spec §9.2)");
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
    // The handshake's own `Welcome` comes first: `connect` checks it and then hands it on rather
    // than swallowing it, because it is the only authority on whether the login screen is up.
    assert!(
        matches!(next(&mut ends.from_server).await, ToClient::Event(Event::Welcome { .. })),
        "the first connection's `Welcome` still reaches the HUD"
    );

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

    // `SessionEnds` is deliberately not `Debug` (its receiver is not), so this is a match rather
    // than `expect_err`.
    let error = match bridge::connect(&server.url(), Some("wrong")).await {
        Err(error) => error,
        Ok(_) => panic!("a refusal is fatal, and a `Welcome` never arrived"),
    };
    assert!(matches!(&error, bridge::BridgeError::Refused { .. }), "{error:?}");
    let text = error.to_string();
    assert!(text.contains("auth") && text.contains("the token is not the one this server has"), "{text}");

    // Long enough for the first retry (100 ms) to have happened had the bridge retried at all.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(server.connections(), 1, "a refused token must not be retried");
}
