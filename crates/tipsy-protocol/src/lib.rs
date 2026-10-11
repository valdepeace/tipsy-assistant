//! Messages exchanged between the tipsy-compadre server and its clients.
//!
//! Text messages are JSON (`Command`, `Event`, and the configuration they carry as `ConfigView`);
//! audio travels as binary `Frame`s.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Tui,
    Web,
    WebTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListeningMode {
    #[default]
    Push,
    Auto,
    HandsFree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantState {
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiLanguage {
    Es,
    En,
}

impl UiLanguage {
    /// Spanish for any `es` / `es-*` tag, English for everything else.
    pub fn from_bcp47(tag: &str) -> Self {
        let primary = tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        if primary == "es" { UiLanguage::Es } else { UiLanguage::En }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Auth,
    RateLimited,
    Timeout,
    Network,
    BadResponse,
    NoKey,
    Mic,
    Config,
    NothingHeard,
    Internal,
    /// The client's `protocol_version` is not this server's (spec §4.1).
    ProtocolVersion,
    /// The server is exposed and `Hello` carried no token (spec §4.1).
    TokenRequired,
    /// The server is exposed and `Hello` carried a token that is not its own (spec §4.1).
    TokenInvalid,
    /// A text frame that is not a well-formed `Command`, or a binary frame that is not an audio
    /// `Frame`. Recoverable: the connection stays open (spec §10).
    BadMessage,
    /// `SetKey` on a connection that is neither loopback nor TLS (spec §9.4). The variant lands here
    /// because the wire needs the spelling; **Task 1.8 owns the refusal that emits it**, and this
    /// task emits nothing with it.
    InsecureTransport,
}

/// Which connector the configuration screen's "Test" button is asking about (spec §9). Plan 2B
/// Task 2 puts it on the wire (`Command::TestProvider`, `Event::TestResult`); Plan 2A Task 5 uses
/// it first, to name a capability a connector does not offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Chat,
    Speech,
    Transcribe,
}

/// The capability as a human reads it, for a message such as
/// "transcription is not supported by the `Gemini` connector". The serde spelling above stays
/// `transcribe` because it is the wire contract; this is only ever shown to a person. It lives
/// here because the orphan rule forbids a `Display` impl for a `tipsy-protocol` type anywhere
/// else, and the enum's own formatting belongs beside the enum.
impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Capability::Chat => "chat",
            Capability::Speech => "speech",
            Capability::Transcribe => "transcription",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub language: String,
}

/// Which login screen a client should show before its HUD (spec `2026-10-11-login-screen-design.md`).
/// `Option::None` on `Welcome` means "no login screen": the client goes straight to the HUD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginView {
    /// Whether the microphone opens with the screen. Typing always works, and is always free.
    pub voice: bool,
}

/// `[server]`, as the configuration screen edits it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerView {
    pub bind: String,
    pub lan: bool,
    /// §4.3: the one field of this whole type that may hold something a client is not entitled to
    /// see. A connection that presented no token receives [`ConfigView::redacted`]'s empty string.
    pub token: String,
}

/// One row of the Providers tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderView {
    pub capability: Capability,
    pub kind: String,
    pub base_url: String,
    pub model: String,
    /// The key's *name*, never the key: the same rule the configuration file follows.
    pub key_name: String,
}

/// `[listening]`, as the screen edits it. `None` is the system default device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListeningView {
    pub mode: ListeningMode,
    /// The row is a text field, so the value travels as the text it holds and the save parses it.
    pub silence_ms: String,
    pub mic_gain: String,
    pub mic_device: Option<String>,
}

/// `[realtime]`, carried and never drawn (Plan 2C Task 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RealtimeView {
    pub enabled: bool,
    pub model: String,
    /// The key's *name*, never the key.
    pub key_name: String,
    pub voice: String,
    pub max_minutes: u32,
}

/// One `[[login.allowed]]` entry (the login-screen plan's wire).
///
/// Deliberately **not** `LoginEntryView`: that name is this tier's section view of the whole
/// `[login]` table, and one word cannot mean both the table and one of its rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllowedPersonView {
    pub name: String,
    pub keywords: Vec<String>,
}

/// `[login]`, carried and never drawn.
///
/// Deliberately **not** `LoginView`: that type already exists above as what `Welcome` tells a client
/// about its login *screen* — one flag. This is the whole `[login]` section an editor edits, entries
/// and all, and it is named for the section rather than for the screen the section configures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginSettingsView {
    pub enabled: bool,
    pub voice: bool,
    pub allowed: Vec<AllowedPersonView>,
}

/// One `[[profiles]]` entry. Deliberately **not** `ProfileView`: that type is the three fields a
/// client is *told* about its active profile; this is the whole entry an editor edits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileEntryView {
    pub id: String,
    pub name: String,
    pub language: String,
    pub prompt: String,
    pub voice_instructions: String,
    pub greeting: String,
    pub wake_word: String,
    /// Sorted by connector name: a map's order is not a screen order, and the row index names back.
    pub voices: Vec<(String, String)>,
}

/// The editable configuration, as the configuration screen edits it (spec §4.3).
///
/// The fields are `tipsy_ui::ConfigForm`'s, field for field, with each section type renamed
/// `…Form` → `…View`. The screen's own view state — the focused tab and row, the text being typed,
/// the notice line, the dirty flag and the "filled" flag — stays on the form and never crosses.
///
/// **No field of this type may hold a secret.** Exactly one field may hold something a client is not
/// entitled to see: `server.token`, which is what [`ServerView`]'s own doc comment says, and
/// [`ConfigView::redacted`] is the one way to take it away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigView {
    pub active_profile: String,
    pub providers: Vec<ProviderView>,
    pub listening: ListeningView,
    pub realtime: RealtimeView,
    pub login: LoginSettingsView,
    pub profiles: Vec<ProfileEntryView>,
    pub server: ServerView,
}

impl ConfigView {
    /// The same view with no token in it: what `Event::Config` answers a client that is neither a
    /// loopback connection nor one that presented the server's token (spec §4.3). It clears the field
    /// rather than removing it, so the answering side has one code path and the other end has no
    /// `Option` to interpret.
    pub fn redacted(mut self) -> Self {
        self.server.token.clear();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Hello { token: Option<String>, client: ClientKind, protocol_version: u32 },
    Ask { text: String },
    Say { text: String },
    ListenStart,
    ListenStop,
    Cancel,
    Reset,
    SetProfile { id: String },
    SetListeningMode { mode: ListeningMode },
    /// The configuration screen's "Test" button: one minimal call against the connector for this
    /// capability. Plan 2B Task 7 runs the probe and answers with `Event::TestResult`.
    TestProvider { capability: Capability },
    /// The login screen is up and the microphone is opening (`login_start` on the wire). The server
    /// answers nothing: what it does is start collecting the audio the client is about to stream.
    LoginStart,
    /// Stop capturing and discard whatever was buffered (`login_stop`). Idempotent.
    LoginStop,
    /// A typed answer (`login_answer`). Judged whether or not a capture is active, so a typing-only
    /// client never has to send `LoginStart` at all.
    LoginAnswer { text: String },
    /// The configuration screen opening: the session answers `Event::Config`.
    GetConfig,
    /// The configuration screen saving. The server replaces the editable configuration, persists it,
    /// re-derives every session's brain and broadcasts `Event::ConfigChanged` (spec §4.2).
    UpdateConfig { config: ConfigView },
    /// A new API key for the *server's* credential store. Refused unless the connection is loopback
    /// or TLS (spec §9.4), and answered with `Event::ConfigChanged` when it is accepted.
    SetKey { name: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Welcome {
        session_id: u64,
        profile: ProfileView,
        ui_language: UiLanguage,
        listening_mode: ListeningMode,
        web_bundle: bool,
        /// `None` — no login screen. Present on every `Welcome` from this server, absent from an older
        /// one's, which `#[serde(default)]` turns into `None`.
        #[serde(default)]
        login: Option<LoginView>,
    },
    Status { state: AssistantState, detail: Option<String> },
    Heard { text: String },
    ReplyDelta { text: String },
    Reply { text: String },
    /// Header for the `Frame::Speech` with the same `seq` that follows it.
    SpeechChunk { seq: u32, sample_rate: u32 },
    Done { first_audio_ms: Option<u64>, total_ms: u64 },
    Stats { cpu: f32, ram: f32, net_in: f64, net_out: f64 },
    Error { code: ErrorCode, message: String, recoverable: bool },
    /// The answer to `Command::TestProvider`. `message` is never empty: the provider's own words
    /// on failure, a short human summary on success. `ok` is `true` only when a real minimal call
    /// answered — never for an error body.
    TestResult { capability: Capability, ok: bool, latency_ms: u64, message: String },
    /// The server's answer to "is a realtime call live?": `active: true` when a call opens its
    /// socket and `false` when that call ends, by any of its endings (Plan 2C). An event rather
    /// than a new `AssistantState`: the server owns the socket, so the indicator cannot be on
    /// while no socket is open, "the call ended" is explicit rather than inferred from a status
    /// transition back to `Idle`, and `AssistantState` keeps meaning exactly one thing — what the
    /// assistant is doing during a turn. Nothing else about the protocol changes: a call's
    /// transcripts still arrive as `Heard`, `ReplyDelta` and `Reply`.
    Live { active: bool },
    /// What the transcriber heard from a login utterance, before the verdict. The screen shows it so
    /// that a misheard accent is visible rather than mysterious.
    LoginHeard { text: String },
    /// The ruling: whether the door opens, and the matched entry's name when it does.
    LoginVerdict { ok: bool, name: Option<String> },
    /// The redacted configuration, answering `Command::GetConfig` (spec §4.4).
    Config { config: ConfigView },
    /// The active profile changed. Carries the same three fields `Welcome` carries.
    ProfileChanged { profile: ProfileView },
    /// The configuration changed. Deliberately no payload: a client that is showing it asks again
    /// with `GetConfig`, and sending the whole configuration on every save would put the token on the
    /// wire more often than it needs to be (spec §4.4).
    ConfigChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Mono 16 kHz PCM from a client microphone.
    Mic(Vec<i16>),
    /// Mono PCM of one spoken sentence, at the rate announced by `Event::SpeechChunk`.
    Speech { seq: u32, samples: Vec<i16> },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("empty frame")]
    Empty,
    #[error("unknown frame kind {0:#04x}")]
    UnknownKind(u8),
    #[error("malformed frame payload")]
    Malformed,
}

const KIND_MIC: u8 = 0x01;
const KIND_SPEECH: u8 = 0x02;

fn samples_to_le(samples: &[i16], out: &mut Vec<u8>) {
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
}

fn samples_from_le(bytes: &[u8]) -> Result<Vec<i16>, FrameError> {
    if bytes.len() % 2 != 0 {
        return Err(FrameError::Malformed);
    }
    Ok(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Frame::Mic(samples) => {
                let mut out = Vec::with_capacity(1 + samples.len() * 2);
                out.push(KIND_MIC);
                samples_to_le(samples, &mut out);
                out
            }
            Frame::Speech { seq, samples } => {
                let mut out = Vec::with_capacity(5 + samples.len() * 2);
                out.push(KIND_SPEECH);
                out.extend_from_slice(&seq.to_le_bytes());
                samples_to_le(samples, &mut out);
                out
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Frame, FrameError> {
        let (&kind, rest) = bytes.split_first().ok_or(FrameError::Empty)?;
        match kind {
            KIND_MIC => Ok(Frame::Mic(samples_from_le(rest)?)),
            KIND_SPEECH => {
                if rest.len() < 4 {
                    return Err(FrameError::Malformed);
                }
                let seq = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
                Ok(Frame::Speech { seq, samples: samples_from_le(&rest[4..])? })
            }
            other => Err(FrameError::UnknownKind(other)),
        }
    }
}

/// Anything a client sends to the server.
#[derive(Debug, Clone, PartialEq)]
pub enum ToServer {
    Command(Command),
    Audio(Frame),
}

/// Anything the server sends to a client.
#[derive(Debug, Clone, PartialEq)]
pub enum ToClient {
    Event(Event),
    Audio(Frame),
}
