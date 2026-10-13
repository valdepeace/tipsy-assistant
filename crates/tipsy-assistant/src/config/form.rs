//! The two conversions between the configuration and the screen's form.
//!
//! `tipsy-ui` may not depend on this crate, so the screen edits plain values (`ConfigForm`) and the
//! native client owns both directions. The round trip is tested here, which is what keeps the two
//! shapes from drifting.

use std::collections::BTreeMap;

use tipsy_protocol::{Capability, ConfigView};
use tipsy_ui::{
    AllowedPersonForm, ConfigForm, ConfigTab, ListeningForm, LoginForm, ProfileForm, ProviderForm, RealtimeForm,
    ServerForm,
};

use super::model::{
    Config, ListeningConfig, LoginConfig, LoginEntry, Profile, ProviderConfig, ProviderKind, ProvidersConfig,
    RealtimeConfig, ServerConfig,
};

/// The form the screen opens on: a plain-value copy of `config`, and nothing else.
pub fn form_of(config: &Config) -> ConfigForm {
    // Built field by field rather than as a struct literal: `ConfigForm` keeps a private `filled`
    // flag, because a form the client has not answered for yet must not be saveable, and only
    // `tipsy-ui` may use literal syntax. `mark_filled` is what says this one carries the real
    // configuration.
    let mut form = ConfigForm::default();
    form.tab = ConfigTab::Providers;
    form.field = 0;
    form.editing = None;
    form.active_profile = config.active_profile.clone();
    // Always three, in `Capability` order: the rows the screen draws and the click areas that
    // select them both come from this list.
    form.providers = vec![
        provider_form(Capability::Chat, &config.providers.chat),
        provider_form(Capability::Speech, &config.providers.speech),
        provider_form(Capability::Transcribe, &config.providers.transcribe),
    ];
    form.listening = ListeningForm {
        mode: config.listening.mode,
        silence_ms: config.listening.silence_ms.to_string(),
        mic_gain: config.listening.mic_gain.to_string(),
        mic_device: config.listening.mic_device.clone(),
    };
    form.profiles = config.profiles.iter().map(profile_form).collect();
    // Carried, not drawn (Plan 2C Task 1): no tab of the screen shows `[realtime]`, but a save
    // rebuilds the whole configuration from this form, so the section has to survive the trip or a
    // save would reset it to its defaults.
    form.realtime = RealtimeForm {
        enabled: config.realtime.enabled,
        model: config.realtime.model.clone(),
        key_name: config.realtime.key.clone(),
        voice: config.realtime.voice.clone(),
        max_minutes: config.realtime.max_minutes,
    };
    // Carried, not drawn: the same rule as `[realtime]` above, and for a section that is *only* ever
    // written by hand. A save that dropped it would lock the owner out of their own login screen.
    form.login = LoginForm {
        enabled: config.login.enabled,
        voice: config.login.voice,
        allowed: config.login
            .allowed
            .iter()
            .map(|entry| AllowedPersonForm { name: entry.name.clone(), keywords: entry.keywords.clone() })
            .collect(),
    };
    form.server = ServerForm {
        bind: config.server.bind.clone(),
        lan: config.server.lan,
        token: config.server.token.clone(),
    };
    form.notice = None;
    form.dirty = false;
    form.mark_filled();
    form
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
        // The one section the screen does not draw, put back exactly as the form carries it: the
        // file's `[realtime]` must not change because something else on the screen was saved.
        realtime: RealtimeConfig {
            enabled: form.realtime.enabled,
            model: form.realtime.model.clone(),
            key: form.realtime.key_name.clone(),
            voice: form.realtime.voice.clone(),
            max_minutes: form.realtime.max_minutes,
        },
        // The one section the screen does not draw *and* the one that is only ever written by hand:
        // it goes back exactly as the form carries it, so a save from the screen cannot erase an
        // allowlist the user typed into the file.
        login: LoginConfig {
            enabled: form.login.enabled,
            voice: form.login.voice,
            allowed: form
                .login
                .allowed
                .iter()
                .map(|entry| LoginEntry { name: entry.name.clone(), keywords: entry.keywords.clone() })
                .collect(),
        },
        profiles,
    })
}

/// The wire's shape for the configuration a file holds: `form_of` and then the form's own
/// conversion. One path, so a `GetConfig` answer can never describe a file differently from the
/// screen that edits it.
pub fn view_of(config: &Config) -> ConfigView {
    ConfigView::from(&form_of(config))
}

/// The configuration a `ConfigView` describes, refusing what `config_from` refuses and by the same
/// names, so a configuration that arrives over the wire is saved exactly as strictly as one typed
/// into the screen.
pub fn config_of(view: ConfigView) -> Result<Config, String> {
    config_from(&ConfigForm::from(view))
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
    use crate::config::{ListeningConfig, LoginEntry, ProviderConfig, ProvidersConfig, ServerConfig};
    use tipsy_protocol::{ClientConfigView, ListeningMode};

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
            // Not something the screen can change, but part of the file the form must not lose; the
            // round trip is what proves it, and the test below names the reason.
            realtime: RealtimeConfig {
                enabled: true,
                model: "gpt-realtime-2.1-mini".into(),
                key: "openai-realtime".into(),
                voice: "cedar".into(),
                max_minutes: 25,
            },
            // The same rule again: `[login]` is written by hand only, so the fixture carries a
            // non-default section — two entries, in this order — and the round trip has to bring it
            // back entry for entry.
            login: LoginConfig {
                enabled: true,
                voice: false,
                allowed: vec![
                    LoginEntry { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] },
                    LoginEntry { name: "María".into(), keywords: vec!["maria".into(), "data".into()] },
                ],
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

    /// The screen draws no `[realtime]` row (Plan 2C Task 1: the section is edited in the file), yet
    /// a save rebuilds the whole configuration from this form. The section therefore has to be
    /// carried through it: a `config_from` that filled it in from `RealtimeConfig::default()` would
    /// turn the mode off, and rewrite the model and the voice, the first time the user saved
    /// anything else on the screen — silently, since no row would show it.
    #[test]
    fn a_save_from_the_screen_keeps_the_realtime_section_it_does_not_draw() {
        let mut config = Config::default();
        config.realtime.enabled = true;
        config.realtime.model = "gpt-realtime-2.1-mini".into();
        config.realtime.key = "openai-realtime".into();
        config.realtime.voice = "cedar".into();
        config.realtime.max_minutes = 25;

        let form = form_of(&config);
        assert_eq!(config_from(&form).unwrap().realtime, config.realtime);
    }

    /// The one section the screen does not draw, through the one conversion that would otherwise drop it.
    /// This is the test Plan 2C added for `[realtime]`, and it is here for the same reason: a save from
    /// the configuration screen must not erase a hand-written allowlist.
    #[test]
    fn a_save_from_the_screen_keeps_the_login_section() {
        let mut config = Config::default();
        config.login.enabled = false;
        config.login.voice = false;
        config.login.allowed = vec![LoginEntry { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] }];

        let form = form_of(&config);
        let back = config_from(&form).expect("an untouched form saves");
        assert_eq!(back.login, config.login, "the section survives the trip through the form");
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

    /// §4.3's rule for the view the server **sends**: it carries the key *names*, one per connector,
    /// and no field of a row could hold a value.
    ///
    /// The earlier version of this test asserted that the JSON did not contain three strings that
    /// occur in no input at all — a tautology no implementation of `view_of` could fail (review V2;
    /// replacing them with the names that *are* in the fixture is what turned it red). The property
    /// worth pinning is a property of the shape, so that is what is asserted here: the rows are
    /// exactly the five fields the type has, `key_name` among them, and the names the configuration
    /// holds are the ones that cross. It bites the day a row grows a field a value could ride in.
    #[test]
    fn a_config_view_carries_key_names_and_no_field_a_value_could_ride_in() {
        let config = config();
        let json = serde_json::to_string(&view_of(&config)).unwrap();

        // The positive half: the names the configuration holds are what crosses, and an empty name is
        // a name too — the transcribe fixture has none, and it must not become a value.
        for name in ["google", "elevenlabs", "openai-realtime", ""] {
            assert!(json.contains(&format!(r#""key_name":"{name}""#)), "`{name}` must cross as a name: {json}");
        }

        // The negative half, as a property of the shape: a value could only ride in a field, and every
        // field of every row is named here. `key_name` holds whatever the configuration's `key` holds,
        // so the guard against a value being *in* the configuration is the file's own convention plus
        // `Secrets` in `logging` — not a needle check over this fixture.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        for row in value["providers"].as_array().expect("the rows are a list") {
            assert_eq!(
                sorted_keys(row),
                ["base_url", "capability", "key_name", "kind", "model"],
                "a provider row's fields are the whole shape of what a client may be told: {row}"
            );
        }
    }

    /// §4.3 and §13.6 as a property of the message a client sends: the view inside
    /// `Command::UpdateConfig` has no field that could hold a provider's address, a key *name* or the
    /// server's token, so none of them can cross in a save whatever the screen does. The full view
    /// still carries all three, which is what the server *tells* a client it is entitled to see.
    ///
    /// The negative assertions are over values that really are in the fixture — the token and a
    /// `base_url` — rather than over strings that occur in no input, which is what made the older
    /// neighbour of this test unfalsifiable (review V2).
    #[test]
    fn the_view_a_client_sends_carries_no_provider_address_key_name_or_token() {
        let config = config();
        let full = serde_json::to_string(&view_of(&config)).unwrap();
        assert!(full.contains("0123456789abcdef0123456789abcdef"), "the full view carries the token: {full}");
        assert!(full.contains("generativelanguage.test"), "and the providers' addresses: {full}");

        let client = serde_json::to_string(&ClientConfigView::from(&view_of(&config))).unwrap();
        let value: serde_json::Value = serde_json::from_str(&client).unwrap();

        // The sections, and the fields of the two the rule is about, so a new field cannot be added
        // in silence. Sorted, because `serde_json`'s map is ordered by key and not by this file.
        let sections = sorted_keys(&value);
        assert_eq!(
            sections,
            ["active_profile", "listening", "login", "profiles", "providers", "realtime", "server"]
        );
        assert_eq!(sorted_keys(&value["server"]), ["bind", "lan"]);
        for row in value["providers"].as_array().expect("the rows are a list") {
            assert_eq!(sorted_keys(row), ["capability", "kind", "model"], "{row}");
        }

        // …and the values are not in the message at all, not merely missing from one field.
        assert!(!client.contains("0123456789abcdef0123456789abcdef"), "the token crossed: {client}");
        assert!(!client.contains("generativelanguage.test"), "a provider's address crossed: {client}");
        assert!(!client.contains("google"), "and so did a key name: {client}");
    }

    /// The keys of a JSON object, sorted: `serde_json`'s map is a `BTreeMap` without its
    /// `preserve_order` feature, so a comparison has to be order-independent or it is a test of serde.
    fn sorted_keys(value: &serde_json::Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value.as_object().expect("an object").keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    /// §4.3's second half, and the shape Task 1.3 needs: the token is the one thing the view carries
    /// that some clients may not see, so it lives in exactly one field and there is exactly one way
    /// to take it away.
    #[test]
    fn the_token_lives_in_one_field_and_redacting_empties_it() {
        let json = serde_json::to_string(&view_of(&config())).unwrap();
        assert!(
            json.contains(r#""token":"0123456789abcdef0123456789abcdef""#),
            "the token crosses as itself, in `server.token`: {json}"
        );
        // One occurrence and one only: a second field carrying the token would be the leak §4.3 rules
        // out, and this is the assertion that notices it.
        assert_eq!(json.matches("0123456789abcdef0123456789abcdef").count(), 1, "{json}");

        let redacted = serde_json::to_string(&view_of(&config()).redacted()).unwrap();
        assert!(!redacted.contains("0123456789abcdef0123456789abcdef"), "{redacted}");
        assert!(redacted.contains(r#""bind":"0.0.0.0:7070""#), "only the token goes: {redacted}");
    }

    /// The ruling: one shape, two representations, and the trip through the wire shape is what keeps
    /// them from drifting. What the screen opens on, carried as a `ConfigView` and brought back, is
    /// the configuration that produced it — value for value, order for order.
    #[test]
    fn a_config_view_round_trips_the_whole_configuration() {
        for config in [Config::default(), config()] {
            let form = form_of(&config);
            let view = ConfigView::from(&form);

            let json = serde_json::to_string(&view).unwrap();
            let back: ConfigView = serde_json::from_str(&json).unwrap();
            assert_eq!(back, view, "the wire shape must survive its own JSON: {json}");

            let again = ConfigForm::from(back);
            assert_eq!(config_from(&again).unwrap(), config, "a view must rebuild the same file");
            assert_eq!(config_of(view.clone()).unwrap(), config, "and so must the one-step path");
        }
    }

    /// `ConfigForm`'s own view state belongs to the screen, not to the wire. A form built from a
    /// `ConfigView` is **not** marked filled, because a message is not a file that was read: `Ctrl+S`
    /// on such a form must still refuse (Plan 2B's whole point for the flag).
    #[test]
    fn a_form_built_from_a_view_is_not_filled_and_keeps_the_screens_own_state() {
        let mut opened = form_of(&Config::default());
        opened.dirty = true;

        let view = ConfigView::from(&opened);
        let rebuilt = ConfigForm::from(view);

        assert!(!rebuilt.filled(), "only a file the client read may mark a form filled");
        assert!(!rebuilt.dirty, "the dirty flag is the screen's own, and starts clear");
        assert_eq!(ConfigTab::Providers, rebuilt.tab, "the screen opens on its first tab");
        assert_eq!(config_from(&rebuilt).unwrap(), Config::default());
    }
}
