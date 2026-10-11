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
use serde_json::Value;
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

// ---------------------------------------------------------------------------------------------
// Task 4: the call façade, driven against a second fake server built for `Call::open`.
//
// The server above scripts one `RealtimeSocket`; these tests drive a whole `Call`, so their server
// is a handshake-plus-a-live-socket the test writes to and reads from frame by frame. Nothing here
// touches the real API, and no key is a fixture: the call is pointed at a loopback port with an
// empty key *name*, which `RealtimeSocket::connect_with` sends as no `Authorization` header at all.
// ---------------------------------------------------------------------------------------------

/// One scripted server for a `Call`: the socket the test writes to and the frames the call sent.
struct FakeCall {
    socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
}

/// The call's own module, under the names the tests drive it by.
use tipsy_compadre::realtime::Call;
use tipsy_compadre::realtime::CallSetup;
use tipsy_compadre::realtime::Heard;
use tipsy_compadre::realtime::events;
use tipsy_protocol::{AssistantState, Event, Frame, ToClient};
use tokio::sync::mpsc::UnboundedReceiver;

/// Binds a loopback port, hands back the `ws://` URL the call is pointed at, and the task that
/// accepts the one connection it opens. The server opens the session before it waits for anything:
/// the client's own `configure` waits for `session.created` before it sends `session.update`, so a
/// server that waited for the client first would be waiting for ever.
async fn call_server() -> (String, tokio::task::JoinHandle<FakeCall>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let url = format!("ws://{}/v1/realtime?model=gpt-realtime-2.1", listener.local_addr().unwrap());
    let accepting = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("the call connects");
        let mut socket = tokio_tungstenite::accept_async(stream).await.expect("the handshake");
        socket
            .send(Message::text(r#"{"type":"session.created","session":{"id":"sess_fixture"}}"#))
            .await
            .expect("the client is listening");
        FakeCall { socket }
    });
    (url, accepting)
}

impl FakeCall {
    async fn send(&mut self, json: impl Into<String>) {
        self.socket.send(Message::Text(json.into().into())).await.expect("the client is listening");
    }

    /// The next JSON text frame the client sent, or `None` when it closed the socket or the
    /// connection died.
    async fn recv_json(&mut self) -> Option<Value> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    return Some(serde_json::from_str(text.as_str()).expect("JSON"))
                }
                // Pings and pongs are keepalive traffic, never client events; the test that cares
                // about the call's own keepalive waits for the *next* text frame instead.
                Some(Ok(Message::Close(_))) | None => return None,
                // A client that ends a call while this side is still reading cancels the connection
                // rather than completing the close handshake; that is an ending, not a failure.
                Some(Err(_)) => return None,
                Some(Ok(_)) => continue,
            }
        }
    }

    /// The handshake both sides agree on: the session is already open, so this waits for the client's
    /// own `session.update`, checks the shape Task 2 owns and answers it. What this task asserts is
    /// the 24 kHz the audio format pins, which is Review Focus 5 — the API's `audio/pcm` supports
    /// that one rate and no other.
    async fn handshake(&mut self) {
        let update = self.recv_json().await.expect("session.update");
        assert_eq!(update["type"], "session.update", "the first frame is the session's configuration");
        assert_eq!(update["session"]["audio"]["input"]["format"]["type"], "audio/pcm");
        assert_eq!(update["session"]["audio"]["input"]["format"]["rate"], 24000);
        assert_eq!(update["session"]["audio"]["output"]["format"]["rate"], 24000);
        self.send(r#"{"type":"session.updated","session":{}}"#).await;
    }
}

/// The call the tests open: loopback, a key name nobody holds — so the handshake carries no
/// `Authorization` header and no fixture is a secret — and a deadline long enough that it never
/// fires unless a test asks for one.
fn setup(url: String) -> CallSetup {
    let config = RealtimeConfig { key: String::new(), ..RealtimeConfig::default() };
    CallSetup {
        options: CallOptions::at(&url),
        url,
        config,
        // The key name above is empty, so the handshake carries no `Authorization` header at all:
        // no fixture here is a secret, and this resolver is what a production call would resolve one
        // through.
        resolver: Arc::new(KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None))),
        model: "gpt-realtime-2.1".into(),
        instructions: "eres un compae".into(),
        voice: "marin".into(),
        silence_ms: 1000,
        language: "es-ES".into(),
        limit: Duration::from_secs(600),
    }
}

/// One `response.output_audio.delta`, as the research note's event table describes it: base64 PCM16
/// inside a JSON text frame, with the item and content indices a later truncate has to name.
///
/// **The note marks this event's field names as inferred, not verified** — its documentation page
/// truncated before the Response section — so this fixture is where the inference is pinned: the
/// parser must accept `item_id`, `content_index` and `delta`. The release checklist's live smoke
/// test is what proves the real API agrees.
fn delta(item_id: &str, content_index: u32, samples: &[i16]) -> String {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    let audio = events::base64_encode(&pcm);
    format!(
        r#"{{"type":"response.output_audio.delta","response_id":"resp_1","item_id":"{item_id}","output_index":0,"content_index":{content_index},"delta":"{audio}"}}"#
    )
}

/// The same frame with **no** `item_id` at all. The note marks the event's payload fields inferred,
/// so a delta whose name is absent is a shape a real server may send; the mapping accepts it and the
/// call must still play it.
fn delta_without_a_name(content_index: u32, samples: &[i16]) -> String {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    let audio = events::base64_encode(&pcm);
    format!(
        r#"{{"type":"response.output_audio.delta","response_id":"resp_1","output_index":0,"content_index":{content_index},"delta":"{audio}"}}"#
    )
}

/// One `response.output_audio_transcript.delta` — the same inferred payload shape, pinned the same
/// way.
fn transcript_delta(text: &str) -> String {
    format!(
        r#"{{"type":"response.output_audio_transcript.delta","response_id":"resp_1","item_id":"item_a","output_index":0,"content_index":0,"delta":"{text}"}}"#
    )
}

fn response_created(id: &str) -> String {
    format!(r#"{{"type":"response.created","response":{{"id":"{id}"}}}}"#)
}

fn response_done(id: &str, status: &str) -> String {
    format!(r#"{{"type":"response.done","response":{{"id":"{id}","status":"{status}"}}}}"#)
}

/// Waits until the call has reported something `until` is happy with, and returns everything it
/// said. A call that says nothing for five seconds is a failure, not a hang.
async fn collect_until(
    from_call: &mut UnboundedReceiver<ToClient>,
    mut until: impl FnMut(&[ToClient]) -> bool,
) -> Vec<ToClient> {
    let mut seen = Vec::new();
    while !until(&seen) {
        match tokio::time::timeout(Duration::from_secs(5), from_call.recv()).await {
            Ok(Some(message)) => seen.push(message),
            Ok(None) => break,
            Err(_) => panic!("the call reported nothing for 5 s: {seen:#?}"),
        }
    }
    seen
}

fn events_of(seen: &[ToClient]) -> Vec<Event> {
    seen.iter()
        .filter_map(|message| match message {
            ToClient::Event(event) => Some(event.clone()),
            ToClient::Audio(_) => None,
        })
        .collect()
}

fn audio_of(seen: &[ToClient]) -> Vec<(u32, Vec<i16>)> {
    seen.iter()
        .filter_map(|message| match message {
            ToClient::Audio(Frame::Speech { seq, samples }) => Some((*seq, samples.clone())),
            _ => None,
        })
        .collect()
}

fn error_of(seen: &[ToClient]) -> Option<String> {
    events_of(seen).into_iter().find_map(|event| match event {
        Event::Error { message, .. } => Some(message),
        _ => None,
    })
}

/// The last live-state the call reported.
fn live_of(seen: &[ToClient]) -> Option<bool> {
    events_of(seen)
        .into_iter()
        .filter_map(|event| match event {
            Event::Live { active } => Some(active),
            _ => None,
        })
        .next_back()
}

/// Whether the call has reported the resting status its ending always carries.
fn idle_of(seen: &[ToClient]) -> bool {
    events_of(seen).iter().any(|event| matches!(event, Event::Status { state: AssistantState::Idle, .. }))
}

#[tokio::test]
async fn a_mic_frame_is_appended_as_twenty_milliseconds_of_24k_audio() {
    let (url, accepting) = call_server().await;
    let (out, _from_call) = tokio::sync::mpsc::unbounded_channel();
    let call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    // One frame as the microphone delivers it: 20 ms of 16 kHz mono PCM16, i.e. 320 samples.
    let frame: Vec<i16> = (0..320).map(|i| ((i as f32 * 0.3).sin() * 12000.0) as i16).collect();
    call.push_mic(frame);

    let append = server.recv_json().await.expect("input_audio_buffer.append");
    assert_eq!(append["type"], "input_audio_buffer.append");
    let audio = append["audio"].as_str().expect("the audio is a base64 string");
    // 24 kHz mono PCM16 is 48 bytes per millisecond (the research note's Interruption 4), so 20 ms
    // of it is 960 bytes: 480 samples, one and a half times the 320 the microphone gave us. A send
    // that skipped the resample would be 320 samples, i.e. 640 bytes and 852 base64 characters.
    assert_eq!(audio.len(), 1280, "960 bytes in base64, i.e. 480 samples at 24 kHz: {audio:?}");
    assert_eq!(audio.len() % 4, 0, "base64 is four characters per three bytes");
}

#[tokio::test]
async fn an_audio_delta_of_twenty_milliseconds_reaches_the_device_at_24000() {
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    // 20 ms of the API's own audio: 480 samples, 960 bytes.
    let samples: Vec<i16> = (0..480).map(|i| (i as i16 % 200) - 100).collect();
    server.send(response_created("resp_1")).await;
    server.send(delta("item_a", 0, &samples)).await;

    let seen = collect_until(&mut from_call, |seen| audio_of(seen).len() == 1).await;
    let played = audio_of(&seen);
    assert_eq!(played[0].1, samples, "the samples arrive whole, in order and signed");
    assert_eq!(played[0].1.len(), 480, "20 ms at 24 kHz, not 20 ms at 16 kHz");
    // The header the client keeps the device's rate from: `on_message` copies `SpeechChunk.sample_rate`
    // into `Client::rate` (`client.rs:167-171`) and `on_speech` hands it straight to
    // `Player::play(samples, self.rate)` (`:203-205`), so this pair *is* the rate the device is told.
    let header = events_of(&seen).into_iter().find_map(|event| match event {
        Event::SpeechChunk { sample_rate, .. } => Some(sample_rate),
        _ => None,
    });
    assert_eq!(header, Some(24000), "the API's rate, never the microphone's 16000");
}

#[tokio::test]
async fn a_delta_for_another_item_is_not_played() {
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    server.send(delta("item_a", 0, &[7i16; 480])).await;
    server.send(delta("item_b", 0, &[9i16; 480])).await;

    let seen = collect_until(&mut from_call, |seen| audio_of(seen).len() == 1).await;
    assert_eq!(audio_of(&seen)[0].1, vec![7i16; 480], "the item this response named is played");
    // The second delta names an item this response is not playing — a cancelled response still
    // arriving, or a race between two of them. Its bytes must not be heard, and they must not be
    // counted either: they would be added to the item a later truncate names.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(from_call.try_recv().is_err(), "nothing of `item_b` may reach the device");
}

#[tokio::test]
async fn a_delta_without_an_item_id_plays_and_does_not_gate_the_rest_of_the_response() {
    // The final review's critical finding. The item gate may fire on a *known* mismatch only: an
    // unnamed delta names no item, so it is not known to disagree with anything. The old gate stored
    // `""` as the item being played and compared every later delta against it, so the named delta
    // below compared unequal and was dropped — nothing reached the device, `finish` early-returned
    // and the HUD got neither a reply nor a `Done` (the note's rule is that a delta without a
    // readable name leaves nothing safe to truncate, never that it silences the call).
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let heard = Heard::new();
    let _call = Call::open(setup(url), Arc::clone(&heard), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    server.send(delta_without_a_name(0, &[3i16; 480])).await;
    server.send(delta("item_a", 0, &[7i16; 480])).await;

    let seen = collect_until(&mut from_call, |seen| audio_of(seen).len() == 2).await;
    assert_eq!(
        audio_of(&seen),
        vec![(0, vec![3i16; 480]), (1, vec![7i16; 480])],
        "the unnamed delta plays, and the named one is not gated out by the name's absence"
    );
    // And the name that did arrive is what a later truncate would use: the unnamed delta left the
    // bookkeeping untouched rather than storing a sentinel that no item could match.
    assert_eq!(heard.item(), Some(("item_a".to_string(), 0)));
}

#[tokio::test]
async fn the_transcripts_of_a_call_reach_the_hud() {
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    // What the user said, then what the model answered, in the lifecycle order the research note's
    // event table gives: the input transcript is transcribed while the response is generated.
    server
        .send(r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"item_1","content_index":0,"delta":"¿y el ticket de la cena?"}"#)
        .await;
    server.send(response_created("resp_1")).await;
    server.send(transcript_delta("Aquí")).await;
    server.send(transcript_delta(" andamos.")).await;
    server.send(delta("item_a", 0, &[5i16; 480])).await;
    server.send(response_done("resp_1", "completed")).await;

    let seen = collect_until(&mut from_call, |seen| {
        events_of(seen).iter().any(|event| matches!(event, Event::Done { .. }))
    })
    .await;

    // The three events the text pipeline also sends, in order: `Heard`, then `ReplyDelta`s, then
    // `Reply` — so the log, the tabs and the selection behave the same way in both modes.
    let kinds: Vec<&str> = events_of(&seen)
        .iter()
        .filter_map(|event| match event {
            Event::Heard { .. } => Some("heard"),
            Event::ReplyDelta { .. } => Some("delta"),
            Event::Reply { .. } => Some("reply"),
            Event::Done { .. } => Some("done"),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, ["heard", "delta", "delta", "reply", "done"], "{:#?}", events_of(&seen));

    // The measurement the text pipeline also makes: the turn started when the utterance was heard,
    // so its total is a real interval rather than a constant. The upper bound is generous on
    // purpose — the point is that it is a duration and not a timestamp.
    let done = events_of(&seen).into_iter().find_map(|event| match event {
        Event::Done { total_ms, .. } => Some(total_ms),
        _ => None,
    });
    assert!(done.is_some_and(|total_ms| total_ms < 2_000), "the turn's own clock: {done:?}");

    // And they land in the HUD's own state, which is the point: a call's turns are drawn exactly
    // like the text pipeline's.
    let mut hud = tipsy_ui::UiState::new();
    for message in &seen {
        if let ToClient::Event(event) = message {
            hud.apply(event);
        }
    }
    assert_eq!(hud.log.len(), 2, "{:#?}", hud.log);
    assert_eq!(hud.log[0].who, tipsy_ui::Who::User);
    assert_eq!(hud.log[0].text, "¿y el ticket de la cena?");
    assert_eq!(hud.log[1].who, tipsy_ui::Who::Assistant);
    assert_eq!(hud.log[1].text, "Aquí andamos.");
    assert!(!hud.log[1].pending, "the reply was said: the spinner stops");
    assert!(hud.log[1].first_audio_ms.is_some(), "the reply's timing is a real measurement");
}

#[tokio::test]
async fn a_socket_that_dies_mid_answer_leaves_the_hud_out_of_the_live_state() {
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    server.send(transcript_delta("Aquí anda")).await;
    server.send(delta("item_a", 0, &[3i16; 480])).await;
    // Mid-answer the socket goes, the way a dead peer leaves it.
    drop(server);

    // The call reports both halves of its ending in one go — `Live { active: false }` and then the
    // idle status — so the collector waits for the pair: stopping at the flag alone would stop one
    // event short of the status it is about to be asked about.
    let seen = collect_until(&mut from_call, |seen| live_of(seen) == Some(false) && idle_of(seen)).await;

    // Review Focus 1: the flag goes down, so the indicator goes down (`UiState::live`, Task 6), and
    // the status comes back to rest so nothing is left spinning.
    assert_eq!(live_of(&seen), Some(false), "the call is over: {seen:#?}");
    assert!(
        events_of(&seen)
            .iter()
            .any(|event| matches!(event, Event::Status { state: AssistantState::Idle, .. })),
        "an ending status must follow: {seen:#?}"
    );
    // A readable error, never an empty string and never a panic.
    let why = error_of(&seen).expect("the user is told what happened");
    assert!(why.contains("socket"), "the message says which side went quiet: {why}");
    // What the reply had said before the socket died is still on the wire: the answer was cut off,
    // not erased.
    assert!(
        events_of(&seen).iter().any(|event| matches!(event, Event::ReplyDelta { text } if text == "Aquí anda")),
        "the partial transcript survives the failure: {seen:#?}"
    );

    // And the HUD itself leaves the call: the flag the indicator is drawn from is down, and the
    // partial reply is still on screen with its spinner stopped (Review Focus 1).
    let mut hud = tipsy_ui::UiState::new();
    for message in &seen {
        if let ToClient::Event(event) = message {
            hud.apply(event);
        }
    }
    assert!(!hud.live, "the HUD is out of its live state: {:#?}", hud.log);
    assert!(!hud.busy(), "and nothing is left spinning: {:#?}", hud.log);
    assert_eq!(hud.log.last().unwrap().text, "Aquí anda", "what was said before the death stays");
}

#[tokio::test]
async fn an_error_event_ends_the_call_with_its_own_message() {
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    // The research note's `error` event: the server's own words, which are what the user gets.
    server
        .send(r#"{"type":"error","error":{"message":"Invalid API key provided","type":"invalid_request_error","code":"invalid_api_key"}}"#)
        .await;

    let seen = collect_until(&mut from_call, |seen| live_of(seen) == Some(false)).await;
    assert_eq!(error_of(&seen).as_deref(), Some("Invalid API key provided"));
    assert!(error_of(&seen).is_some_and(|why| !why.trim().is_empty()));
}

#[tokio::test]
async fn a_closed_port_is_a_readable_error_and_the_call_never_goes_live() {
    // A port nobody is listening on: bind one, note the URL, drop the listener.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let url = format!("ws://{}/v1/realtime?model=gpt-realtime-2.1", listener.local_addr().unwrap());
    drop(listener);

    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);

    let seen = collect_until(&mut from_call, |seen| error_of(seen).is_some() && live_of(seen) == Some(false)).await;
    assert!(
        !events_of(&seen).iter().any(|event| matches!(event, Event::Live { active: true })),
        "a call that never opened is never live: {seen:#?}"
    );
    // `Live { active: false }` still goes out, so a client holding the handle lets go of it.
    assert_eq!(live_of(&seen), Some(false));
    assert!(error_of(&seen).is_some_and(|why| !why.trim().is_empty()));
}

#[tokio::test]
async fn stopping_the_call_closes_the_socket() {
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    call.stop();

    assert!(server.recv_json().await.is_none(), "a stopped call closes its socket");
    let seen = collect_until(&mut from_call, |seen| live_of(seen) == Some(false) && idle_of(seen)).await;
    assert_eq!(live_of(&seen), Some(false), "the indicator goes down: {seen:#?}");
    assert!(
        events_of(&seen)
            .iter()
            .any(|event| matches!(event, Event::Status { state: AssistantState::Idle, .. })),
        "and the status comes back to rest: {seen:#?}"
    );
}

#[tokio::test]
async fn a_call_that_outlives_the_apis_session_cap_reopens_instead_of_ending() {
    // The API ends a session at 60 minutes and documents no close code for it, so a long call
    // reopens before that: the caller watches `session_expired` — never the code of a closed socket
    // — and a call that has outlived its cap reopens whatever the socket did. The cap is zero here
    // because the ending has to be reachable: `max_minutes` is ten by default, so this is the one
    // state a test can put a call in without waiting an hour. Two connections are what proves the
    // reopen, because only a second one can answer for it.
    let server = FakeServer::start(vec![
        Script::Drop(vec![SESSION_CREATED.into()]),
        Script::Drop(vec![SESSION_CREATED.into()]),
        Script::Drop(vec![SESSION_CREATED.into()]),
    ])
    .await;

    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let mut wanted = setup(server.url());
    // The third connection is the reopen of the reopen: a server that never answers it makes the
    // open timeout its ending, and a short one keeps this test quick.
    wanted.options =
        CallOptions::at(server.url()).with_cap(Duration::ZERO).with_open_timeout(Duration::from_millis(200));
    let _call = Call::open(wanted, Heard::new(), out);

    // Both connections are configured, which is the whole of `RealtimeSocket::reopen`.
    let texts = server.texts(2).await;
    for text in &texts {
        let event: Value = serde_json::from_str(text).expect("JSON");
        assert_eq!(event["type"], "session.update", "the reopened call is configured like the first");
        assert_eq!(event["session"]["audio"]["input"]["format"]["rate"], 24000, "and at the same 24 kHz");
    }

    // A reopen is not a new call: the HUD was told the call is live once, when the first socket came
    // up, and the second one changes nothing it can see. It *is* a fresh session, though, and the log
    // says so: the reopened socket is configured from `session.update` alone, with no history
    // replayed, so the assistant has forgotten everything said before the cap.
    let seen = collect_until(&mut from_call, |seen| error_of(seen).is_some()).await;
    let notice = error_of(&seen).expect("the reopen is reported, not silent");
    assert!(notice.contains("60-minute cap"), "the notice names what was reached: {notice}");
    assert!(
        notice.contains("remembers nothing"),
        "and what the user loses: the conversation before the reopen: {notice}"
    );
    assert_eq!(
        events_of(&seen).iter().filter(|event| matches!(event, Event::Live { active: true })).count(),
        1,
        "a reopened session is the same call: {seen:#?}"
    );
}

#[tokio::test]
async fn a_call_that_reaches_its_limit_ends_itself() {
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let mut wanted = setup(url);
    // A limit a test can wait for; `[realtime] max_minutes` is the same deadline in minutes.
    wanted.limit = Duration::from_millis(50);
    let _call = Call::open(wanted, Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    let seen = collect_until(&mut from_call, |seen| live_of(seen) == Some(false)).await;
    let why = error_of(&seen).expect("the user is told why the call ended");
    assert!(why.contains("limit"), "the message names the limit: {why}");
    assert!(server.recv_json().await.is_none(), "a call at its limit closes its socket");
}

// ---------------------------------------------------------------------------------------------
// Task 5: barge-in.
//
// `input_audio_buffer.speech_started` is the server's own report that the user talked over the
// model: playback stops on the client's next frame and the model is told, in milliseconds it can
// trust, how much of its own speech was really heard (research note, Interruption 1–4). The number
// comes from the device's queue, never from the bytes handed over, and a local interrupt cancels
// the response instead of inventing one (Interruption 5).
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_barge_in_truncates_the_item_at_the_milliseconds_that_were_heard() {
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let heard = Heard::new();
    let _call = Call::open(setup(url), Arc::clone(&heard), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    for _ in 0..3 {
        server.send(delta("item_a", 0, &[7i16; 480])).await;
    }
    let _ = collect_until(&mut from_call, |seen| audio_of(seen).len() == 3).await;

    // The client's side of the count, stated by hand: in the running client these three numbers come
    // from `Player::progress` after each chunk was appended (`client.rs`, `on_speech`). Three 20 ms
    // chunks are queued, two have finished, and 12 ms of the third have played.
    heard.begin("item_a", 0);
    heard.push(960, 3, 0);
    heard.push(960, 2, 0);
    heard.push(960, 1, 12);

    // The research note's barge-in event names the *user's* item; the truncate must name the
    // assistant's, which is the one the device was playing.
    server
        .send(r#"{"type":"input_audio_buffer.speech_started","item_id":"item_1","audio_start_ms":3200}"#)
        .await;

    let truncate = server.recv_json().await.expect("the truncate");
    assert_eq!(truncate["type"], "conversation.item.truncate");
    assert_eq!(truncate["item_id"], "item_a", "the assistant's item, never the user's `item_1`");
    assert_eq!(truncate["content_index"], 0, "the note fixes the content index at 0");
    // 2 finished chunks of 960 bytes plus 12 ms × 48 bytes: 2496 bytes ÷ 48 = 52 ms.
    assert_eq!(truncate["audio_end_ms"], 52);
    // And the device is asked to stop, once.
    assert!(heard.take_interrupted(), "the client is told to stop the device on its next frame");
    assert!(!heard.take_interrupted(), "a second frame must not flush again for the same barge-in");
}

#[tokio::test]
async fn what_arrives_after_a_barge_in_is_not_played() {
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let _call = Call::open(setup(url), Heard::new(), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    for _ in 0..3 {
        server.send(delta("item_a", 0, &[7i16; 480])).await;
    }
    let seen = collect_until(&mut from_call, |seen| audio_of(seen).len() == 3).await;
    assert_eq!(audio_of(&seen).len(), 3);

    server
        .send(r#"{"type":"input_audio_buffer.speech_started","item_id":"item_1","audio_start_ms":3200}"#)
        .await;
    // The tail of the response the user talked over is still in flight, both before and after the
    // server's own cancellation.
    server.send(delta("item_a", 0, &[9i16; 480])).await;
    server.send(response_done("resp_1", "cancelled")).await;
    server.send(delta("item_a", 0, &[9i16; 480])).await;

    let seen = collect_until(&mut from_call, |seen| {
        events_of(seen).iter().any(|event| matches!(event, Event::Done { .. }))
    })
    .await;
    assert!(audio_of(&seen).is_empty(), "none of it may reach the device: {:#?}", audio_of(&seen));
}

#[tokio::test]
async fn a_local_interrupt_cancels_the_response() {
    let (url, accepting) = call_server().await;
    let (out, mut from_call) = tokio::sync::mpsc::unbounded_channel();
    let heard = Heard::new();
    let call = Call::open(setup(url), Arc::clone(&heard), out);
    let mut server = accepting.await.expect("the server task");
    server.handshake().await;

    server.send(response_created("resp_1")).await;
    server.send(delta("item_a", 0, &[7i16; 480])).await;
    let _ = collect_until(&mut from_call, |seen| audio_of(seen).len() == 1).await;

    call.interrupt();

    let cancel = server.recv_json().await.expect("the cancel");
    assert_eq!(cancel["type"], "response.cancel");
    assert!(
        cancel.get("response_id").is_none(),
        "the research note's client event is `{{type, response_id?}}`: nothing is named, because \
         cancelling nothing is safe: {cancel:#?}"
    );
    assert!(heard.take_interrupted(), "a local interrupt stops the device too");
}
