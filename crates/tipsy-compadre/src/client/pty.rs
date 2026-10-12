//! The pseudo-terminal that owns a web terminal's child process (spec §7.1–§7.3).
//!
//! One `PtySession` per `/terminal/ws` connection, one child per session. `portable-pty`'s reader and
//! writer are **std-blocking** (`Box<dyn Read + Send>`, `Box<dyn Write + Send>`, research note §5.2)
//! and the crate does not depend on tokio, so this module owns no async I/O at all: the connection
//! pump spawns a blocking task for the reader and another for the writer, and this type just hands
//! the two ends over.
//!
//! **Teardown is explicit and there is deliberately no `Drop`.** Review Focus 4's whole point is that
//! a connection that goes away must kill the child, drop the writer, drop the master and let the
//! reader observe EOF — so a `Drop` that killed would let a later refactor delete the pump's own
//! teardown and still pass every test that only checks "the child is gone", which is exactly the
//! accident that focus exists to catch. It would also make the integration test's child, which has to
//! outlive a dropped session handle, impossible to write.
//!
//! **What a ConPTY child does when a connection dies, measured on this machine** (Windows 11,
//! `portable-pty` 0.9.0, `cmd.exe` as the child): `kill` is `TerminateProcess`, and the OS then
//! reports the pid gone; dropping the writer alone was enough to end `cmd.exe` (`STATUS_CONTROL_C_EXIT`,
//! `0xC000013A`), and dropping the whole session closed the console and ended it too. Neither of those
//! is a contract — the plan's own warning is that a ConPTY child does not always die of a closed
//! stdin — so the pump's teardown kills first and asks the OS afterwards, which is what `tests/pty.rs`
//! asserts.

use std::io::{Read, Write};
use std::sync::Mutex;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// The two `portable-pty` handles, named once so the server's pump does not spell the boxes out.
pub type BoxedRead = Box<dyn Read + Send>;
pub type BoxedWrite = Box<dyn Write + Send>;

/// A child running inside a real pseudo-terminal.
///
/// **The master lives in a `Mutex`, and that is not the plan's `Arc`.** `PtyPair.master` is a
/// `Box<dyn MasterPty + Send>`: `MasterPty` has no `Sync` supertrait, so `dyn MasterPty + Send` is not
/// `Sync` and an `Arc` of it is not `Send` at all — measured, `error[E0277]: (dyn MasterPty + Send)
/// cannot be shared between threads safely`. A `PtySession` has to cross into a task (the pump of
/// Task 2.2 holds one), and `Mutex<T: Send>` is both `Send` and `Sync` without an `unsafe impl`, so the
/// master goes behind a lock. No lock is ever held across a blocking read: the reader and the writer
/// are *taken out* of their slots, and only `resize` and the two takes ever lock this one.
pub struct PtySession {
    master: Mutex<Box<dyn MasterPty + Send>>,
    reader: Mutex<Option<BoxedRead>>,
    writer: Mutex<Option<BoxedWrite>>,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl PtySession {
    /// `program` is launched with exactly `args` — no shell, no command-line parsing, so a path with a
    /// space and an argument with a space are both safe.
    pub fn spawn(program: &std::path::Path, args: &[String], cols: u16, rows: u16) -> Result<PtySession, String> {
        Self::spawn_with_env(program, args, &[], cols, rows)
    }

    /// The same, with `NAME=value` pairs added to the child's own environment — which
    /// `portable-pty` bases on this process's environment and then overlays (its `CommandBuilder`
    /// starts from `std::env::vars_os`), so a variable the server was started with reaches the child
    /// and one named here wins over it.
    ///
    /// **No API key may ever travel this way** (spec §9.3). The tests use it to give the child a
    /// configuration directory of their own and to hand it a `PATH` they control.
    pub fn spawn_with_env(
        program: &std::path::Path,
        args: &[String],
        env: &[String],
        cols: u16,
        rows: u16,
    ) -> Result<PtySession, String> {
        let system = native_pty_system();
        let pair = system.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| {
            format!(
                "no pseudo-terminal could be opened ({e}); the web terminal needs Windows 10 1809 \
                 or later for ConPTY, or a Unix pty"
            )
        })?;

        // `program` is an absolute path decided by the caller — in production the one
        // `std::env::current_exe()` returned. It is never a bare name, because `portable-pty`
        // *does* search `PATH` for a name without a path in it (`CommandBuilder::search_path`, which
        // joins every `PATH` entry and tries `PATHEXT`'s extensions), and a `PATH` entry that is
        // empty, relative or points into a checked-out workspace must not be able to plant the child
        // that then runs with the user's authority (prior art:
        // `.superpowers/sdd/plan-3-research/codewhale-audio-prior-art.md` §5). An absolute path also
        // defeats that search by construction: `Path::join` replaces the base when the argument is
        // absolute, so every candidate it builds is the path itself.
        let mut command = CommandBuilder::new(program);
        // Each argument is appended on its own: `CommandBuilder` quotes the vector itself, so a path
        // or a token containing a space cannot be split.
        command.args(args);
        for pair in env {
            if let Some((name, value)) = pair.split_once('=') {
                command.env(name, value);
            }
        }
        let child = pair.slave.spawn_command(command).map_err(|e| format!("the terminal child could not be started: {e}"))?;

        let reader = pair.master.try_clone_reader().map_err(|e| format!("no PTY reader: {e}"))?;
        // Taken here and nowhere else: "It is invalid to take the writer more than once."
        let writer = pair.master.take_writer().map_err(|e| format!("no PTY writer: {e}"))?;
        // The slave end goes with the child: keeping it open would keep the master from ever seeing
        // EOF. On this platform the slave shares the console with the master rather than owning it,
        // so dropping it closes nothing the session still needs (measured: the child runs on).
        drop(pair.slave);

        Ok(PtySession {
            master: Mutex::new(pair.master),
            reader: Mutex::new(Some(reader)),
            writer: Mutex::new(Some(writer)),
            child: Mutex::new(Some(child)),
        })
    }

    /// The only resize call this project makes. `MasterPty::resize`'s own doc says it "will update the
    /// winsize information maintained by the kernel, and generate a signal for the child to notice and
    /// update its state" — `SIGWINCH` on Unix, `ResizePseudoConsole` behind ConPTY on Windows. There is
    /// **no `libc::kill` anywhere in this phase** (spec §7.2); this call is the whole mechanism, and
    /// `tests/pty.rs` reads the new width back out of a child that asks the OS for it.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let master = self.master.lock().unwrap_or_else(|e| e.into_inner());
        master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| format!("the PTY could not be resized to {cols}x{rows}: {e}"))
    }

    /// The reader, once. Dropping it does not touch the child; dropping the master does.
    pub fn take_reader(&self) -> Option<BoxedRead> {
        self.reader.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// The writer, once. Dropping it sends EOF to the slave end (research note §5.1).
    pub fn take_writer(&self) -> Option<BoxedWrite> {
        self.writer.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// The child's pid, for a test that has to ask the OS whether it is still there.
    pub fn process_id(&self) -> Option<u32> {
        let child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        child.as_ref().and_then(|child| child.process_id())
    }

    /// Whether the child has ended, without blocking on it. `None` means it is still running.
    ///
    /// **This is the only way a pump can learn that a web terminal's child exited**, and ConPTY is
    /// why: the pseudoconsole keeps the console's output pipe open after its client exits, so the
    /// blocking reader parked on the master observes EOF only when the **master** is dropped — which
    /// the pump does after its loop, not before it. Measured on this machine with a PowerShell child
    /// that printed one line and exited: the pump's own reader stayed parked for the whole twenty
    /// seconds it waited, so a page would have shown a dead terminal for ever.
    ///
    /// It is safe to poll: on Windows it is a `GetExitCodeProcess`, and on Unix it is a
    /// `waitpid(…, WNOHANG)`, whose status `std::process::Child` caches — so it neither blocks nor
    /// steals the status a later [`PtySession::wait`] needs.
    pub fn try_wait(&self) -> Result<Option<portable_pty::ExitStatus>, String> {
        let mut guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(child) => child.try_wait().map_err(|e| format!("the terminal child's status could not be read: {e}")),
            None => Ok(None),
        }
    }

    /// An independent handle to the child's `kill`, so the pump can terminate the child from a
    /// `&self` borrow instead of taking the whole session apart. `portable-pty` provides it "in order
    /// to send it signals independently from a thread that may be blocked in `.wait`"
    /// ([`ChildKiller::clone_killer`](https://docs.rs/portable-pty/0.9.0/portable_pty/trait.ChildKiller.html)).
    ///
    /// **On Windows this handle's `kill` reports a failure after succeeding**, because
    /// `portable-pty` 0.9.0's `WinChildKiller` tests `TerminateProcess`'s result the wrong way round
    /// (`if res != 0 { Err(..) }`). Measured: `Err(Os { code: 6, message: "Controlador no válido." })`
    /// while the OS reported the pid gone a moment later. The pump must not read that error as "the
    /// child is still running"; ask [`PtySession::process_id`] and the OS instead.
    pub fn killer(&self) -> Option<Box<dyn ChildKiller + Send + Sync>> {
        let child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        child.as_ref().map(|child| child.clone_killer())
    }

    /// Terminates the child. Idempotent. Does not block on reaping — that is [`PtySession::wait`]'s
    /// job.
    ///
    /// `Child::kill` is reachable because `Child: ChildKiller`, and that spelling is the one that
    /// reports a real answer on Windows: it is `TerminateProcess`, then `Ok(())` either way, while the
    /// `ChildKiller` returned by [`PtySession::killer`] inverts the result. A failed kill is therefore
    /// invisible here, which is why teardown follows it with [`PtySession::wait`].
    pub fn kill(&mut self) -> Result<(), String> {
        let mut guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(child) => child.kill().map_err(|e| format!("the terminal child could not be killed: {e}")),
            None => Ok(()),
        }
    }

    /// Reaps the child, and takes it out of the session: a second call is an error rather than a
    /// second wait on a handle that is already gone. Blocks, so production calls it from
    /// `spawn_blocking`; the tests call it after `kill`.
    pub fn wait(&mut self) -> Result<portable_pty::ExitStatus, String> {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner()).take();
        match slot.as_mut() {
            Some(child) => child.wait().map_err(|e| format!("the terminal child could not be reaped: {e}")),
            None => Err("the terminal child was already reaped".to_string()),
        }
    }

    /// `kill` then `wait`: the pair that proves the process is gone rather than merely disowned.
    pub fn kill_and_wait(&mut self) -> Result<portable_pty::ExitStatus, String> {
        self.kill()?;
        self.wait()
    }
}

/// What a PTY is spawned from. The server's `/terminal/ws` handler takes one of these so that its
/// tests run a real PTY over a command that is not the project's binary, while `serve` uses
/// [`TerminalSpawner`] below. Object-safe on purpose: `server::conn` holds one through a `dyn`.
pub trait PtySpawner: Send + Sync + 'static {
    /// Built per connection: the token can be regenerated while the server runs, so this is a method
    /// and not a field.
    fn args(&self) -> Vec<String>;
    /// `NAME=value` pairs for the child's own environment. Empty in production.
    fn env(&self) -> Vec<String> {
        Vec::new()
    }
    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String>;
}

/// The production spawner: **this process's own binary in `--connect` mode, pointed at this process's
/// own loopback `/ws`** (spec §7.2, W10). `program` is `std::env::current_exe()` — the absolute path
/// of the running executable, resolved by the operating system rather than searched for in `PATH` —
/// and `args` are the `--connect`/`--token` arguments of Task 1.7.
///
/// The child is a *client*, exactly like a user on another machine, so keys, providers, history and
/// the credential store stay in the parent and the child's audio is the server machine's.
#[derive(Default)]
pub struct TerminalSpawner {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
    pub env: Vec<String>,
}

impl PtySpawner for TerminalSpawner {
    fn args(&self) -> Vec<String> {
        self.args.clone()
    }

    fn env(&self) -> Vec<String> {
        self.env.clone()
    }

    /// `self.program` is `std::env::current_exe()` in production (spec §7.2, W10) — the running
    /// executable's own path, which the OS resolved when the process started. No `PATH` lookup, no
    /// relative path, no bare `"tipsy-compadre"`.
    fn spawn(&self, cols: u16, rows: u16) -> Result<PtySession, String> {
        PtySession::spawn_with_env(&self.program, &self.args, &self.env, cols, rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The child's argument vector is built by the caller, so the only thing this unit can pin is
    /// that `TerminalSpawner` hands its own program, arguments and environment through unchanged and
    /// spawns the program it was given. Building that program is the caller's job, and it is
    /// `std::env::current_exe()` — never a name (Task 2.1's integration test proves the difference).
    #[test]
    fn a_spawner_passes_its_program_arguments_and_environment_through() {
        let spawner = TerminalSpawner {
            program: std::path::PathBuf::from(r"C:\does\not\exist\tipsy-compadre.exe"),
            args: vec!["--connect".into(), "ws://127.0.0.1:7070/ws".into()],
            env: vec!["TIPSY_PTY_PROBE=unit".into()],
        };
        assert_eq!(spawner.args(), vec!["--connect".to_string(), "ws://127.0.0.1:7070/ws".to_string()]);
        assert_eq!(spawner.env(), vec!["TIPSY_PTY_PROBE=unit".to_string()]);
        assert!(spawner.spawn(80, 24).is_err(), "a program that does not exist cannot be spawned");
    }

    /// A `PtySession` has to cross into a task: the pump of Task 2.2 holds one beside an axum
    /// connection. This is a compile-time assertion, and it stops compiling if the master's
    /// representation goes back to the `Arc<dyn MasterPty + Send>` the plan named — which is **not**
    /// `Send` (`dyn MasterPty + Send` is not `Sync`, so `Arc` of it is not `Send`), and which cost
    /// `error[E0277]` when it was measured.
    #[test]
    fn a_pty_session_can_cross_into_a_task() {
        fn assert_send_and_sync<T: Send + Sync>() {}
        assert_send_and_sync::<PtySession>();
        assert_send_and_sync::<TerminalSpawner>();
        let _: Box<dyn PtySpawner> = Box::new(TerminalSpawner::default());
    }
}
