//! `cargo run -p xtask -- dist`: the WebAssembly bundle first, then the binary that embeds it.
//!
//! The order is spec §8.3's, verbatim: "fetch and verify xterm.js (W5), `cargo build --target
//! wasm32-unknown-unknown -p tipsy-web --release`, `wasm-bindgen --target web` (CLI version pinned to
//! the locked crate, because a schema mismatch is enforced), then the host binary with `web/`
//! embedded." The four steps are named by [`Step`] so the order is one test and not four comments.
//!
//! **The command is `cargo run -p xtask -- dist`, never `cargo xtask dist`.** See this binary's own
//! module doc: cargo resolves `cargo <name>` by looking for a `cargo-<name>` binary on `PATH`, and this
//! workspace's own binary target is not there. `server::assets`'s `DIST_COMMAND` — the string the
//! missing-bundle page renders — spells the long form, and so does `README.md`.

use std::path::{Path, PathBuf};

/// The four steps, in the order they must run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Task 2.3's fetch-and-verify of the pinned xterm.js (W5).
    Fetch,
    /// `cargo build --target wasm32-unknown-unknown -p tipsy-web --release`.
    BuildWasm,
    /// `wasm-bindgen --target web --out-dir <bundle>`.
    Bindgen,
    /// `cargo build -p tipsy-assistant --release`, which embeds the bundle Task 1.5's `assets.rs` names.
    BuildHost,
}

/// The pages' own assets plus the bundle's two files, named **as the two pages load them**.
///
/// `/assets/vendor/xterm.js` is the *package*'s `"main"`, which no task fetches — Task 2.3 pins the
/// `"module"` build `xterm.mjs`, the separate stylesheet `xterm.css` (without it the terminal has no
/// layout at all) and `addon-fit.mjs`. A `REQUIRED` list naming `xterm.js` made this check fail
/// forever, and because [`Step::BuildHost`] refuses the host build when anything is missing, the
/// bundle could never be called complete.
///
/// `tipsy_web.js` and `tipsy_web_bg.wasm` are the names `wasm-bindgen` derives from the input file's
/// stem — INFERRED, and [`check_bundle_names`] re-reads the directory after the command and reports
/// the mismatch rather than trusting this list.
pub const REQUIRED: [&str; 7] = [
    "web/client/index.html",
    "web/terminal/index.html",
    "web/dist/tipsy_web.js",
    "web/dist/tipsy_web_bg.wasm",
    "web/vendor/xterm.mjs",
    "web/vendor/xterm.css",
    "web/vendor/addon-fit.mjs",
];

pub fn steps() -> [Step; 4] {
    // Spec §8.3's order. The host build is last because `rust-embed` reads the folder when the crate
    // is compiled: "In `release` … the folder path is resolved relative to where `Cargo.toml` is"
    // (rust-embed README, quoted in the research note §2.1), so the bundle must exist before it.
    [Step::Fetch, Step::BuildWasm, Step::Bindgen, Step::BuildHost]
}

/// Cargo's target directory for this run: the environment's own `CARGO_TARGET_DIR` when it names one,
/// and `target/` under the workspace root otherwise. The same value is passed to both `cargo`
/// invocations as `--target-dir`, so the artifact's path is derived once and never guessed twice.
///
/// The environment is read in the caller and passed in, because a test that sets an environment
/// variable is a test that races every other test in the binary.
pub fn target_dir(root: &Path, from_env: Option<std::ffi::OsString>) -> PathBuf {
    match from_env {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        // `.cargo/config.toml`'s `build.target-dir` is deliberately not read: both `cargo` calls below
        // pass `--target-dir` explicitly, and a `G:` directory in this checkout is the trap the
        // constraints file records (artifacts there are low-integrity). The plan's own commands set
        // `CARGO_TARGET_DIR`, which is what the operator will have.
        _ => root.join("target"),
    }
}

/// Where cargo puts the wasm build: `<target-dir>/wasm32-unknown-unknown/release/tipsy_web.wasm` —
/// cargo's documented layout for a package named `tipsy-web`. INFERRED (`<crate-name>.wasm` from the
/// crate name, with `-` written as `_`); [`run`] checks it against the file system before
/// `wasm-bindgen` is asked to read the file.
pub fn wasm_artifact(target_dir: &Path) -> PathBuf {
    target_dir.join("wasm32-unknown-unknown").join("release").join("tipsy_web.wasm")
}

/// The `wasm-bindgen` version in `Cargo.lock`, which is what pins the CLI: wasm-bindgen and
/// wasm-bindgen-cli "are required to have the exact same SCHEMA_VERSION in order to work together" and
/// the 0.2.129 release notes say "The CLI's schema version mismatch error now names the input Wasm file
/// and the path of the running `wasm-bindgen` binary" (research note §3.3) — so the pin is not optional.
pub fn locked_wasm_bindgen(lock: &str) -> Option<String> {
    let mut in_package = false;
    let mut name_matches = false;
    for line in lock.lines().map(str::trim) {
        if line == "[[package]]" {
            in_package = true;
            name_matches = false;
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(value) = line.strip_prefix("name = ") {
            name_matches = value.trim_matches('"') == "wasm-bindgen";
        } else if name_matches && let Some(value) = line.strip_prefix("version = ") {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

/// The files the two pages load, missing from the tree rooted at `root`. Empty when the bundle is
/// complete.
///
/// **It asks about each *file* and not about a directory holding it**, which is the Phase 2 lesson:
/// `web/vendor/` exists in a fresh clone because its `README.md` is committed, so a guard on
/// `.is_dir()` is always true and a skip under it is dead code. `web/dist/` is *not* committed at all,
/// so a checkout where `dist` has never run fails this honestly instead of passing quietly.
pub fn missing_assets(root: &Path, _bundle: &Path) -> Vec<String> {
    REQUIRED.iter().filter(|relative| !root.join(relative).is_file()).map(|relative| relative.to_string()).collect()
}

/// **The two inferred names, checked rather than trusted.** `wasm-bindgen` derives `tipsy_web.js` and
/// `tipsy_web_bg.wasm` from the input file's stem, and that rule is INFERRED — so after the tool has
/// run this asks the directory what it holds and, when the two names are not there, says **what is**.
/// `missing_assets` would report the same absence at [`Step::BuildHost`]; this reports it here, one
/// step earlier, with the evidence that names the correction (`REQUIRED` and the page's `import init`
/// line, which change together).
///
/// **The extra files `wasm-bindgen --target web` writes are not a mismatch**: `tipsy_web.d.ts`,
/// `tipsy_web_bg.wasm.d.ts` and `package.json` are the tool's own output, and a gate that refused them
/// would refuse every real run.
pub fn check_bundle_names(root: &Path, bundle: &Path) -> Result<(), String> {
    let missing: Vec<&str> = REQUIRED
        .iter()
        .copied()
        .filter(|relative| relative.starts_with("web/dist/"))
        .filter(|relative| !root.join(relative).is_file())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut written = bundle_names(bundle)?;
    written.sort();
    let written = if written.is_empty() { "it wrote nothing".to_string() } else { written.join(", ") };
    Err(format!(
        "wasm-bindgen ran, but the bundle does not hold the names this build infers: {} is missing. \
         Under {} there is: {written}. The two names come from the input file's stem, so correct \
         `REQUIRED` in xtask/src/dist.rs and the `import init from ...` line in web/client/index.html \
         together, or check that `--out-dir` names the directory the embed reads.",
        missing.join(", "),
        bundle.display(),
    ))
}

/// Empties the bundle directory before `wasm-bindgen` writes into it.
///
/// **A stale file under an old name would be served for ever** (the phase review's nit):
/// `web/dist/` is what the embed's own `/assets/dist/…` route answers from, and nothing — not
/// [`REQUIRED`], not a page — would ever name a renamed asset again, so a build would keep serving a
/// file nobody builds. `wasm-bindgen` writes its four files by name and never removes anything.
///
/// **The guard is the path's own tail, because this deletes a directory.** `bundle` is
/// `<workspace-root>/web/dist` at every call site, so a path that does not end that way is a caller's
/// bug and this refuses instead of deleting whatever it was handed.
pub fn clear_bundle(bundle: &Path) -> Result<(), String> {
    if !bundle.ends_with(Path::new("web").join("dist")) {
        return Err(format!("{} is not the web bundle directory, so it is not emptied", bundle.display()));
    }
    match std::fs::remove_dir_all(bundle) {
        Ok(()) => Ok(()),
        // Not there yet is the ordinary first run, and not an error.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{}: {error}", bundle.display())),
    }
}

/// The file names directly under the bundle directory — the evidence a name mismatch is diagnosed
/// from. Subdirectories are not listed: the two inferred names are files.
fn bundle_names(bundle: &Path) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(bundle).map_err(|error| format!("{}: {error}", bundle.display()))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", bundle.display()))?;
        if entry.path().is_file() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

/// The bundle is not built: the error names the file and the command that produces it.
pub fn ensure_wasm_built(artifact: &Path) -> Result<(), String> {
    if artifact.is_file() {
        return Ok(());
    }
    Err(format!(
        "{} does not exist: the wasm bundle was not built, and a binary built without it would serve \
         `page_missing()` instead of the web client. Install the target with `rustup target add \
         wasm32-unknown-unknown` (Task 3.1) and build it with `cargo build --target \
         wasm32-unknown-unknown -p tipsy-web --release` (the bundle's own path: {}).",
        artifact.display(),
        artifact.parent().map(|p| p.display().to_string()).unwrap_or_default(),
    ))
}

/// What to tell a user whose `wasm-bindgen` is not the locked one, or `None` when it is.
///
/// The CLI's version has to be the locked crate's, because the schema check is enforced and its error
/// names the input file and the running binary rather than the fix. `installed` is the whole stdout of
/// `wasm-bindgen --version` — `wasm-bindgen 0.2.129` today — and its last whitespace-separated token is
/// the version. Split out from [`check_bindgen_version`] so the message is a host test and not only
/// something a person sees once, at the end of a slow build.
pub fn bindgen_version_error(installed: &str, locked: &str) -> Option<String> {
    let found = installed.split_whitespace().last().unwrap_or("");
    if found == locked {
        return None;
    }
    let found = if found.is_empty() { "nothing".to_string() } else { found.to_string() };
    Some(format!(
        "wasm-bindgen {found} is installed and Cargo.lock pins {locked}. Install the matching CLI:\n    \
         cargo install -f wasm-bindgen-cli --version {locked}"
    ))
}

pub fn run(root: &Path) -> Result<(), String> {
    let target = target_dir(root, std::env::var_os("CARGO_TARGET_DIR"));
    let bundle = root.join("web").join("dist");
    let web = root.join("web");

    for step in steps() {
        match step {
            // Task 2.3's entry point, called by the name its own module gives it (`assets::fetch_all`),
            // not re-implemented here: the pinned xterm.js version and its two digests are Task 2.3's
            // and are never invented here. `web/vendor/README.md` records what was fetched, from where
            // and under what licence (spec §12).
            Step::Fetch => {
                let vendor = web.join("vendor");
                crate::assets::fetch_all(&vendor)?;
                // The absolute path, because Finding 4 of the phase review is that a relative one
                // lands wherever the shell happens to stand while `rust-embed` reads this directory.
                println!("xtask: {} is ready", vendor.display());
            }
            Step::BuildWasm => {
                cargo(&["build", "--target", "wasm32-unknown-unknown", "-p", "tipsy-web", "--release"], &target)?;
                ensure_wasm_built(&wasm_artifact(&target))?;
            }
            Step::Bindgen => {
                let path = root.join("Cargo.lock");
                let lock = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
                let locked = locked_wasm_bindgen(&lock)
                    .ok_or("wasm-bindgen is not in Cargo.lock: add it to tipsy-web before running dist")?;
                check_bindgen_version(&locked)?;
                // Emptied first, so the directory the embed serves holds this build's files and
                // nothing an earlier one left behind (the phase review's nit).
                clear_bundle(&bundle)?;
                std::fs::create_dir_all(&bundle).map_err(|error| format!("{}: {error}", bundle.display()))?;
                let out = bundle.display().to_string();
                let artifact = wasm_artifact(&target).display().to_string();
                run_tool("wasm-bindgen", &["--target", "web", "--out-dir", &out, &artifact])?;
                check_bundle_names(root, &bundle)?;
            }
            Step::BuildHost => {
                let missing = missing_assets(root, &bundle);
                if !missing.is_empty() {
                    return Err(format!(
                        "the bundle is incomplete; missing: {}. Build it before the host binary, or the \
                         embed picks up a directory that cannot serve the web client.",
                        missing.join(", ")
                    ));
                }
                cargo(&["build", "-p", "tipsy-assistant", "--release"], &target)?;
            }
        }
    }

    println!("xtask: the bundle is at {}", bundle.display());
    let binary = target.join("release").join(format!("tipsy-assistant{}", std::env::consts::EXE_SUFFIX));
    if binary.is_file() {
        println!("xtask: {} embeds it", binary.display());
    }
    Ok(())
}

/// Runs a command with the shell's stdio inherited, printing it first: a distribution build that needs
/// the network and several minutes has to say where it is.
fn run_tool(program: &str, args: &[&str]) -> Result<(), String> {
    println!("xtask: {program} {}", args.join(" "));
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .map_err(|error| format!("{program} could not be started: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("{program} {} exited with {status}", args.join(" "))) }
}

/// `cargo`, with `--target-dir` explicit so the artifact's path is the one [`wasm_artifact`] computes
/// rather than whatever a `.cargo/config.toml` says.
fn cargo(args: &[&str], target_dir: &Path) -> Result<(), String> {
    let dir = target_dir.display().to_string();
    let mut full: Vec<&str> = args.to_vec();
    full.push("--target-dir");
    full.push(&dir);
    run_tool("cargo", &full)
}

/// The installed CLI's version against the locked crate's. Its stdout is captured — this is the one
/// place `dist` reads a child's output instead of letting it through.
fn check_bindgen_version(locked: &str) -> Result<(), String> {
    let output = std::process::Command::new("wasm-bindgen")
        .arg("--version")
        .output()
        .map_err(|error| format!("wasm-bindgen could not be started: {error}"))?;
    let printed = String::from_utf8_lossy(&output.stdout);
    match bindgen_version_error(printed.trim(), locked) {
        None => Ok(()),
        Some(message) => Err(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_steps_are_in_spec_8_3s_order() {
        assert_eq!(steps(), [Step::Fetch, Step::BuildWasm, Step::Bindgen, Step::BuildHost]);
    }

    #[test]
    fn the_target_directory_is_the_environments_or_the_workspaces_own() {
        let root = Path::new("E:/w/tipsy-assistant");
        assert_eq!(target_dir(root, None), root.join("target"));
        assert_eq!(target_dir(root, Some("E:/w/.target-sandbox".into())), PathBuf::from("E:/w/.target-sandbox"));
        assert_eq!(target_dir(root, Some("".into())), root.join("target"), "an empty value is not a directory");
    }

    #[test]
    fn the_wasm_artifact_is_cargos_path_for_the_crate() {
        let target = Path::new("E:/w/.target-sandbox");
        assert_eq!(wasm_artifact(target), target.join("wasm32-unknown-unknown").join("release").join("tipsy_web.wasm"));
    }

    #[test]
    fn a_missing_wasm_artifact_fails_readably_instead_of_building_a_bundleless_binary() {
        let missing = Path::new("E:/w/.target-sandbox/wasm32-unknown-unknown/release/tipsy_web.wasm");
        let error = ensure_wasm_built(missing).expect_err("a binary without a bundle is not a distribution");
        assert!(error.contains("tipsy_web.wasm"), "the error names the file: {error}");
        assert!(error.contains("wasm32-unknown-unknown"), "and the target: {error}");
        assert!(error.contains("rustup target add wasm32-unknown-unknown"), "and the step that installs it: {error}");
        assert!(
            error.contains("cargo build --target wasm32-unknown-unknown -p tipsy-web --release"),
            "and the build: {error}"
        );
    }

    #[test]
    fn the_wasm_bindgen_pin_is_read_from_the_lock() {
        // The real entry, quoted from `Cargo.lock:4166-4170`.
        let lock = r#"
[[package]]
name = "wasm-bindgen"
version = "0.2.129"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "9bb54f33acc68fd454578d9820b0bde1a1a3d17aa17bb7b6595806d02886d409"
"#;
        assert_eq!(locked_wasm_bindgen(lock).as_deref(), Some("0.2.129"));
        // `wasm-bindgen-shared` and `wasm-bindgen-macro` come first in the real file and must not be
        // mistaken for the crate: the name is matched whole.
        let neighbours = r#"
[[package]]
name = "wasm-bindgen-macro"
version = "0.2.129"

[[package]]
name = "wasm-bindgen-shared"
version = "0.2.129"
"#;
        assert_eq!(locked_wasm_bindgen(neighbours), None, "a sibling is not the crate");
        assert_eq!(locked_wasm_bindgen(""), None);
    }

    #[test]
    fn the_embed_check_reports_every_asset_the_pages_need() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bundle = root.join("web").join("dist");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(root.join("web").join("client")).unwrap();
        std::fs::write(root.join("web").join("client").join("index.html"), b"").unwrap();
        std::fs::create_dir_all(root.join("web").join("terminal")).unwrap();
        std::fs::write(root.join("web").join("terminal").join("index.html"), b"").unwrap();
        std::fs::write(bundle.join("tipsy_web.js"), b"").unwrap();
        std::fs::write(bundle.join("tipsy_web_bg.wasm"), b"").unwrap();

        // The three files the terminal page actually loads, and the entries are the paths `REQUIRED`
        // spells them with: `web/vendor/…` — the directory the embed reads — and `xterm.mjs`, not the
        // `xterm.js` of the package's `"main"` that was never fetched. The old version of this test
        // expected `vec!["vendor/xterm.js"]`, which is wrong twice over, and then wrote
        // `web/vendor/xterm.js` into a `web/vendor/` it had never created, so it panicked on the
        // unwrap before it could assert anything.
        assert_eq!(
            missing_assets(root, &bundle),
            vec![
                "web/vendor/xterm.mjs".to_string(),
                "web/vendor/xterm.css".to_string(),
                "web/vendor/addon-fit.mjs".to_string(),
            ],
            "the three fetched assets are the ones that are absent"
        );

        std::fs::create_dir_all(root.join("web").join("vendor")).unwrap();
        for name in ["xterm.mjs", "xterm.css", "addon-fit.mjs"] {
            std::fs::write(root.join("web").join("vendor").join(name), b"").unwrap();
        }
        assert!(missing_assets(root, &bundle).is_empty(), "nothing is missing once the fetch has run");
    }

    #[test]
    fn a_wasm_bindgen_that_is_not_the_locked_one_is_told_how_to_install_it() {
        assert_eq!(bindgen_version_error("wasm-bindgen 0.2.129", "0.2.129"), None);
        let error = bindgen_version_error("wasm-bindgen 0.2.128", "0.2.129")
            .expect("a mismatched CLI is refused: the schema check is enforced");
        assert!(error.contains("wasm-bindgen 0.2.128 is installed"), "what is installed: {error}");
        assert!(error.contains("Cargo.lock pins 0.2.129"), "and what is pinned: {error}");
        assert!(
            error.contains("cargo install -f wasm-bindgen-cli --version 0.2.129"),
            "and the command that fixes it: {error}"
        );
    }

    #[test]
    fn the_inferred_bundle_names_are_checked_against_what_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bundle = root.join("web").join("dist");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("tipsy_web.js"), b"").unwrap();
        // `wasm-bindgen --target web` also writes the two type declarations, and they were measured
        // in this tree's own bundle directory (`tipsy_web.d.ts`, 5 169 bytes, and
        // `tipsy_web_bg.wasm.d.ts`, 2 276). They are the tool's own output and not the bundle, so a
        // gate that read them as a name mismatch would refuse every real run.
        std::fs::write(bundle.join("tipsy_web.d.ts"), b"").unwrap();
        std::fs::write(bundle.join("tipsy_web_bg.wasm.d.ts"), b"").unwrap();
        let error =
            check_bundle_names(root, &bundle).expect_err("`tipsy_web_bg.wasm` is the WASM, and it was not written");
        assert!(error.contains("web/dist/tipsy_web_bg.wasm"), "the inferred name that is absent: {error}");
        assert!(error.contains("tipsy_web.js"), "and what is there instead: {error}");

        std::fs::write(bundle.join("tipsy_web_bg.wasm"), b"").unwrap();
        assert!(check_bundle_names(root, &bundle).is_ok(), "both inferred names are present");
    }

    /// **The bundle directory is emptied before `wasm-bindgen` writes into it** (the phase review's
    /// nit): `/assets/dist/…` is served from this directory, and a file an earlier build left under a
    /// renamed asset would be served for ever — `REQUIRED` and the pages name the four files
    /// `wasm-bindgen` writes, never the ones it stopped writing. The guard is tested too, because this
    /// is the one function in the crate that deletes a directory.
    #[test]
    fn the_bundle_directory_is_emptied_before_it_is_written_and_never_by_surprise() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("web").join("dist");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("tipsy_web.js"), b"an older build").unwrap();
        std::fs::write(bundle.join("tipsy_web_old_name.js"), b"an asset nobody builds").unwrap();

        clear_bundle(&bundle).expect("the bundle path is the one `run` builds");
        assert!(!bundle.exists(), "the directory goes, and with it the files it held");
        clear_bundle(&bundle).expect("and clearing it again, before anything created it, is not an error");

        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("tipsy_web.js"), b"this build").unwrap();
        assert_eq!(std::fs::read(bundle.join("tipsy_web.js")).unwrap(), b"this build");

        // The guard: a path that is not the bundle directory is refused rather than deleted, so a
        // future caller with a different root cannot take a directory with it.
        let elsewhere = dir.path().join("web");
        let error = clear_bundle(&elsewhere).expect_err("only a `web/dist` path may be deleted");
        assert!(error.contains("not the web bundle directory"), "{error}");
        assert!(elsewhere.is_dir(), "and the refused path is still there");
    }

    #[test]
    fn the_version_gate_reads_the_installed_cli_and_refuses_a_mismatch() {
        // **The `wasm-bindgen` CLI is not a build artifact of this repository**, and not every machine
        // has it — the README tells you to `cargo install` it. So this asks about the *tool*, says why
        // when it is not there, and does not pretend to have checked anything (the Phase 2 lesson, on a
        // binary rather than a file). What it pins when the tool *is* there is the whole real path: the
        // child process, its stdout, and the refusal a wrong CLI would get.
        let probe = std::process::Command::new("wasm-bindgen").arg("--version").output();
        match probe {
            Err(error) => {
                println!(
                    "skipping: `wasm-bindgen` is not on PATH ({error}). Install it with `cargo install -f wasm-bindgen-cli --version <the version Cargo.lock pins>`"
                );
                return;
            }
            Ok(output) => {
                let printed = String::from_utf8_lossy(&output.stdout);
                println!("`wasm-bindgen --version` printed {:?}", printed.trim());
            }
        }

        let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("Cargo.lock");
        let lock = std::fs::read_to_string(&lock).unwrap_or_else(|error| panic!("{}: {error}", lock.display()));
        let locked = locked_wasm_bindgen(&lock).expect("tipsy-web depends on wasm-bindgen");
        assert!(
            check_bindgen_version(&locked).is_ok(),
            "the installed CLI is the version `Cargo.lock` pins ({locked}), so the gate is open"
        );
        let error = check_bindgen_version("0.0.0-not-the-lock")
            .expect_err("a CLI that is not the locked one is refused before wasm-bindgen runs");
        assert!(
            error.contains("0.0.0-not-the-lock") && error.contains("cargo install -f wasm-bindgen-cli"),
            "the refusal names the pin and the install command: {error}"
        );
    }

    #[test]
    fn the_bundle_the_dist_step_writes_is_the_bundle_the_page_imports() {
        let page = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("web").join("client").join("index.html");
        let page = std::fs::read_to_string(&page).unwrap_or_else(|error| panic!("{}: {error}", page.display()));
        let import = page
            .lines()
            .find(|line| line.contains("import init from"))
            .expect("the page imports the bundle the dist step writes");
        // §8.3's `--out-dir web/dist` is what makes this path resolvable: the embed serves the
        // repository's `web/` under `/assets/`, so `/assets/dist/tipsy_web.js` is
        // `web/dist/tipsy_web.js`, one of the files `REQUIRED` names. Renaming either side — the
        // `--out-dir`, the page's import or `REQUIRED` — breaks this before it breaks a browser.
        let path =
            import.split('"').nth(1).unwrap_or_else(|| panic!("the import line carries its path in quotes: {import}"));
        assert!(
            REQUIRED.contains(&format!("web{}", path.trim_start_matches("/assets")).as_str()),
            "the page imports {path}, which must be `web/` plus the `--out-dir`: {import}"
        );
    }
}
