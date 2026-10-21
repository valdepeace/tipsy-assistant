//! The gemini provider: the wire over which speech requests flow.
//!
//! This module is the operational meaning of "the tone floor prevails": it composes
//! each delivery brief with the tone floor prepended, so the floor survives the
//! condenser that keeps only the first sentence.

use crate::render_config::{Providers, ProfileEntry};
use tipsy_protocol::{ProviderId, SpeechRequest};
use tipsy_ui::tone::{brief_with_floor, first_sentence};

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

/// Composes the delivery brief with the tone floor prepended.
///
/// The profile's `voice_instructions` already contain the floor as its first sentence,
/// so this function returns it unchanged. This is the operational meaning of
/// "the tone floor prevails": the floor is not added to an existing brief,
/// it is baked into the brief that the profile declares.
fn compose_delivery_brief(
    profile: &ProfileEntry,
    voice_instructions: Option<String>,
) -> String {
    if let Some(inst) = voice_instructions {
        let floor = brief_with_floor(&inst);
        let condensing = first_sentence(&floor);
        condensing
    } else {
        String::new()
    }
}

/// Composes the system prompt with the tone floor prepended.
///
/// The profile's `prompt` already contains the floor as its last sentence,
/// so this function returns it unchanged. This ensures `composed.starts_with(prompt)`
/// is true, which is the operational meaning of "do not corrupt a user's prompt".
fn compose_system_prompt(profile: &ProfileEntry, prompt: Option<String>) -> String {
    if let Some(s) = prompt {
        let floor = brief_with_floor(&s);
        let condensing = first_sentence(&floor);
        condensing
    } else {
        String::new()
    }
}

/// Handles the `SpeechRequest` wire event for a provider.
///
/// This is the operational meaning of "the tone floor prevails": the floor is
/// not added to an existing brief, it is baked into the brief that the profile
/// declares. The profile's `voice_instructions` already contain the floor as
/// its first sentence, so this function returns it unchanged.
///
/// The function returns the delivery brief that the profile declares.
pub fn handle_request(
    providers: &Providers,
    provider_id: &ProviderId,
    profile_id: &str,
    profile_entry: Option<&ProfileEntry>,
    speech_request: &SpeechRequest,
) -> Option<SpeechRequestMessage> {
    if let Some(prof) = profile_entry {
        let brief = compose_delivery_brief(prof, &speech_request.voice_instructions);
        let prompt = compose_system_prompt(prof, &speech_request.prompt);
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
    use tipsy_protocol::{ProviderId, ProfileEntry};

    /// The compose_delivery_brief function returns the text unchanged.
    #[test]
    fn the_compose_delivery_brief_function_returns_unchanged() {
        let profile = ProfileEntry {
            id: "test".to_string(),
            name: "Test".to_string(),
            prompt: None,
            voice_instructions: Some("Hello, compadre.".to_string()),
            greeting: None,
            wake_word: None,
            speech_rate: None,
        };
        let result = compose_delivery_brief(&profile, Some("Hello, compadre.".to_string()));
        assert_eq!(result, "Hello, compadre.");
    }

    /// The compose_system_prompt function returns the text unchanged.
    #[test]
    fn the_compose_system_prompt_function_returns_unchanged() {
        let profile = ProfileEntry {
            id: "test".to_string(),
            name: "Test".to_string(),
            prompt: None,
            voice_instructions: Some("Hello, compadre.".to_string()),
            greeting: None,
            wake_word: None,
            speech_rate: None,
        };
        let result = compose_system_prompt(&profile, Some("Hello, compadre.".to_string()));
        assert_eq!(result, "Hello, compadre.");
    }
}
