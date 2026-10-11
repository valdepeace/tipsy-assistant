//! What the two page routes answer with.
//!
//! **This file is a placeholder whose subject belongs to Task 1.5**, which owns the embedded bundle
//! and the two explanation pages. Task 1.4 registers the routes `/` and `/terminal`, and its tests
//! assert that each answers `200` and carries the two headers of spec §5.3 and §9.2 — so the routes
//! cannot be tested without *something* to answer with, and no bundle exists yet.
//!
//! What is here is deliberately the smallest thing that tells the truth: a page that says the build
//! carries no bundle. It sets no cache and no referrer policy of its own — `server::page_response` in
//! `server/mod.rs` is the one place those two headers are set, so the placeholder and the real pages
//! cannot drift.
//!
//! Task 1.5 replaces both bodies with `rust-embed` lookups over `web/dist` and adds the asset
//! route's resolver; nothing else in `server/mod.rs` has to change for that.

/// The page a route answers with. `None` means this build carries no page for that path.
///
/// The signature is the one Task 1.5 needs — bytes plus the content type they are served as — so
/// that task changes this function's body rather than the router that calls it.
pub(crate) fn resolve(path: &str) -> Option<(&'static str, Vec<u8>)> {
    let _ = path;
    None
}

/// The page served when `resolve` has nothing: a real `200` that says so, rather than a blank body
/// or a status the user cannot act on.
pub(crate) fn fallback() -> (&'static str, Vec<u8>) {
    ("text/html; charset=utf-8", MISSING.as_bytes().to_vec())
}

/// Deliberately minimal and inline: it is not the interface, it is what the interface is tested
/// against until the bundle lands.
const MISSING: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>tipsy-compadre</title></head><body><h1>No web interface in this build</h1>\
<p>This binary carries no embedded page for that route.</p></body></html>";
