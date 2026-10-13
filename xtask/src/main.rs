//! `xtask`: the project's own build scripts, so the release steps live in the repository instead of in
//! a README someone has to retype.
//!
//! **Run it as `cargo run -p xtask -- dist`, not `cargo xtask dist`.** Cargo resolves `cargo <name>`
//! by looking for a `cargo-<name>` binary on `PATH`, and this workspace's own binary target is not on
//! it. Measured here: `cargo xtask dist` answers "no such command: xtask" and suggests installing a
//! package. A `.cargo/config.toml` `[alias]` would make the short form work, but that directory is
//! git-ignored in this checkout and does not ship, so the working form is the one above. Task 2.4
//! corrected `server::assets`'s `DIST_COMMAND` — the string the missing-bundle page renders — to the
//! same long form, and `README.md` and `docs/architecture.md` spell it with it.

mod assets;
mod dist;
mod shots;

use std::process::ExitCode;

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let result = match task.as_deref() {
        Some("dist") => dist::run(&repo_root()),
        Some("shots") => shots::run(&repo_root()),
        Some(other) => {
            eprintln!("xtask: unknown task `{other}`; the tasks are `dist` and `shots`");
            return ExitCode::FAILURE;
        }
        None => {
            eprintln!("xtask: which task? `cargo run -p xtask -- dist` or `cargo run -p xtask -- shots`");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
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
/// run from `crates/tipsy-assistant` reported success and left `/terminal` with no xterm.js and a blank
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

// Spec §8.3's four steps are `dist::run`'s: Task 2.3's fetch-and-verify of xterm.js is its first,
// Task 3.6's wasm build and `wasm-bindgen` the next two, and the host binary that embeds `web/` is
// the last. They live in `dist.rs` so that the order is one test (`dist::tests`) rather than four
// comments, and so that `repo_root()` below is the single place the root is resolved from.

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
        // `../../web/` from the tipsy-assistant crate and which `dist` must therefore write into.
        assert!(root.join("web").is_dir(), "the embed's folder lives at {}", root.join("web").display());
        assert!(root.join("web").join("vendor").is_dir(), "and the fetched assets live under it");
    }
}
