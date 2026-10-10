# tipsy-compadre — instructions for Claude

Rust client-server voice assistant for developers with affectionate regional personas.
Read `docs/architecture.md` and the spec `docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md`.

## Current work

Executing **Plan 1** (`docs/superpowers/plans/2026-10-10-plan-1-terminal-core.md`) on branch
`plan-1-terminal-core` with `superpowers:subagent-driven-development`. The SDD workspace, task
briefs, global constraints and the progress ledger (with every ruling taken so far) live in
`.superpowers/sdd/2026-10-10-plan-1-terminal-core/`. **Resume from the ledger**: tasks with a
`Task <N>: complete` line are done; start at the first one without it.

Pending in parallel with Plan 1: two throwaway spikes on Haiku, in a scratch directory outside the
repo — Ratzilla (ratatui `Canvas` with Braille + mouse on wasm32) and Rustpotter (custom Spanish
wake word "oye compae"). Their answers feed Plans 3 and 4. Plans 2–4 are written only after Plan 1
lands, against the real code.

## Budget rules (user is on a tight budget)

- Each task is executed by a subagent on the model tagged in the plan: `[haiku]` by default,
  `[sonnet]` only where tagged. Tests are always written by Haiku.
- **No per-task reviews.** One whole-branch review on Sonnet at the end (Task 18 Step 2).
- Always pass `model` explicitly when dispatching subagents.

## Known environment issues

- Haiku fails with `API Error: 400 system.4.type: Input should be 'text'` when the claude.ai MCP
  connectors are loaded. Start Claude Code with `ENABLE_CLAUDEAI_MCP_SERVERS=false`
  (PowerShell: `$env:ENABLE_CLAUDEAI_MCP_SERVERS="false"; claude`) so Haiku subagents work.
- Building on Windows from Git Bash: remove `/usr/bin` from `PATH` before `cargo` (Git's
  `link.exe` shadows MSVC's): `export PATH="$(echo "$PATH" | tr ':' '\n' | grep -v '^/usr/bin$' | paste -sd:)"`.
- Disk `E:` is nearly full; avoid duplicate `target/` directories.

## Conventions

- Code, comments, docs and commit messages in English; personas in their own language.
- Terminal first: every feature must work in the native terminal before it is done.
- Never clone or imitate the voice of a real person; never use characters or quotes from films or
  other media. Personas are original, affectionate caricatures.
- API keys never go into files; config stores key names only.
