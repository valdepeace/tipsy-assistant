//! Messages exchanged between the tipsy-compadre server and its clients.
//!
//! Text messages are JSON (`Command`, `Event`); audio travels as binary `Frame`s.

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
