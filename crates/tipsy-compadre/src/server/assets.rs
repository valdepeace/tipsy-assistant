//! The bundle the binary carries, and the page that says so when it carries none.
//!
//! `#[allow_missing = true]` is the whole mechanism: with `web/` absent the embed is empty, every
//! lookup is `None`, and the two page routes fall back to [`fallback`]. No `build.rs` gate is
//! needed (spec §5.4; research `web-transport-and-terminal.md:365-386`).
//!
//! **Task 1.4 created this file as a placeholder and pinned two names in it**: its router and four of
//! its six tests call [`resolve`] and [`fallback`], so those keep their names and their shapes and
//! this task fills the bodies rather than renaming them. [`asset`], [`web_bundle`], [`DIST_COMMAND`],
//! [`PAGE_CLIENT`] and [`PAGE_TERMINAL`] are this task's, along with the `/assets/{*path}` route.

use rust_embed::RustEmbed;

/// What `cargo xtask dist` produces. Hashed filenames may be cached; the two pages may not.
///
/// The folder is relative to `crates/tipsy-compadre/Cargo.toml` — `rust-embed` bases a relative
/// `folder` on `CARGO_MANIFEST_DIR` in both modes — and it does not exist until Phase 2 and Phase 3
/// write `web/`, which is what `allow_missing` is for.
#[derive(RustEmbed)]
#[folder = "web/dist/"]
#[allow_missing = true]
struct Bundle;

/// What a user runs to build the bundle this binary does not carry.
pub const DIST_COMMAND: &str = "cargo xtask dist";

/// The web client's host page. Phase 3's Task 3.6 fills it.
pub const PAGE_CLIENT: &str = "client/index.html";
/// The xterm.js host page. Phase 2's Task 2.4 fills it.
pub const PAGE_TERMINAL: &str = "terminal/index.html";

const HTML: &str = "text/html; charset=utf-8";
const OCTET: &str = "application/octet-stream";

/// Whether this binary carries a bundle at all — the truth `Welcome.web_bundle` reports
/// (spec §5.4), so a plain `cargo build` and a `cargo xtask dist` differ.
pub fn web_bundle() -> bool {
    embedding(PAGE_CLIENT).is_some_and(|bytes| !bytes.is_empty())
}

/// The page for a route: the embedded file, or the explanation page when this binary carries none.
/// **Always `Some` for [`PAGE_CLIENT`]'s and [`PAGE_TERMINAL`]'s routes and always `None` for
/// anything else** — which is what makes spec §10's "The bundle is not embedded" row true of this
/// function and not merely of the router that calls it.
///
/// **Keep the name and the signature**: Task 1.4's `server/mod.rs` calls both, and this file was
/// created by that task as a placeholder with exactly this shape.
///
/// The placeholder returned `None` here and let `server::page_of` answer the fallback, which is what
/// this task's own Step 1 test disproves: it asserts that `/` and `/terminal` always answer
/// *something*, and a `None` there makes `page_of`'s second arm the only page a plain `cargo build`
/// ever serves. Delegating to [`fallback`] here makes the promise hold for either caller and costs
/// one call; `page_of`'s `None` arm stays as the router's own belt-and-braces for a path that is not
/// one of the two pages.
pub fn resolve(path: &str) -> Option<(&'static str, Vec<u8>)> {
    match embedding(path) {
        Some(bytes) => Some((HTML, bytes)),
        None if is_page_route(path) => Some(fallback()),
        None => None,
    }
}

/// The two paths a page route may ask about, in either spelling: the route the router registered and
/// the file the embed holds.
fn is_page_route(path: &str) -> bool {
    matches!(path, "/" | "/client" | PAGE_CLIENT | "/terminal" | PAGE_TERMINAL)
}

/// The honest page: which command builds the bundle this binary does not have. Hand-written HTML —
/// there is no markdown renderer in the tree and none is added for one page. **Returns `Vec<u8>`, not
/// the `String` an earlier draft of this task specified**, because Task 1.4 pinned the signature when
/// it made this file a placeholder: the `format!` below therefore ends in `.into_bytes()`.
pub fn fallback() -> (&'static str, Vec<u8>) {
    (
        HTML,
        format!(
            "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
             <title>tipsy-compadre — no web UI in this build</title></head>\n<body>\n\
             <h1>The web UI is not available in this build</h1>\n\
             <p>This binary was built without the web bundle. Build it with:</p>\n\
             <pre>{DIST_COMMAND}</pre>\n\
             <p>then run <code>tipsy-compadre serve</code> again.</p>\n\
             </body></html>\n"
        )
        .into_bytes(),
    )
}

/// A file from the embed, with the content type a browser needs for it.
pub fn asset(path: &str) -> Option<(&'static str, Vec<u8>)> {
    // `EmbeddedFile.data` is a `Cow<'static, [u8]>` (research `:336-346`), owned here so the
    // response body is a `Vec<u8>` and nothing borrows the embed.
    let bytes = Bundle::get(path)?.data.into_owned();
    if bytes.is_empty() {
        return None;
    }
    Some((content_type(path), bytes))
}

/// The bytes of a page in the embed, when it holds one. `client/index.html` and
/// `terminal/index.html` are the only two paths a page route may answer with; a path with `..` or
/// no extension is never one.
fn embedding(path: &str) -> Option<Vec<u8>> {
    let name = match path {
        "/" | "/client" | PAGE_CLIENT => PAGE_CLIENT,
        "/terminal" | PAGE_TERMINAL => PAGE_TERMINAL,
        _ => return None,
    };
    let file = Bundle::get(name)?;
    (!file.data.is_empty()).then(|| file.data.into_owned())
}

/// The content type by extension. `mime_guess::from_path(path).first()` is the verified call
/// (`web-transport-and-terminal.md:468-473`); `first_or_octet_stream()` is marked INFERRED on the
/// same page, so the fallback is written out here rather than taken from the crate. The bundle's own
/// extensions are matched first, which is also what the note recommends (`:476-479`).
fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => HTML,
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        _ => mime_guess::from_path(path).first().map_or(OCTET, |mime| match mime.as_ref() {
            "text/javascript" => "text/javascript",
            "text/css" => "text/css",
            "application/wasm" => "application/wasm",
            "application/json" => "application/json",
            "image/png" => "image/png",
            "image/svg+xml" => "image/svg+xml",
            _ => OCTET,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec §10's "The bundle is not embedded" row, for both routes. What is asserted is the
    /// *property* — an embedded page, or the page naming the command — so the test keeps passing
    /// when Phase 2 and Phase 3 land the real pages, and it fails today only if a route answers
    /// neither.
    ///
    /// **`resolve` here is the name the plan's first draft called `page`**, and the rename is the
    /// plan's own correction rather than a deviation taken here: Task 1.4's router calls
    /// `assets::resolve(path)` and `assets::fallback()`, and the task body's prose says so in as
    /// many words ("keep the name and the signature"). Filling the bodies under other names would
    /// have broken four of Task 1.4's six tests.
    #[test]
    fn page_routes_answer_a_page_or_the_explanation_page() {
        for path in ["/", "/terminal"] {
            let (content_type, body) = resolve(path).expect("a page route always answers something");
            assert_eq!(content_type, "text/html; charset=utf-8", "{path}");
            assert!(!body.is_empty(), "{path} answered an empty page");
            let html = String::from_utf8(body).expect("a page is HTML text");
            if embedding(path).is_none() {
                assert!(html.contains(DIST_COMMAND), "{path} must name the command that builds it");
            } else {
                // Phase 2 or Phase 3 has landed the real page: it is served, and the explanation
                // page is not.
                assert!(!html.contains("not available in this build"), "{path} served the fallback");
            }
        }
        assert!(resolve("/nope").is_none(), "a route that is not a page has no page");
    }

    /// With the folder absent, `web_bundle()` is `false`, which is what `Welcome.web_bundle`
    /// reports and what the HUD's message is for (spec §5.4). The assertion is the property, so it
    /// holds in a `cargo xtask dist` tree too.
    #[test]
    fn the_bundle_flag_agrees_with_the_embed() {
        assert_eq!(web_bundle(), embedding(PAGE_CLIENT).is_some_and(|bytes| !bytes.is_empty()));
    }

    /// `asset` is `None` for anything the bundle does not hold, and an unknown path is not a licence
    /// to walk out of the folder.
    #[test]
    fn an_unknown_asset_is_nothing() {
        // The real embed is empty in this checkout, so every path misses — including the two that
        // would be dangerous if they did not.
        assert_eq!(asset("nope.js"), None);
        assert_eq!(asset("../config.toml"), None);
        assert_eq!(asset(""), None);
    }

    /// The explanation page says which command builds the bundle and what is missing, and says
    /// nothing about the token that reached it.
    #[test]
    fn the_explanation_page_names_the_command() {
        let (content_type, body) = fallback();
        assert_eq!(content_type, "text/html; charset=utf-8");
        let body = String::from_utf8(body).expect("a page is HTML text");
        assert!(body.contains(DIST_COMMAND), "the page must say how to build the bundle");
        assert!(body.contains("web UI"), "and what is not available");
        assert!(!body.to_lowercase().contains("token"), "and must not talk about the token");
    }
}
