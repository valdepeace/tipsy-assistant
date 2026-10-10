use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tipsy_protocol::{ListeningMode, PersonaView};

use super::personas::builtin_personas;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub active_persona: String,
    pub server: ServerConfig,
    pub listening: ListeningConfig,
    pub providers: ProvidersConfig,
    pub personas: Vec<Persona>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            active_persona: "compae".into(),
            server: ServerConfig::default(),
            listening: ListeningConfig::default(),
            providers: ProvidersConfig::default(),
            personas: builtin_personas(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub lan: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { bind: "127.0.0.1:7070".into(), lan: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListeningConfig {
    pub mode: ListeningMode,
    pub silence_ms: u32,
    pub mic_gain: f32,
}

impl Default for ListeningConfig {
    fn default() -> Self {
        Self { mode: ListeningMode::Push, silence_ms: 1000, mic_gain: 12.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    OpenaiCompat,
    Gemini,
    Elevenlabs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    /// Name of the key, never the key itself.
    pub key: String,
}

impl ProviderConfig {
    fn openrouter(model: &str) -> Self {
        Self {
            kind: ProviderKind::OpenaiCompat,
            base_url: "https://openrouter.ai/api/v1".into(),
            model: model.into(),
            key: "openrouter".into(),
        }
    }

    /// Which entry of `Persona::voices` applies to this speech provider.
    pub fn voice_key(&self) -> &'static str {
        let model = self.model.to_ascii_lowercase();
        if self.kind == ProviderKind::Gemini || model.contains("gemini") {
            "gemini"
        } else if self.kind == ProviderKind::Elevenlabs || model.contains("eleven") {
            "elevenlabs"
        } else {
            "default"
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self::openrouter("")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub chat: ProviderConfig,
    pub speech: ProviderConfig,
    pub transcribe: ProviderConfig,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            chat: ProviderConfig::openrouter("google/gemini-2.5-flash"),
            speech: ProviderConfig::openrouter("google/gemini-3.8-flash-tts"),
            transcribe: ProviderConfig::openrouter("openai/whisper-large-v3"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Persona {
    pub id: String,
    pub name: String,
    pub language: String,
    pub prompt: String,
    pub voice_instructions: String,
    pub greeting: String,
    pub wake_word: String,
    /// Keyed by connector name (`ProviderConfig::voice_key`); the value is a human-written voice
    /// name or id. Empty means "use the provider's own default": a connector resolves a missing
    /// entry itself and never fails for it.
    pub voices: BTreeMap<String, String>,
}

impl Persona {
    pub fn view(&self) -> PersonaView {
        PersonaView { id: self.id.clone(), name: self.name.clone(), language: self.language.clone() }
    }

    /// Voice for the given speech provider, if the persona recommends one.
    pub fn voice_for(&self, provider: &ProviderConfig) -> Option<String> {
        self.voices.get(provider.voice_key()).cloned()
    }
}

impl Config {
    /// The active persona, or the first one if `active_persona` is unknown.
    pub fn active(&self) -> &Persona {
        self.personas
            .iter()
            .find(|p| p.id == self.active_persona)
            .or_else(|| self.personas.first())
            .expect("config always holds at least one persona after with_builtin_personas")
    }

    /// Adds every built-in persona the user has not overridden (same `id`).
    pub fn with_builtin_personas(mut self) -> Self {
        for builtin in builtin_personas() {
            if !self.personas.iter().any(|p| p.id == builtin.id) {
                self.personas.push(builtin);
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nine_builtins_are_present_in_the_order_of_the_spec() {
        let ids: Vec<String> = crate::config::personas::builtin_personas().iter().map(|p| p.id.clone()).collect();
        assert_eq!(
            ids,
            [
                "compae", "meigo", "chulapo", "senyor", "good_ol_boy", "hustler", "mate", "bloke", "polite_one"
            ]
        );
    }

    #[test]
    fn wake_words_are_distinct_because_they_are_spoken_aloud() {
        let mut words: Vec<String> =
            crate::config::personas::builtin_personas().iter().map(|p| p.wake_word.clone()).collect();
        let before = words.len();
        words.sort();
        words.dedup();
        assert_eq!(words.len(), before, "two personas share a wake word");
    }

    /// A persona with no entry for the connector's `voice_key` resolves to `None`, so the connector
    /// uses its own default voice. The model is set too: `voice_key` tests the model string before
    /// the kind, so an `Elevenlabs` kind carrying the default OpenRouter speech model
    /// (`google/gemini-3.8-flash-tts`) would resolve the `gemini` key and leave this guard blind —
    /// the resolved key is asserted so that can never be mistaken again. Precedence itself is Plan
    /// 1 behaviour and is not changed here (Plan 2B owns it).
    #[test]
    fn a_persona_without_a_voice_for_the_connector_falls_back_to_the_provider_default() {
        let config = Config::default();
        let mut eleven = config.providers.speech.clone();
        eleven.kind = ProviderKind::Elevenlabs;
        eleven.model = "eleven_flash_v2_5".into();
        assert_eq!(eleven.voice_key(), "elevenlabs", "this test must exercise the elevenlabs key");
        let meigo = crate::config::personas::builtin_personas().into_iter().find(|p| p.id == "meigo").unwrap();
        assert_eq!(meigo.voice_for(&eleven), None, "no entry means the connector's own default voice");
    }
}
