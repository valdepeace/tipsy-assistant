# tipsy-compadre

tipsy-compadre is a voice assistant for software developers with a strong, affectionate regional
personality. The default profile, *El Compae*, is a lazy, joking, slightly tipsy Andalusian who
knows Jira, GitHub, GitLab, CI/CD and code review, dodges work with developer excuses and, in the
end, always gives a correct and useful answer. The project is meant to be shared with the
community: a single executable, cloud providers that anyone can use with their own key, and
profiles that anyone can write for their own region.

**Status: early development — terminal client and server.** The workspace, the protocol, the
configuration and the key handling, the chat, speech and transcription providers (the
OpenAI-compatible connector plus the native Gemini and ElevenLabs ones), the brain, the native
terminal client and its configuration screen all work, and nine profiles ship with it. Two of the
three listening modes work: `push`, where you open and close the microphone by hand, and `auto`,
where a voice detector decides what is an utterance. The server and the remote client work too:
`tipsy-compadre serve` listens, and `tipsy-compadre --connect <ws-url>` attaches a terminal to it.
The **web terminal** works as well — `/terminal` runs that same TUI in a browser tab over a socket of
raw PTY bytes (Phase 2 of the web tier) — and so does the **web client**: `cargo run -p xtask -- dist`
builds the browser HUD into `web/dist/`, the binary embeds it, and `/` renders that HUD from
WebAssembly with the browser's own microphone and speakers (Phase 3). Still to come: the login door's
browser wiring, `hands_free` (it waits for the wake word), plus CI and releases.

## Quick start

You need Rust (the toolchain is pinned to 1.96.1 by `rust-toolchain.toml`), a microphone and
speakers, and either an [OpenRouter](https://openrouter.ai/) API key or a local model server such
as [Ollama](https://ollama.com/) — see [Configuration](#configuration).

```sh
# 1. Tell it where your key is. It asks you to paste it and stores it in the OS credential
#    store; the key itself never touches the repository, the config file or the logs.
cargo run -p tipsy-compadre -- key set openrouter

# 2. Check it, without printing it:
cargo run -p tipsy-compadre -- key status openrouter

# 3. Run it. On the first run it writes a default config.toml and greets you out loud.
cargo run -p tipsy-compadre
```

`F9` — or a click on the gear in the header — opens the configuration screen. It has four tabs
(Providers, Listening, Profiles and Server) whose rows are edited in place, `Ctrl+S` writes
`config.toml` and applies the change, and `F9` or `Esc` closes it again. Nothing reaches the file
before you save, a save that cannot build the connectors it names writes nothing at all, and the
screen's title says `unsaved` while there is something to save. The [Keys](#keys) section lists the
screen's own keys, and [Configuration](#configuration) says which changes take effect at once and
which wait for the next launch.

If you would rather keep the key in the environment, skip step 1 and export it under the name the
providers expect: `TIPSY_KEY_OPENROUTER`. The environment wins over the credential store, so an
exported variable takes precedence over a stored key. In PowerShell:

```powershell
$env:TIPSY_KEY_OPENROUTER = "your-key-here"
cargo run -p tipsy-compadre
```

## Keys

| Key | What it does |
|---|---|
| `Enter` | Ask the typed question: send it to the model and speak the reply |
| `!text` | Do not ask, **recite** `text`: clean it and speak it as-is (type it in the question box, then `Enter`) |
| `Tab` | Open the microphone; press `Tab` again to close it and send what it heard for transcription and reply |
| `F2` | Replay the last spoken reply |
| `F4` | Clear the conversation |
| `F10` | Start a live call, or end the one that is running (`[realtime]`, OpenAI Realtime). `Esc` also ends it, by quitting |
| `←` / `→` | Switch between the `CHAT` and `NETWORK` views |
| `↑` / `↓` | Move the selection through the conversation — `↑` starts at the last line, `↓` at the first — and `F2` replays what is selected |
| `F9` | Open the configuration screen; press it again to close it without saving |
| `Esc` | Quit. While the configuration screen is open, `Esc` closes it instead of quitting |

Some actions also have a mouse equivalent: the `CHAT`/`NETWORK` titles and the **✗ Clear** button are
clickable, and so are the gear and the mode word in the header, which open the configuration screen
the way `F9` does. Keyboard and mouse each need their own binding.

`F10` is the one key with **no mouse equivalent**, and deliberately so: a live call bills for as long
as it is open, so it may only start from an explicit key press, and the indicator in the header is
copy rather than a button. A stray click cannot start one.

While the configuration screen is open it has the keyboard, and the screen is driven from there for
now: `Tab`, `←`/`→`, `Enter`, `Esc` and `F9` change meaning, and the HUD's other keys (`F2`, `F4`)
do nothing in there. These are the keys the client binds in the screen:

| Key | What it does |
|---|---|
| `Tab` / `→` | Move to the next of the four tabs, wrapping at the end |
| `←` | Move to the previous tab |
| `↑` / `↓` | Move the focus between the rows of the current tab |
| `Enter` | Edit the focused row when it holds text, or actuate it when it does not: a toggle, the Test row, or the Server tab's Save row. While a row is being edited, `Enter` applies what was typed |
| `Backspace` | Delete the last character of the text being edited |
| `Ctrl+S` | Save: write `config.toml` and apply the parts that apply live |
| `Ctrl+R` | Mint a fresh server token for the Server tab. Like every other edit it is written only when you save |
| `F9` / `Esc` | Close the screen. Nothing that was edited is written, and the file is left as it was |

The interface is drawn in Spanish when the active profile's language starts with `es` (`NÚCLEO`,
`PREGUNTA`, `CHAT`/`RED`, "Listo, quillo"), and in English otherwise.

## Configuration

```sh
cargo run -p tipsy-compadre -- config path
```

prints the path of `config.toml`; it is `%APPDATA%\tipsy-compadre\config.toml` on Windows,
`~/.config/tipsy-compadre/config.toml` on Linux and
`~/Library/Application Support/tipsy-compadre/config.toml` on macOS. The file is created with
defaults on the first run, and a `config.toml` that does not parse is renamed to
`config.toml.bad-<unix seconds>` and replaced by defaults rather than overwritten in place.

The characters the assistant can speak as are **profiles**: an `active_profile` and, when you write
your own, a `[[profiles]]` array. An older `config.toml` that spelled those keys `active_persona`
and `[[personas]]` still loads exactly as it did — nothing has to be rewritten by hand — and the next
time the file is saved it carries the new spelling.

The default configuration talks to OpenRouter for all three capabilities:

```toml
active_profile = "compae"

[server]
bind = "127.0.0.1:7070"
lan = false
token = ""               # the WebSocket server's own bearer token; empty means none generated yet

[listening]
mode = "push"            # push | auto | hands_free
silence_ms = 1000
mic_gain = 12.0

[providers.chat]
kind = "openai_compat"   # openai_compat | gemini | elevenlabs
base_url = "https://openrouter.ai/api/v1"
model = "google/gemini-2.5-flash"
key = "openrouter"       # the key *name*, never the key itself

[providers.speech]
kind = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
model = "google/gemini-3.8-flash-tts"
key = "openrouter"

[providers.transcribe]
kind = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
model = "openai/whisper-large-v3"
key = "openrouter"
```

`kind` is the connector: `openai_compat`, which is what Ollama, OpenRouter, OpenAI and most local
servers speak, or the native `gemini` and `elevenlabs` connectors. Not every connector offers every
capability — Gemini has no transcription and ElevenLabs has no chat — and a configuration that asks
for one of those pairs is refused by name rather than quietly falling back to another connector.
`key` is the **name** of a key, not the key; an empty `key` means the endpoint wants no key at all,
which is how a local server is configured:

```toml
[providers.chat]
kind = "openai_compat"
base_url = "http://localhost:11434/v1"
model = "llama3.1"
key = ""

[providers.speech]
kind = "openai_compat"
base_url = "http://localhost:11434/v1"
model = "your-speech-model"
key = ""

[providers.transcribe]
kind = "openai_compat"
base_url = "http://localhost:11434/v1"
model = "your-transcription-model"
key = ""
```

As soon as the client starts it speaks the profile's greeting, so the `speech` provider has to work
before you hear anything: `llama3.1` in the example above is a chat model, and a local server has
to be reachable for each capability you actually use. `config.toml` is read when the client starts,
and again every time the configuration screen opens, so a file you edited by hand is what the screen
shows.

### Listening modes

`mode` is one of three values, and all three can be set here or on the Listening tab of the
configuration screen:

- **`push`** (the default): `Tab` opens the microphone and `Tab` again closes it, sending what it
  heard for transcription. Nothing is transcribed until you close it.
- **`auto`**: a voice detector decides what is an utterance. Open the microphone once with `Tab` and
  it keeps listening; an utterance is transcribed when the detector confirms a pause, and nothing
  before the first word — the room, the silence you started talking through — is ever transcribed.
  The assistant's own voice does not reopen the microphone while it speaks. `Tab` still closes the
  microphone by hand, which is how what the detector has confirmed so far is sent mid-utterance.
- **`hands_free`**: not implemented yet. A wake word is what it waits for, and it arrives in a later
  plan; until then the mode behaves like `push` — you open the microphone by hand — and the header
  says so in the interface's language when the session announces the mode, instead of leaving the
  user to guess why nothing is listening.

### What applies when

A save is not a restart, and it does not pretend to be one. Saving applies:

- **the active profile**: the session switches to it and the next turn answers with its prompt;
- **the listening mode**, whichever of the three you chose;
- **the connectors**: the three providers the file names are built and swapped into the running
  session, and every turn started after the save uses them;
- **the microphone gain**.

The next launch brings:

- **a profile's edited texts** — its prompt, voice instructions, greeting, wake word, name and
  language — because the channel between the screen and the session carries a profile *id*, not its
  text, and the session rebuilds the profile from the configuration it started with. A profile added
  by hand after the launch is in the file but not in the running session, so it becomes selectable at
  the next launch;
- **the silence threshold** (`silence_ms`), since the detector is built with it when the app starts.

The microphone device (`mic_device`) is a third case: the Listening tab saves it and the file
carries it, but the client opens the system's default input device whatever it says, so naming a
device does not change what is recorded — at this launch or the next one.

The Server tab — `bind`, the LAN switch and the token — configures the WebSocket server. `serve`
reads those three values; the token is the password every remote client presents, and `Ctrl+R`'s
fresh value is written when you save. Over a socket the token row is **native-only**: a client's save
never carries `server.token`, so the row says where the token belongs instead of showing one the
server would not learn. [Running a server, and attaching to
one](#running-a-server-and-attaching-to-one) is what those three values mean.

### The login screen

With `[login]` configured, a client opens on a screen before the HUD: the dashboard shows through
behind a pixelated veil, and in the middle there is the core, one line and a box. Say who you are, or
type it, and the door opens on the first entry whose keywords are all in the answer:

```toml
[login]
# The screen appears only when this is true *and* an entry below has keywords. A fresh install has
# neither, so nobody meets a locked door by surprise.
enabled = true
# The microphone opens with the screen, and only speech the local detector confirms is transcribed.
# Typing always works and costs nothing, so `false` is a perfectly good way to keep the ritual.
voice = true

[[login.allowed]]
name = "Andrés"                 # what the screen greets you with; may be empty
keywords = ["andres", "core"]   # all of them must be in the answer
```

**This is a ritual, not a lock.** Nothing here is authentication and nothing here protects anything:
no passwords, no accounts, no lockout, no attempt log, and nothing that changes what the server
accepts. **`server.token` above remains the only real requirement**, and turning this screen on adds
no security to the application — it protects the joke, and that is the whole of it.

The rules, so that what you write behaves the way you expect. The **first entry in file order** whose
keywords all appear wins, and its `name` is the greeting; file order is your ranking, so put the most
specific person first. Every keyword is matched as a **whole word**, case- and accent-insensitively —
for the accented letters these profiles write (`á é í ó ú ü ñ ç` and their relatives; any other letter
keeps its own lowercase form, so this is not full Unicode normalization): `andres` matches `Andrés` and
`ANDRÉS`, and it does not match `andresito`; `core` does not match `score`. An entry with several
keywords needs all of them, and a multi-word keyword such as `"andres del equipo"` has to be said in one
run. An entry with no usable keyword is ignored, so `enabled = true` with an empty or half-edited
allowlist shows no screen at all rather than a door nobody can open. The section is authored by hand in
this file — there is no Login tab, and the configuration screen carries it through a save untouched —
and since Plan 3 the same section travels inside `ConfigView`, so a client that presented the token can
read the allowlist over the socket and send it back changed. The server still does the judging, and
`server.token` is still the only door.

The screen is one screen: no second one opens while it is up, `Tab` turns the microphone on and off,
`Enter` sends what you typed — or, with the box empty after a refusal, re-arms the microphone — `Esc`
quits, and every other key is part of your answer. The microphone opens with the screen when `voice`
is true, the top border says so in red while it is open, and `Tab` closes it — and while that capture
runs, the client shows it by setting the same open-microphone state push-to-talk sets. Three refusals
in a row stop the automatic listening and the bottom border points you at this file; `Tab` starts it
again.

## Running a server, and attaching to one

```sh
# The server, headless: the web client at / and the web terminal at /terminal.
cargo run -p tipsy-compadre -- serve

# The same TUI you get from a bare run, drawn on this machine and driven by that server.
cargo run -p tipsy-compadre -- --connect ws://127.0.0.1:7070/ws

# From another machine, once `lan = true` and a token are in config.toml:
cargo run -p tipsy-compadre -- --connect ws://192.168.1.20:7070/ws --token <the Server tab's token>
```

- `serve` records where it is listening — the address it was asked for and the one it bound, which
  differ whenever `--bind` names port `0` — in `tipsy-compadre.log`. `--verbose` prints the bound
  address as well. `--bind` overrides `bind` for one run and never writes the file back.
- **The bind rule is about the address, not the switch.** An address that is not loopback is refused
  without a token, and `lan = true` on `127.0.0.1` is perfectly legal: `lan` is the operator's
  intent and nothing reads it but the log line that records it. A hostname is refused by name rather
  than resolved, because a name may resolve somewhere off this machine.
- The token is 128 bits of OS entropy rendered as 32 lowercase hex characters. It travels in `Hello`
  for a protocol client — **never in the socket's URL** — and as `?token=…` on the two pages, which a
  browser can only send in the URL; both pages therefore answer with `Referrer-Policy: no-referrer`
  and `Cache-Control: no-store`, so the token is not in a `Referer` and not in a cache.
- **A page in your browser can open a loopback listener's `/ws`**, because a WebSocket handshake is
  not subject to the same-origin policy and a loopback socket asks for no token. So the socket refuses
  (`403`) a request that carries an `Origin` which is not this server's own host; `--connect` sends no
  `Origin` at all and is unaffected, which is what keeps the local attach above working. A browser
  client must therefore reach the socket under the same host it was served from.
- **Nothing generates a first-run token yet.** A new `config.toml` carries `token = ""`, and the one
  thing that mints one is the Server tab's `Ctrl+R`, which fills the row and writes it when you save.
  With an empty token `serve` is loopback-only, which is what the bind rule enforces; a first-run
  token is on the release checklist below, together with the requirement that showing it once must
  not put it in the log. `TIPSY_CONFIG_DIR` moves `config.toml`, `tipsy-compadre.log` and a generated
  certificate somewhere else for one run, so the checks below never touch your own configuration.
- A client's save **cannot move a provider's `base_url` or `key_name`, or `server.token`**: those
  three are not in the message a client sends (`Command::UpdateConfig` carries `ClientConfigView`),
  and the server keeps its own values. Everything else on the screen — the bind address, the
  listening settings, the profiles, each connector's kind and model — does cross.
- `/` is the web client and `/terminal` the web terminal. Both are served from the bundle the binary
  embeds, and what the embed reads is the repository's `web/` directory — so a plain `cargo build`
  already serves the checked-in `web/terminal/index.html`, and `web/vendor/` has to be fetched
  (`cargo run -p xtask -- dist`) or the page loads with no xterm.js and renders as a blank black
  rectangle. **Run that command from somewhere inside the repository** — its root, or any crate
  directory. It resolves `web/vendor/` from the `xtask` crate's own location and not from the
  directory you are standing in, so the fetched files always land where the embed reads them, and it
  prints the absolute path it wrote. (Before that correction it used a relative path: run from
  `crates/tipsy-compadre` it printed `web/vendor is ready` while writing
  `crates/tipsy-compadre/web/vendor/`, which nothing reads — a blank terminal from a command that
  reported success.) Each run also re-checks the fetched files against the sha256 recorded in
  `web/vendor/README.md`, and refuses with both digests if one has been truncated or edited. Until
  Phase 3 lands `web/client/index.html`, `/` answers with a page that names the command.
  **`cargo xtask dist` is not a working spelling**: cargo looks for `cargo-xtask` on `PATH`, and the
  long form above is the one that works.
- `--connect` runs the TUI and nothing else: it starts no server, builds no provider and reads no
  API key, because every provider call happens in the server process.
- `--verbose` also writes the log to stderr. The log file is `tipsy-compadre.log`, beside
  `config.toml`, and it never contains the token, a key or audio: every complete line is scrubbed on
  its way out, and both copies go through the same scrubber. A `--connect` URL is printed without its
  query string, so a token somebody pasted into the URL stays out of the terminal, the log and the
  bug report.

### TLS, and what it does and does not do

`serve --tls` encrypts the connection, which is what makes the token and a newly set API key safe on
a LAN. With no `--tls-cert`/`--tls-key` pair it generates a self-signed certificate once, beside
`config.toml`, and the run that minted it records its SHA-256 fingerprint — as `AA:BB:…`, the string
a browser's certificate viewer shows — in `tipsy-compadre.log`, so the two can be compared.
`--verbose` prints those lines too.

**It does not unlock the browser's microphone from another machine.** Browsers expose
`getUserMedia` only in a secure context; `http://127.0.0.1` already is one, so the microphone works
on the server machine with no TLS at all; and a certificate the browser does not trust is not a
secure context. For a browser on another device, use a certificate that device already trusts —
`mkcert` with its root installed, or a real certificate for a name that resolves to the LAN
address — through `--tls-cert`/`--tls-key`.

## Realtime voice mode

`F10` starts and ends a **live call**: one duplex WebSocket session with OpenAI's Realtime API
(`gpt-realtime-2.1` by default) in which the microphone streams up, the model's speech comes down as
it is generated, the server segments the turns, and you can talk over it mid-sentence. The header
shows a red `EN VIVO` / `LIVE` for as long as the call lasts, and the call ends by itself at
`[realtime] max_minutes`.

**A call is not free, and it does not hide what it costs.** Realtime bills per token, not per minute,
and an open microphone is billed while it streams: the research note's rates are one input token per
100 ms of audio and one output token per 50 ms of the model's own speech, at $32 per 1M audio input
tokens and $64 per 1M audio output tokens for `gpt-realtime-2.1`. **Doing that arithmetic here**,
rather than repeating a summary of it: ten input tokens a second is 600 a minute, which at $32 per 1M
is about **$0.02 per minute of listening**; twenty output tokens a second is 1200 a minute, which at
$64 per 1M is about **$0.08 per minute of the model speaking**. The same rates on
`gpt-realtime-2.1-mini` ($10/$20 per 1M) give about **$0.006 and $0.024**. Every `response.done`
carries the session's own `usage` if you want to check them against your own bill — this client does
not display it. Input transcription
is part of this mode — the client configures `gpt-4o-transcribe` so that what you said can appear in
the log — and it bills separately on that model's own rate card.

The mode is off until you turn it on, and it will not start while it is off: the key set the section
names (`openai` by default) has to resolve first. Turn detection reuses `[listening] silence_ms`, so
one number, not two, decides how long a pause ends your turn.

A call that reaches the API's own 60-minute cap reopens a fresh session and the assistant starts from
nothing: the call carries on and the microphone stays open, but nothing said before the reopen is
remembered. The log says when it happened.

```toml
[realtime]
enabled = false          # a call bills while it is open: this is the switch
model = "gpt-realtime-2.1"
key = "openai"           # the key *name*, never the key itself
voice = "marin"          # realtime voices are their own list, not the profile's `voices` table
max_minutes = 10         # the call ends by itself here
```

The cheaper push-to-talk variant — framing the input by hand instead of streaming an open microphone
— is documented as a future option and is not implemented.

## Privacy

Everything you ask is sent to the providers in `config.toml`: the text of your questions, the
conversation history and the assistant's replies go to the chat provider, each reply is sent to
the speech provider to be spoken, and the audio recorded while the microphone is open goes to the
transcription provider. Nothing is sent anywhere else, and there is no telemetry. If you point
those providers at a server on your own machine, nothing leaves it.

API keys stay in the operating system's credential store — Windows Credential Manager, the macOS
Keychain or the Secret Service on Linux — under the service `tipsy-compadre`, or in the environment
variable you exported. `key status` prints only a redacted form of a key, `key set` prints no more
than that when it confirms, and nothing ever writes a key into `config.toml`.

Nothing above reaches the log either, and that is enforced rather than promised: `tipsy-compadre.log`
is written through a scrubber, so the server's token, every key value this process resolved and any
sample dump of recorded audio are removed before a line reaches the file — and before it reaches the
`--verbose` copy on stderr, which goes through the same scrubber.

## Profile guideline

Regional humour is affectionate. Play with the friendly clichés of a place; never mock or insult
people or groups, never use slurs, never imitate real people. Profiles are **original characters**:
a profile never claims to be a real person or a character from films, books or other media, and
never quotes them.

## Documentation

- [`docs/architecture.md`](docs/architecture.md) — how the client, the session, the brain, the
  providers and the interface fit together.
- [`docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md`](docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md)
  — the v1 design spec: decisions, protocol, profiles, errors, milestones.

## Building

The toolchain is pinned to Rust 1.96.1 and every crate uses `edition = "2024"`.

```sh
cargo build
cargo test
```

**Windows, from Git Bash:** remove `/usr/bin` from `PATH` before `cargo`, because Git's `link.exe`
shadows MSVC's:

```sh
export PATH="$(echo "$PATH" | tr ':' '\n' | grep -v '^/usr/bin$' | paste -sd:)"
```

In PowerShell no change is needed.

### Building the full distribution

Two steps, wrapped in one command: the WebAssembly bundle first, then the binary that embeds it.

```sh
cargo run -p xtask -- dist
```

It needs the network once per clean checkout (it fetches the pinned xterm.js for the web terminal) and
the wasm target, which the pinned toolchain does not install by itself:

```sh
rustup target add wasm32-unknown-unknown
```

The command runs `cargo build --target wasm32-unknown-unknown -p tipsy-web --release`,
`wasm-bindgen --target web` (the CLI's version must be the one `Cargo.lock` pins — `cargo install -f
wasm-bindgen-cli --version <that version>`), and finally `cargo build -p tipsy-compadre --release` with
the bundle embedded. Run it from somewhere inside the repository — its root or any crate directory:
`dist` resolves `web/vendor/` from the `xtask` crate's own location rather than from your shell's
working directory, so the fetched assets land where the embed reads them, and it prints the absolute
path it wrote. A file that is present and does not match the pinned digest is **refused** rather than
re-fetched, so delete the file it names and run it again.

**There is no short form.** `cargo xtask dist` is not a working spelling and no document may carry it:
cargo resolves `cargo <name>` by looking for a `cargo-<name>` binary on `PATH`, and this workspace's own
binary target is not there, so it answers ``error: no such command: `xtask` ``. `server::assets`'s
`DIST_COMMAND` (the string the missing-bundle page renders), `docs/architecture.md` and this section all
spell the long form. An `[alias]` in `.cargo/config.toml` would work on your own machine, but `.cargo/`
is git-ignored here and never ships, so nothing documented may depend on it.

**A plain `cargo build` produces a binary without the web client.** The server still runs and `/`
answers with the checked-in page, whose script the server does not have; the page that says how to build
the bundle is what a build with no `web/client/index.html` serves. A *debug* build reads `web/` from the
directory you run the binary from, so `cargo run -p tipsy-compadre` started at the repository root
serves a bundle that is already built, and a binary copied elsewhere does not.

## How to verify the web terminal

The web terminal runs the real TUI in a browser over a socket that carries raw PTY bytes and no
protocol of its own. **There is no browser in this repository's test harness**, so what a browser and
only a browser adds — rendering, xterm.js's `onData` and `onBinary`, `FitAddon` and `ResizeObserver` —
is verified by hand, once, with these steps. Everything a test can reach is in `server::conn`'s own
tests: that the page is the one the bundle serves, that every asset it names is embedded, and that it
is behind the same token rule the socket is.

1. `cargo run -p xtask -- dist` and then `cargo run -p tipsy-compadre -- serve`. Start the first one
   from inside the repository — its root or any crate directory, it does not matter which, because
   `dist` writes to the repository's own `web/vendor/` and prints the path it used; a shell standing
   outside the workspace cannot find the package at all. `serve` records where
   it is listening in `tipsy-compadre.log`; `--verbose` prints the bound address too. It spawns its
   web terminal's child as **its own running binary** (`std::env::current_exe()`), never as
   `tipsy-compadre` looked up in `PATH`: do not start a child by hand for this check, and if you do,
   give it an absolute path. A `PATH` entry that is empty, relative, or points into this checkout could
   otherwise plant the program that runs with your authority.
2. Open `http://127.0.0.1:7070/terminal`. When the configuration holds a token, append
   `?token=<the value the Server tab shows>` to that URL: the page reads it out of its own query
   string and puts it on the socket's URL as well, because a browser can set neither a header on a
   navigation nor one on a WebSocket handshake. **Reach the page under the host name you type and
   nothing else** — the route refuses a socket whose `Origin` is not the `Host` the request arrived
   on, and `localhost:7070` and `127.0.0.1:7070` are different authorities.
3. The HUD appears: the header with the profile's name, the two rings, the oscilloscope, the status
   line and the log panel. A blank black rectangle means `web/vendor/` was never fetched (step 1); a
   page that says the web UI is not available in this build means the binary was built before
   `web/terminal/index.html` existed.
4. Click the **⚙ gear at the right edge of the header**, or the mode word immediately to its left.
   Those two are the only clickable controls on that row and they open the configuration screen; the
   profile's own badge (`◉ NÚCLEO`) is not clickable. This step is the proof that a mouse position
   arrives as the cell the TUI thinks it is.
5. Drag the browser window's edge. The TUI reflows to the new size — the panels take the new width and
   the header stays one line — instead of keeping its old width. The child reads its console width
   once per line and the server debounces the resize by 100 ms, so the new width shows up on the
   child's next line of output rather than instantly.
6. `Ctrl+C` at the prompt that started the server stops it; the browser shows `[disconnected]`. The
   server's last frame is `{"code":N,"type":"exit"}` carrying the child's own exit code, and it is the
   only text frame this socket ever sends. **The keys are in that order and not the other one**: the
   server builds the frame with `serde_json::json!`, whose `Value` is a sorted map, so `code` precedes
   `type` on the wire. Nothing that parses JSON can tell, which is exactly why the bytes are written
   out here rather than the shape.
7. After step 6, `tasklist | Select-String tipsy-compadre` lists nothing. If a `tipsy-compadre.exe`
   is still there after the tab is closed, the teardown in `server/conn.rs` is leaking a child.

## Release checklist

The manual checks of the design spec (§13), terminal first. Nothing here replaces `cargo test`; it is
what a test cannot hear.

- [ ] The client starts, greets you out loud, and answers a typed question in the log.
- [ ] `Tab` opens the microphone, what you said appears as `tú …`, and `F2` replays the reply.
- [ ] `F9` opens the configuration screen, a change saves, and it takes effect without a restart.
- [ ] **A live call, one spoken exchange (`F10`).** With `[realtime] enabled = true` and a key stored
      or exported, press `F10`: the header shows a red `EN VIVO` / `LIVE`, say one sentence, see it in
      the log as `tú …`, hear the answer, watch its transcript appear while it is spoken, and press
      `F10` again to end the call — the indicator must go.
      This is the only check that exercises the real socket, the real 24 kHz audio path and the real
      barge-in. The payload fields of `response.output_audio.delta` and
      `response.output_audio_transcript.delta` are **inferred from sibling events in the research
      note, not verified** (its documentation page truncated before the Response section), so that
      shape is pinned in three places and nowhere else: the two `INFERRED PAYLOAD` fixtures in
      `crates/tipsy-compadre/src/realtime/events.rs`, the reads beside them in the same file's mapping
      (`field(...)` for `delta`, and `optional_field(...)` for the `item_id` a frame may legitimately
      omit), and the two builders (`delta`, `transcript_delta`) in
      `crates/tipsy-compadre/tests/realtime.rs`. They change together. If the transcript stays empty
      while the audio plays, that inference is what is wrong, and the field name in those three
      places is what to change.
- [ ] Talk over the call mid-answer: the speech stops at once, the log keeps only what was said
      before the interruption, and the model does not carry on from the cut-off sentence.
- [ ] **No key reaches a log.** The realtime socket's `Authorization: Bearer <key>` header is
      protected by the subscriber's ceiling: `tungstenite` logs the request line — header included —
      at `trace!`, and `logging.rs` filters both of its layers at `DEBUG`, so `trace!` is
      unreachable and stays that way (the test `the_subscriber_never_enables_trace` fails if a layer
      is ever raised). Both layers are also scrubbed on the way out, so the server's token, a key
      value this process resolved and any sample dump cannot reach the file or the `--verbose` copy.
      The same invariant sits beside `RealtimeSocket` in
      `crates/tipsy-compadre/src/realtime/socket.rs`.
- [ ] **A live `serve` and a `--connect` attach.** Point `TIPSY_CONFIG_DIR` at a scratch directory so
      none of this touches your own `%APPDATA%\tipsy-compadre`, start `tipsy-compadre serve` on a
      loopback port, attach with `tipsy-compadre --connect ws://127.0.0.1:<port>/ws`, and type. Then
      over the LAN: `serve --bind 0.0.0.0:7070` with a token, and attach from the other machine with
      `--token`. **The refusal path matters as much as the attach**: with no token, or a wrong one,
      the client must print `the server refused the connection: token_required` (or `token_invalid`)
      on an ordinary terminal *before* the alternate screen appears, and no line may contain the
      token itself — not the refusal, not the log, not the `--verbose` banner.
- [ ] **`serve --tls` really serves.** Start `serve --tls` on a loopback port and fetch
      `https://127.0.0.1:<port>/?token=…` — or point a browser at it — then repeat with
      `--tls-cert`/`--tls-key` and a certificate the client already trusts. The generated pair sits
      beside `config.toml` and the run that minted it writes the SHA-256 fingerprint to
      `tipsy-compadre.log`, which `--verbose` also prints: compare it with the browser's certificate
      viewer. **This item is why the check exists — the phase's first live run found that the TLS
      listener never started at all**, because `run` bound the port and then asked the TLS server to
      bind it a second time; the fix wave takes the socket that was already bound and checked.
- [ ] **Generate a first-run token, and show it once.** *Not implemented today*: a fresh
      `config.toml` has `token = ""` and only the Server tab's `Ctrl+R` mints one. When it lands, a
      first run must write a token, show it to the user exactly once, and **showing it must not put
      it in a log** — not in `tipsy-compadre.log`, not in the `--verbose` copy, not in a URL, and not
      in any socket frame the client has not authenticated for.
- [ ] **The login door in the browser, with the system microphone disabled.** Rebuild the bundle first —
      the page loads `web/dist`, which is generated and never committed, so
      `cargo run -p xtask -- dist` comes before the server does — then turn the microphone off
      in the OS (Windows: Settings → Privacy → Microphone → "Let desktop apps access your
      microphone"), open `http://127.0.0.1:7070/` with `[login]` configured and `voice = true`, and
      press the gate. **Expected:** the screen opens *typing-only* — no `● rec` on its top border, no
      microphone in the OS' own in-use indicator — and its one line reads `Sin micro: escribe, que
      también vale`. Type an allowed answer and press `Enter`: the door opens, the veil lifts, and the
      HUD appears. **Do this half first, with the device really taken away**: it is the branch no test
      can reach. Then turn the microphone back on and repeat; the same screen opens with the red
      `● rec`. While the door is shut a human must look at three things no test can see: that the veil
      is unreadable (the HUD is visibly there and visibly out of focus; `Veil::Dim` is the one-constant
      escape hatch in `render_login.rs` if `▒`/`░` land badly in the browser's font — login spec §7 and
      §13), that the core inside the widget pulses while you speak, and that the gate's own button was
      removed from the page once the microphone opened rather than sitting over the canvas. Record
      which it was, and record whether `Veil::Dim` had to be switched on. The keyboard half is worth
      the same eye: the door answers `Tab`, `Enter` and `Esc` only, and the HUD behind it answers
      nothing until the screen is down.
- [ ] The real microphone and speakers in the browser, and the wake word in Spanish and English,
      once those milestones land.
