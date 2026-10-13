//! Test-only helpers for "is this process still running?". Shared by `tests/pty.rs` and
//! `tests/terminal_ws.rs` through `#[path = "support/process.rs"] mod process;`.

#![allow(dead_code)]

use std::process::Stdio;

/// `cmd.exe`'s own absolute path. It is not written as `process::Command::new("cmd.exe")` on purpose:
/// one of the tests in `tests/pty.rs` puts a directory it controls at the front of `PATH`, so every
/// system program these helpers start is named by its full path and never searched for.
#[cfg(windows)]
pub const CMD: &str = r"C:\Windows\System32\cmd.exe";

/// The PowerShell 5.1 the tests use to read a PID out of the OS, again by absolute path. It ships
/// with Windows 10 and later, which is the same floor ConPTY has (research note §5.3:
/// `CreatePseudoConsole` needs 1809).
#[cfg(windows)]
pub const POWERSHELL: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";

/// A relative `.ps1` path turned into the absolute path `powershell.exe -File` wants.
#[cfg(windows)]
pub fn abs(root: &std::path::Path, rel: &str) -> String {
    root.join(rel).to_string_lossy().into_owned()
}

/// Whether the PID is still running, asked of the operating system rather than of a handle we hold:
/// a dropped or disowned handle says nothing. Exit 0 is "alive", exit 1 is "gone".
#[cfg(windows)]
pub fn alive(pid: u32) -> bool {
    let script = format!(
        "$p = Get-Process -Id {pid} -ErrorAction SilentlyContinue; if ($null -eq $p) {{ exit 1 }} else {{ exit 0 }}"
    );
    std::process::Command::new(POWERSHELL)
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The same question on Linux, where a child this test has not reaped yet **still has its `/proc`
/// entry**: it is a zombie, and asking whether the path exists answers "alive" for ever, which is what
/// made `tests/pty.rs`'s `wait_for_exit` spend its whole deadline on a child that had already left and
/// then fail with *the child never exited*. The state field of `/proc/<pid>/stat` is the difference:
/// `Z` is gone-but-unreaped, and gone is what these tests mean.
///
/// The field is read from after the **last** `)`, because the command name between the parentheses is
/// the one field of that file that may contain spaces and parentheses of its own.
#[cfg(target_os = "linux")]
pub fn alive(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    match stat.rsplit(')').next().and_then(|rest| rest.split_whitespace().next()) {
        Some("Z" | "X") => false,
        Some(_) => true,
        // Nothing after the name is a file this platform does not write, and assuming life would hang
        // every caller on its deadline.
        None => false,
    }
}

/// The same question on a platform with neither `Get-Process` nor `/proc`.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// Polls [`alive`] until the PID is gone, up to 50 tries of 100 ms. Returns whether it went away.
pub fn gone(pid: u32) -> bool {
    for _ in 0..50 {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}
