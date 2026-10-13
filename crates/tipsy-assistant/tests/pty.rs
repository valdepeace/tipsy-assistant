//! The PTY against a real child process, on this machine's own ConPTY (Windows 10 1809 or later —
//! `CreatePseudoConsole`'s documented floor). The child is `tipsy-assistant --connect`, exactly what
//! `serve`'s `/terminal/ws` will run (spec §7.2, W10).
//!
//! What each test is for:
//!   * the `--connect` child surviving a socket it cannot reach far enough to build a provider stack
//!     (or, as measured here, failing on that socket with the project's own `FAILURE`, never with
//!     `MISSING_KEY`), with a real keystroke going in and real bytes coming out
//!   * `MasterPty::resize` reaching the child as a changed console width — the only resize call in
//!     this phase, so that no `libc::kill`, `SIGWINCH` or `ResizePseudoConsole` appears in the tree
//!   * teardown (Review Focus 4): the process is **gone**, not merely disowned
//!   * the child is the binary the caller named by absolute path, never a name a poisoned `PATH`
//!     could answer for
//!
//! **ConPTY asks the terminal for the cursor position and then waits for the answer.** Its first byte
//! is `ESC[6n`, and until the terminal replies with a cursor position report it delivers nothing else
//! — measured on this machine: four bytes and an empty configuration directory for as long as the
//! query went unanswered, with the child still alive past the moment it should have exited; and the
//! whole banner the instant `ESC[1;1R` was written. A real terminal answers this (xterm.js, Task 2.4,
//! does it natively), so [`Terminal::tick`] answers it here. A test that does not will read four bytes
//! and time out.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tipsy_assistant::client::pty::{PtySession, PtySpawner, TerminalSpawner};

#[path = "support/process.rs"]
mod process;

/// How long a test will wait for a child to say something before calling it a failure. A ConPTY child
/// takes seconds to say its first word: the console is created, the query above is answered, and only
/// then does a debug-build binary's banner arrive.
const WAIT: Duration = Duration::from_secs(20);

/// Nothing is listening here: `spawn`'s child gives up on the connection at once and, because it is a
/// `--connect` client, never builds a provider, never reads a key and never binds a socket.
const UNREACHABLE: &str = "ws://127.0.0.1:1/ws";

/// The `--token` the child is handed. It reaches a server that is not there, so it authenticates
/// nothing — the point is that the argument is accepted.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// ConPTY's device status report, and the cursor position report that answers it.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_REPORT: &[u8] = b"\x1b[1;1R";

/// The value the planted-`PATH` test marks the process it built, as a directory name so that the
/// child's own `--verbose` line (`log: …`) is what carries it back.
#[cfg(windows)]
const PROBE: &str = "pty-security-probe-4b1e9a";

fn project_binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_tipsy-assistant"))
}

/// One real pseudo-terminal: the session, its writer, and the reader on a thread of its own.
///
/// The reader has to be drained continuously and off the test's thread, for two reasons that each
/// cost a hung test otherwise: `read` on a live master blocks with nothing to read, and a ConPTY
/// child whose output is never drained stops making progress.
struct Terminal {
    session: PtySession,
    writer: Box<dyn Write + Send>,
    out: Arc<Mutex<Vec<u8>>>,
    eof: Arc<Mutex<bool>>,
    answered: bool,
}

impl Terminal {
    /// Spawns through a spawner, exactly as the server's terminal route will (Task 2.2).
    fn spawn(spawner: &dyn PtySpawner) -> Terminal {
        let session = spawner.spawn(80, 24).expect("openpty and spawn_command must work on this machine");
        Terminal::of(session)
    }

    /// Takes the two blocking ends and starts the drain.
    fn of(session: PtySession) -> Terminal {
        let reader = session.take_reader().expect("the reader is taken exactly once");
        let writer = session.take_writer().expect("the writer is taken exactly once");
        let out = Arc::new(Mutex::new(Vec::new()));
        let eof = Arc::new(Mutex::new(false));
        let sink = out.clone();
        let flag = eof.clone();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut chunk = [0u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => {
                        *flag.lock().unwrap() = true;
                        return;
                    }
                    Ok(n) => sink.lock().unwrap().extend_from_slice(&chunk[..n]),
                    Err(_) => {
                        *flag.lock().unwrap() = true;
                        return;
                    }
                }
            }
        });
        Terminal { session, writer, out, eof, answered: false }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("the PTY write must land");
        self.writer.flush().expect("the PTY write must be flushed");
    }

    fn bytes(&self) -> Vec<u8> {
        self.out.lock().unwrap().clone()
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes()).into_owned()
    }

    /// The EOF flag itself, so a test can watch for the reader's end after the session is dropped:
    /// EOF is the third half of teardown, and it is what says the blocking reader let go.
    #[cfg(windows)]
    fn eof_flag(&self) -> Arc<Mutex<bool>> {
        self.eof.clone()
    }

    /// One poll: answers ConPTY's cursor query the first time it appears, the way a terminal does,
    /// then lets a little time pass.
    fn tick(&mut self) {
        let query = self.bytes().windows(CURSOR_QUERY.len()).any(|window| window == CURSOR_QUERY);
        if query && !self.answered {
            self.answered = true;
            self.send(CURSOR_REPORT);
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    /// Waits for `wanted` to appear in everything the PTY has produced, and returns that text.
    fn wait_for(&mut self, wanted: &str) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let text = self.text();
            if text.contains(wanted) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "the child never said {wanted:?}; it was {}; what it said was {}",
                self.state(),
                tail(&text)
            );
            self.tick();
        }
    }

    /// What the OS currently says about the child, for a failure message: "still running" and "gone
    /// without saying that" are different bugs, and the second one is invisible from the PTY alone.
    fn state(&self) -> String {
        match self.session.process_id() {
            Some(pid) => format!("pid {pid}, alive={}", process::alive(pid)),
            None => "a child with no pid".to_string(),
        }
    }

    /// Types `key` until the console echoes it back.
    ///
    /// **A keystroke written before the console has painted does not come back.** Measured: the same
    /// byte that was echoed at the prompt was dropped when it was written immediately after `spawn`,
    /// because the console host discards input it has no line to put in yet. Repeating it is what
    /// makes the assertion about the PTY rather than about a race.
    #[cfg(windows)]
    fn type_until_echoed(&mut self, key: u8) {
        let deadline = Instant::now() + WAIT;
        let wanted = char::from(key).to_string();
        loop {
            if self.text().contains(&wanted) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the console never echoed {key:?}: the input half of the PTY is not reaching the child; it was {}; \
                 what it said was {}",
                self.state(),
                tail(&self.text())
            );
            self.send(&[key]);
            self.tick();
        }
    }

    /// Waits for the OS itself to call the child gone, then reaps it for its exit code. Polling is
    /// what keeps a child that never exits from hanging the test inside a blocking `wait`.
    fn wait_for_exit(&mut self) -> u32 {
        let pid = self.session.process_id().expect("the child has a pid on this platform");
        let deadline = Instant::now() + WAIT;
        while process::alive(pid) {
            assert!(Instant::now() < deadline, "the child never exited; the PTY said {}", tail(&self.text()));
            self.tick();
        }
        self.session.wait().expect("a child the OS calls gone must be reapable").exit_code()
    }

    /// Waits for the child's own width report to go above `before`, and returns the new width.
    #[cfg(windows)]
    fn wait_for_width_above(&mut self, before: u16) -> u16 {
        let deadline = Instant::now() + WAIT;
        loop {
            let width = width_of(&self.text());
            if width > before {
                return width;
            }
            assert!(
                Instant::now() < deadline,
                "the child never reported a width above {before}; the PTY said {}",
                tail(&self.text())
            );
            self.tick();
        }
    }
}

/// The last `WIDTH=` the child has printed, or 0 if it has printed none yet. A TUI repaints, so the
/// last one is the current one.
///
/// The widths are collected rather than read off the end of the iterator: `str::Split` with a
/// closure predicate is not a `DoubleEndedIterator`, so `next_back` does not compile on this chain.
#[cfg(windows)]
fn width_of(bytes: &str) -> u16 {
    let widths: Vec<u16> = bytes
        .split("WIDTH=")
        .skip(1)
        .filter_map(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .filter_map(|digits| digits.parse::<u16>().ok())
        .collect();
    widths.last().copied().unwrap_or(0)
}

/// The end of what a child said, for a failure message that is worth reading.
fn tail(text: &str) -> String {
    let mut chars: Vec<char> = text.chars().rev().take(400).collect();
    chars.reverse();
    chars.into_iter().collect()
}

/// The spawner the first test uses: the real binary, from the absolute path Cargo gives the test, with
/// the production argument vector. It is deliberately **not** the security test's spawner — that one
/// goes through `current_exe()` exactly as `server::run` will, and proves the difference a `PATH`
/// lookup would make.
///
/// The configuration directory is the test's own, and it is handed over in the child's environment:
/// the child must neither read nor create the developer's real
/// `%APPDATA%\tipsy-assistant\config.toml` (and `load_or_create` writes one when the directory is
/// empty, so a test that left this out would touch it). Nothing in production sets that variable;
/// `spawn_with_env` exists for exactly this, and no API key may ever travel this way.
fn spawner(config_dir: &Path) -> TerminalSpawner {
    TerminalSpawner {
        program: project_binary().to_path_buf(),
        args: vec!["--connect".into(), UNREACHABLE.into(), "--token".into(), TOKEN.into(), "--verbose".into()],
        env: vec![format!("TIPSY_CONFIG_DIR={}", config_dir.display())],
    }
}

/// The child is the real binary in `--connect` mode, in a real pseudo-terminal: it says its own
/// banner, its socket layer reports the refusal, and the exit code proves no provider stack was built.
///
/// **The exit code is the half of spec §7.2 that can be observed.** This task's plan asserted that the
/// child would still be *alive* after a failed connection; the tree says otherwise — `run_remote`
/// propagates the bridge's error and `remote_client` exits `FAILURE` — so aliveness proves nothing.
/// What does prove it is *which* failure: a child that had built the provider stack would have exited
/// `MISSING_KEY` (2) from `main.rs`'s bare-run path, and this one exits `1` from the attach that never
/// happened, after naming the address it could not reach.
#[test]
fn the_connect_child_fails_on_its_bridge_and_never_on_a_missing_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut terminal = Terminal::spawn(&spawner(dir.path()));

    // The refusal, not the banner, is what this waits for: ConPTY flushes the two lines at different
    // moments (measured: the banner as the console paints, the refusal as the child exits), and no
    // keystroke is written here — this child never reads stdin, so an echo would say nothing. The
    // input half of the PTY is `killing_the_child_leaves_no_process_behind`'s assertion.
    let text = terminal.wait_for("cannot reach");
    assert!(text.contains("mode: remote TUI"), "the `--connect` arm never ran: {}", tail(&text));
    assert!(
        text.contains(&format!("cannot reach {UNREACHABLE}")),
        "the child never reported its bridge's own refusal: {}",
        tail(&text)
    );

    let code = terminal.wait_for_exit();
    assert_ne!(code, 2, "the child built a provider stack: it exited with MISSING_KEY (spec §7.2)");
    assert_eq!(code, 1, "the attach's own failure is the project's FAILURE code");
}

/// `MasterPty::resize` is the only resize call this phase makes, and this is the test that says it
/// reaches the child: a PowerShell child reads its console width out of the OS, the test resizes the
/// PTY, and the width it prints afterwards is the new one.
///
/// **The newline is load-bearing.** The child reads its width once per line, so a resize while it is
/// parked in `ReadLine` is invisible: the loop cannot re-read until that call returns. Measured on
/// this machine — with no newline the only `WIDTH=` in the output stays the old one however long the
/// test waits; with one, the next line reports 100. This task's plan resized and never sent it, so its
/// assertion could not have passed.
#[cfg(windows)]
#[test]
fn a_resize_reaches_the_child_as_a_new_console_width() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("width.ps1");
    std::fs::write(
        &script,
        "while ($true) { [Console]::Out.Write('WIDTH=' + [Console]::WindowWidth); [Console]::Out.Flush(); $null = [Console]::In.ReadLine() }\r\n",
    )
    .unwrap();

    let args: Vec<String> = vec![
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        process::abs(dir.path(), "width.ps1"),
    ];
    let session = PtySession::spawn(Path::new(process::POWERSHELL), &args, 40, 20)
        .expect("openpty and spawn_command must work on this machine");
    let mut terminal = Terminal::of(session);

    let before = terminal.wait_for_width_above(0);
    assert!(before > 0, "no width was readable: {}", tail(&terminal.text()));

    // A keystroke gives the console something of the child's own to echo, so the reading side is
    // demonstrably live at the moment of the resize rather than buffered.
    terminal.send(b"Z");
    terminal.session.resize(100, 24).expect("resize must succeed");
    terminal.send(b"\r");
    let after = terminal.wait_for_width_above(before);

    terminal.session.kill_and_wait().expect("the resize reporter is killed like any other child");
    assert!(
        after > before,
        "MasterPty::resize is what reaches the child: before={before} after={after}; the PTY said {}",
        tail(&terminal.text())
    );
}

/// Review Focus 4, and the reason this task exists: the process is **gone**, in the OS's own answer,
/// not merely a handle that was dropped.
#[cfg(windows)]
#[test]
fn killing_the_child_leaves_no_process_behind() {
    let session = PtySession::spawn(Path::new(process::CMD), &["/Q".into(), "/K".into(), "pause".into()], 80, 24)
        .expect("openpty and spawn_command must work on this machine");
    let mut terminal = Terminal::of(session);

    // `Z` appears nowhere in what `cmd.exe` prints on its own — its own title is a path ending in
    // `cmd.exe` — so seeing it is the console having echoed the keystroke this test wrote.
    terminal.type_until_echoed(b'Z');

    let pid = terminal.session.process_id().expect("the child has a pid on this platform");
    assert!(process::alive(pid), "the child is running before the teardown");

    // The status itself is not asserted: a terminated ConPTY child reports a code that belongs to the
    // platform's own termination path (`TerminateProcess` on Windows, a signal on Unix), and pinning
    // it here would pin the platform rather than the behaviour. The behaviour is the assertion below.
    let _status = terminal.session.kill_and_wait().expect("kill then wait must reap the child");
    assert!(process::gone(pid), "the PTY child was not reaped: pid {pid} is still running");

    // The third half of teardown: the master goes with the session, and the blocking reader must then
    // observe EOF rather than sit in `read` for the rest of the run. The flag outlives the drop.
    let eof = terminal.eof_flag();
    assert!(!*eof.lock().unwrap(), "the master is still open, so the reader has seen no EOF yet");
    drop(terminal);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !*eof.lock().unwrap() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(*eof.lock().unwrap(), "the blocking reader never observed EOF after the master closed");
}

/// **The child is the binary the caller named, not a name a `PATH` lookup could answer for.** A `PATH`
/// entry — an empty one (`::`), a relative one (`.`) or a repository-local one — is writable by
/// whatever else is in the working directory, and the child here is a full client whose behaviour the
/// browser drives over `/terminal/ws`. This test plants a real executable under the name a lookup
/// would find, and proves it never runs.
///
/// Two spawns, because neither alone is enough:
///   * the **control** goes through this same module with the bare name `TerminalSpawner` must never
///     carry, and shows the planted copy is what a lookup runs — so the plant is not decoration;
///   * the **real** spawn is the caller's absolute path with the same poisoned `PATH`, and the child
///     that runs is the one this test built the environment for.
///
/// The planted file is a copy of a real executable rather than a written `.cmd`: portable-pty's own
/// search joins each `PATH` entry with the name and tries `PATHEXT`'s extensions, and the result is
/// handed to `CreateProcessW`, which cannot run a `.cmd`.
#[cfg(windows)]
#[test]
fn the_child_is_the_binary_the_caller_named_and_a_planted_path_entry_never_runs() {
    let dir = tempfile::tempdir().unwrap();
    let planted = dir.path().join("tipsy-assistant.exe");
    std::fs::copy(process::CMD, &planted).unwrap();
    let poisoned = format!("{};{}", dir.path().display(), std::env::var("PATH").unwrap_or_default());

    // The control. `spawn_with_env` is the same entry point the security test uses, and the bare name
    // is the one thing production must never pass.
    let control_env = vec![format!("PATH={poisoned}")];
    let control = PtySession::spawn_with_env(
        Path::new("tipsy-assistant"),
        &["/c".into(), "echo PLANTED-CONTROL".into()],
        &control_env,
        80,
        24,
    )
    .expect("the planted copy is what a PATH lookup finds");
    let mut control = Terminal::of(control);
    control.wait_for("PLANTED-CONTROL");
    control.session.kill_and_wait().expect("the control child is killed like any other");

    // The real spawn: the absolute path, and the same poisoned `PATH` inside the child's own
    // environment. The probe rides in the configuration directory's name, so the child's own
    // `--verbose` line is what reports it back — a value only the environment this test built can
    // contain.
    let probe_dir = dir.path().join(PROBE);
    let spawner = TerminalSpawner {
        program: project_binary().to_path_buf(),
        args: vec!["--connect".into(), UNREACHABLE.into(), "--verbose".into()],
        env: vec![format!("PATH={poisoned}"), format!("TIPSY_CONFIG_DIR={}", probe_dir.display())],
    };
    let mut terminal = Terminal::spawn(&spawner);
    let text = terminal.wait_for("mode: remote TUI");

    assert!(
        !text.contains("PLANTED-CONTROL"),
        "a binary planted in PATH ran with the user's authority; the PTY said {}",
        tail(&text)
    );
    assert!(
        text.contains(PROBE),
        "the child that ran did not carry this test's own probe, so it was not the process this test \
         built the environment for: {}",
        tail(&text)
    );
    terminal.session.kill_and_wait().expect("the real child is killed like any other");
}
