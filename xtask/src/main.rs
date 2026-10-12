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

/// Spec §8.3's order: fetch and verify xterm.js, then the wasm build, then the host binary that
/// embeds `web/`. Task 3.6 owns the second and third steps; this task owns the first, so that the
/// page Task 2.4 writes has its assets before the bundle has anything to embed.
fn dist() -> ExitCode {
    let vendor = std::path::Path::new("web/vendor");
    match assets::fetch_all(vendor) {
        Ok(()) => {
            println!("xtask: web/vendor is ready");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}
