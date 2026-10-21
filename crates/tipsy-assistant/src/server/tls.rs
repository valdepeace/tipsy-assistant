//! The certificate `--tls` serves with, and the trust rule that keeps a key off a plaintext LAN.
//!
//! Two rules live here rather than in the listener. The first is §5.5's, and it has **three** states
//! (TLS off is the absence of the value, so `TlsFiles` has two): mint a self-signed pair once beside
//! `config.toml` and reuse it, or read a pair the user named and refuse to start — naming the path —
//! when one of those files is absent or unparseable. Which of the two is asked for arrives as
//! `crate::server::TlsFiles`, which **Task 1.4 owns** in `server/mod.rs` beside `Settings`; this file
//! consumes it and never declares it. The second rule is §9.4's: a `SetKey` may only cross a loopback
//! or an encrypted connection, and [`refuse_set_key`] is that decision.
//!
//! **§5.5's promise is confidentiality, not microphone access.** `http://127.0.0.1` is already a
//! secure context (research note §4.4), so a client on this machine needs no TLS to open a
//! microphone; what `--tls` buys is that the token and a new API key do not cross a LAN in the clear.
//! Nothing in this file, and none of `serve`'s three `--tls` lines, may claim more than that.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum_server::tls_rustls::RustlsConfig;
use rcgen::{CertifiedKey, generate_simple_self_signed};
use sha2::{Digest, Sha256};
use tipsy_protocol::{Command, ErrorCode, Event, ToClient, ToServer};

use crate::server::TlsFiles;

/// The generated pair's two file names, in the config directory the `config path` subcommand prints
/// (research note §4.3 names both).
pub const CERT_FILE: &str = "serve-cert.pem";
pub const KEY_FILE: &str = "serve-key.pem";

/// Everything [`load_or_generate`] needs that the CLI's answer does not carry: the two facts
/// `TlsFiles` deliberately holds neither of. `files` is the cross-task value (Task 1.4's `TlsFiles`,
/// the one `Settings.tls` holds); `config_dir` is where `Generated` writes and `hostnames` is what it
/// is valid for, both known only to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Which of the two states `serve` asked for.
    pub files: TlsFiles,
    /// The directory `config.toml` lives in: `serve-cert.pem` and `serve-key.pem` go here.
    pub config_dir: PathBuf,
    /// The names a generated certificate is valid for. For a LAN bind this must include the literal
    /// address the browser will type, because SANs are what a browser checks (research note §4.1).
    ///
    /// **The bare IP, never `IP:port`.** `rcgen`'s `generate_simple_self_signed` turns a string that
    /// parses as an `IpAddr` into `SanType::IpAddress` and anything else into `SanType::DnsName`
    /// (read in `rcgen` 0.14.10's `certificate.rs`, `CertificateParams::new`), and `"127.0.0.1:7070"`
    /// does **not** parse as an `IpAddr` — it would become a DNS name no browser ever matches. The
    /// caller builds this list from the parsed bind address for exactly that reason.
    pub hostnames: Vec<String>,
}

/// The pair `serve` will serve with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate {
    pub cert: PathBuf,
    pub key: PathBuf,
    /// `Some` only when this call minted the pair: the SHA-256 of the certificate's DER, as
    /// `AA:BB:…`, which is the same string a browser's certificate viewer shows. Computed here and
    /// never a constant.
    pub fingerprint: Option<String>,
}

/// Why TLS could not be set up. Both variants carry the paths: a user has to look at a file.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("cannot use the certificate pair: {cert} + {key}: {message}")]
    Pair { cert: PathBuf, key: PathBuf, message: String },
    #[error("cannot generate a self-signed certificate in {dir}: {message}")]
    Generate { dir: PathBuf, message: String },
}

/// The pair to serve with: the two files the user named, or the one beside `config.toml`, generated
/// here if it is not there yet. Called from `server::run`'s TLS branch, which is the only place that
/// can turn a `TlsError` into the refusal `serve` prints.
///
/// Generating writes two files with restrictive permissions where the platform has them, and nothing
/// else: the private key never leaves this directory, and no fingerprint is logged twice.
///
/// **"Logged", not "printed".** The fingerprint goes to `tipsy-assistant.log` through `server::run`,
/// which is where the user compares it with a browser's certificate viewer; a plain `serve` writes
/// nothing to the terminal at all, and `--verbose` is what mirrors the log there.
pub fn load_or_generate(paths: Paths) -> Result<Certificate, TlsError> {
    if let TlsFiles::Supplied { cert, key } = paths.files {
        // Exactly these two files: nothing is minted, nothing is written and nothing is replaced.
        // `rustls_config` is where a file that is missing or unparsable is refused, naming both.
        return Ok(Certificate { cert, key, fingerprint: None });
    }
    let cert = paths.config_dir.join(CERT_FILE);
    let key = paths.config_dir.join(KEY_FILE);
    if cert.exists() && key.exists() {
        // Generated by an earlier run: reuse it. A fingerprint is logged once, at generation.
        return Ok(Certificate { cert, key, fingerprint: None });
    }
    // Either nothing is there or only half of a pair is: mint both, so a key that does not match its
    // certificate is not a state this function can return.
    generate(&paths, cert, key)
}

/// Mints one pair, writes it, and reports the fingerprint of what it wrote.
///
/// Both files are removed first, because a call that gets here means at least one of them is missing
/// and a certificate left beside a key it does not match is worse than either. `write_private` then
/// claims each name with `create_new`. Two `serve --tls` starting in the same millisecond on a fresh
/// install can still interleave their writes; what that costs is the one failure `rustls_config`
/// refuses by name — a pair that does not match — and never a mismatched pair served silently.
fn generate(paths: &Paths, cert: PathBuf, key: PathBuf) -> Result<Certificate, TlsError> {
    // An empty list would mint a certificate with no SAN at all, which no browser can match; the one
    // name that is always true of a server on this machine is the one it falls back to.
    let names = if paths.hostnames.is_empty() { vec!["localhost".to_string()] } else { paths.hostnames.clone() };
    let CertifiedKey { cert: minted, signing_key } = generate_simple_self_signed(names)
        .map_err(|e| TlsError::Generate { dir: paths.config_dir.clone(), message: e.to_string() })?;
    let fingerprint = fingerprint(minted.der());
    let key_pem = signing_key.serialize_pem();
    let cert_pem = minted.pem();
    for path in [&cert, &key] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(TlsError::Generate { dir: paths.config_dir.clone(), message: e.to_string() }),
        }
    }
    write_private(&paths.config_dir, &key, key_pem.as_bytes())
        .map_err(|e| TlsError::Generate { dir: paths.config_dir.clone(), message: e.to_string() })?;
    write_private(&paths.config_dir, &cert, cert_pem.as_bytes())
        .map_err(|e| TlsError::Generate { dir: paths.config_dir.clone(), message: e.to_string() })?;
    Ok(Certificate { cert, key, fingerprint: Some(fingerprint) })
}

/// The SHA-256 of a certificate's DER, in the notation a browser's certificate viewer uses.
fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(":")
}

/// Writes a file only the current user should read. Unix gets mode `0o600` (research note §4.3); on
/// Windows the file goes to `%APPDATA%`, which is already per-user, and this makes no claim beyond
/// that. `create_new` is what makes "generated once" true even if two `serve`s start together.
fn write_private(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.flush()
        }
        // Another `serve` won the race and wrote the pair first: its pair is as good as ours, and
        // overwriting it would rotate the certificate under a browser that just accepted it.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// The `RustlsConfig` the listener hands to `bind_rustls`. This is where the pair is parsed, so a
/// file that is not a certificate, or a key that does not match it, is refused here — with both
/// paths in the message (spec §10).
pub async fn rustls_config(cert: &Certificate) -> Result<RustlsConfig, TlsError> {
    RustlsConfig::from_pem_file(&cert.cert, &cert.key).await.map_err(|e| TlsError::Pair {
        cert: cert.cert.clone(),
        key: cert.key.clone(),
        message: e.to_string(),
    })
}

/// §9.4's whole rule: a secret may travel towards the server only over a loopback connection or over
/// TLS. "Loopback" is the *peer's* address: a client on this machine is the same machine, however it
/// reached us.
pub fn accepts_a_secret(peer: SocketAddr, tls: bool) -> bool {
    tls || peer.ip().is_loopback()
}

/// The answer a bridge sends **instead of** forwarding a `SetKey`, or `None` when the command is not
/// a `SetKey` or the connection may carry one.
///
/// `recoverable: true` is the honest flag: the same screen works once the connection is TLS, and
/// nothing about the refusal ends the connection. The **code** is the machine meaning — Task 1.1's
/// `ErrorCode::InsecureTransport`, the one value a client, a test or the session's own `SetKey` arm
/// may branch on — and the **message** below is a sentence for a person, which names the rule and
/// never the value that was refused.
pub fn refuse_set_key(command: &ToServer, peer: SocketAddr, tls: bool) -> Option<ToClient> {
    if !matches!(command, ToServer::Command(Command::SetKey { .. })) || accepts_a_secret(peer, tls) {
        return None;
    }
    Some(ToClient::Event(Event::Error {
        code: ErrorCode::InsecureTransport,
        message: "a new API key may only be sent over this machine's loopback connection or over TLS; \
                  restart the server with --tls, or set the key on the server machine"
            .to_string(),
        recoverable: true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::routing::get;
    // The error type `server::run` maps a failed TLS setup into, so the third-state test can assert
    // that §10's refusal reaches the CLI's message rather than stopping at this module; and the
    // three-state value Task 1.4 declares, which the tests below name.
    use crate::server::{ServerError, TlsFiles};

    /// The paths a `serve --tls` with no `--tls-cert` uses: generate beside `config.toml`, and a SAN
    /// list holding the address the browser will type plus `localhost` (research note §4.1).
    fn generated(dir: &std::path::Path) -> Paths {
        Paths {
            files: TlsFiles::Generated,
            config_dir: dir.to_path_buf(),
            hostnames: vec!["127.0.0.1".to_string(), "localhost".to_string()],
        }
    }

    /// The other state: exactly these two files, and no minting.
    fn supplied(cert: PathBuf, key: PathBuf, dir: &std::path::Path) -> Paths {
        Paths { files: TlsFiles::Supplied { cert, key }, config_dir: dir.to_path_buf(), hostnames: Vec::new() }
    }

    /// The certificate is generated **once**: the second call finds the pair and leaves it alone.
    #[test]
    fn the_certificate_is_generated_once_and_then_reused() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_generate(generated(dir.path())).unwrap();
        assert_eq!(first.cert, dir.path().join("serve-cert.pem"));
        assert_eq!(first.key, dir.path().join("serve-key.pem"));
        let fingerprint = first.fingerprint.clone().expect("a minted pair has a fingerprint to log once");
        assert_eq!(fingerprint.len(), 32 * 3 - 1, "SHA-256 as 32 `AA:BB` pairs: {fingerprint}");
        assert!(fingerprint.split(':').all(|byte| byte.len() == 2 && byte.chars().all(|c| c.is_ascii_hexdigit())));
        assert!(std::fs::read_to_string(&first.cert).unwrap().starts_with("-----BEGIN CERTIFICATE-----"));

        let before = std::fs::read(&first.cert).unwrap();
        let second = load_or_generate(generated(dir.path())).unwrap();
        assert!(second.fingerprint.is_none(), "only the run that minted the pair logs a fingerprint");
        assert_eq!(std::fs::read(&second.cert).unwrap(), before, "the second call must not mint another");
        assert_eq!(second.key, first.key);

        // Half a pair on disk is a half-written pair: both are regenerated rather than mismatched.
        std::fs::remove_file(&second.key).unwrap();
        let third = load_or_generate(generated(dir.path())).unwrap();
        assert!(third.fingerprint.is_some(), "a missing key means the pair is minted again");
        assert_ne!(std::fs::read(&third.cert).unwrap(), before);
    }

    /// `TlsFiles::Supplied` means *exactly these two files*: they are used as they are, never
    /// overwritten, and nothing is written beside `config.toml`.
    #[test]
    fn a_supplied_pair_is_used_and_never_generated_over() {
        let dir = tempfile::tempdir().unwrap();
        let minted = load_or_generate(generated(dir.path())).unwrap();
        let cert_before = std::fs::read(&minted.cert).unwrap();
        let key_before = std::fs::read(&minted.key).unwrap();

        // Deliberately a directory with nothing in it: a supplied pair must not be looked for beside
        // `config.toml` at all.
        let unused = dir.path().join("unused");
        let used = load_or_generate(Paths {
            files: TlsFiles::Supplied { cert: minted.cert.clone(), key: minted.key.clone() },
            config_dir: unused.clone(),
            hostnames: Vec::new(),
        })
        .unwrap();
        assert_eq!(used.cert, minted.cert);
        assert_eq!(used.key, minted.key);
        assert!(used.fingerprint.is_none(), "this process did not mint the pair, so it logs no fingerprint");
        assert_eq!(std::fs::read(&used.cert).unwrap(), cert_before);
        assert_eq!(std::fs::read(&used.key).unwrap(), key_before);
        assert!(!unused.exists(), "a supplied pair writes nothing");
    }

    /// The third state, and the one a bare `Option<(PathBuf, PathBuf)>` could not express: a pair the
    /// user *named* that is not there must refuse to start and name the path — never quietly generate
    /// a certificate in its place (spec §5.5, §10). The last assertion is the one that makes the
    /// refusal reach the CLI: `ServerError` carries this error transparently, and the message a failed
    /// `serve` prints is the one checked here.
    #[tokio::test]
    async fn a_supplied_pair_that_is_not_there_refuses_to_start_and_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("no-cert.pem");
        let key_path = dir.path().join("no-key.pem");

        let cert = load_or_generate(supplied(cert_path.clone(), key_path.clone(), dir.path())).unwrap();
        assert!(cert.fingerprint.is_none(), "a named pair is never replaced by a minted one");
        assert!(
            !dir.path().join(CERT_FILE).exists() && !dir.path().join(KEY_FILE).exists(),
            "no certificate may appear beside config.toml when the user pointed at files"
        );

        let error = rustls_config(&cert).await.expect_err("a pair that is not there cannot be parsed");
        let text = error.to_string();
        assert!(text.contains(&cert_path.display().to_string()), "{text}");
        assert!(text.contains(&key_path.display().to_string()), "{text}");

        // The same sentence `serve` prints, through the variant `server::run` maps it into.
        let server_error: ServerError = error.into();
        assert!(
            server_error.to_string().contains(&cert_path.display().to_string()),
            "§10's refusal must reach the CLI with the path in it: {server_error}"
        );

        // A pair that is present and readable but is not a certificate is refused the same way.
        std::fs::write(&cert_path, "not a certificate at all").unwrap();
        std::fs::write(&key_path, "not a key at all").unwrap();
        let cert = load_or_generate(supplied(cert_path.clone(), key_path, dir.path())).unwrap();
        let error = rustls_config(&cert).await.expect_err("garbage is refused, not served");
        let text = error.to_string();
        assert!(text.contains(&cert_path.display().to_string()), "{text}");
        assert!(!text.contains("not a certificate at all"), "the file's contents are not a log line: {text}");
    }

    /// §9.4's rule, both halves: refused on plaintext from off this machine, accepted on loopback and
    /// on TLS.
    #[test]
    fn a_secret_may_only_cross_a_loopback_or_an_encrypted_connection() {
        let loopback: SocketAddr = "127.0.0.1:54123".parse().unwrap();
        let lan: SocketAddr = "192.168.1.30:54123".parse().unwrap();
        assert!(accepts_a_secret(loopback, false), "a client on this machine is the same machine");
        assert!(accepts_a_secret(loopback, true));
        assert!(accepts_a_secret(lan, true), "TLS is what makes a LAN connection good enough");
        assert!(!accepts_a_secret(lan, false), "a plaintext LAN connection must not carry a key");

        let set_key = ToServer::Command(Command::SetKey { name: "openrouter".into(), value: "sk-or-v1-xyz".into() });
        let refused = refuse_set_key(&set_key, lan, false).expect("a key must not cross a plaintext LAN");
        match refused {
            ToClient::Event(Event::Error { code, recoverable, message }) => {
                // The **code** is the assertion: it is the machine meaning Task 1.1 owns, and what
                // the screen and `session.rs` branch on. The message is a sentence for a person, so
                // nothing here matches its wording — only that it must not repeat the value it
                // refused, which is the one thing a refusal message could get wrong.
                assert_eq!(code, ErrorCode::InsecureTransport);
                assert!(recoverable, "the screen can retry it once the connection is TLS");
                assert!(message.len() > 40, "and the message is a sentence, not a code: {message}");
                assert!(!message.contains("sk-or-v1-xyz"), "the refusal must not repeat the key: {message}");
            }
            other => panic!("a refusal is an `Event::Error`, got {other:?}"),
        }
        assert!(refuse_set_key(&set_key, loopback, false).is_none());
        assert!(refuse_set_key(&set_key, lan, true).is_none());
        // Anything that is not a `SetKey` is nobody's business here.
        let ask = ToServer::Command(Command::Ask { text: "¿qué pasa?".into() });
        assert!(refuse_set_key(&ask, lan, false).is_none());
    }

    /// The listener is a TLS listener: the generated pair is what it serves, and the same route
    /// answers the same way over HTTPS as it does in the clear. Task 1.4 hands the *same* `Router` to
    /// `bind` and to `bind_rustls`, so this is the half of that claim a listener can prove.
    #[tokio::test]
    async fn a_tls_listener_serves_the_same_routes_as_the_plain_one() {
        let dir = tempfile::tempdir().unwrap();
        let cert = load_or_generate(generated(dir.path())).unwrap();
        let config = rustls_config(&cert).await.unwrap();
        let app = || Router::new().route("/", get(|| async { "the same page" }));

        let plain = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let plain_addr = plain.local_addr().unwrap();
        let plain_task = tokio::spawn(async move { axum::serve(plain, app()).await });

        let tls_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        tls_listener.set_nonblocking(true).unwrap();
        let tls_addr = tls_listener.local_addr().unwrap();
        let tls_task = tokio::spawn(async move {
            axum_server::from_tcp_rustls(tls_listener, config)
                .expect("a bound plain socket becomes a TLS listener")
                .serve(app().into_make_service())
                .await
        });

        let plain_body = reqwest::get(format!("http://{plain_addr}/")).await.unwrap().text().await.unwrap();
        let client = reqwest::Client::builder().danger_accept_invalid_certs(true).build().unwrap();
        let tls_body = client.get(format!("https://{tls_addr}/")).send().await.unwrap().text().await.unwrap();

        assert_eq!(plain_body, "the same page");
        assert_eq!(tls_body, plain_body, "TLS changes who may connect, not what the routes answer");

        plain_task.abort();
        tls_task.abort();
    }
}
