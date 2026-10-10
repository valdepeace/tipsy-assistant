//! One conversation turn: transcribe → stream the LLM → split → synthesise in parallel → deliver in order.

use std::time::Duration;

use tipsy_protocol::ToClient;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::audio::pcm::Pcm;
use crate::providers::{ChatMessage, Providers};

pub struct BrainConfig {
    pub chat_timeout: Duration,
    pub speech_timeout: Duration,
    pub transcribe_timeout: Duration,
    pub retry_backoff: Duration,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            chat_timeout: Duration::from_secs(30),
            speech_timeout: Duration::from_secs(30),
            transcribe_timeout: Duration::from_secs(60),
            retry_backoff: Duration::from_millis(800),
        }
    }
}

pub struct TurnPersona {
    pub prompt: String,
    pub voice_instructions: String,
    pub voice: Option<String>,
    pub language: String,
}

pub enum TurnInput {
    Ask(String),
    Utterance(Pcm),
    Say(String),
}

pub struct Brain {
    #[allow(dead_code)]
    providers: Providers,
    #[allow(dead_code)]
    persona: TurnPersona,
    #[allow(dead_code)]
    config: BrainConfig,
}

impl Brain {
    pub fn new(providers: Providers, persona: TurnPersona, config: BrainConfig) -> Self {
        Self { providers, persona, config }
    }

    pub async fn run_turn(
        &self,
        _history: &mut Vec<ChatMessage>,
        _input: TurnInput,
        _out: UnboundedSender<ToClient>,
        _cancel: CancellationToken,
    ) {
    }
}
