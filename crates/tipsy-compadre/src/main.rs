//! The `tipsy-compadre` binary: run the client, or manage the API keys it uses.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tipsy_compadre::client;
use tipsy_compadre::config::store;
use tipsy_compadre::keys::{redact, KeyResolver, KeySource, OsKeyring, SecretStore};
use tipsy_compadre::providers::build_providers;

/// Exit code used when a provider cannot be built for want of a key.
const MISSING_KEY: u8 = 2;

#[derive(Parser)]
#[command(name = "tipsy-compadre", version, about = "A tipsy, lazy, joking voice assistant for software developers")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
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
}

#[derive(Subcommand)]
enum KeyCmd {
    /// Store a key in the operating system's credential store.
    Set { name: String },
    /// Report where the key would come from, without printing it.
    Status { name: String },
    /// Remove a key from the operating system's credential store.
    Delete { name: String },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the path of `config.toml`.
    Path,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        None => run_client(),
        Some(Cmd::Key { command }) => key_command(command),
        Some(Cmd::Config { command }) => config_command(command),
    }
}

/// The bare run: load the configuration, build the providers and hand over to the client.
fn run_client() -> ExitCode {
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
