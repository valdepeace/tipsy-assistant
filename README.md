# tipsy-compadre

tipsy-compadre is a voice assistant for software developers with a strong, affectionate regional
personality. The default profile, *El Compae*, is a lazy, joking, slightly tipsy Andalusian who
knows Jira, GitHub, GitLab, CI/CD and code review, dodges work with developer excuses and, in the
end, always gives a correct and useful answer. The project is meant to be shared with the
community: a single executable, cloud providers that anyone can use with their own key, and
profiles that anyone can write for their own region.

**Status: early development — terminal client only.** The workspace, the protocol, the
configuration and the key handling, the chat, speech and transcription providers (the
OpenAI-compatible connector plus the native Gemini and ElevenLabs ones), the brain, the native
terminal client and its configuration screen all work, and nine profiles ship with it. Two of the
three listening modes work: `push`, where you open and close the microphone by hand, and `auto`,
where a voice detector decides what is an utterance. Still to come: `hands_free` (it waits for the
wake word), the WebSocket server, the web client and the web terminal, plus CI and releases.

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

The Server tab — `bind`, the LAN switch and the token — is for the WebSocket server, which does not
exist yet: the three values are written to the file and nothing listens on them today.

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
      `crates/tipsy-compadre/src/realtime/events.rs`, the two `field(...)` reads beside them in the
      same file's mapping, and the two builders (`delta`, `transcript_delta`) in
      `crates/tipsy-compadre/tests/realtime.rs`. They change together. If the transcript stays empty
      while the audio plays, that inference is what is wrong, and the field name in those three
      places is what to change.
- [ ] Talk over the call mid-answer: the speech stops at once, the log keeps only what was said
      before the interruption, and the model does not carry on from the cut-off sentence.
- [ ] The real microphone and speakers in the browser, and the wake word in Spanish and English,
      once those milestones land.
