# Tipsy floating window spike

Standalone Tauri/WebView2 proof of concept. It does not join the workspace or change the production CLI.

Build the web client first from the repository root with `cargo run -p xtask -- dist`, then start a local Tipsy server that serves the generated bundle. In another PowerShell window:

```powershell
cargo run -p tipsy-assistant -- serve --bind 127.0.0.1:7070
```

In a second PowerShell window:

```powershell
cargo run --manifest-path poc/floating-window/Cargo.toml -- http://127.0.0.1:7070/
```

The spike opens the supplied loopback HTTP page in a topmost window and adds `floating=1` to its URL. It starts at 1120×760, shrinks to 220×165 and moves to the bottom-right when it loses focus, and expands again when focused. In floating mode, the WebGL canvas follows the WebView viewport and Ratzilla updates its terminal grid after each resize; the compact view draws the animated core with the existing voice oscilloscope below it. Clicking the compact view uses the existing F3 microphone toggle. The normal browser URL and the TUI keep their existing layouts. Use only a loopback URL; the spike rejects non-local hosts and HTTPS is intentionally outside this check.

The small HTML page and icon are only Tauri's required fallback/build assets. The intended visual/audio check is the real Tipsy page served from the local `tipsy-assistant serve` process. Mic/audio still need to be inspected in the running WebView2 window.
