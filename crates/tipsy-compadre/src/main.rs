//! The `tipsy-compadre` binary: run the client, manage the API keys it uses, attach to a remote
//! server, or run one headless.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tipsy_compadre::client;
use tipsy_compadre::config::store;
use tipsy_compadre::keys::{redact, KeyResolver, KeySource, OsKeyring, SecretStore};
use tipsy_compadre::providers::{ProviderSlot, build_providers};

/// Exit code used when a provider cannot be built for want of a key.
const MISSING_KEY: u8 = 2;

// The CLI of spec §3.2. The command's `about` is written out below rather than left to the doc
// comment, because clap folds a doc comment into the help text and this one is a note to the reader
// of the code, not copy for a user.
//
// Task 1.8 registers `--tls`, `--tls-cert` and `--tls-key`, and only then: nothing is accepted
// before it is implemented. `Debug` on all four types below is what the CLI test needs —
// `Cli::try_parse_from(..).unwrap_err()` requires the success half of the `Result` to be `Debug`,
// and the `serve` arm's failure message formats the command it did get.
#[derive(Debug, Parser)]
#[command(name = "tipsy-compadre", version, about = "A tipsy, lazy, joking voice assistant for software developers")]
struct Cli {
    /// Attach the TUI to a remote server instead of running one here.
    #[arg(long, value_name = "WS-URL")]
    connect: Option<String>,
    /// The token the remote server requires. Only read with `--connect`.
    #[arg(long, value_name = "TOKEN", requires = "connect")]
    token: Option<String>,
    /// Say more about what this process is doing. Task 1.9 also routes the log to stderr with it.
    #[arg(long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Manage the API keys the providers use.
    Key {
        #[command(subcommand)]
        command: KeyCmd,
    },
    /// Report where the configuration is kept.
    Config {
        #[command(subcommand)]
        command: ConfigCmd,
    },
    /// Run the server headless: the web client at `/`, the web terminal at `/terminal`.
    Serve {
        /// Override the configured bind address for this run only; the file is never written.
        #[arg(long, value_name = "ADDR")]
        bind: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum KeyCmd {
    /// Store a key in the operating system's credential store.
    Set { name: String },
    /// Report where the key would come from, without printing it.
    Status { name: String },
    /// Remove a key from the operating system's credential store.
    Delete { name: String },
}

#[derive(Debug, Subcommand)]
enum ConfigCmd {
    /// Print the path of `config.toml`.
    Path,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match (cli.connect, cli.command) {
        // `--connect` runs the TUI and nothing else: no server, no providers, no credential store.
        (Some(url), None) => remote_client(url, cli.token, cli.verbose),
        (Some(_), Some(_)) => {
            eprintln!("tipsy-compadre: --connect runs the TUI on its own; it takes no subcommand");
            ExitCode::FAILURE
        }
        (None, None) => run_client(cli.verbose),
        (None, Some(Cmd::Key { command })) => key_command(command),
        (None, Some(Cmd::Config { command })) => config_command(command),
        (None, Some(Cmd::Serve { bind })) => serve_command(bind, cli.verbose),
    }
}

/// `--verbose` before Task 1.9's `tracing` bootstrap: the one line that says which mode this process
/// is in, and where. The token is never printed — only whether one was given (spec §9.2).
fn announce(verbose: bool, line: impl std::fmt::Display) {
    if verbose {
        eprintln!("tipsy-compadre: {line}");
    }
}

/// The bare run: load the configuration, build the providers and hand over to the client.
fn run_client(verbose: bool) -> ExitCode {
    announce(verbose, "mode: server and TUI in one process");
    let path = store::default_path();
    let loaded = match store::load_or_create(&path) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("tipsy-compadre: {}: {e}", path.display());
            return ExitCode::from(MISSING_KEY);
        }
    };
    if let Some(warning) = &loaded.warning {
        eprintln!("tipsy-compadre: {warning}");
    }
    // Building here, and not inside the client, is what turns a missing key into exit code 2.
    match build_providers(&loaded.config.providers, &KeyResolver::system()) {
        Ok(_) => match run_terminal(path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("tipsy-compadre: {e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("tipsy-compadre: {e}");
            ExitCode::from(MISSING_KEY)
        }
    }
}

/// Runs the client on a runtime of its own: the session, the system monitor and the audio bridges
/// are all tasks.
fn run_terminal(path: PathBuf) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("no async runtime: {e}"))?;
    runtime.block_on(async move { client::run(path) })
}

/// The `--connect` run. No configuration is required beyond this machine's own settings, and a
/// refusal is reported before the terminal is entered, so the user reads it on a clean screen.
fn remote_client(url: String, token: Option<String>, verbose: bool) -> ExitCode {
    announce(
        verbose,
        format!(
            "mode: remote TUI; attaching to {url}; token {}",
            if token.is_some() { "given" } else { "not given" }
        ),
    );
    match run_remote_terminal(url, token) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tipsy-compadre: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_remote_terminal(url: String, token: Option<String>) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("no async runtime: {e}"))?;
    runtime.block_on(async move { client::run_remote(url, token).await })
}

/// The `serve` arm: the file's configuration, with `--bind` overriding it for this run only, handed
/// to the server of Task 1.4 — the listener, the router and the bind rule are its.
///
/// The resolver and the connectors are built *here*, in the server process, which is the whole point
/// of the split: the client that attaches over a socket never resolves a key. A file that names a key
/// nobody holds exits with the same code a bare run's does (`MISSING_KEY`).
fn serve_command(bind: Option<String>, verbose: bool) -> ExitCode {
    let path = store::default_path();
    let loaded = match store::load_or_create(&path) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("tipsy-compadre: {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    if let Some(warning) = &loaded.warning {
        eprintln!("tipsy-compadre: {warning}");
    }
    let mut config = loaded.config;
    if let Some(bind) = bind {
        // This run only. `serve --bind` never writes the file back (spec §5.1).
        config.server.bind = bind;
    }
    announce(
        verbose,
        format!(
            "mode: server; bind {}; config {}; token {}",
            config.server.bind,
            path.display(),
            if config.server.has_token() { "set" } else { "not set" }
        ),
    );
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("tipsy-compadre: no async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let keys = KeyResolver::system();
    let providers = match build_providers(&config.providers, &keys) {
        Ok(providers) => ProviderSlot::new(providers),
        Err(e) => {
            eprintln!("tipsy-compadre: {e}");
            return ExitCode::from(MISSING_KEY);
        }
    };
    // `Settings` is Task 1.4's, and this literal is its final shape, all eight fields. `bind`, `lan`
    // and `token` are the three the bind rule and the handshake read; `config`, `config_path`, `keys`
    // and `providers` are what `server::run` builds the `Hub` and every session from; `tls` stays
    // `None` until Task 1.8 fills it in.
    let settings = tipsy_compadre::server::Settings {
        bind: config.server.bind.clone(),
        lan: config.server.lan,
        token: config.server.token.clone(),
        config,
        config_path: path,
        keys,
        providers,
        tls: None,
    };
    // The listener reports the address it actually bound — `--bind 127.0.0.1:0` is a real request for
    // a free port — and `server::run` sends it only after §5.1's whole rule has passed, so a run that
    // ends before this line has proved a refusal rather than a slow start. A `--verbose` run says
    // where the server really is instead of where it was asked to be.
    let (bound, address) = tokio::sync::oneshot::channel();
    runtime.block_on(async move {
        let server = tokio::spawn(tipsy_compadre::server::run(settings, bound));
        if verbose && let Ok(address) = address.await {
            eprintln!("tipsy-compadre: listening on {address}");
        }
        match server.await {
            Ok(Ok(())) => ExitCode::SUCCESS,
            Ok(Err(e)) => {
                eprintln!("tipsy-compadre: {e}");
                ExitCode::FAILURE
            }
            Err(e) => {
                eprintln!("tipsy-compadre: the server task ended: {e}");
                ExitCode::FAILURE
            }
        }
    })
}

fn key_command(command: KeyCmd) -> ExitCode {
    let store = OsKeyring;
    let result = match command {
        KeyCmd::Set { name } => set_key(&name, &store),
        KeyCmd::Status { name } => status_key(&name),
        KeyCmd::Delete { name } => delete_key(&name, &store),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tipsy-compadre: {e}");
            ExitCode::FAILURE
        }
    }
}

fn set_key(name: &str, store: &dyn SecretStore) -> Result<(), String> {
    print!("Paste the key for {name}: ");
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
    let key = line.trim();
    if key.is_empty() {
        return Err(format!("no key was given for `{name}`"));
    }
    store.set(name, key).map_err(|e| e.to_string())?;
    println!("Saved {name} ({})", redact(key));
    Ok(())
}

fn status_key(name: &str) -> Result<(), String> {
    let resolver = KeyResolver::system();
    match resolver.resolve(name).map_err(|e| e.to_string())? {
        Some((key, KeySource::Env)) => println!("env ({})", redact(&key)),
        Some((key, KeySource::Store)) => println!("credential store ({})", redact(&key)),
        None => println!("missing"),
    }
    Ok(())
}

fn delete_key(name: &str, store: &dyn SecretStore) -> Result<(), String> {
    store.delete(name).map_err(|e| e.to_string())?;
    println!("Deleted {name}");
    Ok(())
}

fn config_command(command: ConfigCmd) -> ExitCode {
    match command {
        ConfigCmd::Path => {
            println!("{}", store::default_path().display());
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cli_parses_a_remote_attach_and_the_serve_subcommand() {
        let cli = Cli::try_parse_from([
            "tipsy-compadre",
            "--connect",
            "ws://127.0.0.1:7070/ws",
            "--token",
            "abc",
        ])
        .unwrap();
        assert_eq!(cli.connect.as_deref(), Some("ws://127.0.0.1:7070/ws"));
        assert_eq!(cli.token.as_deref(), Some("abc"));
        assert!(cli.command.is_none());
        assert!(!cli.verbose);

        let cli = Cli::try_parse_from(["tipsy-compadre", "serve", "--bind", "127.0.0.1:0", "--verbose"]).unwrap();
        assert!(cli.verbose, "`--verbose` is global, so it parses on either side of the subcommand");
        match cli.command {
            Some(Cmd::Serve { bind }) => assert_eq!(bind.as_deref(), Some("127.0.0.1:0")),
            other => panic!("`serve` must be a subcommand, got {other:?}"),
        }

        // The three ways in that existed before this task parse exactly as they did.
        assert!(Cli::try_parse_from(["tipsy-compadre"]).unwrap().command.is_none());
        assert!(matches!(
            Cli::try_parse_from(["tipsy-compadre", "config", "path"]).unwrap().command,
            Some(Cmd::Config { .. })
        ));
        assert!(matches!(
            Cli::try_parse_from(["tipsy-compadre", "key", "status", "openrouter"]).unwrap().command,
            Some(Cmd::Key { .. })
        ));
    }

    #[test]
    fn connect_needs_a_url_and_a_token_needs_connect() {
        let error = Cli::try_parse_from(["tipsy-compadre", "--connect"]).unwrap_err();
        assert!(error.to_string().contains("--connect"), "a bare `--connect` must name the flag: {error}");

        let error = Cli::try_parse_from(["tipsy-compadre", "--token", "abc"]).unwrap_err();
        assert!(error.to_string().contains("--connect"), "`--token` without `--connect` is a usage error: {error}");
    }

    #[test]
    fn the_tls_flags_do_not_exist_until_their_task_does() {
        // Task 1.8 registers `--tls`, `--tls-cert` and `--tls-key` and replaces this test: nothing
        // is accepted before it is implemented.
        assert!(Cli::try_parse_from(["tipsy-compadre", "serve", "--tls"]).is_err());
    }
}
