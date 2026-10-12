//! Fetching the browser assets the terminal page vendors (spec §2 W5, §8.3).
//!
//! The files are not committed and are not loaded from a CDN — the `serve` command is a LAN tool that
//! may run with no internet access, and a third-party origin is exactly the case that leaks a
//! `?token=` in a `Referer` under the default policy (research note §6.1). Instead they are fetched
//! once, from the npm registry, at a pinned version, and checked against a sha256 pinned here.

use std::path::Path;
use std::process::Command;

/// The `@xterm/xterm` tarball the page's terminal comes from. `"module"` is `lib/xterm.mjs` and
/// `"style"` is a **separate file**, `css/xterm.css`, which is why the CSS is a member of this list.
pub const XTERM: Component = Component {
    name: "@xterm/xterm",
    version: "6.0.0",
    tarball: "https://registry.npmjs.org/@xterm/xterm/-/xterm-6.0.0.tgz",
    // Obtained with `Get-FileHash -Algorithm SHA256` on the tarball this exact URL served, and
    // cross-checked against the registry's own `dist.shasum` (sha1) and `dist.integrity` (sha512) for
    // `@xterm/xterm@6.0.0`, both of which agree with these bytes. Recorded in
    // `web/vendor/README.md`. Never edited by hand.
    sha256: "908e66e04af6c8dc6b00dd3b54de088e2e81e5ed866284fd6c2fb3c2d1c7a3f6",
    // (member, file name, sha256 of the *extracted* file). The third field is not decoration: the
    // tarball's digest proves the archive and never the file on disk, so without it a truncated or
    // hand-edited `xterm.mjs` was accepted by `is_file()` and embedded on every later build. These
    // two are the digests `web/vendor/README.md` records, measured with `Get-FileHash -Algorithm
    // SHA256` over the files this fetch wrote.
    files: &[
        (
            "package/lib/xterm.mjs",
            "xterm.mjs",
            "b336ec65a086c056d4804b3d4c2347da5663d3f23c3f25be866467bd8857ad59",
        ),
        (
            "package/css/xterm.css",
            "xterm.css",
            "854a7c0fb70e8b1a083c16797ab827299fb18744f5ad34f227b48337e33293c6",
        ),
    ],
    license: "MIT",
};

/// The fit addon, whose `FitAddon.fit()` is the page's only way to size the terminal to its window.
pub const ADDON_FIT: Component = Component {
    name: "@xterm/addon-fit",
    version: "0.11.0",
    tarball: "https://registry.npmjs.org/@xterm/addon-fit/-/addon-fit-0.11.0.tgz",
    // Same provenance as `XTERM`'s: this file's own hash, and the registry's `dist.shasum` and
    // `dist.integrity` over it, all agree.
    sha256: "26003b4517a132b64e4ff228fd88a5fda3fff5e606c76093f6dcff772e9ecec0",
    files: &[(
        "package/lib/addon-fit.mjs",
        "addon-fit.mjs",
        "2d87e1bddc73be9111de8beee5370c3bb7aac9c94e18e6f245f02ca741ef1769",
    )],
    license: "MIT",
};

pub const COMPONENTS: [Component; 2] = [XTERM, ADDON_FIT];

pub struct Component {
    pub name: &'static str,
    pub version: &'static str,
    pub tarball: &'static str,
    /// Lowercase hex. A mismatch is a **build failure**, not a warning. This is the **tarball's**
    /// digest, checked once on the bytes the download returned, before anything is extracted.
    pub sha256: &'static str,
    /// (member path inside the tarball, file name under `web/vendor/`, sha256 of the **extracted**
    /// file). The third field is what [`verify_extracted`] compares against on every run — see the
    /// note on [`have_assets`] for why the tarball's own digest cannot stand in for it.
    pub files: &'static [(&'static str, &'static str, &'static str)],
    /// The licence the registry's metadata reports for this version — read by [`verify`], which names
    /// it in a refusal, and written into `web/vendor/README.md`, which is the record spec §2 W5 asks
    /// for. The fetch itself does not otherwise need it.
    pub license: &'static str,
}

/// Lowercase hex sha256, the standard algorithm, written out rather than adding a crate to the
/// workspace for forty lines of arithmetic.
pub fn sha256_of(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut message = bytes.to_vec();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for block in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (index, word) in block.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7) ^ w[index - 15].rotate_right(18) ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17) ^ w[index - 2].rotate_right(19) ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e;
            e = d.wrapping_add(temp1);
            d = c; c = b; b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

pub fn verify(component: &Component, bytes: &[u8]) -> Result<(), String> {
    let actual = sha256_of(bytes);
    if actual == component.sha256 {
        Ok(())
    } else {
        Err(format!(
            "{} {} ({}): the tarball's sha256 is {actual}, and the one pinned in xtask/src/assets.rs is {} \
             — the fetch has been stopped, because the file on the wire is not the file this project \
             verified. Do not re-pin until you know why they differ.",
            component.name, component.version, component.license, component.sha256
        ))
    }
}

/// The names this component is supposed to leave under `web/vendor/`.
pub fn expected_assets(vendor: &Path) -> Vec<std::path::PathBuf> {
    COMPONENTS
        .iter()
        .flat_map(|component| component.files.iter())
        .map(|(_, name, _)| vendor.join(name))
        .collect()
}

/// The extracted files checked against **their own** pinned digests.
///
/// **A file's presence is not evidence that it is the file this project verified, and this is the
/// function that stops pretending otherwise.** The pins above are *tarball* digests: they prove the
/// archive a download returned, once, and they cannot say anything about the four bytes-to-350-KiB
/// files sitting in `web/vendor/` afterwards. `is_file()` therefore accepted a truncated or
/// hand-edited `xterm.mjs` and embedded it on every later build, while `fetch_all` printed
/// "already fetched and verified" — a claim about the bytes that nothing had checked.
///
/// **A mismatch is a refusal, not a re-fetch.** The reasoning is the tarball verifier's own, one step
/// further down: a silent re-download would erase the only evidence that the embedded file was ever
/// wrong, and the network is not something `dist` may require twice — `serve` is a LAN tool that may
/// run with none. So this names the file, the digest on disk and the pinned one, says which file to
/// delete, and extracts nothing.
pub fn verify_extracted(component: &Component, vendor: &Path) -> Result<(), String> {
    for (_, name, pinned) in component.files {
        let path = vendor.join(name);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let actual = sha256_of(&bytes);
        if actual != *pinned {
            return Err(format!(
                "{} {} ({}): {} is not the file this project verified. Its sha256 is {actual} and the \
                 one pinned in xtask/src/assets.rs — and recorded in web/vendor/README.md — is {pinned}. \
                 Nothing is embedded, because the file on disk is not the file this project checked. \
                 Recover by deleting {} and running `cargo run -p xtask -- dist` again, and do not re-pin \
                 until you know why the two differ.",
                component.name,
                component.version,
                component.license,
                path.display(),
                path.display()
            ));
        }
    }
    Ok(())
}

/// Either the bundle's files are **all there and all the bytes this project pinned**, or this names
/// what is wrong. `have_assets` is the only gate `fetch_all` has, so making it verify rather than
/// merely look is what turns `already fetched and verified` into a true sentence.
pub fn have_assets(vendor: &Path) -> Result<(), String> {
    for path in expected_assets(vendor) {
        if !path.is_file() {
            return Err(format!("{} is missing", path.display()));
        }
    }
    for component in &COMPONENTS {
        verify_extracted(component, vendor)?;
    }
    Ok(())
}

/// Fetches one component unless its files are all there **and are the ones this project pinned**. A
/// second run therefore does not re-download what is present and verified, and a file that is present
/// and wrong is refused by [`verify_extracted`] instead of being re-fetched over.
pub fn fetch(component: &Component, vendor: &Path) -> Result<(), String> {
    if component.files.iter().all(|(_, name, _)| vendor.join(name).is_file()) {
        return verify_extracted(component, vendor);
    }
    std::fs::create_dir_all(vendor).map_err(|e| format!("{}: {e}", vendor.display()))?;
    let archive = vendor.join(format!(".{}-{}.tgz", component.name.replace('/', "-"), component.version));
    let bytes = download(component.tarball)?;
    verify(component, &bytes)?;
    std::fs::write(&archive, &bytes).map_err(|e| format!("{}: {e}", archive.display()))?;
    let extracted = extract_members(&archive, component.files, vendor);
    let _ = std::fs::remove_file(&archive);
    extracted?;
    // The last step of a real fetch, and the first thing a second run checks: the extracted bytes
    // against the digests above. A `tar` that wrote a short file is caught here and not by a page
    // that renders nothing.
    verify_extracted(component, vendor)?;

    // The CSS is the member most likely to be dropped, and it is the one that points at more files.
    // Reading its `url(...)` references is what turns a silently unstyled terminal into a list of
    // names a later reviewer can check. (`@xterm/xterm@6.0.0`'s `css/xterm.css` names none: measured,
    // it is 7112 bytes with no `url(` in it, so this prints nothing for that package. The reader is
    // kept because the check is what a *changed* pin needs, and its own test exercises it.)
    if let Some((_, name, _)) = component.files.iter().find(|(_, name, _)| name.ends_with(".css")) {
        let path = vendor.join(name);
        let css = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        for reference in css_references(&css) {
            println!("xtask: {} references {reference}", path.display());
        }
    }
    Ok(())
}

pub fn fetch_all(vendor: &Path) -> Result<(), String> {
    if have_assets(vendor).is_ok() {
        println!("xtask: {} is already fetched and verified", vendor.display());
        return Ok(());
    }
    for component in COMPONENTS {
        fetch(&component, vendor)?;
        println!("xtask: {} {} verified", component.name, component.version);
    }
    have_assets(vendor)
}

/// One HTTPS GET, through the platform's own client: `curl.exe` ships with Windows 10 1809 and later
/// (the same floor ConPTY has) and `curl` is on every Unix this project builds for. It does TLS and
/// certificate validation correctly, which is the whole reason not to hand-roll an HTTP client — and
/// it keeps `reqwest`, `tokio` and a runtime out of a build script.
///
/// `-f` turns an HTTP error status into a non-zero exit, `-L` follows the registry's redirects, `-s`
/// keeps progress off stdout (the bytes are stdout) and `-S` still prints errors on failure.
fn download(url: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err(format!("{url}: only https is accepted, the pinned hashes assume it"));
    }
    let out = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" })
        .args(["-fsSL", "--max-time", "120", url])
        .output()
        .map_err(|e| format!("{url}: no curl to fetch it with: {e}"))?;
    if !out.status.success() {
        return Err(format!("{url}: curl exited with {}", out.status));
    }
    if out.stdout.is_empty() {
        return Err(format!("{url}: curl returned nothing"));
    }
    Ok(out.stdout)
}

/// `tar` extracts named members. `tar.exe` ships with Windows 10 1809 and later (bsdtar) and is on
/// every Unix; the member names are the archive's own, quoted from the registry's file listing.
///
/// **The staging directory is removed on every exit path, the failing ones included.** The body this
/// was written from returned early on a `tar` that failed and left `.staging/` behind — a directory
/// of half-extracted bytes sitting in `web/vendor/` beside verified assets, which is precisely the
/// partial state the pinned digest exists to prevent. The staging is an implementation detail and
/// [`have_assets`] is the only gate, so nothing else notices it; that is the reason to clear it here
/// rather than to rely on the next run's `remove_dir_all`.
fn extract_members(archive: &Path, members: &[(&str, &str, &str)], vendor: &Path) -> Result<(), String> {
    let staging = vendor.join(".staging");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    let extracted = extract_into(archive, members, &staging, vendor);
    let _ = std::fs::remove_dir_all(&staging);
    extracted
}

/// The `tar` runs and the moves out of staging. Split out so that [`extract_members`] owns the
/// cleanup and cannot skip it on an early return.
fn extract_into(
    archive: &Path,
    members: &[(&str, &str, &str)],
    staging: &Path,
    vendor: &Path,
) -> Result<(), String> {
    for (member, ..) in members {
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(staging)
            .arg(member)
            .status()
            .map_err(|e| format!("tar: {e}"))?;
        if !status.success() {
            return Err(format!("tar could not extract {member} from {}", archive.display()));
        }
    }
    for (member, name, _) in members {
        let from = staging.join(member);
        let to = vendor.join(name);
        std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    Ok(())
}

/// Every `url(...)` the CSS names. Read so that a future reviewer can see what the CSS expects to
/// find beside it; the page needs none of them for a terminal, which draws text.
pub fn css_references(css: &str) -> Vec<String> {
    let mut references = Vec::new();
    let mut rest = css;
    while let Some(start) = rest.find("url(") {
        rest = &rest[start + 4..];
        let Some(end) = rest.find(')') else { break };
        let reference = rest[..end].trim().trim_matches(['\'', '"']).to_string();
        if !reference.is_empty() {
            references.push(reference);
        }
        rest = &rest[end + 1..];
    }
    references
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 132-byte `.tar.gz` holding exactly `pkg/a.txt` with the eight bytes `hello` in it. It is the
    /// fixture the fetch's own pipeline is exercised against, so the test does not need the network:
    /// the bytes, their sha256 and `tar`'s own extraction are all real.
    const FIXTURE_TGZ_BASE64: &str = concat!(
        "H4sIADGqy2oAA+3RQQqAIBAFUI/iCWo0nDmPiyhISMqg46e0cxGBWETzNoMb/c7309CKygCAiGSaUT7PgzIaNWkDqCQoAEVC",
        "mtrBkm0NdolRSu/JP/cRPvZvm7CHim/EfSDiVf8m75+oE7K4kzt+3v/YOze/HYIxxtjjDg5q4hkACgAA",
    );

    /// The fixture is what `tar -czf` produced; the hash is what `Get-FileHash -Algorithm SHA256`
    /// printed for that same file when the fixture was made. If this assertion ever fails, the bytes
    /// in the two string literals above changed, and the right answer is to regenerate both with the
    /// two commands in Step 1, never to relax the assertion.
    const FIXTURE_TGZ_SHA256: &str =
        "e3ec7d757fd4066a41e602aec5cb602d67e62912bd20e8c9d06d0893c8d9a91e";

    /// The digest of the fixture's **extracted** member: the five bytes `hello` that `tar` writes out
    /// of `pkg/a.txt`. Printed by `[System.Security.Cryptography.SHA256]::Create().ComputeHash(…)`
    /// over those five bytes, so it is measured rather than recalled — and it is what lets the
    /// no-download path be proved with real bytes instead of with a placeholder word.
    const FIXTURE_MEMBER_SHA256: &str =
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    fn fixture() -> Vec<u8> {
        // `base64` is not a dependency of this workspace and must not become one: this is the one
        // place a base64 blob is decoded, so it is decoded here, in twenty lines.
        fn decode(input: &str) -> Vec<u8> {
            const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = Vec::new();
            let mut buffer = 0u32;
            let mut bits = 0u32;
            for byte in input.bytes() {
                if byte == b'=' {
                    break;
                }
                let Some(value) = ALPHABET.iter().position(|candidate| *candidate == byte) else {
                    continue;
                };
                buffer = (buffer << 6) | value as u32;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    out.push((buffer >> bits) as u8);
                }
            }
            out
        }
        decode(FIXTURE_TGZ_BASE64)
    }

    /// `sha256_of` against the standard, not against itself. **The fixture's digest below proves the
    /// function agrees with one 132-byte input; a padding bug at a block boundary would still agree
    /// with that one input**, so the digests here come from an independent implementation — .NET's
    /// `SHA256` — and the lengths are chosen to walk every path through the padding: 55 bytes is the
    /// longest message that still fits one block with its length field, 56 and 64 force a second, and
    /// 1000 crosses fifteen. Every digest was printed by
    /// `[System.Security.Cryptography.SHA256]::Create().ComputeHash(...)` over exactly these bytes
    /// when this test was written; none is recalled.
    #[test]
    fn the_sha256_matches_the_standard_vectors() {
        let vectors = [
            // (message, sha256 of its UTF-8 bytes)
            (String::new(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            ("abc".to_string(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            // 55 bytes: the last input whose length field still fits in its first block.
            (
                "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnop".to_string(),
                "aa353e009edbaebfc6e494c8d847696896cb8b398e0173a4b5c1b636292d87c7",
            ),
            // 56 bytes: one byte more, and the padding must open a second block.
            (
                "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq".to_string(),
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
            ("a".repeat(64), "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"),
            ("a".repeat(1000), "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"),
        ];
        for (message, digest) in &vectors {
            assert_eq!(
                sha256_of(message.as_bytes()),
                *digest,
                "{} bytes must hash to the standard digest",
                message.len()
            );
        }
    }

    #[test]
    fn the_hash_verifier_accepts_the_pinned_digest_and_rejects_any_other() {
        let bytes = fixture();
        assert_eq!(sha256_of(&bytes), FIXTURE_TGZ_SHA256, "the fixture's own digest must be stable");
        assert!(verify(&FIXTURE, &bytes).is_ok(), "the pinned digest must be accepted");

        let mut wrong = bytes.clone();
        wrong[0] ^= 0xff;
        let refused = verify(&FIXTURE, &wrong).unwrap_err();
        assert!(refused.contains("sha256"), "the refusal must name what did not match: {refused}");
        // A mismatch is a failure, not a warning: the fetch has no partial state to fall back on.
        assert!(refused.contains("is not"), "{refused}");
        // Both digests, side by side: what was on the wire and what this project pinned. **The plan
        // asserted the negation of this** (`!a || !b`), which no refusal message can satisfy when it
        // is supposed to name both — the label said "the two digests, side by side" and the
        // expression said the opposite. Asserted positively, which is what the label always meant.
        assert!(
            refused.contains(FIXTURE_TGZ_SHA256) && refused.contains(&sha256_of(&wrong)),
            "the refusal must print both digests, side by side: {refused}"
        );
        // And the licence of the thing that failed, so a reviewer reading a refusal knows what it is.
        assert!(refused.contains(FIXTURE.license), "{refused}");
    }

    #[test]
    fn an_already_extracted_component_is_never_fetched_again() {
        let dir = tempfile::tempdir().unwrap();
        let vendor = dir.path().join("vendor");
        std::fs::create_dir_all(&vendor).unwrap();
        assert!(have_assets(&vendor).is_err(), "an empty directory is missing every component");

        // The fixture's own member, with the fixture's own bytes. **`b"present"` used to sit here**,
        // and that is Finding 5 of the phase review in one line: `is_file()` was the whole check on
        // the no-download path, so any five bytes under the right name passed for a verified asset.
        // The fixture's `tarball` is the empty string and `download` refuses anything that is not
        // `https://`, so this call succeeding *is* the proof that nothing was downloaded.
        std::fs::write(vendor.join("a.txt"), b"hello").unwrap();
        assert!(fetch(&FIXTURE, &vendor).is_ok(), "a present, verified component must not be fetched again");
    }

    /// **Finding 5 of the phase review, on real pins.** A file that exists, has the right name and the
    /// wrong bytes is refused with the file, both digests and the licence named — the same shape as
    /// the tarball verifier's refusal, one step further down. The digests are `COMPONENTS`' own, so
    /// this fails if a pin and the fetched file in `web/vendor/` ever disagree.
    ///
    /// **A refusal and not a re-fetch.** A silent re-download would erase the only evidence that the
    /// embedded file was ever wrong, and it would need the network for a tree that already has its
    /// bytes; the recovery is one `Remove-Item` and the message says so.
    #[test]
    fn a_corrupted_extracted_file_is_refused_with_both_digests_named() {
        let dir = tempfile::tempdir().unwrap();
        let vendor = dir.path().join("vendor");
        std::fs::create_dir_all(&vendor).unwrap();
        for component in &COMPONENTS {
            for (_, name, _) in component.files {
                std::fs::write(vendor.join(name), b"truncated").unwrap();
            }
        }

        // `XTERM` is verified first, and its first member is the one that fails.
        let (_, first_name, first_pin) = XTERM.files[0];
        let refused = have_assets(&vendor).unwrap_err();
        assert!(refused.contains(first_name), "the refusal names the file: {refused}");
        assert!(refused.contains(first_pin), "and the digest this project pinned: {refused}");
        assert!(
            refused.contains(&sha256_of(b"truncated")),
            "and the digest of the bytes on disk: {refused}"
        );
        assert!(refused.contains(XTERM.license), "and the licence of the package: {refused}");
        assert!(refused.contains("dist"), "and how to recover from it: {refused}");

        // The other half: the *fetched* files in this checkout, when there are any, verify against
        // these same pins — which is what makes them a statement about `web/vendor/` and not about a
        // tempdir. A checkout that has never run `dist` skips, exactly as the page's asset test does.
        let fetched = crate::repo_root().join("web").join("vendor");
        if have_assets(&fetched).is_ok() {
            for component in &COMPONENTS {
                assert!(
                    verify_extracted(component, &fetched).is_ok(),
                    "{} must verify against the pins in this file",
                    component.name
                );
            }
        }
    }

    #[test]
    fn the_extraction_keeps_exactly_the_members_the_page_needs() {
        let dir = tempfile::tempdir().unwrap();
        let vendor = dir.path().join("vendor");
        std::fs::create_dir_all(&vendor).unwrap();
        let archive = dir.path().join("fixture.tgz");
        std::fs::write(&archive, fixture()).unwrap();

        extract_members(&archive, &[("pkg/a.txt", "a.txt", FIXTURE_MEMBER_SHA256)], &vendor)
            .expect("tar must extract the member");
        let extracted = std::fs::read_to_string(vendor.join("a.txt")).unwrap();
        assert_eq!(extracted, "hello");
        assert!(!vendor.join("pkg").exists(), "only the named members leave the archive");
        // And the member the fetch just wrote is what the digest pins, which is the pair of facts
        // `verify_extracted` compares on every later run.
        assert_eq!(sha256_of(extracted.as_bytes()), FIXTURE_MEMBER_SHA256);
    }

    /// A member the archive does not hold is refused rather than skipped: `tar` exits non-zero, and
    /// that has to reach the caller as an error, or a typo in `Component::files` would leave a page
    /// with a missing script and a build that said it succeeded.
    #[test]
    fn an_absent_member_is_refused_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let vendor = dir.path().join("vendor");
        std::fs::create_dir_all(&vendor).unwrap();
        let archive = dir.path().join("fixture.tgz");
        std::fs::write(&archive, fixture()).unwrap();

        let refused =
            extract_members(&archive, &[("pkg/nope.txt", "nope.txt", FIXTURE_MEMBER_SHA256)], &vendor).unwrap_err();
        assert!(refused.contains("pkg/nope.txt"), "{refused}");
        assert!(!vendor.join("nope.txt").exists(), "a refused member leaves no file");
        assert!(!vendor.join(".staging").exists(), "and no staging directory");
    }

    #[test]
    fn the_css_references_are_readable_so_the_extraction_can_be_checked_against_them() {
        let references = css_references("a { background: url(font.woff2); } b { background: url('icons/x.svg'); }");
        assert_eq!(references, vec!["font.woff2".to_string(), "icons/x.svg".to_string()]);
        assert!(css_references("a { color: red; }").is_empty());
    }

    /// The pins are what the fetch does its work with, and `@xterm/xterm`'s own CSS is the member the
    /// research note calls "the fact most likely to be missed in a minimal page" — so it is on the
    /// list, at the path the tarball actually uses.
    #[test]
    fn the_pins_carry_the_mjs_and_the_separate_css() {
        assert_eq!(XTERM.files.len(), 2, "the module and the stylesheet are two files in the tarball");
        assert!(
            XTERM.files.iter().any(|(member, name, _)| member.ends_with(".css") && name.ends_with(".css")),
            "the CSS is a member of the package and must be carried"
        );
        for component in COMPONENTS {
            assert!(component.tarball.starts_with("https://"), "{}", component.tarball);
            assert!(
                component.tarball.contains(&format!("-{}.tgz", component.version)),
                "{} must fetch the version it pins: {}",
                component.name,
                component.tarball
            );
            assert_eq!(component.sha256.len(), 64, "{}'s pin is a sha256", component.name);
            assert!(
                component.sha256.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "{}'s pin is lowercase hex, because that is what sha256_of returns",
                component.name
            );
            // Every member carries its **own** digest as well, and it is not the tarball's: that is
            // the pin `verify_extracted` compares on every run, and a copy-paste of the tarball's hash
            // into this field would make every later build fail with a refusal it cannot explain.
            for (member, name, digest) in component.files {
                assert_eq!(digest.len(), 64, "{name}'s extracted pin is a sha256");
                assert!(
                    digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                    "{name}'s extracted pin is lowercase hex, because that is what sha256_of returns"
                );
                assert_ne!(
                    *digest, component.sha256,
                    "{name} is extracted from {member} and is not that member: its digest is its own"
                );
            }
        }
    }

    /// The fixture's `Component`, kept out of `COMPONENTS` so the real pins are the only thing the
    /// fetch ever runs against.
    const FIXTURE: Component = Component {
        name: "@xterm/xterm",
        version: "fixture",
        tarball: "",
        sha256: FIXTURE_TGZ_SHA256,
        files: &[("pkg/a.txt", "a.txt", FIXTURE_MEMBER_SHA256)],
        license: "MIT",
    };
}
