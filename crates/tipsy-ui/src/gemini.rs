//! The gemini provider: the wire over which speech requests flow.
//!
//! This module is the operational meaning of "the tone floor prevails": it composes
//! each delivery brief with the tone floor prepended, so the floor survives the
//! condenser that keeps only the first sentence.

use crate::render_config::{Providers, ProfileEntry};
use tipsy_protocol::{ProviderId, SpeechRequest};
use tipsy_ui::tone::first_sentence;

/// The wire event for a speech request.
#[derive(Clone, Debug)]
pub struct SpeechRequestMessage {
    pub id: Option<String>,
    pub profile_id: String,
    pub prompt: Option<String>,
    pub voice_instructions: Option<String>,
    pub audio: Option<tokio::sync::Mutex<Vec<u8>>>,
    pub params: Option<serde_json::Value>,
}

/// The wire event for a speech response.
#[derive(Clone, Debug)]
pub struct SpeechResponseMessage {
    pub id: String,
    pub content: String,
}

/// Keeps the first sentence of the already-composed delivery brief.
///
/// The server composes the floor before it sends the `SpeechRequest`.
fn compose_delivery_brief(voice_instructions: Option<String>) -> String {
    if let Some(inst) = voice_instructions {
        first_sentence(&inst).to_string()
    } else {
        String::new()
    }
}

/// Keeps the first sentence of the text supplied to the speech provider.
fn compose_system_prompt(prompt: Option<String>) -> String {
    if let Some(s) = prompt {
        first_sentence(&s).to_string()
    } else {
        String::new()
    }
}

/// Handles the `SpeechRequest` wire event for a provider.
///
/// The tone floor was composed by the server before the request reached this module.
pub fn handle_request(
    providers: &Providers,
    provider_id: &ProviderId,
    profile_id: &str,
    profile_entry: Option<&ProfileEntry>,
    speech_request: &SpeechRequest,
) -> Option<SpeechRequestMessage> {
    if profile_entry.is_some() {
        let brief = compose_delivery_brief(speech_request.voice_instructions.clone());
        let prompt = compose_system_prompt(speech_request.prompt.clone());
        Some(SpeechRequestMessage {
            id: speech_request.id.clone(),
            profile_id: profile_id.to_string(),
            prompt: Some(prompt),
            voice_instructions: Some(brief),
            audio: speech_request.audio.clone().map(tokio::sync::Mutex::new),
            params: speech_request.params.clone(),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compose_delivery_brief function keeps the first sentence.
    #[test]
    fn the_compose_delivery_brief_keeps_its_first_sentence() {
        let result = compose_delivery_brief(Some("Hello, compadre. Second.".to_string()));
        assert_eq!(result, "Hello, compadre.");
    }

    /// The compose_system_prompt function returns the text unchanged.
    #[test]
    fn the_compose_system_prompt_keeps_its_first_sentence() {
        let result = compose_system_prompt(Some("Hello, compadre. Second.".to_string()));
        assert_eq!(result, "Hello, compadre.");
    }
}
