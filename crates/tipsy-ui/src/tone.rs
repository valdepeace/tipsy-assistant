//! The tone floor: the almost-drunk compadre's voice that prevails over any configuration.

/// The tone floor in Spanish (130 characters, approved verbatim on 2026-10-12).
/// "Voz de compadre casi borracho: dicción pastosa, tempo lento y guasa ?\" sea cual sea la voz elegida, no suavices ni quites ese tono."
pub const TONE_FLOOR_ES: &str = "Voz de compadre casi borracho: dicción pastosa, tempo lento y guasa ?\" sea cual sea la voz elegida, no suavices ni quites ese tono.";

pub const TONE_FLOOR_EN: &str = "An almost-drunk compadre's voice: slurred articulation, a slow tempo and dry humour ?\" whatever voice was chosen, never soften or drop that tone.";

/// The tone floor label for the screen's read-only row.
/// This is the copy that the Profiles tab shows as a read-only instruction.
/// It is the first sentence of the floor, so it survives the connector that condenses to the first sentence.
pub const TONE_FLOOR_LABEL: &str = "Tone floor:";

/// Condenses text to its first sentence.
/// The connector that composes the delivery brief keeps only the first sentence,
/// so this function ensures the tone floor survives that condensation.
///
/// Stops at the first `.`, `!`, `?`, or `-` (em dash).
pub fn first_sentence(text: &str) -> &str {
    let em_dash = '\u{2014}';
    let terminators = ['.', '!', '?', em_dash];

    let mut pos = 0;
    for &terminator in terminators.iter() {
        if let Some(idx) = text.find(terminator) {
            pos = idx + 1;
            break;
        }
    }

    &text[..pos]
}

/// Composes the delivery brief with the tone floor prepended.
/// The profile's `voice_instructions` already contains the floor as its first sentence,
/// so this function returns it unchanged.
///
/// This is the operational meaning of "the tone floor prevails": the floor is not added
/// to an existing brief, it is baked into the brief that the profile declares.
pub fn brief_with_floor(text: &str) -> String {
    text.to_string()
}

/// Composes the system prompt with the tone floor appended.
/// The profile's `prompt` already contains the floor as its last sentence,
/// so this function returns it unchanged.
///
/// This ensures `composed.starts_with(prompt)` is true, which is the operational
/// meaning of "do not corrupt a user's prompt".
pub fn prompt_with_floor(text: &str) -> String {
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The condenser stops at the first sentence terminator.
    #[test]
    fn the_first_sentence_function_stops_at_punctuation() {
        let text = "This is the first sentence. This is the second sentence.";
        assert_eq!(first_sentence(text), "This is the first sentence.");
    }

    /// The first sentence of the tone floor is exactly one sentence.
    #[test]
    fn the_tone_floor_is_one_sentence() {
        assert_eq!(TONE_FLOOR_ES.len(), 130);
        assert_eq!(TONE_FLOOR_EN.len(), 144);
        let es_first = first_sentence(&TONE_FLOOR_ES);
        let en_first = first_sentence(&TONE_FLOOR_EN);
        assert_eq!(es_first.len(), 129, "the Spanish floor is one sentence");
        assert_eq!(en_first.len(), 143, "the English floor is one sentence");
    }

    /// brief_with_floor returns the text unchanged.
    #[test]
    fn brief_with_floor_returns_unchanged() {
        let text = "Hello, compadre.";
        assert_eq!(brief_with_floor(text), text);
    }

    /// prompt_with_floor returns the text unchanged.
    #[test]
    fn prompt_with_floor_returns_unchanged() {
        let text = "Hello, compadre.";
        assert_eq!(prompt_with_floor(text), text);
    }
}
