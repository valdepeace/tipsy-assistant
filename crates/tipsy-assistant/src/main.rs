//! The `tipsy-assistant` binary: run the client, manage the API keys it uses, attach to a remote
//! server, or run one headless.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tipsy_assistant::client;
use tipsy_assistant::client::bridge::DisplayUrl;
use tipsy_assistant::config::store;
use tipsy_assistant::keys::{KeyResolver, KeySource, OsKeyring, SecretStore, redact};
use tipsy_assistant::logging;
use tipsy_assistant::providers::{ProviderSlot, build_providers};

/// Exit code used when a provider cannot be built for want of a key.
const MISSING_KEY: u8 = 2;

// The CLI of spec §3.2. The command's `about` is written out below rather than left to the doc
// comment, because clap folds a doc comment into the help text and this one is a note to the reader
// of the code, not copy for a user.
//
// Task 1.8 registers `--tls`, `--tls-cert` and `--tls-key` on `serve`; before it, nothing is accepted
// before it is implemented. `Debug` on all four types below is what the CLI test needs —
// `Cli::try_parse_from(..).unwrap_err()` requires the success half of the `Result` to be `Debug`,
// and the `serve` arm's failure message formats the command it did get.
#[derive(Debug, Parser)]
#[command(name = "tipsy-assistant", version, about = "A tipsy, lazy, joking voice assistant for software developers")]
struct Cli {
    /// Attach the TUI to a remote server instead of running one here.
    #[arg(long, value_name = "WS-URL")]
    connect: Option<String>,
    /// The token the remote server requires. Only read with `--connect`.
    #[arg(long, value_name = "TOKEN", requires = "connect")]
    token: Option<String>,
    /// Say more about what this process is doing, and copy the log to stderr as well.
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
        /// Serve over TLS. The token and a new API key are then safe on a LAN; it does not by itself
        /// unlock the browser microphone from another machine (spec §9.5).
        #[arg(long)]
        tls: bool,
        /// A certificate to serve with. Needs `--tls-key`; implies `--tls`.
        #[arg(long, value_name = "PEM", requires = "tls_key")]
        tls_cert: Option<PathBuf>,
        /// The private key for `--tls-cert`.
        #[arg(long, value_name = "PEM", requires = "tls_cert")]
        tls_key: Option<PathBuf>,
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
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    // Before anything else, so the first event of the run is recorded and every line after it has
    // somewhere to go (spec §9.6). A log that cannot be opened is a line on stderr, not a refusal:
    // the assistant works without its log.
    let config_path = store::default_path();
    match logging::init(&logging::log_path(&config_path), cli.verbose) {
        Ok(path) => announce(cli.verbose, format!("log: {}", path.display())),
        Err(e) => eprintln!("tipsy-assistant: no log file: {e}"),
    }
    // The token a `--connect` run presents is the same secret the server compares, so it is
    // registered before any arm below can say anything about the attach (spec §9.6).
    if let Some(token) = cli.token.as_deref() {
        logging::hide(token);
    }
    match (cli.connect, cli.command) {
        // `--connect` runs the TUI and nothing else: no server, no providers, no credential store.
        (Some(url), None) => remote_client(url, cli.token, cli.verbose),
        (Some(_), Some(_)) => {
            eprintln!("tipsy-assistant: --connect runs the TUI on its own; it takes no subcommand");
            ExitCode::FAILURE
        }
        (None, None) => run_client(cli.verbose),
        (None, Some(Cmd::Key { command })) => key_command(command),
        (None, Some(Cmd::Config { command })) => config_command(command),
        (None, Some(Cmd::Serve { bind, tls, tls_cert, tls_key })) => {
            serve_command(bind, tls, tls_cert, tls_key, cli.verbose)
        }
    }
}

/// `--verbose`'s startup line: which mode this process is in, and where. The token is never printed —
/// only whether one was given (spec §9.2).
///
/// **It is an `eprintln!` and stays one now that `logging` exists.** It answers "what did I just run",
/// which is a line for the terminal it was started from rather than a record of what happened.
/// Everything that *is* a record — the address the listener bound, the certificate's fingerprint,
/// what TLS does not buy — goes through `logging`, and reaches `tipsy-assistant.log` whether or not
/// `--verbose` is passed. This line never reaches a file.
fn announce(verbose: bool, line: impl std::fmt::Display) {
    if verbose {
        eprintln!("tipsy-assistant: {line}");
    }
}

/// The bare run: load the configuration, build the providers and hand over to the client.
fn run_client(verbose: bool) -> ExitCode {
    announce(verbose, "mode: server and TUI in one process");
    let path = store::default_path();
    let loaded = match store::load_or_create(&path) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("tipsy-assistant: {}: {e}", path.display());
            return ExitCode::from(MISSING_KEY);
        }
    };
    if let Some(warning) = &loaded.warning {
        eprintln!("tipsy-assistant: {warning}");
    }
    // The token must never reach a log line, and it is registered the moment it is in hand
    // (spec §9.6). Deliberately *here* and in `serve`, rather than once in `main`: an unconditional
    // `load_or_create` up there would make `config path` and `key status` create the file on a fresh
    // machine and, worse, rename and reset a corrupt one — a read-only subcommand with a
    // destructive side effect.
    logging::hide(&loaded.config.server.token);
    // Building here, and not inside the client, is what turns a missing key into exit code 2.
    match build_providers(&loaded.config.providers, &KeyResolver::system()) {
        Ok(_) => match run_terminal(path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("tipsy-assistant: {e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("tipsy-assistant: {e}");
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

/// The `--verbose` startup line for a `--connect` run: where this process is attaching and whether a
/// token was given. **The URL is a [`DisplayUrl`]**, so a query — which is where a token would be, and
/// the only place one ever was on this path — is not part of it. That is the whole of F1's rule for
/// this line: `eprintln!` never passes through the log's scrubber, so the value printed here has to be
/// safe before it arrives (`--connect` with a token in the URL is what a shell history holds).
fn remote_banner(url: &str, token: Option<&str>) -> String {
    format!(
        "mode: remote TUI; attaching to {}; token {}",
        DisplayUrl::new(url),
        if token.is_some() { "given" } else { "not given" }
    )
}

/// The `--connect` run. No configuration is required beyond this machine's own settings, and a
/// refusal is reported before the terminal is entered, so the user reads it on a clean screen.
fn remote_client(url: String, token: Option<String>, verbose: bool) -> ExitCode {
    announce(verbose, remote_banner(&url, token.as_deref()));
    match run_remote_terminal(url, token) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tipsy-assistant: {e}");
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
///
/// **This function does no file I/O for TLS.** It turns the three flags into the one value
/// `Settings.tls` takes; the loading, the refusal that names a missing pair and the fingerprint all
/// belong to `server::run`'s TLS branch, which is also what keeps a second error type out of this
/// file (spec §5.5, §10).
fn serve_command(
    bind: Option<String>,
    tls: bool,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    verbose: bool,
) -> ExitCode {
    let path = store::default_path();
    let loaded = match store::load_or_create(&path) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("tipsy-assistant: {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    if let Some(warning) = &loaded.warning {
        eprintln!("tipsy-assistant: {warning}");
    }
    // This process is about to hold the token for as long as it runs, so it is registered before
    // anything can log it (spec §9.6). The `--verbose` line below says "set", never the value.
    logging::hide(&loaded.config.server.token);
    let mut config = loaded.config;
    if let Some(bind) = bind {
        // This run only. `serve --bind` never writes the file back (spec §5.1).
        config.server.bind = bind;
    }
    announce(
        verbose,
        format!(
            "mode: server; bind {}; config {}; token {}{}",
            config.server.bind,
            path.display(),
            if config.server.has_token() { "set" } else { "not set" },
            if tls { "; tls" } else { "" }
        ),
    );
    // One of §5.5's three states. `--tls-cert`/`--tls-key` is `Supplied`; `--tls` alone is
    // `Generated`; nothing is `None`. clap has already refused a lone `--tls-cert` or `--tls-key`
    // (`requires`, above), so the `zip` below cannot silently drop a file the user named — and a
    // `(true, None)` is what is left when `--tls` was given with no pair. `TlsFiles` is Task 1.4's
    // type, declared beside `Settings` in `server/mod.rs`; this arm only fills it in.
    let tls = match (tls || tls_cert.is_some(), tls_cert.zip(tls_key)) {
        (false, _) => None,
        (true, Some((cert, key))) => Some(tipsy_assistant::server::TlsFiles::Supplied { cert, key }),
        (true, None) => Some(tipsy_assistant::server::TlsFiles::Generated),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("tipsy-assistant: no async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let keys = KeyResolver::system();
    let providers = match build_providers(&config.providers, &keys) {
        Ok(providers) => ProviderSlot::new(providers),
        Err(e) => {
            eprintln!("tipsy-assistant: {e}");
            return ExitCode::from(MISSING_KEY);
        }
    };
    // `Settings` is Task 1.4's, and this literal is its final shape, all eight fields. `bind`, `lan`
    // and `token` are the three the bind rule and the handshake read; `config`, `config_path`, `keys`
    // and `providers` are what `server::run` builds the `Hub` and every session from.
    let settings = tipsy_assistant::server::Settings {
        bind: config.server.bind.clone(),
        lan: config.server.lan,
        token: config.server.token.clone(),
        config,
        config_path: path,
        keys,
        providers,
        tls,
    };
    // The listener reports the address it actually bound — `--bind 127.0.0.1:0` is a real request for
    // a free port — and `server::run` sends it only after §5.1's whole rule has passed, so a run that
    // ends before this line has proved a refusal rather than a slow start. A `--verbose` run says
    // where the server really is instead of where it was asked to be.
    let (bound, address) = tokio::sync::oneshot::channel();
    runtime.block_on(async move {
        let server = tokio::spawn(tipsy_assistant::server::run(settings, bound));
        if verbose && let Ok(address) = address.await {
            eprintln!("tipsy-assistant: listening on {address}");
        }
        match server.await {
            Ok(Ok(())) => ExitCode::SUCCESS,
            Ok(Err(e)) => {
                eprintln!("tipsy-assistant: {e}");
                ExitCode::FAILURE
            }
            Err(e) => {
                eprintln!("tipsy-assistant: the server task ended: {e}");
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
            eprintln!("tipsy-assistant: {e}");
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
        let cli =
            Cli::try_parse_from(["tipsy-assistant", "--connect", "ws://127.0.0.1:7070/ws", "--token", "abc"]).unwrap();
        assert_eq!(cli.connect.as_deref(), Some("ws://127.0.0.1:7070/ws"));
        assert_eq!(cli.token.as_deref(), Some("abc"));
        assert!(cli.command.is_none());
        assert!(!cli.verbose);

        let cli = Cli::try_parse_from(["tipsy-assistant", "serve", "--bind", "127.0.0.1:0", "--verbose"]).unwrap();
        assert!(cli.verbose, "`--verbose` is global, so it parses on either side of the subcommand");
        match cli.command {
            // The three TLS fields belong to the test below; this one is about `--bind`.
            Some(Cmd::Serve { bind, .. }) => assert_eq!(bind.as_deref(), Some("127.0.0.1:0")),
            other => panic!("`serve` must be a subcommand, got {other:?}"),
        }

        // The three ways in that existed before this task parse exactly as they did.
        assert!(Cli::try_parse_from(["tipsy-assistant"]).unwrap().command.is_none());
        assert!(matches!(
            Cli::try_parse_from(["tipsy-assistant", "config", "path"]).unwrap().command,
            Some(Cmd::Config { .. })
        ));
        assert!(matches!(
            Cli::try_parse_from(["tipsy-assistant", "key", "status", "openrouter"]).unwrap().command,
            Some(Cmd::Key { .. })
        ));
    }

    #[test]
    fn connect_needs_a_url_and_a_token_needs_connect() {
        let error = Cli::try_parse_from(["tipsy-assistant", "--connect"]).unwrap_err();
        assert!(error.to_string().contains("--connect"), "a bare `--connect` must name the flag: {error}");

        let error = Cli::try_parse_from(["tipsy-assistant", "--token", "abc"]).unwrap_err();
        assert!(error.to_string().contains("--connect"), "`--token` without `--connect` is a usage error: {error}");
    }

    /// F1: a `--connect` URL with a token in it is what a shell history holds, and the `--verbose`
    /// banner is printed with `eprintln!`, which no `ScrubWriter` touches. The line printed is this
    /// function's, so this is that line — and the refused-attach half of the same rule is pinned
    /// where the sentence a user copies is built (`client::bridge`'s
    /// `an_unreachable_url_never_carries_a_token_into_its_message`).
    #[test]
    fn the_connect_banner_never_prints_the_token() {
        // A fixture, not a key: the workspace rule is that secrets appear by name and never in a file
        // or a fixture, and this string exists only to prove that it does not cross.
        const SECRET: &str = "9f8e7d6c5b4a39281706f5e4d3c2b1a0";
        let line = remote_banner(&format!("ws://192.168.1.5:7070/ws?token={SECRET}"), Some(SECRET));
        assert!(!line.contains(SECRET), "the banner put the token on a terminal: {line}");
        assert!(!line.contains('?'), "the query is what carried it: {line}");
        assert!(line.contains("ws://192.168.1.5:7070/ws"), "and the address a user needs stays: {line}");
        assert!(line.contains("token given"), "the line still says whether one was given: {line}");

        assert_eq!(
            remote_banner("ws://127.0.0.1:7070/ws", None),
            "mode: remote TUI; attaching to ws://127.0.0.1:7070/ws; token not given"
        );
    }

    /// Task 1.7's `the_tls_flags_do_not_exist_until_their_task_does` asserted that `serve --tls` did
    /// **not** parse, and said in as many words that this task would replace it. It is replaced here
    /// by its positive half rather than deleted: TLS is registered now, and the three shapes a user
    /// can ask for are the three `Settings.tls` states (spec §5.5). What the flags *mean* — the
    /// certificate's loading, the refusal that names a path, the fingerprint — belongs to
    /// `server::run`, and is tested in `server/tls.rs`.
    #[test]
    fn the_cli_registers_tls_and_its_pair() {
        let cli = Cli::try_parse_from(["tipsy-assistant", "serve", "--tls"]).unwrap();
        match cli.command {
            Some(Cmd::Serve { tls, tls_cert, tls_key, .. }) => {
                assert!(tls);
                assert!(tls_cert.is_none() && tls_key.is_none());
            }
            other => panic!("`serve` must be a subcommand, got {other:?}"),
        }

        // Naming a pair is naming TLS: it is registered, and it is the pair that will be served.
        let cli =
            Cli::try_parse_from(["tipsy-assistant", "serve", "--tls-cert", "c.pem", "--tls-key", "k.pem"]).unwrap();
        match cli.command {
            Some(Cmd::Serve { tls_cert, tls_key, .. }) => {
                assert_eq!(tls_cert.as_deref(), Some(std::path::Path::new("c.pem")));
                assert_eq!(tls_key.as_deref(), Some(std::path::Path::new("k.pem")));
            }
            other => panic!("`serve` must be a subcommand, got {other:?}"),
        }

        // Half a pair is a usage error: one file without the other is a configuration mistake, not a
        // default (spec §5.5: "used when both are given").
        let error = Cli::try_parse_from(["tipsy-assistant", "serve", "--tls-cert", "c.pem"]).unwrap_err();
        assert!(error.to_string().contains("--tls-key"), "{error}");
        let error = Cli::try_parse_from(["tipsy-assistant", "serve", "--tls-key", "k.pem"]).unwrap_err();
        assert!(error.to_string().contains("--tls-cert"), "{error}");
    }
}
