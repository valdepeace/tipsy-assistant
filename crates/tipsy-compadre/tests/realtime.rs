//! The realtime socket against a local fake server: the handshake it makes, the frames it reads,
//! the two ways a call ends, and the reopen at the session cap.
//!
//! `wiremock` cannot help here — it serves HTTP, and this is a duplex WebSocket — so the fake server
//! is a `TcpListener` plus `tokio_tungstenite::accept_hdr_async`, which is enough to see the request
//! URI and every header the client sends, and to script what comes back.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tipsy_compadre::config::RealtimeConfig;
use tipsy_compadre::keys::{KeyResolver, MemoryStore, SecretStore};
use tipsy_compadre::realtime::socket::{CallOptions, CallSpec, RealtimeError, RealtimeSocket, SocketEvent};
use tipsy_protocol::ErrorCode;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

/// The one key value any test here uses. It is a fixture, not a key: the workspace rule is that keys
/// appear by name and never in a file, a log or a fixture, and this string exists only to prove that
/// the header is built from the *resolved* value.
const TEST_KEY: &str = "fake-key-for-tests-only";

/// Frames the tests script. The payload shapes are Task 3's business — the socket treats every text
/// frame as opaque — so these carry only the note's `type` strings and a field or two.
const SESSION_CREATED: &str = r#"{"type":"session.created","session":{"id":"sess_fixture"}}"#;
const SESSION_UPDATED: &str = r#"{"type":"session.updated","session":{"id":"sess_fixture"}}"#;
const AUDIO_DELTA: &str = r#"{"type":"response.output_audio.delta","delta":"AAECAw=="}"#;
const TRANSCRIPT_DELTA: &str = r#"{"type":"response.output_audio_transcript.delta","delta":"Ozú"}"#;
const SPEECH_STARTED: &str =
    r#"{"type":"input_audio_buffer.speech_started","item_id":"item_0","audio_start_ms":1200}"#;

/// A line that is exactly this, instead of JSON, is sent as a WebSocket **ping**: keepalive traffic
/// is real traffic, and the reader must skip it without disturbing the order of the events.
const PING: &str = "__ping__";

fn resolver() -> KeyResolver {
    let store = MemoryStore::default();
    store.set("openai", TEST_KEY).unwrap();
    KeyResolver::new(Box::new(store), Box::new(|_| None))
}

fn empty_resolver() -> KeyResolver {
    KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None))
}

fn spec() -> CallSpec {
    CallSpec {
        instructions: "Habla como un compae andaluz, con cariño y sin prisa.".into(),
        language: "es-ES".into(),
        silence_ms: 1500,
    }
}

/// What the fake server does on one accepted connection. Every variant starts by sending the frames
/// the test scripts — always `session.created` first, because the client waits for it before it
/// sends `session.update` (the note's ordering).
#[derive(Clone)]
enum Script {
    /// Send, then keep the socket open and record what the client sends.
    Open(Vec<String>),
    /// Send, wait for the client's next frame, then close with this code and reason.
    Close(Vec<String>, u16, String),
    /// Send, wait for the client's next frame, then drop the TCP connection with no close frame.
    Drop(Vec<String>),
}

#[derive(Clone, Default)]
struct Handshake {
    target: String,
    headers: Vec<(String, String)>,
}

impl Handshake {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

struct FakeServer {
    addr: SocketAddr,
    handshakes: Arc<Mutex<Vec<Handshake>>>,
    texts: Arc<Mutex<Vec<String>>>,
}

/// The `accept_hdr_async` callback: it records the request the client made and then accepts it. It is
/// a type rather than a closure only so the `allow` below can sit on the exact function clippy
/// complains about — tungstenite's handshake callback signature returns the whole `ErrorResponse` by
/// value, and that is not this test's decision to change.
struct RecordHandshake(Arc<Mutex<Handshake>>);

impl tokio_tungstenite::tungstenite::handshake::server::Callback for RecordHandshake {
    #[allow(clippy::result_large_err)]
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        let mut handshake = self.0.lock().unwrap();
        handshake.target = request.uri().to_string();
        handshake.headers = request
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str().to_string(), value.to_str().unwrap_or_default().to_string()))
            .collect();
        Ok(response)
    }
}

impl FakeServer {
    /// Starts the listener on a random loopback port. The `n`-th accepted connection runs
    /// `scripts[n]`; a connection with no script is accepted and left silent.
    async fn start(scripts: Vec<Script>) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handshakes: Arc<Mutex<Vec<Handshake>>> = Arc::new(Mutex::new(Vec::new()));
        let texts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (seen, recorded) = (handshakes.clone(), texts.clone());
        tokio::spawn(async move {
            let mut accepted = 0usize;
            while let Ok((stream, _)) = listener.accept().await {
                let script = scripts.get(accepted).cloned().unwrap_or(Script::Open(Vec::new()));
                accepted += 1;
                let slot: Arc<Mutex<Handshake>> = Arc::new(Mutex::new(Handshake::default()));
                let callback_slot = slot.clone();
                let seen = seen.clone();
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let Ok(socket) = tokio_tungstenite::accept_hdr_async(stream, RecordHandshake(callback_slot)).await
                    else {
                        return;
                    };
                    seen.lock().unwrap().push(slot.lock().unwrap().clone());
                    serve(socket, script, recorded).await;
                });
            }
        });
        FakeServer { addr, handshakes, texts }
    }

    /// The URL `CallOptions::at` takes: the fake server serves the real path.
    fn url(&self) -> String {
        format!("ws://{}/v1/realtime", self.addr)
    }

    /// Waits for the `index`-th handshake. The accept loop records it just after the 101 response, so
    /// a test that returns from `connect` may still be a scheduling hop ahead of it.
    async fn handshake(&self, index: usize) -> Handshake {
        for _ in 0..200 {
            if let Some(handshake) = self.handshakes.lock().unwrap().get(index).cloned() {
                return handshake;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the fake server never accepted connection {index}");
    }

    async fn connections_at_least(&self, wanted: usize) {
        for _ in 0..200 {
            if self.handshakes.lock().unwrap().len() >= wanted {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the fake server accepted {} connections, wanted {wanted}", self.handshakes.lock().unwrap().len());
    }

    /// Waits for at least `n` client frames and returns everything recorded so far.
    async fn texts(&self, n: usize) -> Vec<String> {
        for _ in 0..200 {
            let recorded = self.texts.lock().unwrap().clone();
            if recorded.len() >= n {
                return recorded;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the fake server recorded {} client frames, wanted {n}", self.texts.lock().unwrap().len());
    }
}

/// How a scripted connection ends, once the client's first frame has been seen.
enum Ending {
    KeepOpen,
    Close(u16, String),
    /// The abrupt death: no close frame, so the client can only learn of it from the transport.
    Drop,
}

async fn serve(
    mut socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    script: Script,
    texts: Arc<Mutex<Vec<String>>>,
) {
    let (lines, ending) = match script {
        Script::Open(lines) => (lines, Ending::KeepOpen),
        Script::Close(lines, code, reason) => (lines, Ending::Close(code, reason)),
        Script::Drop(lines) => (lines, Ending::Drop),
    };
    for line in lines {
        let message =
            if line == PING { Message::Ping(Vec::<u8>::new().into()) } else { Message::text(line) };
        if socket.send(message).await.is_err() {
            return;
        }
    }
    // Wait for one client frame before ending the connection: `session.update` is what the client
    // sends the moment it has seen `session.created`, and waiting for it turns "the call is
    // configured, then the socket dies" into a deterministic sequence instead of a race.
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                texts.lock().unwrap().push(text.as_str().to_string());
                break;
            }
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return,
        }
    }
    match ending {
        Ending::KeepOpen => {
            while let Some(Ok(message)) = socket.next().await {
                if let Message::Text(text) = message {
                    texts.lock().unwrap().push(text.as_str().to_string());
                }
            }
        }
        Ending::Close(code, reason) => {
            let frame = CloseFrame { code: CloseCode::from(code), reason: reason.into() };
            let _ = socket.send(Message::Close(Some(frame))).await;
            // Complete the close handshake: read the client's own close reply, so its `close()` has
            // a live socket to say goodbye to and does not fail on a connection we already dropped.
            while let Some(Ok(message)) = socket.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
            let _ = socket.close(None).await;
        }
        Ending::Drop => drop(socket),
    }
}

async fn connect_to(server: &FakeServer) -> RealtimeSocket {
    RealtimeSocket::connect_with(CallOptions::at(server.url()), &resolver(), &RealtimeConfig::default(), &spec())
        .await
        .expect("the fake server accepts the handshake and sends session.created")
}

/// The handshake the client makes, and the session it configures: the URL, the query, the header and
/// the `session.update` that follows `session.created`.
#[tokio::test]
async fn the_handshake_carries_the_model_query_and_the_bearer_key() {
    let server = FakeServer::start(vec![Script::Open(vec![SESSION_CREATED.into()])]).await;
    let socket = connect_to(&server).await;

    let handshake = server.handshake(0).await;
    assert_eq!(
        handshake.target, "/v1/realtime?model=gpt-realtime-2.1",
        "the note's endpoint with the model as a query parameter"
    );
    assert_eq!(
        handshake.header("authorization"), Some("Bearer fake-key-for-tests-only"),
        "the note: `Authorization: Bearer $OPENAI_API_KEY` on the handshake"
    );
    assert_eq!(handshake.header("openai-beta"), None, "GA needs no `OpenAI-Beta: realtime=v1`");
    assert_eq!(handshake.header("openai-safety-identifier"), None, "optional, and this process has no hashed user id");

    let first: serde_json::Value = serde_json::from_str(&server.texts(1).await[0]).unwrap();
    assert_eq!(first["type"], "session.update");
    assert_eq!(first["session"]["audio"]["input"]["format"], serde_json::json!({ "type": "audio/pcm", "rate": 24_000 }));
    assert_eq!(first["session"]["audio"]["output"]["format"], serde_json::json!({ "type": "audio/pcm", "rate": 24_000 }));
    assert_eq!(first["session"]["audio"]["output"]["voice"], "marin");
    assert_eq!(first["session"]["audio"]["input"]["turn_detection"]["type"], "server_vad");
    assert_eq!(first["session"]["audio"]["input"]["turn_detection"]["silence_duration_ms"], serde_json::json!(1500));

    // The key never reaches a log: the type's `Debug` prints neither the resolved value nor the word
    // `Bearer` (the Global Constraint says the header must not be logged even at debug level).
    let printed = format!("{socket:?}");
    assert!(!printed.contains(TEST_KEY), "{printed}");
    assert!(!printed.contains("Bearer"), "{printed}");
    assert!(!printed.contains(&spec().instructions), "the profile's own text is not printed either: {printed}");
}

/// A scripted exchange, frame by frame: text is handed back byte for byte (the socket parses
/// nothing), and keepalive traffic never appears as an event.
#[tokio::test]
async fn a_scripted_exchange_is_read_frame_by_frame_in_order() {
    let server = FakeServer::start(vec![Script::Open(vec![
        SESSION_CREATED.into(),
        PING.into(),
        SESSION_UPDATED.into(),
        AUDIO_DELTA.into(),
        TRANSCRIPT_DELTA.into(),
        SPEECH_STARTED.into(),
    ])])
    .await;
    let mut socket = connect_to(&server).await;

    // `connect` consumed `session.created` and sent `session.update`; the ping that follows it in the
    // script is skipped, so the next event is the one after it, unchanged.
    assert_eq!(socket.recv().await.unwrap(), SocketEvent::Text(SESSION_UPDATED.into()));
    assert_eq!(socket.recv().await.unwrap(), SocketEvent::Text(AUDIO_DELTA.into()));

    // Our own keepalive: a ping the API's side answers with a pong, which the reader also skips.
    socket.ping().await.unwrap();
    assert_eq!(socket.recv().await.unwrap(), SocketEvent::Text(TRANSCRIPT_DELTA.into()));
    assert_eq!(socket.recv().await.unwrap(), SocketEvent::Text(SPEECH_STARTED.into()));
}

/// A close the server sends arrives with its code and reason; nothing interprets them (the note says
/// the codes are undocumented), and closing a socket that is already closed is not a failure.
#[tokio::test]
async fn a_clean_close_is_reported_with_its_code_and_reason() {
    let server = FakeServer::start(vec![Script::Close(
        vec![SESSION_CREATED.into(), SESSION_UPDATED.into()],
        1000,
        "call ended".into(),
    )])
    .await;
    let mut socket = connect_to(&server).await;

    assert_eq!(socket.recv().await.unwrap(), SocketEvent::Text(SESSION_UPDATED.into()));
    assert_eq!(
        socket.recv().await.unwrap(),
        SocketEvent::Closed { code: Some(1000), reason: "call ended".into() }
    );
    // The socket reports, the caller decides: reopening at the cap is `session_expired`'s decision,
    // never a close code's (the note documents no code for the 60-minute limit). Closing twice is
    // fine, because a call that is already over is over.
    socket.close().await.unwrap();
    socket.close().await.unwrap();
}

/// The other death: no close frame at all. It must surface as a readable error, not as a hang.
#[tokio::test]
async fn an_abrupt_death_is_a_network_error_not_a_hang() {
    let server = FakeServer::start(vec![Script::Drop(vec![SESSION_CREATED.into()])]).await;
    let mut socket = connect_to(&server).await;

    let error = tokio::time::timeout(Duration::from_secs(5), socket.recv())
        .await
        .expect("recv must return when the peer dies without a close frame")
        .expect_err("a death with no close frame is an error, never a silent Ok");
    assert!(matches!(error, RealtimeError::Network(_)), "{error:?}");
    assert_eq!(error.code(), ErrorCode::Network, "the HUD log shows a network error: {error}");
}

/// The 60-minute cap. The test cannot wait an hour, so `CallOptions::with_cap` gives it 200 ms — the
/// production value (`SESSION_CAP`) is the note's "maximum duration of a Realtime session is 60
/// minutes" and reconnection is app-managed.
#[tokio::test]
async fn a_call_reopens_when_the_session_cap_is_reached() {
    let server = FakeServer::start(vec![
        Script::Open(vec![SESSION_CREATED.into()]),
        Script::Open(vec![SESSION_CREATED.into()]),
    ])
    .await;
    let options = CallOptions::at(server.url()).with_cap(Duration::from_millis(200));
    let mut socket =
        RealtimeSocket::connect_with(options, &resolver(), &RealtimeConfig::default(), &spec()).await.unwrap();

    assert!(!socket.session_expired(), "a fresh socket has its whole cap ahead of it");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(socket.session_expired(), "the cap is what the caller watches, never a close code");

    socket.reopen(&resolver()).await.unwrap();
    assert!(!socket.session_expired(), "a reopened socket starts a new session");
    server.connections_at_least(2).await;

    let texts = server.texts(2).await;
    assert_eq!(texts.len(), 2, "each connection is configured once: {texts:?}");
    for text in &texts {
        let event: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(event["type"], "session.update", "the reopened call is configured like the first");
        assert_eq!(event["session"]["audio"]["input"]["turn_detection"]["silence_duration_ms"], serde_json::json!(1500));
    }
}

/// A server that accepts the handshake and then says nothing must not hold the call forever: the open
/// timeout is what makes a dead endpoint an error instead of a spinner.
#[tokio::test]
async fn a_silent_server_times_out_instead_of_hanging() {
    let server = FakeServer::start(vec![Script::Open(Vec::new())]).await;
    let options = CallOptions::at(server.url()).with_open_timeout(Duration::from_millis(150));
    let error = RealtimeSocket::connect_with(options, &resolver(), &RealtimeConfig::default(), &spec())
        .await
        .expect_err("a server that never opens a session is a failure, not a wait");
    assert_eq!(error, RealtimeError::Timeout);
    assert_eq!(error.code(), ErrorCode::Timeout);
}

/// A key *name* the resolver cannot find is the connectors' `MissingKey`, and it fails before the
/// network is touched: no server is started in this test.
#[tokio::test]
async fn a_missing_key_names_the_key_and_the_variable() {
    let error = RealtimeSocket::connect(&empty_resolver(), &RealtimeConfig::default(), &spec())
        .await
        .expect_err("the default key name `openai` is not in an empty resolver");
    assert_eq!(error, RealtimeError::MissingKey { name: "openai".into(), env: "TIPSY_KEY_OPENAI".into() });
    assert!(error.to_string().contains("tipsy-compadre key set openai"), "{error}");
    assert!(error.to_string().contains("TIPSY_KEY_OPENAI"), "{error}");
    assert_eq!(error.code(), ErrorCode::NoKey);
    assert!(!error.to_string().contains("sk-"), "the message names the key, never a value: {error}");
}

/// An empty key name means "no key", exactly as `providers::key_for` treats it: the handshake goes
/// out with no `Authorization` header at all, so a loopback stub can be pointed at without one.
#[tokio::test]
async fn an_empty_key_name_sends_no_authorization_header() {
    let server = FakeServer::start(vec![Script::Open(vec![SESSION_CREATED.into()])]).await;
    let config = RealtimeConfig { key: String::new(), ..RealtimeConfig::default() };
    let _socket = RealtimeSocket::connect_with(CallOptions::at(server.url()), &resolver(), &config, &spec())
        .await
        .unwrap();

    let handshake = server.handshake(0).await;
    assert_eq!(handshake.header("authorization"), None, "no key name, no header");
    assert_eq!(handshake.target, "/v1/realtime?model=gpt-realtime-2.1");
}
