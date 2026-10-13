//! `/terminal/ws` against a real listener on `127.0.0.1:0`, a real PTY child and a real WebSocket
//! client. The children are `cmd.exe`, PowerShell and `/bin/sh` rather than the project's binary
//! because this test is about the *pump*: Task 2.1 already pins the child, and the pump is what has
//! to hand a TUI's bytes back and forth unmodified.
//!
//! What each test is for:
//!   * the token rule, which `/terminal` and `/terminal/ws` share, spelled once and tested without a
//!     socket
//!   * raw bytes both ways: a `data` message reaches the child and the child's own output comes back
//!     as binary frames, with ConPTY's `ESC[6n` gate answered through the pump on the way
//!   * a debounced `resize` reaching the child as a new console width
//!   * the wrong token and a foreign `Origin` never upgrading (`403`, the ledger's rule for this
//!     route, not only `/ws`)
//!   * more than the connection cap refused, **and nothing spawned for the refusal**
//!   * teardown: one socket's death kills its own child and leaves the other connection's alone
//!   * the child's own exit code reaching the page in the one text frame this socket ever sends
//!
//! **ConPTY asks the terminal for the cursor position and then waits for the answer.** Its first byte
//! is `ESC[6n`, and until the terminal replies with a cursor position report it delivers nothing else
//! (Task 2.1 measured it: four bytes and an empty configuration directory for nine seconds). A real
//! terminal answers this and xterm.js does it natively, so [`Terminal::poll`] answers it here — and
//! [`Terminal::saw_cursor_query`] is the assertion that this pump carried that byte to the client
//! unmodified, which is the whole reason this task must filter nothing.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use futures::{SinkExt, StreamExt};
use tipsy_assistant::client::pty::{PtySession, PtySpawner};
use tipsy_assistant::server::conn::{MAX_TERMINAL_CONNECTIONS, TerminalRouteState, router, terminal_access};
use tokio::sync::Semaphore;
use tokio_tungstenite::tungstenite::Message;

#[path = "support/process.rs"]
mod process;

/// How long a test waits for a child to say something before calling it a failure. A ConPTY child
/// takes seconds to say its first word, and the debug build's process table is busy.
const WAIT: Duration = Duration::from_secs(20);

/// How long one upgrade may take before the test calls it refused.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The token these tests configure. It is a fixture, not a secret: it reaches a listener on
/// `127.0.0.1` that this test started, and nothing resolves a key from it.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// ConPTY's device status report, and the cursor position report that answers it.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_REPORT: &str = "\u{1b}[1;1R";

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The spawner most tests use: a shell that stays alive and echoes what it is given. Cheap to start,
/// and the ping pong it plays is what the cap and token tests need and nothing more.
struct TestSpawner {
    /// How many children this spawner has actually started. The cap and the `Origin` refusal are
    /// asserted against it, so "refused without spawning anything" is observed rather than claimed.
    spawns: Arc<AtomicUsize>,
}

impl TestSpawner {
    fn new() -> Self {
        Self { spawns: Arc::new(AtomicUsize::new(0)) }
    }

    fn count(&self) -> usize {
        self.spawns.load(Ordering::SeqCst)
    }
}

impl PtySpawner for TestSpawner {
    fn args(&self) -> Vec<String> {
        Vec::new()
    }

    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        #[cfg(windows)]
        let (program, args) = (PathBuf::from(process::CMD), vec!["/Q".to_string(), "/K".to_string()]);
        #[cfg(not(windows))]
        let (program, args) = (PathBuf::from("/bin/sh"), vec!["-i".to_string()]);
        PtySession::spawn(&program, &args, cols, rows)
    }
}

/// The arguments a PowerShell child is started with. `-File` plus an absolute path, which is how
/// `tests/pty.rs` starts the same shells.
#[cfg(windows)]
fn powershell_args(dir: &Path, script: &str) -> Vec<String> {
    vec![
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        process::abs(dir, script),
    ]
}

/// A child that echoes every line it reads back as `ECHO=<line>`, so a byte written to the socket can
/// be found again in the socket's own output without knowing what a shell's banner says in this
/// machine's language.
struct EchoSpawner {
    dir: PathBuf,
}

impl PtySpawner for EchoSpawner {
    fn args(&self) -> Vec<String> {
        Vec::new()
    }

    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String> {
        #[cfg(windows)]
        {
            let script = "while ($true) { $line = [Console]::In.ReadLine(); if ($null -eq $line) { break }; \
                          [Console]::Out.Write('ECHO=' + $line + \"`n\"); [Console]::Out.Flush() }\r\n";
            let path = self.dir.join("echo.ps1");
            std::fs::write(&path, script).map_err(|e| format!("{}: {e}", path.display()))?;
            PtySession::spawn(Path::new(process::POWERSHELL), &powershell_args(&self.dir, "echo.ps1"), cols, rows)
        }
        #[cfg(not(windows))]
        {
            PtySession::spawn(
                Path::new("/bin/sh"),
                &["-c".into(), "while IFS= read -r line; do printf 'ECHO=%s\\n' \"$line\"; done".into()],
                cols,
                rows,
            )
        }
    }
}

/// A PowerShell child that prints its console width and then parks on `ReadLine`. Its width is read
/// once per line, so a resize is only observable after a newline makes that call return.
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
        let path = self.dir.join("width.ps1");
        std::fs::write(
            &path,
            "while ($true) { [Console]::Out.Write('WIDTH=' + [Console]::WindowWidth); \
             [Console]::Out.Flush(); $null = [Console]::In.ReadLine() }\r\n",
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
        PtySession::spawn(Path::new(process::POWERSHELL), &powershell_args(&self.dir, "width.ps1"), cols, rows)
    }
}

/// A child that keeps reading its input for ever and announces **its own** pid once.
///
/// The pid is the child's, printed by the child itself: the pid of the process the test started is a
/// different process on this platform, which is the mistake this spawner exists to avoid. Each spawn
/// gets its own script, so two connections own two distinguishable children.
#[derive(Default)]
struct KeepAliveSpawner {
    dir: PathBuf,
    started: AtomicUsize,
}

impl PtySpawner for KeepAliveSpawner {
    fn args(&self) -> Vec<String> {
        Vec::new()
    }

    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String> {
        let index = self.started.fetch_add(1, Ordering::SeqCst);
        #[cfg(windows)]
        {
            let name = format!("keepalive-{index}.ps1");
            let script = "[Console]::Out.Write('KEEPALIVE<' + $PID + \">`n\"); [Console]::Out.Flush()\r\n\
                          while ($true) { $null = [Console]::In.ReadLine() }\r\n";
            let path = self.dir.join(&name);
            std::fs::write(&path, script).map_err(|e| format!("{}: {e}", path.display()))?;
            PtySession::spawn(Path::new(process::POWERSHELL), &powershell_args(&self.dir, &name), cols, rows)
        }
        #[cfg(not(windows))]
        {
            let _ = index;
            PtySession::spawn(
                Path::new("/bin/sh"),
                &["-c".into(), "printf 'KEEPALIVE<%s>\\n' $$; while read -r _; do :; done".into()],
                cols,
                rows,
            )
        }
    }
}

/// A child that says one word and exits with a code of its own, so the page's one text frame carries
/// a number the test chose rather than a constant the server wrote.
struct OneShotSpawner;

impl PtySpawner for OneShotSpawner {
    fn args(&self) -> Vec<String> {
        Vec::new()
    }

    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String> {
        #[cfg(windows)]
        let (program, args) = (
            PathBuf::from(process::POWERSHELL),
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                "[Console]::Out.Write('BYE'); [Console]::Out.Flush(); exit 7".into(),
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = (PathBuf::from("/bin/sh"), vec!["-c".into(), "printf BYE; exit 7".into()]);
        PtySession::spawn(&program, &args, cols, rows)
    }
}

/// The two-line state the plan's trait exists for: the real server passes `ServeState`, and this
/// passes what the test built.
#[derive(Clone)]
struct TestState {
    token: Option<String>,
    spawner: Arc<dyn PtySpawner>,
    /// **The cap is per listener, not a process-wide counter.** A `static` counter would make these
    /// tests race each other — they run in parallel — and it would let two simultaneous upgrades past
    /// a check-then-act in production, where the cap is a memory bound rather than a suggestion.
    terminals: Arc<Semaphore>,
}

impl TestState {
    fn new(token: Option<&str>, spawner: Arc<dyn PtySpawner>) -> Self {
        Self {
            token: token.map(str::to_string),
            spawner,
            terminals: Arc::new(Semaphore::new(MAX_TERMINAL_CONNECTIONS)),
        }
    }
}

impl TerminalRouteState for TestState {
    fn websocket_token(&self) -> Option<String> {
        self.token.clone()
    }

    fn terminal_spawner(&self) -> Arc<dyn PtySpawner> {
        Arc::clone(&self.spawner)
    }

    fn terminal_gate(&self) -> Arc<Semaphore> {
        Arc::clone(&self.terminals)
    }
}

/// A listener on `127.0.0.1:0` (no test in this repository hard-codes a port) serving
/// `/terminal/ws`, and the address it bound.
async fn serve(state: TestState) -> std::net::SocketAddr {
    let app = router(Router::new()).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a loopback port");
    let addr = listener.local_addr().expect("a bound address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

/// One terminal socket, speaking the page's half of the wire.
struct Terminal {
    socket: Socket,
    /// Every byte the server sent as a binary frame, exactly as it arrived.
    bytes: Vec<u8>,
    /// The same bytes, lossily as text, for the assertions that are about words.
    text: String,
    /// The text frames the server sent. There is exactly one kind, and it is the exit report.
    control: Vec<String>,
    answered: bool,
    saw_cursor_query: bool,
}

impl Terminal {
    async fn open(addr: std::net::SocketAddr, token: Option<&str>) -> Result<Terminal, String> {
        let query = token.map(|token| format!("?token={token}")).unwrap_or_default();
        let url = format!("ws://{addr}/terminal/ws{query}");
        let opened = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(&url))
            .await
            .map_err(|_| format!("the upgrade of {url} never answered"))?;
        let (socket, _) = opened.map_err(|e| e.to_string())?;
        Ok(Terminal {
            socket,
            bytes: Vec::new(),
            text: String::new(),
            control: Vec::new(),
            answered: false,
            saw_cursor_query: false,
        })
    }

    /// One poll: one frame, at most, and the cursor report the first time ConPTY asks for it.
    /// Returns whether the socket is still there.
    async fn poll(&mut self) -> bool {
        match tokio::time::timeout(Duration::from_millis(200), self.socket.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => {
                self.bytes.extend_from_slice(&bytes);
                self.text.push_str(&String::from_utf8_lossy(&bytes));
                if !self.answered && self.bytes.windows(CURSOR_QUERY.len()).any(|window| window == CURSOR_QUERY) {
                    self.answered = true;
                    self.saw_cursor_query = true;
                    self.send_data(CURSOR_REPORT).await;
                }
                true
            }
            Ok(Some(Ok(Message::Text(text)))) => {
                self.control.push(text.to_string());
                true
            }
            Ok(Some(Ok(_))) => true,
            Ok(Some(Err(_))) | Ok(None) => false,
            Err(_) => true,
        }
    }

    /// The page's `onData`: one text message, the string xterm.js handed over.
    async fn send_data(&mut self, data: &str) {
        self.send_json(serde_json::json!({ "type": "data", "data": data })).await;
    }

    async fn send_resize(&mut self, cols: u16, rows: u16) {
        self.send_json(serde_json::json!({ "type": "resize", "cols": cols, "rows": rows })).await;
    }

    async fn send_json(&mut self, message: serde_json::Value) {
        self.socket
            .send(Message::text(message.to_string()))
            .await
            .expect("the terminal socket must accept a text message");
    }

    /// The page's opening padding frame, **byte for byte and frame for frame**: the page writes
    /// `new TextEncoder().encode("tipsterm-hello-pad".padEnd(256, "-"))` — `tipsterm-hello-pad` is
    /// **18** ASCII bytes (`tipsterm` 8, `-` 1, `hello` 5, `-` 1, `pad` 3), `padEnd` fills to 256 with
    /// `-`, so the value is those 18 bytes and 238 dashes — and `ws.send` sends a `Uint8Array` as a
    /// **binary** frame.
    ///
    /// **Both halves of that were wrong here before, and Finding 1 of the phase review is why.** The
    /// page sent the same string with `ws.send(string)`, which the browser's own API sends as a
    /// **text** frame: the route's `Message::Binary` arm — the only arm that frame has — was never
    /// reached by the page, and the pad survived only because the text branch drops unparseable JSON.
    /// This helper padded by *repeating* the text to 256, so even its bytes were not the page's past
    /// the eighteenth. A shape the page does not send is not a shape this test may pin.
    ///
    /// The round-trip assertion in `raw_bytes_round_trip_in_both_directions` is what makes this frame
    /// worth sending at all: the terminal's own output must never contain `tipsterm-hello-pad`.
    async fn send_padding(&mut self) {
        let mut padding = Vec::with_capacity(256);
        padding.extend_from_slice(b"tipsterm-hello-pad");
        padding.resize(256, b'-');
        assert_eq!(padding.len(), 256, "the wire table's own number");
        assert_eq!(padding[18], b'-', "the eighteenth byte is where the dashes start");
        self.socket
            .send(Message::Binary(padding.into()))
            .await
            .expect("the terminal socket must accept the padding frame");
    }

    async fn wait_for(&mut self, wanted: &str) -> String {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            if self.text.contains(wanted) {
                return self.text.clone();
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the terminal never said {wanted:?}; it said {}",
                tail(&self.text)
            );
            self.poll().await;
        }
    }

    /// Types `line` and waits for `marker` to come back, repeating the line because a line written
    /// before the console has painted is discarded (Task 2.1 measured it, and it is the console
    /// host's behaviour rather than this pump's).
    async fn type_until(&mut self, line: &str, marker: &str) -> String {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            if self.text.contains(marker) {
                return self.text.clone();
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "typing {line:?} never produced {marker:?}; the terminal said {}",
                tail(&self.text)
            );
            self.send_data(&format!("{line}\r")).await;
            for _ in 0..4 {
                self.poll().await;
            }
        }
    }

    /// The child's own pid, as the child printed it, out of this connection's own stream.
    async fn child_pid(&mut self) -> u32 {
        let text = self.wait_for("KEEPALIVE<").await;
        let start = text.rfind("KEEPALIVE<").expect("the marker was found") + "KEEPALIVE<".len();
        let rest = &text[start..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().expect("the child prints its pid as digits")
    }

    /// The last console width the child reported, or 0 if it has reported none yet.
    fn width(&self) -> u16 {
        let widths: Vec<u16> = self
            .text
            .split("WIDTH=")
            .skip(1)
            .filter_map(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
            .filter_map(|digits| digits.parse::<u16>().ok())
            .collect();
        widths.last().copied().unwrap_or(0)
    }

    /// Waits for the child to report a width above `before`.
    async fn wait_for_width_above(&mut self, before: u16) -> u16 {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let width = self.width();
            if width > before {
                return width;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the child never reported a width above {before}; the terminal said {}",
                tail(&self.text)
            );
            self.poll().await;
        }
    }

    /// The same wait, with a newline typed between polls.
    ///
    /// **The child reads its console width once per line**, so a newline is what makes it read again,
    /// and it has to be written *after* the server's debounce has elapsed and the resize has actually
    /// reached the console — a line typed into the window before that reads the old width and parks
    /// again. Repeating it is what makes this an assertion about the pump rather than about timing.
    async fn nudge_until_width_above(&mut self, before: u16) -> u16 {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let width = self.width();
            if width > before {
                return width;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the child never reported a width above {before}; the terminal said {}",
                tail(&self.text)
            );
            self.send_data("\r").await;
            for _ in 0..4 {
                self.poll().await;
            }
        }
    }

    /// The exit code out of the one text frame this socket ever sends, if it has arrived.
    fn exit_code(&self) -> Option<u64> {
        self.control.iter().rev().find_map(|frame| {
            let value: serde_json::Value = serde_json::from_str(frame).ok()?;
            (value.get("type")?.as_str()? == "exit").then(|| value.get("code")?.as_u64())?
        })
    }

    async fn wait_for_exit(&mut self) -> u64 {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            if let Some(code) = self.exit_code() {
                return code;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the server never reported the child's exit; the terminal said {}",
                tail(&self.text)
            );
            self.poll().await;
        }
    }

    /// Hangs up the way a closed tab does, and lets the TCP connection go.
    async fn hang_up(mut self) {
        let _ = self.socket.close(None).await;
    }
}

/// The end of what a child said, for a failure message that is worth reading.
fn tail(text: &str) -> String {
    let mut chars: Vec<char> = text.chars().rev().take(400).collect();
    chars.reverse();
    chars.into_iter().collect()
}

/// Waits for a cheap condition to hold, **yielding to the runtime between checks**.
///
/// The yielding is not politeness. This file's tests and the server under test share one
/// current-thread runtime, and `process::gone` — the shared helper, which polls `Get-Process` through
/// a `std::thread::sleep` loop — blocks that runtime outright. Measured: a socket was hung up on, and
/// the server's own pump never got a slice to notice the close, so the child it should have killed
/// five seconds earlier was still running when `process::gone` gave up. Every wait in this file is an
/// `await` for that reason.
async fn until(mut condition: impl FnMut() -> bool, what: &str) -> bool {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if condition() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            eprintln!("[terminal_ws] timed out waiting for {what}");
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The async twin of `process::gone`: the OS's own answer, asked without starving the server.
async fn child_gone(pid: u32) -> bool {
    until(|| !process::alive(pid), &format!("pid {pid} to go")).await
}

/// The pure half of the token rule, tested without a socket because it is the thing both routes
/// share. **`presented` is the decoded query value**, which is what axum's `Query` extraction hands
/// the handler — the plan's `Some("token=abc")` shape is the raw query and would never match.
#[test]
fn the_token_rule_is_loopback_and_websocket_alike() {
    // No token configured: the door is open, which is §5.1's loopback default.
    assert!(terminal_access(None, None).is_ok());
    assert!(terminal_access(Some("anything"), None).is_ok());
    // A token configured: it must be presented, and it must be the one.
    assert!(terminal_access(Some("abc"), Some("abc")).is_ok());
    assert!(terminal_access(None, Some("abc")).is_err());
    assert!(terminal_access(Some("nope"), Some("abc")).is_err());
    assert!(terminal_access(Some(""), Some("abc")).is_err(), "an empty presentation is not a token");
    // The refusal names the rule, never the value.
    let message = terminal_access(Some("nope"), Some("abc")).unwrap_err();
    assert!(!message.contains("nope") && !message.contains("abc"), "{message}");
}

/// Raw bytes in both directions, through the pump and ConPTY, with the cursor query answered the way
/// a real terminal answers it.
#[tokio::test]
async fn raw_bytes_round_trip_in_both_directions() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(TestState::new(None, Arc::new(EchoSpawner { dir: dir.path().to_path_buf() }))).await;
    let mut terminal = Terminal::open(addr, None).await.expect("the listener must accept the upgrade");

    // The page's own first frame, which the pump must ignore rather than write into the PTY.
    terminal.send_padding().await;

    // A whole line in, and the child's own rendering of it back out. The exact string is what makes
    // this a round trip rather than a proof that something arrived.
    let marker = "ECHO=pump-round-trip";
    terminal.type_until("pump-round-trip", marker).await;

    assert!(
        terminal.saw_cursor_query,
        "the pump never carried ConPTY's ESC[6n to the client, so the gate was never answered: {}",
        tail(&terminal.text)
    );
    assert!(
        !terminal.text.contains("tipsterm-hello-pad"),
        "a padding frame was written into the PTY: {}",
        tail(&terminal.text)
    );
    terminal.hang_up().await;
}

/// `MasterPty::resize` is the only resize call this phase makes, and this is the route that has to
/// carry the page's debounced request to it.
///
/// **The newline is load-bearing**, and Task 2.1 measured why: the child reads its width once per
/// line, so a resize while it is parked in `ReadLine` is invisible until that call returns. The
/// plan's version of this test resized and never sent one, so its assertion could not have passed.
#[cfg(windows)]
#[tokio::test]
async fn a_resize_message_reaches_the_child() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(TestState::new(None, Arc::new(WidthReporterSpawner { dir: dir.path().to_path_buf() }))).await;
    let mut terminal = Terminal::open(addr, None).await.expect("the listener must accept the upgrade");

    let before = terminal.wait_for_width_above(0).await;
    assert!(before > 0, "the child must report the width the pump spawned it with: {}", tail(&terminal.text));

    terminal.send_resize(100, 24).await;
    let after = terminal.nudge_until_width_above(before).await;
    assert!(
        after > before,
        "the page's resize never reached the child: before={before} after={after}; it said {}",
        tail(&terminal.text)
    );
    terminal.hang_up().await;
}

/// The wrong token never upgrades, and neither does a missing one: this route's gate is an HTTP
/// status before the socket exists, so nothing is spawned for a request it refuses.
#[tokio::test]
async fn the_wrong_token_never_upgrades() {
    let spawner = Arc::new(TestSpawner::new());
    let addr = serve(TestState::new(Some(TOKEN), Arc::clone(&spawner) as Arc<dyn PtySpawner>)).await;

    assert!(Terminal::open(addr, None).await.is_err(), "a missing token must not upgrade");

    // One wrong character is enough: the whole value is compared.
    let mut wrong = TOKEN.to_string();
    wrong.pop();
    wrong.push(if TOKEN.ends_with('0') { '1' } else { '0' });
    assert_ne!(wrong, TOKEN, "the wrong token must actually differ");
    assert!(Terminal::open(addr, Some(&wrong)).await.is_err(), "a wrong token must not upgrade");
    assert_eq!(spawner.count(), 0, "a refused request must not spawn a child");

    let right = Terminal::open(addr, Some(TOKEN)).await.expect("the right token must upgrade");
    // The child is spawned by a task of its own, so `count == 1` is waited for rather than asserted
    // the instant the upgrade answered.
    assert!(
        until(|| spawner.count() == 1, "the accepted connection's child").await,
        "an accepted connection never spawned a child"
    );
    right.hang_up().await;
}

/// The cap **observed** rather than asserted in a comment: one more than `MAX_TERMINAL_CONNECTIONS`
/// is answered `503` before the upgrade, nothing is spawned for it, and a permit is released again
/// when a connection ends.
#[tokio::test]
async fn more_than_the_terminal_cap_is_refused_and_a_refusal_spawns_nothing() {
    let spawner = Arc::new(TestSpawner::new());
    let addr = serve(TestState::new(None, Arc::clone(&spawner) as Arc<dyn PtySpawner>)).await;

    let mut held = Vec::new();
    for _ in 0..MAX_TERMINAL_CONNECTIONS {
        held.push(Terminal::open(addr, None).await.expect("a connection inside the cap must upgrade"));
    }
    assert!(
        until(|| spawner.count() == MAX_TERMINAL_CONNECTIONS, "each accepted connection's child").await,
        "each accepted connection has to spawn its own child, and {} did not",
        MAX_TERMINAL_CONNECTIONS
    );

    let refused = match Terminal::open(addr, None).await {
        Ok(terminal) => {
            terminal.hang_up().await;
            panic!("one more than the cap must be refused");
        }
        Err(message) => message,
    };
    assert!(
        refused.contains("503") || refused.contains("Service Unavailable"),
        "the refusal has to be the pre-upgrade status, not a socket that closes under the page: {refused}"
    );
    assert_eq!(spawner.count(), MAX_TERMINAL_CONNECTIONS, "the refused connection spawned a child anyway");

    // The permit goes back when a connection ends, or the second browser could never reload.
    let freed = held.pop().expect("one connection is held");
    freed.hang_up().await;
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        match Terminal::open(addr, None).await {
            Ok(terminal) => {
                terminal.hang_up().await;
                break;
            }
            Err(_) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the cap never released the permit of a connection that ended"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    for terminal in held {
        terminal.hang_up().await;
    }
}

/// The ledger's own ruling for this route: the `/ws` `Origin` check is what keeps a page the user
/// merely visits out of a loopback listener, and `/terminal/ws` needs it on the same reasoning — the
/// child it spawns is a full client of this server, so a foreign page that reached this route could
/// drive that client's own `UpdateConfig`.
#[tokio::test]
async fn a_foreign_origin_never_opens_a_terminal() {
    let spawner = Arc::new(TestSpawner::new());
    let addr = serve(TestState::new(None, Arc::clone(&spawner) as Arc<dyn PtySpawner>)).await;

    /// One upgrade attempt whose `Origin` is set when asked for and absent otherwise — absent is the
    /// CLI, which no browser ever is.
    async fn attach(addr: std::net::SocketAddr, origin: Option<&str>) -> Result<Socket, String> {
        use tokio_tungstenite::tungstenite::{ClientRequestBuilder, http::Uri};
        let uri: Uri = format!("ws://{addr}/terminal/ws").parse().expect("a literal url parses");
        let mut request = ClientRequestBuilder::new(uri);
        if let Some(origin) = origin {
            request = request.with_header("Origin", origin);
        }
        tokio_tungstenite::connect_async(request).await.map(|(socket, _)| socket).map_err(|e| e.to_string())
    }

    let foreign =
        attach(addr, Some("http://evil.example")).await.expect_err("another page's origin must not open a terminal");
    assert!(foreign.contains("403"), "refused before the upgrade, with a status instead of a token: {foreign}");
    assert_eq!(spawner.count(), 0, "a foreign page spawned a terminal child");

    // This listener's own page, under the name this request arrived with, is admitted.
    let mut own = attach(addr, Some(&format!("http://127.0.0.1:{}", addr.port())))
        .await
        .expect("the listener's own page may open a terminal");
    assert!(
        until(|| spawner.count() == 1, "the admitted page's child").await,
        "the admitted page never spawned a child"
    );
    assert_eq!(spawner.count(), 1, "and the foreign page's refusal spawned nothing");
    let _ = own.close(None).await;

    // And no `Origin` at all, which is what a CLI client sends.
    let cli = attach(addr, None).await.expect("a client that sends no Origin is the CLI");
    drop(cli);
}

/// Review Focus 4, on the path that ships: a connection that goes away kills **its** child and leaves
/// the other connection's child running. The pids come from the children themselves, and the OS is
/// asked about them — never the handle this process holds, and never the killer's own lie.
#[tokio::test]
async fn killing_a_socket_takes_its_child_and_leaves_the_other_running() {
    let dir = tempfile::tempdir().unwrap();
    let spawner = Arc::new(KeepAliveSpawner { dir: dir.path().to_path_buf(), started: AtomicUsize::new(0) });
    let addr = serve(TestState::new(None, Arc::clone(&spawner) as Arc<dyn PtySpawner>)).await;

    let mut first = Terminal::open(addr, None).await.expect("the first connection must upgrade");
    let first_pid = first.child_pid().await;
    let mut second = Terminal::open(addr, None).await.expect("the second connection must upgrade");
    let second_pid = second.child_pid().await;
    assert_ne!(first_pid, second_pid, "two connections must own two children");
    assert!(process::alive(first_pid) && process::alive(second_pid), "both children must be running");

    first.hang_up().await;
    assert!(child_gone(first_pid).await, "the child of a connection that ended is still running: pid {first_pid}");
    assert!(process::alive(second_pid), "one socket's teardown killed the other connection's child: pid {second_pid}");

    second.hang_up().await;
    assert!(child_gone(second_pid).await, "the second child outlived its socket: pid {second_pid}");
}

/// The one text frame this socket ever sends carries the child's **own** exit code, not a constant:
/// the child says one word, exits with 7, and 7 is what the page reads. This is also the exit poll's
/// test — the code can only come from the OS, because ConPTY keeps the console's output pipe open
/// after its client exits and the reader therefore never reports EOF.
#[tokio::test]
async fn the_childs_own_exit_code_reaches_the_page() {
    let addr = serve(TestState::new(None, Arc::new(OneShotSpawner))).await;
    let mut terminal = Terminal::open(addr, None).await.expect("the listener must accept the upgrade");

    // The child's last words first, then the notice that it ended: the exit frame may overtake the
    // final chunk on the wire, and this is about both arriving rather than about their order.
    terminal.wait_for("BYE").await;
    let code = terminal.wait_for_exit().await;
    assert_eq!(code, 7, "the exit frame must carry the child's own code; it said {}", tail(&terminal.text));
}
