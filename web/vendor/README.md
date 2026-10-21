# Vendored browser assets

This file is the record of what the build downloads, from where, and under what licence (spec §2 W5).
It is **the one file in this directory that is committed**: `.gitignore` carries `web/vendor/*` and
`!web/vendor/README.md`, so the fetched files stay out of the repository and this record stays in it.

The files are fetched by `cargo run -p xtask -- dist` and embedded into the binary by `rust-embed`.
(Every document in this phase, `server::assets`'s `DIST_COMMAND` and the page it renders spell that
`cargo xtask dist`; cargo resolves `cargo <name>` from `PATH` and this workspace's binary is not on it,
so the short form fails with `no such command`. Use the `cargo run` form.) They are not loaded from a
CDN: `serve` is a LAN tool that may run with no internet access, and a third-party origin is exactly the
case that leaks a `?token=` in a `Referer` under the default policy.

| Package | Version | Fetched from | sha256 | Licence |
|---|---|---|---|---|
| `@xterm/xterm` | 6.0.0 | `https://registry.npmjs.org/@xterm/xterm/-/xterm-6.0.0.tgz` | `908e66e04af6c8dc6b00dd3b54de088e2e81e5ed866284fd6c2fb3c2d1c7a3f6` | MIT (registry metadata) |
| `@xterm/addon-fit` | 0.11.0 | `https://registry.npmjs.org/@xterm/addon-fit/-/addon-fit-0.11.0.tgz` | `26003b4517a132b64e4ff228fd88a5fda3fff5e606c76093f6dcff772e9ecec0` | MIT (registry metadata) |

Files kept, per package:

- `@xterm/xterm` → `xterm.mjs` (`package/lib/xterm.mjs`, the package's `"module"`) and
  `xterm.css` (`package/css/xterm.css`, the package's `"style"` — a separate file in the tarball, and
  the terminal renders with no layout without it).
- `@xterm/addon-fit` → `addon-fit.mjs` (`package/lib/addon-fit.mjs`).

The hashes above are the ones pinned in `xtask/src/assets.rs`. A mismatch is a **build failure**:
`xtask` stops, prints both digests and the licence, and extracts nothing. To re-pin, download the
tarball, run `Get-FileHash -Algorithm SHA256`, and change both the constant and this table in the same
commit — never one without the other.

The two digests were taken on 2026-10-12 with `Get-FileHash -Algorithm SHA256` over the tarballs those
exact URLs served, and cross-checked against the registry's own published digests for those versions
(`dist.shasum`, sha1, and `dist.integrity`, sha512) — both agree with the bytes on this machine.

**The three digests below are pinned in the source as well, and checked on every run.** They are the
*extracted* files' hashes, not the tarballs': `b336ec65a086c056d4804b3d4c2347da5663d3f23c3f25be866467bd8857ad59`
(`xterm.mjs`), `854a7c0fb70e8b1a083c16797ab827299fb18744f5ad34f227b48337e33293c6` (`xterm.css`) and
`2d87e1bddc73be9111de8beee5370c3bb7aac9c94e18e6f245f02ca741ef1769` (`addon-fit.mjs`).
A tarball digest can only prove the archive a download returned, once; it says nothing about the file
sitting in `web/vendor/` afterwards, so `is_file()` alone accepted a truncated or hand-edited asset and
embedded it on every later build while the fetch printed "already fetched and verified". `xtask` now
runs `verify_extracted` on every `dist` — the first thing a second run does and the last step of a
real fetch — and a mismatch is a **refusal**, not a re-download: it names the file, the digest on disk
and the pinned one, and extracts nothing. A silent re-fetch would erase the only evidence that the
embedded file was ever wrong, and the network is not something `dist` may require twice. Delete the
named file and run `cargo run -p xtask -- dist` again.

To check this table against the tree without touching the network:

```powershell
Get-FileHash web/vendor/xterm.mjs, web/vendor/xterm.css, web/vendor/addon-fit.mjs -Algorithm SHA256
```

