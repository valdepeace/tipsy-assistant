//! `xtask`: the project's own build scripts, so the release steps live in the repository instead of in
//! a README someone has to retype.
//!
//! **Run it as `cargo run -p xtask -- dist`, not `cargo xtask dist`.** The plan, `server::assets`'s
//! `DIST_COMMAND` and the page that constant renders all spell it `cargo xtask dist`; cargo resolves
//! `cargo <name>` by looking for `cargo-<name>` on `PATH`, and this workspace's own binary target is
//! not on it. Measured here: `cargo xtask dist` answers "no such command: xtask" and suggests
//! installing a package. A `.cargo/config.toml` `[alias]` would make the short form work, but that
//! directory is git-ignored in this checkout and does not ship, so the working form is the one above.

mod assets;

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("dist") => dist(),
        Some(other) => {
            eprintln!("xtask: unknown task `{other}`; the tasks are `dist`");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("xtask: which task? `cargo run -p xtask -- dist`");
            ExitCode::FAILURE
        }
    }
}

/// The repository root, resolved from **this crate's own compile-time location** and never from the
/// process's working directory.
///
/// **Measured, and this is Finding 4 of the phase review.** `dist` used to hand `fetch_all` the
/// relative `Path::new("web/vendor")`. Run from an empty directory, it fetched, verified and printed
/// "web/vendor is ready" into `<that directory>/web/vendor`, while `rust-embed` reads the
/// repository's own `web/` (`` `#[folder = "../../web/"]` `` in `server/assets.rs`) — so the same command
/// run from `crates/tipsy-compadre` reported success and left `/terminal` with no xterm.js and a blank
/// screen. `env!("CARGO_MANIFEST_DIR")` is baked in at compile time, so the assets land in the one
/// directory the embed reads, whatever the shell's current directory is.
///
/// `cargo run -p xtask -- dist` still has to be started from inside the workspace — that is how cargo
/// finds the workspace at all — but it no longer has to be started from the root, and the absolute
/// path it prints says where it wrote.
pub(crate) fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the xtask crate sits directly under the repository root")
        .to_path_buf()
}

/// Spec §8.3's order: fetch and verify xterm.js, then the wasm build, then the host binary that
/// embeds `web/`. Task 3.6 owns the second and third steps; this task owns the first, so that the
/// page Task 2.4 writes has its assets before the bundle has anything to embed.
fn dist() -> ExitCode {
    let vendor = repo_root().join("web").join("vendor");
    match assets::fetch_all(&vendor) {
        Ok(()) => {
            println!("xtask: {} is ready", vendor.display());
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The whole of Finding 4, as an assertion.** `xtask/` has its own `Cargo.toml`, so that alone
    /// proves nothing about where the root is: `Cargo.lock`, `crates/` and the workspace manifest's
    /// own `[workspace]` heading are what only the repository root has. A `repo_root()` that returned
    /// the process's working directory instead — which is what a relative `web/vendor` effectively
    /// did — would answer `xtask/` for every `cargo test -p xtask`, and every assertion below fails
    /// there.
    #[test]
    fn the_repository_root_is_not_the_processes_working_directory() {
        let root = repo_root();
        assert!(!root.ends_with("xtask"), "{} is this crate, not the repository", root.display());
        assert!(root.join("Cargo.lock").is_file(), "{} must be the workspace root", root.display());
        assert!(root.join("crates").is_dir(), "{} must be the workspace root", root.display());
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("the workspace manifest");
        assert!(manifest.contains("[workspace]"), "the root's manifest is the workspace's");
        // And the one directory this matters for: the embed's folder, which `server/assets.rs` spells
        // `../../web/` from the tipsy-compadre crate and which `dist` must therefore write into.
        assert!(root.join("web").is_dir(), "the embed's folder lives at {}", root.join("web").display());
        assert!(root.join("web").join("vendor").is_dir(), "and the fetched assets live under it");
    }
}
