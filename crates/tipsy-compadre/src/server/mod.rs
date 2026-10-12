//! The headless server: the listener, the router and the two checks that keep a LAN behind a token.

pub mod assets;
pub mod conn;
pub mod hub;
pub mod tls;

pub use hub::Hub;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, Path as AxumPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use tipsy_protocol::ClientKind;

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
/// the token is *not* printed, because a `Debug` line reaches a log more often than anyone intends
/// (spec §9.6).
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("bind", &self.bind)
            .field("lan", &self.lan)
            .field("token", &if self.token.trim().is_empty() { "<none>" } else { "<set>" })
            .field("config", &self.config)
            .field("config_path", &self.config_path)
            .field("keys", &"<key resolver>")
            .field("providers", &"<provider slot>")
            .field("tls", &self.tls)
            .finish()
    }
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
    BadBind { bind: String },
    /// Not loopback, and no token (spec §5.1 step 2, spec §10).
    LanWithoutToken { addr: SocketAddr },
    /// The bind landed somewhere other than what was asked for, with no token to guard it
    /// (spec §5.1 step 3).
    BoundWiderThanAsked { addr: SocketAddr },
    Io(std::io::Error),
    /// TLS could not be set up: a pair the user named is absent or unparseable, or a self-signed pair
    /// could not be minted (spec §5.5, §10). **Task 1.8's variant**: it cannot come from Task 1.4,
    /// because `TlsError` does not exist until `server/tls.rs` does. It is transparent in `Display`
    /// and in `source` for one reason: `TlsError`'s sentence already names the paths a user has to
    /// look at, and it is the line a failed `serve` prints.
    Tls(tls::TlsError),
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
            ServerError::LanWithoutToken { addr } => write!(
                f,
                "refusing to bind {addr}: it is not a loopback address and no token is configured. \
                 Set a token, or set `lan = true` in [server] together with a token."
            ),
            ServerError::BoundWiderThanAsked { addr } => write!(
                f,
                "bound to {addr}, which is not a loopback address, and no token is configured"
            ),
            ServerError::Io(e) => write!(f, "cannot listen: {e}"),
            // Transparent on purpose: `TlsError`'s own sentence already names the paths a user has
            // to look at, and wrapping it again here would put a second prefix in front of them.
            ServerError::Tls(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServerError::Io(e) => Some(e),
            ServerError::Tls(e) => Some(e),
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
pub(crate) fn state(settings: &Settings, hub: Hub, policy: SessionPolicy, tls: bool) -> ServeState {
    ServeState { token: token_of(&settings.token), hub, policy, tls }
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
                // `tipsy-compadre.log`, where a reader who missed the screen can still find it. The
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
    let app = mount(state(&settings, hub, policy, tls_config.is_some()));
    // **The same `app` value in both arms**, which is what makes "a TLS listener serves the same
    // routes as the plain one" true by construction rather than by two routers being kept in step.
    // `into_make_service_with_connect_info::<SocketAddr>()` is what gives the `/ws` handler the peer
    // address Task 1.8's §9.4 guard reads; without it that extractor refuses the request, so both
    // arms need it.
    let handle = axum_server::Handle::<SocketAddr>::new();
    let outcome = match tls_config {
        Some(config) => {
            axum_server::tls_rustls::bind_rustls(local, config)
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
        // `/terminal/ws` belongs to Task 2.2 and is deliberately not registered here: it is a raw
        // PTY byte socket with its own handshake, and it is a 404 until Phase 2 lands.
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
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type.parse().expect("a MIME type is a header value"));
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
/// `ConnectInfo<SocketAddr>` is the peer's address, and Task 1.8's §9.4 guard is the one thing that
/// needs it: the rule is about **who** sent a `SetKey`, not about what the configuration says. The
/// `true` passed to `conn::serve` is whether this listener terminates TLS — the other half of the
/// same decision, and the one fact a socket cannot read off itself.
async fn ws(
    State(state): State<ServeState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    upgrade: WebSocketUpgrade,
) -> Response {
    // The research note reads axum 0.8.9's own defaults: the write buffer is 128 KiB and
    // `max_write_buffer_size` is **unlimited** until it is set (`web-transport-and-terminal.md:1339-1344`).
    // A peer that stops reading must not grow the server's memory, so the ceiling is explicit.
    //
    // The read side gets the same treatment: axum's `max_message_size` default is **64 MiB** and
    // nothing in the protocol caps a frame's sample count, so one inbound message could have added
    // tens of megabytes to a session's buffer. `max_frame_size` is set from the same number so no
    // single frame can slip past the message ceiling.
    let options = conn::ConnOptions::default();
    let upgrade = upgrade
        .write_buffer_size(options.write_buffer_size)
        .max_write_buffer_size(options.max_write_buffer_size)
        .max_message_size(options.max_message_size)
        .max_frame_size(options.max_message_size);
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

    #[tokio::test]
    async fn a_loopback_bind_with_no_token_starts() {
        let addr = start(settings("127.0.0.1:0", "")).await.expect("loopback needs no token");
        assert!(addr.ip().is_loopback(), "the server must be on loopback, got {addr}");
        assert!(addr.port() != 0, "the kernel's port is reported, not the 0 that was asked for");
    }

    #[tokio::test]
    async fn a_wildcard_bind_with_no_token_refuses_and_names_lan() {
        // `0.0.0.0` is unspecified, not loopback (research note `:540`), so the rule catches it.
        let error = start(settings("0.0.0.0:0", "")).await.expect_err("0.0.0.0 is every interface");
        let message = error.to_string();
        assert!(message.contains("0.0.0.0"), "the message names the address it refused: {message}");
        assert!(message.contains("lan = true"), "and the switch that would make it legal: {message}");
        assert!(message.contains("token"), "and the token it needs: {message}");
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
        for provider in [&mut exposed.config.providers.chat, &mut exposed.config.providers.speech, &mut exposed.config.providers.transcribe] {
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
        let loopback = start(settings("127.0.0.1:0", "correct-horse")).await.expect("a loopback bind needs no token to be legal");
        let local = attach(loopback.port(), None).await;
        assert!(local.contains(r#""type":"welcome""#), "a loopback listener asks for no token: {local}");
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
        tracing::subscriber::set_global_default(subscriber)
            .expect("this binary installs no other global subscriber");

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
}
