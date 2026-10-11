use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tipsy_protocol::{ListeningMode, ProfileView};

use super::profiles::builtin_profiles;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The profile the assistant speaks as. `active_persona` was this field's name until the rename,
    /// so the old key is a serde alias: a `config.toml` written before it still selects the same
    /// profile, and saving always writes `active_profile`.
    #[serde(alias = "active_persona")]
    pub active_profile: String,
    pub server: ServerConfig,
    pub listening: ListeningConfig,
    pub providers: ProvidersConfig,
    /// The realtime voice mode (spec §6, plan 2C). A section of its own because a call is a *mode of
    /// the client*: `Capability` stays chat/speech/transcribe and `build_providers` never sees it.
    pub realtime: RealtimeConfig,
    /// The user's own profiles first, the built-ins `with_builtin_profiles` appends after them.
    /// `personas` was this field's name until the rename, so the old array key is a serde alias too:
    /// a file written with `[[personas]]` still loads, and is only ever written back as `[[profiles]]`.
    #[serde(alias = "personas")]
    pub profiles: Vec<Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            active_profile: "compae".into(),
            server: ServerConfig::default(),
            listening: ListeningConfig::default(),
            providers: ProvidersConfig::default(),
            realtime: RealtimeConfig::default(),
            profiles: builtin_profiles(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub lan: bool,
    /// The token the WebSocket server will require (spec §9; Plan 3 puts it to use). Empty means
    /// "none generated yet": the screen shows the unset state and its Regenerate button fills it
    /// in. It is *our* bearer token, not a provider key, so the file is its home — and it still
    /// never belongs in an event, a notice or a log line.
    pub token: String,
}

impl ServerConfig {
    /// False while no token has been generated: a fresh install, or a file written before this
    /// plan. A whitespace-only value (a hand-edited file) counts as no token.
    pub fn has_token(&self) -> bool {
        !self.token.trim().is_empty()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { bind: "127.0.0.1:7070".into(), lan: false, token: String::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListeningConfig {
    pub mode: ListeningMode,
    pub silence_ms: u32,
    pub mic_gain: f32,
    /// Input device for the native client (spec §9). `None` — the default, and the only value the
    /// web client can have — means the system default device; the serializer leaves the key out
    /// rather than writing an empty name.
    pub mic_device: Option<String>,
}

impl Default for ListeningConfig {
    fn default() -> Self {
        Self { mode: ListeningMode::Push, silence_ms: 1000, mic_gain: 12.0, mic_device: None }
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

    /// Which entry of `Profile::voices` applies to this speech provider.
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

/// `[realtime]`: one duplex WebSocket session with OpenAI's Realtime API.
///
/// There is deliberately no `silence_ms` here. The plan's Decision is that server-side turn
/// detection is configured with `[listening] silence_ms`, so two numbers never describe one
/// behaviour; the socket reads that field through `CallSpec`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RealtimeConfig {
    /// Off unless the user turns it on: a streaming socket bills audio while it is open, so the
    /// mode is never silently on (plan 2C, Review Focus 3).
    pub enabled: bool,
    /// A session-acceptable Realtime model id (the note's table). `gpt-realtime-2.1` is its current
    /// default choice; `gpt-realtime-2.1-mini` is the cheaper one.
    pub model: String,
    /// Name of the key, never the key itself — resolved by `KeyResolver` like every provider key.
    pub key: String,
    /// One of the note's session voices (`alloy, ash, ballad, coral, echo, sage, shimmer, verse,
    /// marin, cedar`). Deliberately *not* the profile's `voices` table: that table maps speech
    /// connectors, and realtime is not one of them (plan 2C, "Decisions this plan makes").
    pub voice: String,
    /// A call ends by itself after this many minutes (plan 2C's cost Decision). The API's own cap is
    /// 60 minutes; this is the number the user chose.
    pub max_minutes: u32,
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: "gpt-realtime-2.1".into(),
            key: "openai".into(),
            voice: "marin".into(),
            max_minutes: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
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

impl Profile {
    pub fn view(&self) -> ProfileView {
        ProfileView { id: self.id.clone(), name: self.name.clone(), language: self.language.clone() }
    }

    /// Voice for the given speech provider, if the profile recommends one.
    pub fn voice_for(&self, provider: &ProviderConfig) -> Option<String> {
        self.voices.get(provider.voice_key()).cloned()
    }
}

impl Config {
    /// The active profile, or the first one if `active_profile` is unknown.
    pub fn active(&self) -> &Profile {
        self.profiles
            .iter()
            .find(|p| p.id == self.active_profile)
            .or_else(|| self.profiles.first())
            .expect("config always holds at least one profile after with_builtin_profiles")
    }

    /// Adds every built-in profile the user has not overridden (same `id`).
    pub fn with_builtin_profiles(mut self) -> Self {
        for builtin in builtin_profiles() {
            if !self.profiles.iter().any(|p| p.id == builtin.id) {
                self.profiles.push(builtin);
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
        let ids: Vec<String> = crate::config::profiles::builtin_profiles().iter().map(|p| p.id.clone()).collect();
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
            crate::config::profiles::builtin_profiles().iter().map(|p| p.wake_word.clone()).collect();
        let before = words.len();
        words.sort();
        words.dedup();
        assert_eq!(words.len(), before, "two profiles share a wake word");
    }

    /// A profile with no entry for the connector's `voice_key` resolves to `None`, so the connector
    /// uses its own default voice. The model is set too: `voice_key` tests the model string before
    /// the kind, so an `Elevenlabs` kind carrying the default OpenRouter speech model
    /// (`google/gemini-3.8-flash-tts`) would resolve the `gemini` key and leave this guard blind —
    /// the resolved key is asserted so that can never be mistaken again. Precedence itself is Plan
    /// 1 behaviour and is not changed here (Plan 2B owns it).
    #[test]
    fn a_profile_without_a_voice_for_the_connector_falls_back_to_the_provider_default() {
        let config = Config::default();
        let mut eleven = config.providers.speech.clone();
        eleven.kind = ProviderKind::Elevenlabs;
        eleven.model = "eleven_flash_v2_5".into();
        assert_eq!(eleven.voice_key(), "elevenlabs", "this test must exercise the elevenlabs key");
        let meigo = crate::config::profiles::builtin_profiles().into_iter().find(|p| p.id == "meigo").unwrap();
        assert_eq!(meigo.voice_for(&eleven), None, "no entry means the connector's own default voice");
    }

    /// The section's defaults, quoted from the plan and the research note: off, the note's current
    /// model, one key *name*, one of the two voices the note recommends, and the plan's own limit.
    #[test]
    fn realtime_defaults_are_off_and_named() {
        let realtime = RealtimeConfig::default();
        assert!(!realtime.enabled, "the mode must never be silently on");
        assert_eq!(realtime.model, "gpt-realtime-2.1", "the note's current default choice");
        assert_eq!(realtime.key, "openai", "the name `KeyResolver` looks up in TIPSY_KEY_OPENAI");
        assert_eq!(realtime.voice, "marin", "one of the two voices the note recommends");
        assert_eq!(realtime.max_minutes, 10, "the plan's default call limit");

        // `[realtime]` has no `silence_ms`: turn detection reuses `[listening] silence_ms` (the
        // plan's Decisions section), which is why the two are asserted together here and why
        // `RealtimeConfig` has no such field to drift from it.
        let config = Config::default();
        assert_eq!(config.listening.silence_ms, 1000);
        assert_eq!(config.realtime, realtime, "a default config carries the default section");
    }
}
