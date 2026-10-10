# tipsy-compadre

tipsy-compadre is a voice assistant for software developers with a strong, affectionate regional
personality. The default profile, *El Compae*, is a lazy, joking, slightly tipsy Andalusian who
knows Jira, GitHub, GitLab, CI/CD and code review, dodges work with developer excuses and, in the
end, always gives a correct and useful answer. The project is meant to be shared with the
community: a single executable, cloud providers that anyone can use with their own key, and
profiles that anyone can write for their own region.

**Status: early development — terminal client only.** The workspace, the protocol, the
configuration and the key handling, the OpenAI-compatible chat, speech and transcription
providers, the brain and the native terminal client all work. Still to come: the configuration
screen, the other profiles (`meigo`, `chulapo`, `good_ol_boy`…), the native Gemini and ElevenLabs
connectors, the `auto` and `hands_free` listening modes, the WebSocket server, the web client and
the web terminal, plus CI and releases. Today's listening mode is `push`: you open and close the
microphone by hand.

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
| `←` / `→` | Switch between the `CHAT` and `NETWORK` views |
| `Esc` | Quit |

Every action also has a mouse equivalent: the `CHAT`/`NETWORK` titles and the **✗ Clear** button are
clickable, and clicking a line of the conversation selects it. Keyboard and mouse each need their
own binding.

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

[listening]
mode = "push"            # push | auto | hands_free (only push is implemented in this plan)
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

`kind` is the connector: only `openai_compat` is implemented so far, and it is what Ollama,
OpenRouter, OpenAI and most local servers speak. `key` is the **name** of a key, not the key; an
empty `key` means the endpoint wants no key at all, which is how a local server is configured:

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
to be reachable for each capability you actually use. The configuration is read at startup, so quit
and run the client again after editing `config.toml`.

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
