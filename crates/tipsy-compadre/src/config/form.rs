//! The two conversions between the configuration and the screen's form.
//!
//! `tipsy-ui` may not depend on this crate, so the screen edits plain values (`ConfigForm`) and the
//! native client owns both directions. The round trip is tested here, which is what keeps the two
//! shapes from drifting.

use std::collections::BTreeMap;

use tipsy_protocol::Capability;
use tipsy_ui::{ConfigForm, ConfigTab, ListeningForm, ProfileForm, ProviderForm, ServerForm};

use super::model::{Config, ListeningConfig, Profile, ProviderConfig, ProviderKind, ProvidersConfig, ServerConfig};

/// The form the screen opens on: a plain-value copy of `config`, and nothing else.
pub fn form_of(config: &Config) -> ConfigForm {
    ConfigForm {
        tab: ConfigTab::Providers,
        field: 0,
        editing: None,
        active_profile: config.active_profile.clone(),
        // Always three, in `Capability` order: the rows the screen draws and the click areas that
        // select them both come from this list.
        providers: vec![
            provider_form(Capability::Chat, &config.providers.chat),
            provider_form(Capability::Speech, &config.providers.speech),
            provider_form(Capability::Transcribe, &config.providers.transcribe),
        ],
        listening: ListeningForm {
            mode: config.listening.mode,
            silence_ms: config.listening.silence_ms.to_string(),
            mic_gain: config.listening.mic_gain.to_string(),
            mic_device: config.listening.mic_device.clone(),
        },
        profiles: config.profiles.iter().map(profile_form).collect(),
        server: ServerForm {
            bind: config.server.bind.clone(),
            lan: config.server.lan,
            token: config.server.token.clone(),
        },
        notice: None,
        dirty: false,
    }
}

/// The configuration the form describes, or the reason it cannot be one.
///
/// Every message names the row it came from: this is what the screen's notice line shows when a
/// save is refused, and nobody can fix "invalid digit found in string" without knowing the row.
pub fn config_from(form: &ConfigForm) -> Result<Config, String> {
    let mut providers = ProvidersConfig::default();
    for row in &form.providers {
        let provider = provider_config(row)?;
        match row.capability {
            Capability::Chat => providers.chat = provider,
            Capability::Speech => providers.speech = provider,
            Capability::Transcribe => providers.transcribe = provider,
        }
    }
    let profiles = form.profiles.iter().map(profile_from).collect();
    Ok(Config {
        active_profile: form.active_profile.clone(),
        server: ServerConfig {
            bind: form.server.bind.clone(),
            lan: form.server.lan,
            token: form.server.token.clone(),
        },
        listening: ListeningConfig {
            mode: form.listening.mode,
            silence_ms: number("silence_ms", &form.listening.silence_ms)?,
            mic_gain: gain("mic_gain", &form.listening.mic_gain)?,
            mic_device: form.listening.mic_device.clone(),
        },
        providers,
        profiles,
    })
}

/// One row of the Providers tab as a connector configuration — the same conversion a save makes, so
/// a Test (Task 7) can never read a row differently from the save that follows it.
pub fn provider_config(row: &ProviderForm) -> Result<ProviderConfig, String> {
    Ok(ProviderConfig {
        kind: kind_from(&row.kind)?,
        base_url: row.base_url.clone(),
        model: row.model.clone(),
        key: row.key_name.clone(),
    })
}

fn provider_form(capability: Capability, config: &ProviderConfig) -> ProviderForm {
    ProviderForm {
        capability,
        kind: kind_name(config.kind).to_string(),
        base_url: config.base_url.clone(),
        model: config.model.clone(),
        key_name: config.key.clone(),
    }
}

fn profile_form(profile: &Profile) -> ProfileForm {
    ProfileForm {
        id: profile.id.clone(),
        name: profile.name.clone(),
        language: profile.language.clone(),
        prompt: profile.prompt.clone(),
        voice_instructions: profile.voice_instructions.clone(),
        greeting: profile.greeting.clone(),
        wake_word: profile.wake_word.clone(),
        // A `BTreeMap` iterates in key order, so the rows of the voices table come out sorted and go
        // back into a `BTreeMap` unchanged.
        voices: profile.voices.iter().map(|(connector, voice)| (connector.clone(), voice.clone())).collect(),
    }
}

fn profile_from(form: &ProfileForm) -> Profile {
    Profile {
        id: form.id.clone(),
        name: form.name.clone(),
        language: form.language.clone(),
        prompt: form.prompt.clone(),
        voice_instructions: form.voice_instructions.clone(),
        greeting: form.greeting.clone(),
        wake_word: form.wake_word.clone(),
        voices: form.voices.iter().cloned().collect::<BTreeMap<String, String>>(),
    }
}

/// The connector's name as the file and the form spell it (serde's own `snake_case`).
fn kind_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenaiCompat => "openai_compat",
        ProviderKind::Gemini => "gemini",
        ProviderKind::Elevenlabs => "elevenlabs",
    }
}

fn kind_from(name: &str) -> Result<ProviderKind, String> {
    match name {
        "openai_compat" => Ok(ProviderKind::OpenaiCompat),
        "gemini" => Ok(ProviderKind::Gemini),
        "elevenlabs" => Ok(ProviderKind::Elevenlabs),
        other => Err(format!("unknown connector `{other}`: openai_compat, gemini or elevenlabs")),
    }
}

fn number(row: &str, text: &str) -> Result<u32, String> {
    text.trim().parse().map_err(|_| format!("{row}: `{text}` is not a whole number"))
}

fn gain(row: &str, text: &str) -> Result<f32, String> {
    text.trim().parse().map_err(|_| format!("{row}: `{text}` is not a number"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::{ListeningConfig, ProviderConfig, ProvidersConfig, ServerConfig};
    use tipsy_protocol::ListeningMode;

    /// A configuration with every kind of value the screen can change, so the round trip has
    /// something to lose: three connectors of three kinds, both `Option` shapes, a token, a user
    /// profile that overrides nothing.
    fn config() -> Config {
        // Built as one `Config` literal rather than by mutating `Config::default()` so that clippy's
        // `field_reassign_with_default` stays quiet about a test helper.
        let mut profiles = Config::default().profiles;
        profiles.insert(
            0,
            Profile {
                id: "mine".into(),
                name: "Mi Compadre".into(),
                language: "es-ES".into(),
                prompt: "Habla como un compae.".into(),
                voice_instructions: "andaluz, tranquilo".into(),
                greeting: "¿Qué pasa, quillo?".into(),
                wake_word: "oye compae".into(),
                voices: BTreeMap::from([
                    ("gemini".to_string(), "Charon".to_string()),
                    ("elevenlabs".to_string(), "george".to_string()),
                ]),
            },
        );
        Config {
            active_profile: "mine".into(),
            server: ServerConfig {
                bind: "0.0.0.0:7070".into(),
                lan: true,
                token: "0123456789abcdef0123456789abcdef".into(),
            },
            listening: ListeningConfig {
                mode: ListeningMode::HandsFree,
                silence_ms: 1500,
                mic_gain: 9.5,
                mic_device: Some("Rode NT-USB".into()),
            },
            providers: ProvidersConfig {
                chat: ProviderConfig {
                    kind: ProviderKind::Gemini,
                    base_url: "https://generativelanguage.test/v1beta".into(),
                    model: "gemini-2.5-flash".into(),
                    key: "google".into(),
                },
                speech: ProviderConfig {
                    kind: ProviderKind::Elevenlabs,
                    base_url: "https://api.elevenlabs.test".into(),
                    model: "eleven_flash_v2_5".into(),
                    key: "elevenlabs".into(),
                },
                transcribe: ProviderConfig {
                    kind: ProviderKind::OpenaiCompat,
                    base_url: "http://localhost:11434/v1".into(),
                    model: "openai/whisper-large-v3".into(),
                    key: String::new(),
                },
            },
            profiles,
        }
    }

    /// The contract that keeps the two shapes from drifting: what the screen opens on, saved
    /// unchanged, is the configuration that produced it — value for value, order for order.
    #[test]
    fn the_form_and_the_configuration_round_trip() {
        for config in [Config::default(), config()] {
            let form = form_of(&config);
            assert_eq!(config_from(&form).unwrap(), config, "a save must write the same file back");
        }
    }

    /// The three capabilities are always there, in the protocol's own order, spelled the way the
    /// file spells them: the rows the screen draws and the clicks that select them both come from
    /// this list.
    #[test]
    fn the_form_holds_the_three_capabilities_in_order() {
        let form = form_of(&Config::default());
        let capabilities: Vec<Capability> = form.providers.iter().map(|p| p.capability).collect();
        assert_eq!(capabilities, [Capability::Chat, Capability::Speech, Capability::Transcribe]);

        assert_eq!(form.providers[0].kind, "openai_compat");
        assert_eq!(form.providers[0].base_url, "https://openrouter.ai/api/v1");
        assert_eq!(form.providers[0].model, "google/gemini-2.5-flash");
        assert_eq!(form.providers[0].key_name, "openrouter");
        assert_eq!(form.listening.silence_ms, "1000");
        assert_eq!(form.listening.mic_gain, "12");
        assert_eq!(form.server.bind, "127.0.0.1:7070");
        assert!(form.server.token.is_empty(), "no token until the Server tab generates one");
        assert!(!form.dirty, "a form nobody has touched holds nothing unsaved");
        assert!(form.notice.is_none());
        assert_eq!(form.profiles.len(), Config::default().profiles.len());
    }

    /// `mic_device` is the form's one `Option`: absent means the system default and an empty string
    /// means a device with no name, which is a different and useless value.
    #[test]
    fn an_absent_mic_device_stays_absent_through_the_round_trip() {
        let mut config = config();
        config.listening.mic_device = None;
        let form = form_of(&config);
        assert_eq!(form.listening.mic_device, None);
        assert_eq!(config_from(&form).unwrap().listening.mic_device, None);

        config.listening.mic_device = Some(String::new());
        let form = form_of(&config);
        assert_eq!(form.listening.mic_device.as_deref(), Some(""));
        assert_eq!(config_from(&form).unwrap().listening.mic_device, Some(String::new()));
    }

    /// A save that cannot be built is refused *by the name of its row*: "invalid digit found in
    /// string" tells nobody what to fix, and this message is what the notice line shows.
    #[test]
    fn a_value_that_does_not_parse_is_refused_by_the_name_of_its_row() {
        let mut form = form_of(&Config::default());

        form.listening.silence_ms = "mil".into();
        let error = config_from(&form).unwrap_err();
        assert!(error.contains("silence_ms") && error.contains("mil"), "{error}");

        form.listening.silence_ms = "1000".into();
        form.listening.mic_gain = "doce".into();
        let error = config_from(&form).unwrap_err();
        assert!(error.contains("mic_gain") && error.contains("doce"), "{error}");

        form.listening.mic_gain = "12".into();
        form.providers[1].kind = "openia_compat".into();
        let error = config_from(&form).unwrap_err();
        assert!(error.contains("openia_compat"), "{error}");
    }

    /// The connector names the form holds are serde's own: the file, the form and the wire must not
    /// grow a second vocabulary.
    #[test]
    fn connector_names_are_the_serialised_ones() {
        for kind in [ProviderKind::OpenaiCompat, ProviderKind::Gemini, ProviderKind::Elevenlabs] {
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(kind_name(kind)));
            assert_eq!(kind_from(kind_name(kind)).unwrap(), kind);
        }
    }
}
