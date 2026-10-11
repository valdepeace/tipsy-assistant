//! The headless server: the listener, the router and the two checks that keep a LAN behind a token.

pub mod assets;
pub mod conn;
pub mod hub;

pub use hub::Hub;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path as AxumPath, Query, State};
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
#[derive(Clone, Debug)]
pub enum TlsFiles {
    Generated,
    Supplied { cert: PathBuf, key: PathBuf },
}

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
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServerError::Io(e) => Some(e),
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
pub(crate) fn state(settings: &Settings, hub: Hub, policy: SessionPolicy) -> ServeState {
    ServeState { token: token_of(&settings.token), hub, policy }
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
fn guard_post_bind(listener: &tokio::net::TcpListener, token: Option<&str>) -> Result<(), ServerError> {
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
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(ServerError::Io)?;
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
    };
    // Task 1.8's TLS branch goes here: `tls::load_or_generate(files)` against `settings.tls`, with
    // the listener handed to `axum_server::tls_rustls::bind_rustls` instead of `axum::serve`. Doing
    // the loading inside `run` is what makes an absent or unparseable `--tls-cert` a `ServerError`
    // naming the path (spec §10) rather than a second error type in the CLI. `None` is no TLS, and
    // that is the only branch this task implements.
    let _ = &settings.tls;
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
    let app = mount(state(&settings, hub, policy));
    if let Err(e) = axum::serve(listener, app).await {
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
async fn ws(State(state): State<ServeState>, query: Query<TokenQuery>, upgrade: WebSocketUpgrade) -> Response {
    if !token_ok(&state, &query) {
        return refused();
    }
    // The research note reads axum 0.8.9's own defaults: the write buffer is 128 KiB and
    // `max_write_buffer_size` is **unlimited** until it is set (`web-transport-and-terminal.md:1339-1344`).
    // A peer that stops reading must not grow the server's memory, so the ceiling is explicit.
    let options = conn::ConnOptions::default();
    let upgrade = upgrade
        .write_buffer_size(options.write_buffer_size)
        .max_write_buffer_size(options.max_write_buffer_size);
    // Taken out before the upgrade so the async block owns what it needs: `spawn` needs the hub
    // itself, and `serve` needs another handle to it.
    let hub = state.hub.clone();
    let policy = state.policy.clone();
    upgrade.on_upgrade(move |socket| async move {
        // `Hub::spawn` is the only path a connection uses: the session is attached to the registry
        // **before** `Welcome` goes out, which is what removes the race a spawn-then-attach ordering
        // would leave (Task 1.3; spec §5.2). It clones the shared slot the `Hub` was built with, so
        // this handler passes no providers and builds none.
        let ends = hub.spawn(policy);
        // `ClientKind::Web`: this socket carries the HUD protocol, not raw PTY bytes. `/terminal/ws`
        // (`ClientKind::WebTerminal`) is Task 2.2's. The kind travels with the socket because the
        // session's own front door is `Command::Hello`, which every connection sends first.
        conn::serve(socket, ends, hub.clone(), ClientKind::Web).await;
    })
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// Spec §9.1: a server with no token accepts anything, and §5.1 guarantees that server is on
/// loopback. A server with a token needs it on the upgrade for the pages and, until Task 1.2's
/// `Hello` validation answers, for the socket too — the query form is what a browser can send on a
/// navigation, which is the whole reason §9.2 pays for it.
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
        // out wider than the address that was requested.
        let wide = tokio::net::TcpListener::bind("0.0.0.0:0").await.expect("0.0.0.0 is bindable");
        assert!(!wide.local_addr().unwrap().ip().is_loopback(), "0.0.0.0 is not loopback");
        assert!(matches!(guard_post_bind(&wide, None), Err(ServerError::BoundWiderThanAsked { .. })));
        assert!(guard_post_bind(&wide, Some("t")).is_ok(), "a token makes it legal after the bind too");
        let tight = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert!(guard_post_bind(&tight, None).is_ok(), "a loopback bind passes with no token");
    }
}
