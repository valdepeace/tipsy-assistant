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
