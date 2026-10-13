//! The headless server: the listener, the router and the two checks that keep a LAN behind a token.

pub mod assets;
pub mod conn;
pub mod hub;
pub mod tls;

pub use hub::Hub;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use tipsy_protocol::ClientKind;
use tokio::sync::Semaphore;

use crate::client::pty::{PtySpawner, TerminalSpawner};
use crate::config::Config;
use crate::keys::KeyResolver;
use crate::providers::ProviderSlot;
use crate::session::SessionPolicy;

/// Everything `run` needs, as `main.rs` reads it out of the file with `--bind` already applied for
/// this run (spec §5.1). The token is a plain `String`: it is ours, it is compared on the upgrade,
/// and it is never logged (spec §9.6).
///
/// `providers` is the **one shared slot** the whole server runs on, built by the caller from this
/// configuration — the same pattern `client.rs` already uses for the in-process client, and why
/// `ProviderSlot::replace` exists (`providers/mod.rs:175`). `Hub::new` takes it and clones it into
/// every session it spawns, and `reconfigure_all` is what swaps the connectors in it when the
/// configuration changes (spec §5.2).
///
/// **`Clone` is deliberately not derived, and that is the one thing this struct does not do that the
/// plan's contract asks of it.** `ProviderSlot` is `Clone`, but cloning it would give a second *name*
/// for the same provider stack rather than a second stack, and nothing clones a `Settings`: `run`
/// takes it by value, builds the hub from it, and hands the same value to the router state, exactly
/// once. `Debug` is hand-written below for the same reason: the derive would demand `Debug` of the
/// three connector trait objects behind `ProviderSlot`, which do not have it.
pub struct Settings {
    pub bind: String,
    /// The `lan` switch. It is deliberately **not** part of the refusal test — the bound address is,
    /// because `lan = true` on a loopback address is still loopback and needs no token. It rides
    /// here so Task 1.9's log line can say which mode the operator asked for.
    pub lan: bool,
    pub token: String,
    /// The configuration every session spawned from this server starts with. W3 puts it on the wire
    /// and the `Hub` broadcasts its changes; the server still owns the file.
    pub config: Config,
    /// Where that configuration lives, which the `Hub` needs to persist a `SetProfile` or an
    /// `UpdateConfig` (`config/mod.rs`, `config/store.rs:20-26`).
    pub config_path: PathBuf,
    /// The key resolver the server builds providers with, and the one `SetKey` writes to. A plain
    /// value: `KeyResolver` is `Clone` (it holds its store behind an `Arc`), so the hub and the
    /// router state can both be handed it without either owning the other's copy.
    pub keys: KeyResolver,
    /// The shared provider slot, built from `config` before the server starts.
    pub providers: ProviderSlot,
    /// What `--tls` asked for (spec §5.5, W2), or `None` for no TLS at all. Task 1.8's
    /// `server/tls.rs` reads this; the type is defined here because `Settings` cannot be spelled
    /// without it and Task 1.7 builds a `Settings` literal before 1.8 lands.
    pub tls: Option<TlsFiles>,
}

/// Hand-written, not derived: the derive would require `ProviderSlot` — hence the three connector
/// trait objects behind it — to be `Debug`, and it is not. The two handles are shown as handles, and
/// neither the token nor the configuration is printed: `Config`'s own `Debug` printed `server.token`
/// when this formatter was first written, which made it claim more than it did, and a `Debug` line
/// reaches a log more often than anyone intends (spec §9.6). `ServerConfig`'s formatter is redacted
/// now, so that specific leak is closed where the secret lives; this one still prints a *summary*
/// rather than the whole configuration, because a log line is not the place for one.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("bind", &self.bind)
            .field("lan", &self.lan)
            .field("token", &if self.token.trim().is_empty() { "<none>" } else { "<set>" })
            // A summary rather than `&self.config`: a whole configuration in a log line is more than
            // a `Debug` line should carry, and this is the formatter that promised to be safe.
            .field("config", &config_summary(&self.config))
            .field("config_path", &self.config_path)
            .field("keys", &"<key resolver>")
            .field("providers", &"<provider slot>")
            .field("tls", &self.tls)
            .finish()
    }
}

/// The configuration as a `Debug` may print it: what describes it, and not the whole of it. The token
/// is no longer a reason to keep it short — `ServerConfig`'s formatter is hand-written and redacted
/// (`config/model.rs`) — but a log line is still not the place for a whole configuration.
fn config_summary(config: &Config) -> String {
    format!(
        "Config {{ active_profile: {:?}, profiles: {}, bind: {:?}, lan: {} }}",
        config.active_profile,
        config.profiles.len(),
        config.server.bind,
        config.server.lan
    )
}

/// Which certificate the listener uses, when TLS is on at all (spec §5.5, W2).
///
/// Three states, which is why it is not a path pair: `Generated` is `--tls` alone and means "mint a
/// self-signed pair in the config directory on first use, reuse it after that"; `Supplied` is `--tls`
/// with `--tls-cert`/`--tls-key` and means "read exactly these, and refuse to start naming the path
/// when one is missing or unparseable" (spec §10). A pair of paths cannot tell those apart, so a
/// mistyped `--tls-cert` would be answered with a silently generated certificate instead of the
/// refusal. `TlsFiles` keeps `Clone` + `Debug` honest because it is both — which is also why
/// `Settings` can hold it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TlsFiles {
    Generated,
    Supplied { cert: PathBuf, key: PathBuf },
}

/// Why `run` refused to start, or stopped (spec §5.1, §5.5, §10). `Display` is hand-written below
/// rather than derived, because each sentence has to carry the fact a user needs — the address that
/// was refused, or the path that would not load — and the one variant that wraps another error says
/// so transparently.
#[derive(Debug)]
pub enum ServerError {
    /// `bind` is not an explicit `IP:port`. A hostname is refused on purpose: `TcpListener::bind`
    /// takes `ToSocketAddrs`, so an unqualified name can resolve to something off this machine.
    BadBind {
        bind: String,
    },
    /// Not loopback, and no token (spec §5.1 step 2, spec §10).
    LanWithoutToken {
        addr: SocketAddr,
    },
    /// The bind landed somewhere other than what was asked for, with no token to guard it
    /// (spec §5.1 step 3).
    BoundWiderThanAsked {
        addr: SocketAddr,
    },
    Io(std::io::Error),
    /// TLS could not be set up: a pair the user named is absent or unparseable, or a self-signed pair
    /// could not be minted (spec §5.5, §10). **Task 1.8's variant**: it cannot come from Task 1.4,
    /// because `TlsError` does not exist until `server/tls.rs` does. It is transparent in `Display`
    /// and in `source` for one reason: `TlsError`'s sentence already names the paths a user has to
    /// look at, and it is the line a failed `serve` prints.
    Tls(tls::TlsError),
    /// A refusal that is none of the above and needs no type of its own: the server cannot name its
    /// own executable, which is how Task 2.1's terminal spawner is built (`std::env::current_exe()`
    /// failing while `run` assembles the spawner). **Task 2.1's variant**, spelled exactly as its
    /// `Produces` block requires. It carries a rendered sentence rather than a cause, so `source`
    /// returns `None` for it.
    Message(String),
}

impl From<tls::TlsError> for ServerError {
    fn from(error: tls::TlsError) -> Self {
        ServerError::Tls(error)
    }
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::BadBind { bind } => write!(
                f,
                "bind must be an explicit IP:port and {bind:?} is not one: a hostname can resolve to \
                 something off this machine. Use e.g. 127.0.0.1:7070 or 0.0.0.0:7070."
            ),
            // The address is in the sentence because the operator has to be told *which* address was
            // refused: `--bind` overrides the file, so "the one in config.toml" is a guess.
            //
            // **The advice names the address and the token, and deliberately not `lan`.** `lan` is not
            // a term in §5.1's rule: the rule reads the address, `lan = true` on a loopback address is
            // legal and changes nothing, and setting it on this one would still be refused. The
            // sentence used to offer it, which sent a user to a switch that cannot help them
            // (`Settings.lan`'s own comment records the same thing).
            ServerError::LanWithoutToken { addr } => write!(
                f,
                "refusing to bind {addr}: it is not a loopback address and no token is configured. \
                 Set a token in [server], or bind a loopback address such as 127.0.0.1:7070."
            ),
            ServerError::BoundWiderThanAsked { addr } => {
                write!(f, "bound to {addr}, which is not a loopback address, and no token is configured")
            }
            ServerError::Io(e) => write!(f, "cannot listen: {e}"),
            // Transparent on purpose: `TlsError`'s own sentence already names the paths a user has
            // to look at, and wrapping it again here would put a second prefix in front of them.
            ServerError::Tls(e) => write!(f, "{e}"),
            // Already a sentence: whoever built it named what could not be done.
            ServerError::Message(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServerError::Io(e) => Some(e),
            ServerError::Tls(e) => Some(e),
            // `Message` carries a rendered sentence, not a cause: there is nothing to walk to.
            ServerError::Message(_) => None,
            _ => None,
        }
    }
}

/// What every route shares. Cloned per request by axum. There is no provider stack here: the `Hub`
/// holds it, and a connection gets a session from `Hub::spawn`.
#[derive(Clone)]
pub(crate) struct ServeState {
    pub(crate) token: Option<String>,
    pub(crate) hub: Hub,
    /// What every session this listener starts runs under, decided once from the **bound** address:
    /// loopback needs no token, anything wider does (spec §4.1, §9.1). It is the same `SessionPolicy`
    /// `Hub::spawn` takes.
    pub(crate) policy: SessionPolicy,
    /// Whether this listener terminates TLS. It rides here because it is the one fact of §9.4's rule
    /// a socket cannot read off itself: the `SetKey` guard is `tls || peer.ip().is_loopback()`, and
    /// the peer comes from the connection while this comes from `Settings.tls`.
    pub(crate) tls: bool,
    /// What a `/terminal/ws` connection spawns its child from: **this binary in `--connect` mode,
    /// pointed at this process's own `/ws`** (spec §7.2, W10). Built once, by `run`, from the address
    /// the listener actually bound — which is why it cannot live on `Settings`, whose `bind` is what
    /// was *asked* for.
    pub(crate) spawner: Arc<dyn PtySpawner>,
    /// The terminal connection cap, shared by this listener's `/terminal/ws` handler and nothing
    /// else. `Arc` because a request is served by a clone of this state; the permit it hands out is
    /// what bounds live PTY children (`conn::MAX_TERMINAL_CONNECTIONS`).
    pub(crate) terminals: Arc<Semaphore>,
}

/// The `/ws` URL a terminal child attaches to (spec §7.2, W10).
///
/// **Not always the bound address, because a wildcard is not an address.** A listener on `0.0.0.0:7070`
/// or `[::]:7070` answers on loopback too, and `ws://0.0.0.0:7070/ws` is not something a client can
/// reach — on Windows it is a refused connection rather than a lucky one — so the wildcard is spelled
/// as the loopback address of its own family. A listener bound to one specific address is that
/// address: if it is `192.168.1.5:7070`, nothing is listening on `127.0.0.1` at all, and the child —
/// which is a local process like any other — has to use the address that is really there. IPv6 needs
/// its brackets, or the port becomes part of the host.
fn terminal_child_url(bound: SocketAddr) -> String {
    let host = match bound.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    match host {
        IpAddr::V4(ip) => format!("ws://{ip}:{}/ws", bound.port()),
        IpAddr::V6(ip) => format!("ws://[{ip}]:{}/ws", bound.port()),
    }
}

/// The argument vector a terminal child is started with: the `--connect` URL above, and `--token`
/// **only when this server has one**. An empty `--token` would be a token presented as the empty
/// string, which is a different claim from presenting none; and on loopback there is no door to open
/// with one (spec §5.3, §9.1).
fn terminal_args(bound: SocketAddr, token: Option<&str>) -> Vec<String> {
    let mut args = vec!["--connect".to_string(), terminal_child_url(bound)];
    if let Some(token) = token {
        args.push("--token".to_string());
        args.push(token.to_string());
    }
    args
}

/// The spawner every `/terminal/ws` connection on this listener uses.
///
/// `program` is [`std::env::current_exe`] and never a name: `portable-pty` searches `PATH` for a name
/// with no path in it, and a `PATH` entry that is empty, relative or points into a checked-out
/// workspace must not be able to plant the child that then runs with the user's authority (Task 2.1's
/// security test, and the prior art its comment quotes). This is the one variant of `ServerError`
/// that can carry the refusal.
fn terminal_spawner(bound: SocketAddr, token: Option<&str>) -> Result<Arc<dyn PtySpawner>, ServerError> {
    let program = std::env::current_exe()
        .map_err(|e| ServerError::Message(format!("no current executable for the web terminal: {e}")))?;
    Ok(Arc::new(TerminalSpawner { program, args: terminal_args(bound, token), env: Vec::new() }))
}

impl conn::TerminalRouteState for ServeState {
    fn websocket_token(&self) -> Option<String> {
        // The same value `/terminal`'s own `?token=` is compared against, and trimmed the same way.
        // Never logged.
        self.token.clone()
    }

    fn terminal_spawner(&self) -> Arc<dyn PtySpawner> {
        Arc::clone(&self.spawner)
    }

    fn terminal_gate(&self) -> Arc<Semaphore> {
        Arc::clone(&self.terminals)
    }
}

/// The token as the routes compare it. `has_token` (`config/model.rs:59-61`) trims, so a
/// whitespace-only value in a hand-edited file is no token here either.
fn token_of(token: &str) -> Option<String> {
    (!token.trim().is_empty()).then(|| token.trim().to_string())
}

/// What every route shares, assembled from the settings the listener was handed and the hub built
/// from them. **`settings` is read for the token only**: everything else the router might want is on
/// the hub by the time this runs, which is why `run` builds the hub first and passes both in. There
/// is no provider stack here either — the hub holds it, and a connection gets a session from
/// `Hub::spawn`.
///
/// `spawner` is built by `run` from the address it really bound, because the terminal child has to
/// attach to this very listener and `Settings.bind` is only what was asked for.
pub(crate) fn state(
    settings: &Settings,
    hub: Hub,
    policy: SessionPolicy,
    tls: bool,
    spawner: Arc<dyn PtySpawner>,
) -> ServeState {
    ServeState {
        token: token_of(&settings.token),
        hub,
        policy,
        tls,
        spawner,
        terminals: Arc::new(Semaphore::new(conn::MAX_TERMINAL_CONNECTIONS)),
    }
}

/// Spec §5.1 steps 1 and 2, before the bind: the user-facing refusal.
fn guard_pre_bind(bind: &str, token: Option<&str>) -> Result<SocketAddr, ServerError> {
    let addr: SocketAddr = bind.parse().map_err(|_| ServerError::BadBind { bind: bind.to_string() })?;
    // `is_loopback()` is the whole rule: `0.0.0.0` is unspecified rather than loopback, so it fails
    // here too. `is_unspecified()` would only change the wording, and the wording lives in `Display`.
    if !addr.ip().is_loopback() && token.is_none() {
        return Err(ServerError::LanWithoutToken { addr });
    }
    Ok(addr)
}

/// Spec §5.1 step 3, after the bind. The listener is dropped by the caller when this returns `Err`.
/// It takes a **`std::net`** listener because `run` binds once, synchronously, and hands the same
/// socket to whichever listener serves it (spec §5.1 step 3 is about the socket, not the runtime).
fn guard_post_bind(listener: &std::net::TcpListener, token: Option<&str>) -> Result<(), ServerError> {
    let addr = listener.local_addr().map_err(ServerError::Io)?;
    if !addr.ip().is_loopback() && token.is_none() {
        return Err(ServerError::BoundWiderThanAsked { addr });
    }
    Ok(())
}

/// Serves until the process ends. `bound` is sent the address once **both** halves of §5.1's rule
/// have passed, and that is also the point a caller may start connecting.
pub async fn run(settings: Settings, bound: tokio::sync::oneshot::Sender<SocketAddr>) -> Result<(), ServerError> {
    let token = token_of(&settings.token);
    let addr = guard_pre_bind(&settings.bind, token.as_deref())?;
    // Bound synchronously, and only then handed to whichever listener serves it: `axum_server` (the
    // TLS arm) is built from a `std::net::TcpListener`, and one bind means one place where §5.1's
    // post-bind half is checked — before TLS setup, so a certificate that cannot be read refuses to
    // start rather than becoming a listener that fails on its first connection (spec §10).
    //
    // **`set_nonblocking(true)` is required and is not a detail.** `axum_server::from_tcp` wraps the
    // socket with `tokio::net::TcpListener::from_std`, and on this platform a *blocking* socket given
    // to it does not merely misbehave: `accept_stream` parks a runtime worker inside `accept(2)`, so
    // the first connection is left waiting for ever and the whole test binary stalls. Measured here,
    // not assumed — the identical listener with `set_nonblocking(true)` answers 200 immediately.
    let listener = std::net::TcpListener::bind(addr).map_err(ServerError::Io)?;
    listener.set_nonblocking(true).map_err(ServerError::Io)?;
    guard_post_bind(&listener, token.as_deref())?;
    let local = listener.local_addr().map_err(ServerError::Io)?;
    let _ = bound.send(local);
    // The bound address decides the policy every session from this listener runs under (Task 1.2's
    // `SessionPolicy`; spec §4.1, §9.1): loopback needs nothing, anything wider needs the token.
    let policy = SessionPolicy {
        require_token: !local.ip().is_loopback(),
        // The token the routes compare is the trimmed one, so a whitespace-only value in a
        // hand-edited file is no token for a session either.
        token: token.clone().unwrap_or_default(),
        loopback: local.ip().is_loopback(),
        // A socket that never says `Hello` is refused and let go after this long: with the upgrade's
        // own token check gone, this is the only ceiling on a connection that sends nothing at all.
        handshake_timeout: crate::session::HANDSHAKE_TIMEOUT,
    };
    // Task 1.8's TLS branch. The loading is here and not in `main.rs` for two reasons: a certificate
    // that cannot be read has to travel as one of this function's own errors, and the CLI must not
    // grow a second error type of its own (spec §5.5, §10).
    //
    // The SAN list is built from the **parsed** address, never from `settings.bind`: `rcgen` turns a
    // string that parses as an `IpAddr` into an IP SAN and anything else — `"127.0.0.1:7070"`
    // included — into a DNS name, which no browser matches. A certificate without the literal
    // address in its SANs is a certificate the browser rejects (research note §4.1).
    let tls_config = match &settings.tls {
        None => None,
        Some(files) => {
            let paths = tls::Paths {
                files: files.clone(),
                config_dir: settings.config_path.parent().unwrap_or(Path::new(".")).to_path_buf(),
                hostnames: vec![local.ip().to_string(), "localhost".to_string()],
            };
            let certificate = tls::load_or_generate(paths)?;
            if let Some(fingerprint) = &certificate.fingerprint {
                // Recorded once, by the run that minted the pair, so a user can compare it with the
                // browser's certificate viewer (spec §5.5). It is a log record rather than a
                // `println!`: `--verbose` puts it on the terminal, and every run leaves it in
                // `tipsy-assistant.log`, where a reader who missed the screen can still find it. The
                // fingerprint survives the redaction rules by shape — `AA:BB:…` is not one opaque
                // word — which `logging`'s own tests pin.
                tracing::info!(path = %certificate.cert.display(), "generated a self-signed certificate");
                tracing::info!(fingerprint = %fingerprint, "SHA-256 of the certificate, as a browser shows it");
            }
            // What the flag buys, and what it does not, where a user reads it (spec §9.5). This is
            // the ruling the README repeats, and the one a certificate viewer cannot tell anyone.
            tracing::warn!(
                "TLS encrypts this connection, so the token and a new API key are safe on a LAN. It \
                 does not unlock the browser microphone from another machine: that needs a \
                 certificate the browser already trusts (--tls-cert/--tls-key), or a client on this \
                 machine, where http://127.0.0.1 is already a secure context."
            );
            Some(tls::rustls_config(&certificate).await?)
        }
    };
    // The listener is up, and this is the one record that says where and how. `asked` and `bound`
    // differ whenever `--bind 127.0.0.1:0` asked for a port the kernel chose; **`lan` is read here
    // and nowhere else** (spec §5.1) — it is the mode the operator asked for, and the *bound* address
    // is what §5.1's rule and every session's policy are built from. The token is not part of the
    // record, and neither is anything a log reader could turn into one (spec §9.6).
    tracing::info!(
        asked = %settings.bind,
        bound = %local,
        lan = settings.lan,
        tls = tls_config.is_some(),
        "listening"
    );
    // The `Hub` is built once, before the first connection, and outlives every session (spec §5.2).
    // It takes the one shared provider slot: `Hub::spawn` clones it into each session, and
    // `reconfigure_all` is what replaces its contents when the configuration changes. It also takes
    // the resolver, which is why this runs before the state is assembled: the hub owns it from here
    // on, and the state only needs the token off this same value.
    let hub = Hub::new(
        settings.config.clone(),
        settings.config_path.clone(),
        settings.keys.clone(),
        settings.providers.clone(),
    );
    // The terminal child is this binary in `--connect` mode, pointed at the address this listener
    // really bound (spec §7.2, W10) — built here rather than on `Settings`, which carries the address
    // that was *asked* for and so knows nothing about a port the kernel chose.
    let spawner = terminal_spawner(local, token.as_deref())?;
    let app = mount(state(&settings, hub, policy, tls_config.is_some(), spawner));
    // **The same `app` value in both arms**, which is what makes "a TLS listener serves the same
    // routes as the plain one" true by construction rather than by two routers being kept in step.
    // `into_make_service_with_connect_info::<SocketAddr>()` is what gives the `/ws` handler the peer
    // address Task 1.8's §9.4 guard reads; without it that extractor refuses the request, so both
    // arms need it.
    let handle = axum_server::Handle::<SocketAddr>::new();
    let outcome = match tls_config {
        Some(config) => {
            // **The socket this function already bound, and not a second `bind`.** `bind_rustls`
            // binds the address itself, and the `std` listener above is still holding that address —
            // so `serve --tls` reported `listening` and then died on Windows with
            // `WSAEADDRINUSE (10048)`, having generated a certificate for a server that was never
            // reachable. `from_tcp_rustls` takes the one socket, which `guard_post_bind` has already
            // checked and which is already non-blocking (`from_tcp` needs that, as Task 1.8's ledger
            // records for the plaintext arm). Found by the phase's first live `serve --tls` run; every
            // test waited only for the `bound` message, which is sent before this branch.
            axum_server::tls_rustls::from_tcp_rustls(listener, config)
                .map_err(ServerError::Io)?
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
        }
        None => {
            axum_server::from_tcp(listener)
                .map_err(ServerError::Io)?
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
        }
    };
    if let Err(e) = outcome {
        tracing::error!(error = %e, "the listener stopped with an error");
    }
    Ok(())
}

pub(crate) fn mount(state: ServeState) -> Router {
    Router::new()
        .route("/", get(page_client))
        .route("/terminal", get(page_terminal))
        // The embedded bundle: the wasm module, its JS glue, xterm.js and its stylesheet
        // (spec §5.3). Task 1.4 deliberately left this route out — `assets::asset` did not exist
        // until Task 1.5 — and the handler answers 404 for anything the embed does not hold.
        .route("/assets/{*path}", get(asset_of))
        .route("/ws", get(ws))
        // The raw PTY byte socket (Task 2.2). It is merged in rather than spelled here because
        // `conn::router` is the one place the route's own state bound is declared — `ServeState`
        // implements `conn::TerminalRouteState` above, and Task 2.2's tests merge the same function
        // into a router of their own, so the route under test and the route that ships cannot drift.
        .merge(conn::router(Router::<ServeState>::new()))
        .with_state(state)
}

fn page_of(path: &str) -> Response {
    match assets::resolve(path) {
        Some((content_type, bytes)) => page_response(content_type, bytes),
        None => {
            let (content_type, html) = assets::fallback();
            page_response(content_type, html)
        }
    }
}

async fn page_client(State(state): State<ServeState>, query: Query<TokenQuery>) -> Response {
    if !token_ok(&state, &query) {
        return refused();
    }
    page_of("/")
}

async fn page_terminal(State(state): State<ServeState>, query: Query<TokenQuery>) -> Response {
    if !token_ok(&state, &query) {
        return refused();
    }
    page_of("/terminal")
}

/// One file of the embedded bundle (spec §5.3), under the same token rule as the two pages: a
/// hashed asset name is not a secret, but the route it is reached by is behind the same door, so a
/// server that requires a token does not hand its bundle to a caller without one.
///
/// **A miss is a `404`, not the explanation page**: the page belongs to a navigation, and a browser
/// that asked for `missing.wasm` wants to know the file is absent rather than to receive HTML it
/// would try to instantiate. The body names nothing — no path, no token (spec §9.6).
async fn asset_of(
    State(state): State<ServeState>,
    AxumPath(path): AxumPath<String>,
    query: Query<TokenQuery>,
) -> Response {
    if !token_ok(&state, &query) {
        return refused();
    }
    match assets::asset(&path) {
        Some((content_type, bytes)) => asset_response(content_type, bytes),
        None => (StatusCode::NOT_FOUND, "no such asset in this build").into_response(),
    }
}

/// The one place an asset's headers are set. It differs from [`page_response`] in exactly one thing:
/// a bundle file may be cached, where a page may not — the token travels as a query parameter and a
/// cached page is a page one user's token opened for the next (spec §5.3, §9.2). Nothing here is
/// `no-store`, and nothing here is a page.
fn asset_response(content_type: &'static str, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(header::CONTENT_TYPE, content_type.parse().expect("a MIME type is a header value"));
    response
}

/// The one place the two page headers are set, so the explanation page carries them too: it is
/// reached with the same `?token=` (spec §5.3, §9.2, RFC 6750 §2.3).
fn page_response(content_type: &'static str, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type.parse().expect("a MIME type is a header value"));
    headers.insert("referrer-policy", "no-referrer".parse().expect("a static policy is a value"));
    headers.insert("cache-control", "no-store".parse().expect("a static directive is a value"));
    response
}

/// The protocol socket. Registered here; everything past the upgrade belongs to Task 1.6.
///
/// **The token is not required here, and that is deliberate** (the token's transport, F1/F2). The
/// client sends it in `Hello`, the session is the only thing that can judge it, and the two pages
/// keep their `?token=…` because a browser cannot put one anywhere else. Demanding it on the upgrade
/// as well would have been a second gate for a value that reaches this process in a frame, and the
/// URL that satisfied it is where the token leaked — into shell history, into a `Referer` and into
/// the log line of anything that prints the address (spec §9.2, §9.1). What keeps a silent socket
/// from holding a session is `SessionPolicy::handshake_timeout`, not an HTTP status.
///
/// **What replaced that check is the `Origin` check below, and it is the only thing that keeps a
/// *page* out of a *loopback* listener.** A WebSocket handshake is not subject to the same-origin
/// policy: nothing in a browser stops a page the user merely visits from opening
/// `ws://127.0.0.1:<port>/ws`, and `SessionPolicy::require_token` is false on loopback by design, so
/// such a page *is* admitted. That matters because `Command::UpdateConfig` carries the client's shape
/// (`ClientServerView` holds `bind` and `lan`) into `Hub::apply_config`, which persists it — so a page
/// could leave `bind = "0.0.0.0"` and `lan = true` behind for the **next** `serve` to bind the LAN
/// with. `--connect` sends no `Origin` at all, which is why the documented CLI attach is unaffected;
/// a page served from `/` sends this listener's own origin and is allowed. See
/// [`origin_is_this_host`] for what "its own" means, and [`forbidden`] for the status.
///
/// `ConnectInfo<SocketAddr>` is the peer's address, and Task 1.8's §9.4 guard is the one thing that
/// needs it: the rule is about **who** sent a `SetKey`, not about what the configuration says. The
/// `true` passed to `conn::serve` is whether this listener terminates TLS — the other half of the
/// same decision, and the one fact a socket cannot read off itself.
async fn ws(
    State(state): State<ServeState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    // Refused **before** the upgrade, so no session is spawned for a request that fails this: the
    // page never reaches `Hello`, and the hub never sees it.
    //
    // `403` and not `401`: no credential can change the answer — a token would not make another
    // page's origin this listener's own, and a loopback listener asks for no token at all — so
    // `401`'s "authenticate and try again" would be a lie, and `403` is the status for a request
    // that is understood, well-formed and refused. `400` is what axum itself answers a handshake
    // whose WebSocket headers are wrong, and this request's are not.
    if !origin_is_this_host(&headers) {
        return forbidden();
    }
    // The research note reads axum 0.8.9's own defaults: the write buffer is 128 KiB and
    // `max_write_buffer_size` is **unlimited** until it is set (`web-transport-and-terminal.md:1339-1344`).
    // A peer that stops reading must not grow the server's memory, so the ceiling is explicit.
    //
    // The read side gets the same treatment: axum's `max_message_size` default is **64 MiB** and
    // nothing in the protocol caps a frame's sample count, so one inbound message could have added
    // tens of megabytes to a session's buffer. `max_frame_size` is set from the same number so no
    // single frame can slip past the message ceiling.
    //
    // **`conn::with_limits` is the one place those four bounds are applied**, and this call is the
    // production half of it: the same function is what `conn.rs`'s test handler calls, and
    // `server::tests::the_read_ceiling_is_applied_on_the_route_that_ships` drives this router. Before
    // that test and that one place existed, deleting these bounds from this handler left the whole
    // workspace green (M1).
    let upgrade = conn::with_limits(upgrade, conn::ConnOptions::default());
    // Taken out before the upgrade so the async block owns what it needs: `spawn` needs the hub
    // itself, and `serve` needs another handle to it.
    let hub = state.hub.clone();
    let policy = state.policy.clone();
    // Whether this listener terminates TLS, taken off the state the router was built with. It travels
    // into the bridge because a socket cannot read its own transport: §9.4's rule is `tls || loopback`.
    let tls = state.tls;
    upgrade.on_upgrade(move |socket| async move {
        // `Hub::spawn` is the only path a connection uses: the session is attached to the registry
        // **before** `Welcome` goes out, which is what removes the race a spawn-then-attach ordering
        // would leave (Task 1.3; spec §5.2). It clones the shared slot the `Hub` was built with, so
        // this handler passes no providers and builds none.
        let ends = hub.spawn(policy);
        // `ClientKind::Web`: this socket carries the HUD protocol, not raw PTY bytes. `/terminal/ws`
        // (`ClientKind::WebTerminal`) is Task 2.2's. The kind travels with the socket because the
        // session's own front door is `Command::Hello`, which every connection sends first.
        conn::serve(socket, ends, hub.clone(), ClientKind::Web, peer, tls).await;
    })
}

/// Why a `/ws` upgrade whose `Origin` is another page's is refused. The status is the whole story:
/// **nothing a caller can present would change the answer** — a token is not what is missing, and the
/// loopback listener this is here to protect asks for none — so `401` would send a client looking for
/// a credential that does not exist, and `400` would call a well-formed request malformed.
///
/// The body names no address, no header and no token, and the refusal is not logged: a page may ask
/// as often as it likes, and each refusal is cheap precisely because no session was spawned for it.
fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "the Origin of this request is not this server's own host").into_response()
}

/// Whether a `/ws` upgrade is allowed by the one rule the absent token check left behind: **an
/// `Origin` that is present must be the host the request actually arrived on**.
///
/// - **Absent is allowed.** `--connect` and every other CLI client send no `Origin` at all — a header
///   only a browser adds — and that is the documented local attach (README, release checklist). The
///   check is about a browser; a browser always sends one.
/// - **Present and unreadable is refused.** `null` (a sandboxed frame, a `file://` page), another
///   scheme, a value with no authority, a `Host` that cannot be read: none of them is *provably* this
///   host, and "not provably ours" is not a state to admit. No legitimate client sends one.
/// - **Compared against the request's own host, never a spelling written here.** The reference is the
///   `Host` header, so a page served from `http://127.0.0.1:PORT` and one served from
///   `http://localhost:PORT` each open a socket (and send a `Host`) naming the name they were loaded
///   from, and each matches. This is also why the *page* must connect back to the host it was served
///   from: `http://localhost:PORT` is not `127.0.0.1:PORT`, and the two are different authorities
///   whatever they resolve to.
/// - Host names are compared case-insensitively, and a port omitted on both sides — or named on one
///   side and equal to the other's scheme default — is the same authority.
///
/// `/ws` and `/terminal/ws` both call this, and that is the point of it being one function: the
/// ledger's ruling after the second fix wave is that the terminal route needs the same check on the
/// same reasoning — the child it spawns is itself a client of `/ws` (`conn.rs`'s own comment records
/// why, and `progress.md` the wave that found it).
pub(crate) fn origin_is_this_host(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Some((origin_host, origin_port, default_port)) = origin.to_str().ok().and_then(origin_authority) else {
        return false;
    };
    let Some(host) = headers.get(header::HOST).and_then(|value| value.to_str().ok()) else {
        // A browser always sends `Host`; without one there is nothing to compare the origin to.
        return false;
    };
    let Some((host_name, host_port)) = authority_parts(host) else {
        return false;
    };
    origin_host == host_name && origin_port.unwrap_or(default_port) == host_port.unwrap_or(default_port)
}

/// `(host, port, the scheme's default port)` out of an `Origin`, or `None` when it is not an
/// `http`/`https` origin that can be read at all. The scheme must be one of those two: a browser's
/// page is served over one of them, and `file://`, `chrome-extension://` and `null` are pages this
/// listener has no business admitting.
fn origin_authority(origin: &str) -> Option<(String, Option<u16>, u16)> {
    let (scheme, rest) = origin.split_once("://")?;
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" => 80,
        // What a TLS listener's own page sends. The scheme is the page's, not the socket's; the
        // authority is the half that has to match.
        "https" => 443,
        _ => return None,
    };
    // An origin is `scheme://host[:port]` with nothing after it. A path, a query or a fragment is not
    // part of one, and reading past it cannot make a foreign host match, so it is ignored rather than
    // refused. The array pattern is also what keeps clippy at its seven pre-existing findings.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let (name, port) = authority_parts(authority)?;
    Some((name, port, default_port))
}

/// `(host, port)` out of an authority such as `127.0.0.1:7070`, `localhost` or `[::1]:7070`, or
/// `None` when it is not one: an empty name, whitespace, userinfo, a path, or a port that is not a
/// number. The name is folded to lower case because host names are case-insensitive and an `Origin`
/// is not required to preserve the case the user typed.
fn authority_parts(authority: &str) -> Option<(String, Option<u16>)> {
    if authority.is_empty() || authority.chars().any(|c| matches!(c, '@' | ' ' | '\t' | '/')) {
        return None;
    }
    let (name, port) = if let Some(rest) = authority.strip_prefix('[') {
        // An IPv6 literal, which is where a colon is not a port separator.
        let (name, tail) = rest.split_once(']')?;
        (format!("[{name}]"), tail.strip_prefix(':'))
    } else {
        match authority.split_once(':') {
            Some((name, port)) => (name.to_string(), Some(port)),
            None => (authority.to_string(), None),
        }
    };
    let port = match port {
        Some(text) => Some(text.parse().ok()?),
        None => None,
    };
    if name.is_empty() {
        return None;
    }
    Some((name.to_ascii_lowercase(), port))
}

/// The token the two **pages** are reached with, and the one shape a browser can send on a
/// navigation: spec §9.2's `?token=…`, which the two `no-store`/`no-referrer` headers exist for. It
/// is deliberately not read by `/ws`: a socket sends the token in `Hello`, where the session can
/// judge it, and a URL is the one place a secret is copied into shell history and logs.
#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// Spec §9.1: a server with no token accepts anything, and §5.1 guarantees that server is on
/// loopback. A server with a token needs it for its two pages and for its assets; the protocol
/// socket answers with a `Hello` refusal instead (see [`ws`]).
fn token_ok(state: &ServeState, query: &TokenQuery) -> bool {
    match &state.token {
        None => true,
        Some(expected) => query.token.as_deref() == Some(expected.as_str()),
    }
}

/// Never echoes the token, never names it, never logs it (spec §9.6).
fn refused() -> Response {
    (StatusCode::UNAUTHORIZED, "the token is missing or wrong").into_response()
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::keys::{KeyResolver, MemoryStore, SecretStore};

    /// How long a test lets the server do its work before calling it refused.
    const PATIENCE: Duration = Duration::from_millis(250);

    /// A real configuration file in a tempdir, and its path. The path matters now: `Hub::new` takes
    /// it, because a `SetProfile` or an `UpdateConfig` persists through the same file the server
    /// loaded (spec §5.2). The built-in `compae` profile is what `Config::active()` needs.
    fn config() -> (Config, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = crate::config::store::load_or_create(&path).unwrap().config;
        (config, path)
    }

    /// A resolver with a placeholder for each key name the default configuration asks for.
    ///
    /// **These are not secrets and this test must not depend on one existing.** The shared slot is
    /// filled from the configuration's key *names* (`model.rs:104`, `:113`, `:147-150`) when the
    /// caller builds it; without a value for each name, `build_providers` answers `MissingKey` and
    /// the server cannot start at all on a machine that has never run `key set`. The values never
    /// leave this process, never reach a provider, and never appear in a fixture — the connectors
    /// are built and no request is made.
    fn keys() -> KeyResolver {
        let store = MemoryStore::default();
        for name in ["openrouter", "openai", "elevenlabs"] {
            store.set(name, "test-placeholder-not-a-secret").expect("a memory store never fails");
        }
        KeyResolver::new(Box::new(store), Box::new(|_| None))
    }

    /// A real provider stack for the shared slot. The connectors are constructed and never called:
    /// no page route and no bind test starts a session.
    fn providers(config: &Config) -> ProviderSlot {
        ProviderSlot::new(
            crate::providers::build_providers(&config.providers, &keys())
                .expect("the placeholder resolver must satisfy the default configuration"),
        )
    }

    fn settings(bind: &str, token: &str) -> Settings {
        let (config, config_path) = config();
        Settings {
            bind: bind.into(),
            lan: false,
            token: token.into(),
            providers: providers(&config),
            config,
            config_path,
            keys: keys(),
            tls: None,
        }
    }

    /// Starts `run` on its own task and returns the address it bound, or the error it refused with.
    /// `run` never returns while it is serving, so "an address came back" is the success signal and
    /// "the task ended" is the refusal one.
    async fn start(settings: Settings) -> Result<SocketAddr, ServerError> {
        let (bound_tx, bound_rx) = tokio::sync::oneshot::channel();
        let mut handle = tokio::spawn(run(settings, bound_tx));
        if let Ok(addr) = bound_rx.await {
            return Ok(addr);
        }
        match tokio::time::timeout(PATIENCE, &mut handle).await {
            Ok(Ok(Err(e))) => Err(e),
            Ok(Ok(Ok(()))) => panic!("the server returned without binding and without an error"),
            Ok(Err(join)) => panic!("the server task panicked: {join}"),
            Err(_) => panic!("no bound address and no refusal inside {PATIENCE:?}"),
        }
    }

    /// The status one page answers with, with or without a `?token=`.
    async fn page_status(addr: SocketAddr, path: &str, token: Option<&str>) -> u16 {
        let url = match token {
            Some(t) => format!("http://{addr}{path}?token={t}"),
            None => format!("http://{addr}{path}"),
        };
        reqwest::get(url).await.unwrap().status().as_u16()
    }

    /// F11: `Settings`' hand-written `Debug` said the token was not printed and then printed the whole
    /// configuration, whose own `Debug` prints it. A formatter written to be safe has to be.
    ///
    /// The token is put in **both** fields on purpose: `Settings::token` was already collapsed, and the
    /// leak was `Settings::config.server.token` — which is the one a test that only set the first
    /// would not see.
    #[test]
    fn settings_debug_never_prints_the_token() {
        let mut settings = settings("127.0.0.1:0", "");
        settings.token = "correct-horse".into();
        settings.config.server.token = "correct-horse".into();

        let line = format!("{settings:?}");
        assert!(!line.contains("correct-horse"), "the token reached a `Debug` line: {line}");
        assert!(line.contains(r#"token: "<set>""#), "and the line still says whether one is set: {line}");
        assert!(line.contains("127.0.0.1:0"), "the fields that describe the run are still there: {line}");
    }

    #[tokio::test]
    async fn a_loopback_bind_with_no_token_starts() {
        let addr = start(settings("127.0.0.1:0", "")).await.expect("loopback needs no token");
        assert!(addr.ip().is_loopback(), "the server must be on loopback, got {addr}");
        assert!(addr.port() != 0, "the kernel's port is reported, not the 0 that was asked for");
    }

    /// The refusal names the **address**, the token and nothing else. `lan` is not a term in §5.1's
    /// rule — it is read for the log line and no guard — so the sentence no longer offers a switch
    /// that could not make this bind legal (F15's residue; the README's "the bind rule is about the
    /// address, not the switch" is the same claim a user reads first).
    #[tokio::test]
    async fn a_wildcard_bind_with_no_token_refuses_and_names_the_address() {
        // `0.0.0.0` is unspecified, not loopback (research note `:540`), so the rule catches it.
        let error = start(settings("0.0.0.0:0", "")).await.expect_err("0.0.0.0 is every interface");
        let message = error.to_string();
        assert!(message.contains("0.0.0.0"), "the message names the address it refused: {message}");
        assert!(message.contains("127.0.0.1"), "and an address that would be legal instead: {message}");
        assert!(message.contains("token"), "and the token it needs: {message}");
        assert!(!message.contains("lan = true"), "and not the decorative switch that cannot help: {message}");
    }

    #[tokio::test]
    async fn a_hostname_bind_is_refused_before_the_bind() {
        let error = start(settings("localhost:7070", "")).await.expect_err("a hostname may resolve off-box");
        let message = error.to_string();
        assert!(message.contains("localhost:7070"), "the message names the address: {message}");
        assert!(message.contains("IP:port"), "and says what shape it wanted: {message}");
    }

    #[tokio::test]
    async fn a_loopback_bind_with_a_token_starts_and_the_wrong_token_is_refused() {
        let addr = start(settings("127.0.0.1:0", "correct-horse")).await.expect("a token never stops a loopback bind");
        assert_eq!(page_status(addr, "/", Some("correct-horse")).await, 200, "the token it holds opens the page");
        assert_eq!(page_status(addr, "/", Some("wrong-horse")).await, 401, "another token does not");
        assert_eq!(page_status(addr, "/terminal", None).await, 401, "and both pages need it");
    }

    /// **F2, at the route.** The documented remote attach presents the token in `Hello` and sends no
    /// query at all. `/ws` used to answer a socket with no `?token=` with a **401 before the upgrade**,
    /// which made that attach impossible against any server holding a token and pushed users onto the
    /// URL form that leaks it into shell history, a `Referer` and every log that prints an address
    /// (spec §9.1, §9.2). The token is judged by the session now, exactly once, in the frame it
    /// belongs to.
    ///
    /// The three providers point at a refused loopback port rather than at their real hosts, so the
    /// accepted halves run the real greeting and make no outbound request to a third party.
    ///
    /// The two listeners here are the two halves of §4.1, and they are deliberately different: on an
    /// **exposed** one the token is the door, and on a **loopback** one there is no door to lock —
    /// `SessionPolicy::require_token` is built from the bound address, which the design says is the
    /// only fact about exposure. The pages keep their `?token=…` on both.
    #[tokio::test]
    async fn the_socket_takes_its_token_in_hello_and_not_in_the_url() {
        use futures::{SinkExt, StreamExt};
        use tipsy_protocol::{ClientKind, Command, PROTOCOL_VERSION};
        use tokio_tungstenite::tungstenite::Message;

        type Client = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

        async fn text_frame(socket: &mut Client) -> String {
            let message = tokio::time::timeout(PATIENCE, socket.next())
                .await
                .expect("the bridge must answer inside the patience window")
                .expect("the socket must stay open")
                .expect("the socket must not error");
            match message {
                Message::Text(text) => text.to_string(),
                other => panic!("the server sends JSON text here, got {other:?}"),
            }
        }

        /// One attach with no query at all, answering with the first text frame the server sends.
        async fn attach(port: u16, token: Option<&str>) -> String {
            let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws"))
                .await
                .expect("the upgrade must not require a query");
            let hello = Command::Hello {
                token: token.map(str::to_string),
                client: ClientKind::Web,
                protocol_version: PROTOCOL_VERSION,
            };
            socket.send(Message::text(serde_json::to_string(&hello).unwrap())).await.unwrap();
            text_frame(&mut socket).await
        }

        // An exposed listener: `0.0.0.0` is not loopback, so its sessions require the token.
        let mut exposed = settings("0.0.0.0:0", "correct-horse");
        for provider in [
            &mut exposed.config.providers.chat,
            &mut exposed.config.providers.speech,
            &mut exposed.config.providers.transcribe,
        ] {
            provider.base_url = "http://127.0.0.1:9/v1".into();
        }
        exposed.providers = providers(&exposed.config);
        let bound = start(exposed).await.expect("a token makes a wide bind legal");

        let welcome = attach(bound.port(), Some("correct-horse")).await;
        assert!(welcome.contains(r#""type":"welcome""#), "the token in `Hello` is what opens the session: {welcome}");

        let wrong = attach(bound.port(), Some("wrong-horse")).await;
        assert!(wrong.contains(r#""type":"error""#) && wrong.contains("token_invalid"), "{wrong}");
        assert!(!wrong.contains("wrong-horse"), "the refused token must not be echoed: {wrong}");

        let absent = attach(bound.port(), None).await;
        assert!(absent.contains("token_required"), "and an exposed server needs one presented at all: {absent}");

        // A loopback listener is this machine, so its sessions require no token: the documented local
        // attach (`--connect ws://127.0.0.1:7070/ws`, no `--token`) works even with a token in the
        // file. The pages are the half that still demands it here.
        let loopback =
            start(settings("127.0.0.1:0", "correct-horse")).await.expect("a loopback bind needs no token to be legal");
        let local = attach(loopback.port(), None).await;
        assert!(local.contains(r#""type":"welcome""#), "a loopback listener asks for no token: {local}");
    }

    /// **H1, the blocker the fix wave's scoped re-review found.** Removing the route's `?token=` check
    /// was right — the token belongs in `Hello` — but that check was also the only thing keeping a
    /// **page** out of a loopback listener, because a WebSocket handshake is not subject to the
    /// same-origin policy and `SessionPolicy` asks for no token on loopback. What a page gains is not
    /// a key: `Command::UpdateConfig` carries the client's shape, `ClientServerView` holds `bind` and
    /// `lan`, and `Hub::apply_config` **persists** it — so a page the user merely visits could leave a
    /// LAN-wide bind behind for the next `serve` to use.
    ///
    /// The listener here holds a token on purpose: what is under test is that the `Origin` header is
    /// what refuses the page, not the token, which a loopback session never asks for.
    ///
    /// Three shapes, in the order that matters: the CLI's (no `Origin` — it must attach), the
    /// listener's own page in **both** spellings (`127.0.0.1` and `localhost`, each reaching the
    /// server under its own name), and any other page (refused before the upgrade).
    #[tokio::test]
    async fn a_foreign_origin_is_refused_and_a_client_with_no_origin_attaches() {
        use futures::{SinkExt, StreamExt};
        use tipsy_protocol::{ClientKind, Command, PROTOCOL_VERSION};
        use tokio_tungstenite::tungstenite::{ClientRequestBuilder, Error, Message, http::Uri};

        /// One attach to `host:port`. `Origin` is set when asked for and **absent** otherwise, which is
        /// exactly the request `--connect` sends: `tungstenite`'s `IntoClientRequest for &str` adds no
        /// `Origin`, and `ClientRequestBuilder` starts from that same request and only appends.
        async fn attach(host: &str, port: u16, origin: Option<&str>) -> Result<String, Error> {
            let uri: Uri = format!("ws://{host}:{port}/ws").parse().expect("a literal url parses");
            let mut request = ClientRequestBuilder::new(uri);
            if let Some(origin) = origin {
                request = request.with_header("Origin", origin);
            }
            let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
            let hello = Command::Hello { token: None, client: ClientKind::Web, protocol_version: PROTOCOL_VERSION };
            socket
                .send(Message::text(serde_json::to_string(&hello).unwrap()))
                .await
                .expect("the socket must stay open");
            let message = tokio::time::timeout(PATIENCE, socket.next())
                .await
                .expect("the bridge must answer inside the patience window")
                .expect("the socket must stay open")
                .expect("the socket must not error");
            match message {
                Message::Text(text) => Ok(text.to_string()),
                other => panic!("the server sends JSON text here, got {other:?}"),
            }
        }

        // The greeting would otherwise reach the real provider hosts; a refused loopback port answers
        // it in microseconds and keeps this test off the network, exactly as the token test above does.
        let mut loopback = settings("127.0.0.1:0", "correct-horse");
        for provider in [
            &mut loopback.config.providers.chat,
            &mut loopback.config.providers.speech,
            &mut loopback.config.providers.transcribe,
        ] {
            provider.base_url = "http://127.0.0.1:9/v1".into();
        }
        loopback.providers = providers(&loopback.config);
        let addr = start(loopback).await.expect("a token never stops a loopback bind");
        let port = addr.port();

        // The documented local attach: no `Origin` at all, and no token, against a listener whose
        // configuration holds one. This is the half the fix must not break.
        let cli =
            attach("127.0.0.1", port, None).await.expect("a client that sends no Origin is the CLI, and must attach");
        assert!(cli.contains(r#""type":"welcome""#), "a loopback listener asks for no token: {cli}");

        // The listener's own page: it was served from this host, so this origin is this host — and
        // each spelling is compared against the `Host` the request arrived with, not against a name
        // written into the code.
        for (host, origin) in
            [("127.0.0.1", format!("http://127.0.0.1:{port}")), ("localhost", format!("http://localhost:{port}"))]
        {
            let own = attach(host, port, Some(&origin))
                .await
                .unwrap_or_else(|error| panic!("the listener's own page at {origin} must attach: {error}"));
            assert!(own.contains(r#""type":"welcome""#), "the page served from {origin} is this host: {own}");
        }

        // Any other page: refused before the upgrade, so no session is ever spawned for it.
        let foreign = attach("127.0.0.1", port, Some("http://evil.example"))
            .await
            .expect_err("a page that is not this host must not reach a session");
        match foreign {
            Error::Http(response) => assert_eq!(
                response.status().as_u16(),
                403,
                "the refusal is a decision, not a missing credential: 401 would ask for a token that \
                 would not help"
            ),
            other => panic!("a foreign Origin must fail the handshake with an HTTP status, got {other:?}"),
        }
    }

    /// The `Origin` comparison itself, without a socket: every shape a browser can send, and the two
    /// spellings of this machine. A browser sends `scheme://host[:port]`, and the host it has to match
    /// is the `Host` header the request arrived with.
    #[test]
    fn an_origin_matches_the_host_the_request_arrived_on() {
        fn headers(origin: Option<&str>, host: Option<&str>) -> HeaderMap {
            let mut headers = HeaderMap::new();
            if let Some(origin) = origin {
                headers.insert(header::ORIGIN, origin.parse().expect("a literal origin header"));
            }
            if let Some(host) = host {
                headers.insert(header::HOST, host.parse().expect("a literal host header"));
            }
            headers
        }

        // Absent is the CLI, and it is the half that must never be refused.
        assert!(origin_is_this_host(&headers(None, None)), "`--connect` sends no Origin at all");
        assert!(
            origin_is_this_host(&headers(None, Some("127.0.0.1:7070"))),
            "and the check is about the header, not the host"
        );

        // The listener's own page, in either spelling, case-folded, and with a default port omitted
        // on both sides.
        assert!(origin_is_this_host(&headers(Some("http://127.0.0.1:7070"), Some("127.0.0.1:7070"))));
        assert!(origin_is_this_host(&headers(Some("http://localhost:7070"), Some("localhost:7070"))));
        assert!(origin_is_this_host(&headers(Some("https://LOCALHOST:7070"), Some("localhost:7070"))));
        assert!(origin_is_this_host(&headers(Some("http://127.0.0.1"), Some("127.0.0.1"))));
        assert!(origin_is_this_host(&headers(Some("http://example.test:80"), Some("example.test"))));

        // A page anywhere else. The two loopback spellings are deliberately not interchangeable:
        // `localhost:7070` and `127.0.0.1:7070` are different authorities, and the page that must be
        // admitted is the one served from the name the socket names.
        assert!(!origin_is_this_host(&headers(Some("http://evil.example"), Some("127.0.0.1:7070"))));
        assert!(!origin_is_this_host(&headers(Some("http://localhost:7070"), Some("127.0.0.1:7070"))));
        assert!(!origin_is_this_host(&headers(Some("http://127.0.0.1:7071"), Some("127.0.0.1:7070"))), "another port");
        assert!(
            !origin_is_this_host(&headers(Some("http://127.0.0.1:7070"), Some("127.0.0.1:7071"))),
            "either way round"
        );

        // Present and unreadable: not provably this host, so refused. `null` is what a sandboxed frame
        // and a `file://` page send, and neither scheme is one this listener serves a page over.
        assert!(!origin_is_this_host(&headers(Some("null"), Some("127.0.0.1:7070"))));
        assert!(!origin_is_this_host(&headers(Some("file://127.0.0.1:7070"), Some("127.0.0.1:7070"))));
        assert!(!origin_is_this_host(&headers(Some("chrome-extension://abcdef"), Some("127.0.0.1:7070"))));
        assert!(!origin_is_this_host(&headers(Some("127.0.0.1:7070"), Some("127.0.0.1:7070"))), "no scheme at all");
        assert!(!origin_is_this_host(&headers(Some("http://"), Some("127.0.0.1:7070"))));
        assert!(!origin_is_this_host(&headers(Some("http://127.0.0.1:not-a-port"), Some("127.0.0.1:7070"))));
        assert!(!origin_is_this_host(&headers(Some("http://user@127.0.0.1:7070"), Some("127.0.0.1:7070"))));
        // An `Origin` with no `Host` to compare it to is refused rather than guessed at.
        assert!(!origin_is_this_host(&headers(Some("http://127.0.0.1:7070"), None)));
    }

    /// **M1: the read ceiling on the route that ships.** `READ_CEILING` was applied in two places and
    /// only `conn.rs`'s test handler's copy was covered, so deleting it from `server::ws` left the
    /// workspace at 505 passed, 0 failed: the value and the mechanism were proven, and the production
    /// route's *use* of them was not. Two things close that now — `conn::with_limits` is the one place
    /// either handler applies the bounds, and this test drives `run`'s real router, so it is the
    /// shipping path and not a copy of it.
    ///
    /// The frame is large on purpose: this listener uses `ConnOptions::default()`, so the only frame
    /// that proves anything here is one over 256 KiB — 140 000 samples are 280 000 bytes, which is over
    /// `READ_CEILING` and far under axum's own 64 MiB default. A frame under the ceiling goes first, so
    /// the connection is known to have been alive and being read a moment before the assertion.
    #[tokio::test]
    async fn the_read_ceiling_is_applied_on_the_route_that_ships() {
        use futures::{SinkExt, StreamExt};
        use tipsy_protocol::{ClientKind, Command, Frame, PROTOCOL_VERSION};
        use tokio_tungstenite::tungstenite::Message;

        // The greeting would otherwise reach the real provider hosts; a refused loopback port answers
        // it in microseconds.
        let mut loopback = settings("127.0.0.1:0", "correct-horse");
        for provider in [
            &mut loopback.config.providers.chat,
            &mut loopback.config.providers.speech,
            &mut loopback.config.providers.transcribe,
        ] {
            provider.base_url = "http://127.0.0.1:9/v1".into();
        }
        loopback.providers = providers(&loopback.config);
        let addr = start(loopback).await.expect("a token never stops a loopback bind");

        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}/ws", addr.port()))
            .await
            .expect("the upgrade must not require a query");
        let hello = Command::Hello { token: None, client: ClientKind::Web, protocol_version: PROTOCOL_VERSION };
        socket.send(Message::text(serde_json::to_string(&hello).unwrap())).await.unwrap();

        let small = Frame::Mic(vec![0i16; 8_000]).encode();
        assert!(small.len() < conn::READ_CEILING, "the frame under the ceiling has to be under it");
        socket.send(Message::Binary(small.into())).await.unwrap();

        let oversized = Frame::Mic(vec![0i16; 140_000]).encode();
        assert!(oversized.len() > conn::READ_CEILING, "the frame has to be over the ceiling to prove it");
        socket.send(Message::Binary(oversized.into())).await.unwrap();

        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(!left.is_zero(), "a message over the read ceiling must end the connection");
            match tokio::time::timeout(left, socket.next()).await {
                Ok(Some(Ok(_))) => continue,
                Ok(Some(Err(_))) | Ok(None) => break,
                Err(_) => panic!("the connection survived a message over the production read ceiling"),
            }
        }
    }

    /// **The first thing the phase's live check caught.** `serve --tls` called
    /// `bind_rustls(local, …)`, which binds the address a *second* time while the `std` listener from
    /// `run` still holds it: on Windows that is `WSAEADDRINUSE (10048)`, so a run logged
    /// `listening … tls=true` and then stopped with nothing serving at all. Every TLS test waited only
    /// for the `bound` message, which `run` sends before this branch, so none of them could see it.
    /// This one speaks HTTPS to the port the run reported.
    ///
    /// The client accepts the self-signed certificate on purpose: what is under test is that something
    /// is listening and terminating TLS, not the trust chain (§9.5 covers that part).
    #[tokio::test]
    async fn a_tls_listener_really_serves_https() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = crate::config::store::load_or_create(&path).unwrap().config;
        let mut settings = settings("127.0.0.1:0", "correct-horse");
        settings.config = config;
        settings.config_path = path.clone();
        settings.tls = Some(TlsFiles::Generated);
        let addr = start(settings).await.expect("a loopback bind needs no token, with TLS or without");

        let client = reqwest::Client::builder().danger_accept_invalid_certs(true).build().expect("a test client");
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            client.get(format!("https://{addr}/?token=correct-horse")).send(),
        )
        .await
        .expect("the TLS listener must answer inside the patience window")
        .expect("the TLS listener must be there to answer");
        assert_eq!(response.status().as_u16(), 200, "a TLS listener serves the same pages as the plain one");
        assert_eq!(
            response.headers().get("referrer-policy").and_then(|v| v.to_str().ok()),
            Some("no-referrer"),
            "and carries the same page headers"
        );
        assert!(path.with_file_name("serve-cert.pem").exists(), "the generated pair sits beside the config");
    }

    #[tokio::test]
    async fn both_pages_carry_the_two_headers() {
        let addr = start(settings("127.0.0.1:0", "")).await.expect("loopback needs no token");
        for path in ["/", "/terminal"] {
            let response = reqwest::get(format!("http://{addr}{path}")).await.unwrap();
            assert_eq!(response.status(), 200, "{path}");
            assert_eq!(
                response.headers().get("referrer-policy").and_then(|v| v.to_str().ok()),
                Some("no-referrer"),
                "{path} must not put the token in a Referer (spec §5.3, §9.2)"
            );
            assert_eq!(
                response.headers().get("cache-control").and_then(|v| v.to_str().ok()),
                Some("no-store"),
                "{path} must not be cached (spec §9.2, RFC 6750 §2.3)"
            );
        }
    }

    #[tokio::test]
    async fn both_halves_of_the_bind_rule_are_exercised() {
        // Step 1 and step 2, before the bind, called directly so a regression names itself.
        assert!(matches!(guard_pre_bind("0.0.0.0:0", None), Err(ServerError::LanWithoutToken { .. })));
        assert!(matches!(guard_pre_bind("bind.me:7070", None), Err(ServerError::BadBind { .. })));
        assert!(guard_pre_bind("0.0.0.0:0", Some("t")).is_ok(), "a token makes a wide bind legal");
        assert!(guard_pre_bind("127.0.0.1:7070", None).is_ok(), "loopback needs nothing");

        // Step 3, after the bind. The only way to reach a non-loopback `local_addr` here is to ask
        // for one directly — which is exactly what the third check exists for: a listener that came
        // out wider than the address that was requested. `std::net`, because that is the socket `run`
        // binds before it decides which listener serves it.
        let wide = std::net::TcpListener::bind("0.0.0.0:0").expect("0.0.0.0 is bindable");
        assert!(!wide.local_addr().unwrap().ip().is_loopback(), "0.0.0.0 is not loopback");
        assert!(matches!(guard_post_bind(&wide, None), Err(ServerError::BoundWiderThanAsked { .. })));
        assert!(guard_post_bind(&wide, Some("t")).is_ok(), "a token makes it legal after the bind too");
        let tight = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        assert!(guard_post_bind(&tight, None).is_ok(), "a loopback bind passes with no token");
    }

    /// The seam Task 1.9 exists to close: `run` used to `println!` the minted pair's path and its
    /// fingerprint and `eprintln!` the four §9.5 lines. They are `tracing` records now, and this is
    /// what says so — the real subscriber over a tempfile, read back. It installs the **global** slot
    /// rather than a thread-local one on purpose: `run` is a task of its own, and a thread-local
    /// default would not reach it.
    ///
    /// Nothing here touches `%APPDATA%`: the configuration, the log and the generated certificate all
    /// live in the tempdir this test made. Running the binary instead would have written the user's
    /// own configuration, which is why no test does that (`store::default_path` has no override).
    ///
    /// `lan = true` on a loopback bind is legal — §5.1's rule is about the bound address — and it is
    /// the value that makes `lan=true` below this test's own line rather than another run's.
    #[tokio::test]
    async fn the_tls_lines_are_log_records_and_the_log_says_which_mode_was_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = crate::config::store::load_or_create(&path).unwrap().config;
        let log = crate::logging::log_path(&path);
        let subscriber = crate::logging::subscriber_at(&log, crate::logging::Secrets::default(), false).unwrap();
        tracing::subscriber::set_global_default(subscriber).expect("this binary installs no other global subscriber");

        let mut settings = settings("127.0.0.1:0", "");
        settings.lan = true;
        settings.config = config;
        settings.config_path = path.clone();
        settings.tls = Some(TlsFiles::Generated);
        let addr = start(settings).await.expect("a loopback bind needs no token, with TLS or without");
        assert!(addr.port() != 0);

        // The bound address is sent before the TLS branch and before the `listening` record, so the
        // file is awaited rather than assumed; `lan=true` is this test's own line, so waiting for it
        // is waiting for the last of the four records. What this asserts is the file's contents, and
        // the file is written by whoever runs.
        let mut written = String::new();
        for _ in 0..100 {
            written = std::fs::read_to_string(&log).unwrap_or_default();
            if written.contains("lan=true") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(written.contains("generated a self-signed certificate"), "{written}");
        assert!(written.contains("SHA-256 of the certificate"), "{written}");
        assert!(written.contains("does not unlock the browser microphone"), "{written}");
        assert!(written.contains("lan=true"), "the log says which mode the operator asked for: {written}");
        assert!(written.contains("tls=true"), "and that this listener is encrypted: {written}");
    }

    /// The address the terminal child attaches to, which is **not** always the address this listener
    /// was handed. A wildcard bind answers on loopback but cannot be *reached* as `0.0.0.0`, and a
    /// listener on one specific interface is not listening on loopback at all — so both halves have a
    /// way to be wrong, and this is the test that says which one wins.
    #[test]
    fn a_terminal_child_attaches_to_the_address_that_is_really_there() {
        assert_eq!(terminal_child_url("127.0.0.1:7070".parse().unwrap()), "ws://127.0.0.1:7070/ws");
        assert_eq!(
            terminal_child_url("0.0.0.0:7070".parse().unwrap()),
            "ws://127.0.0.1:7070/ws",
            "the wildcard answers on loopback, and `0.0.0.0` is not a host a client can reach"
        );
        assert_eq!(
            terminal_child_url("192.168.1.5:7070".parse().unwrap()),
            "ws://192.168.1.5:7070/ws",
            "one interface means nothing is listening on loopback, so that address is the only one"
        );
        assert_eq!(
            terminal_child_url("[::1]:7070".parse().unwrap()),
            "ws://[::1]:7070/ws",
            "an IPv6 literal needs its brackets or the port becomes part of the host"
        );
        assert_eq!(terminal_child_url("[::]:7070".parse().unwrap()), "ws://[::1]:7070/ws", "the same for `::`");
        assert_eq!(terminal_child_url("[2001:db8::1]:7070".parse().unwrap()), "ws://[2001:db8::1]:7070/ws");
    }

    /// The child's argument vector: the URL above, and `--token` only when this server holds one.
    /// Presenting an empty token is a different claim from presenting none, and on loopback there is
    /// no door for one to open (spec §5.3, §9.1).
    #[test]
    fn a_terminal_child_is_handed_the_token_only_when_there_is_one() {
        // A fixture, not a secret: it reaches a child this test never starts, and nothing resolves a
        // key from it. The workspace rule is that a real key appears by name and never in a fixture.
        let token = "fixture-token-0123456789abcdef";
        let bound = "127.0.0.1:7070".parse().unwrap();

        let with = terminal_args(bound, Some(token));
        assert_eq!(with[0], "--connect");
        assert_eq!(with[1], "ws://127.0.0.1:7070/ws");
        assert_eq!(&with[2..], ["--token", token], "an exposed listener's child has to present the token in Hello");

        let without = terminal_args(bound, None);
        assert_eq!(without, vec!["--connect".to_string(), "ws://127.0.0.1:7070/ws".to_string()]);
        assert!(!without.iter().any(|arg| arg == "--token"), "no token means no `--token` argument at all");
    }
}
