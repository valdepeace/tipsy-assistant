use tipsy_protocol::*;

// The seven section views, named because this file builds them field by field. `Capability`,
// `ProfileView`, `ListeningMode` and `UiLanguage` come from the glob above; naming them twice would
// be an unused import or a duplicate, depending on how rustc resolves the two.
use tipsy_protocol::{
    AllowedPersonView, ConfigView, ListeningView, LoginSettingsView, ProfileEntryView, ProviderView, RealtimeView,
    ServerView,
};

fn roundtrip_command(c: Command) {
    let json = serde_json::to_string(&c).unwrap();
    let back: Command = serde_json::from_str(&json).unwrap();
    assert_eq!(c, back, "json was {json}");
}

fn roundtrip_event(e: Event) {
    let json = serde_json::to_string(&e).unwrap();
    let back: Event = serde_json::from_str(&json).unwrap();
    assert_eq!(e, back, "json was {json}");
}

#[test]
fn commands_roundtrip() {
    roundtrip_command(Command::Hello {
        token: Some("t".into()),
        client: ClientKind::Tui,
        protocol_version: PROTOCOL_VERSION,
    });
    roundtrip_command(Command::Ask { text: "¿Qué pasa, compae? ñandú…".into() });
    roundtrip_command(Command::Say { text: "hola".into() });
    roundtrip_command(Command::ListenStart);
    roundtrip_command(Command::ListenStop);
    roundtrip_command(Command::Cancel);
    roundtrip_command(Command::Reset);
    roundtrip_command(Command::SetProfile { id: "compae".into() });
    roundtrip_command(Command::SetListeningMode { mode: ListeningMode::HandsFree });
}

#[test]
fn events_roundtrip() {
    roundtrip_event(Event::Welcome {
        session_id: 7,
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: UiLanguage::Es,
        listening_mode: ListeningMode::Push,
        web_bundle: false,
        // This fixture is the "no login screen" `Welcome`; the `Some` case is the next test's.
        login: None,
    });
    roundtrip_event(Event::Status { state: AssistantState::Thinking, detail: None });
    roundtrip_event(Event::Heard { text: "oye".into() });
    roundtrip_event(Event::ReplyDelta { text: "Ozú".into() });
    roundtrip_event(Event::Reply { text: "Ozú, quillo.".into() });
    roundtrip_event(Event::SpeechChunk { seq: 3, sample_rate: 24000 });
    roundtrip_event(Event::Done { first_audio_ms: Some(900), total_ms: 4000 });
    roundtrip_event(Event::Done { first_audio_ms: None, total_ms: 10 });
    roundtrip_event(Event::Stats { cpu: 12.5, ram: 40.0, net_in: 1024.0, net_out: 2.0 });
    roundtrip_event(Event::Error { code: ErrorCode::RateLimited, message: "slow down".into(), recoverable: true });
}

/// What the HUD shows for the whole of a realtime call (Plan 2C Task 1): one more `Event`, tagged
/// with `type` and spelled snake_case like every other variant of the crate's tagged events.
#[test]
fn the_live_event_roundtrips() {
    // The server sends `active: true` when a call opens and `active: false` when it ends; both are
    // round-tripped, and both are then pinned by the JSON the HUD's own clients will see.
    for active in [true, false] {
        roundtrip_event(Event::Live { active });
    }
    let json = serde_json::to_string(&Event::Live { active: true }).unwrap();
    assert_eq!(json, r#"{"type":"live","active":true}"#);
    assert_eq!(
        serde_json::from_str::<Event>(r#"{"type":"live","active":false}"#).unwrap(),
        Event::Live { active: false }
    );
}

#[test]
fn json_shape_is_tagged_snake_case() {
    let json = serde_json::to_string(&Command::SetListeningMode { mode: ListeningMode::HandsFree }).unwrap();
    assert_eq!(json, r#"{"type":"set_listening_mode","mode":"hands_free"}"#);
    let json = serde_json::to_string(&Command::ListenStart).unwrap();
    assert_eq!(json, r#"{"type":"listen_start"}"#);
}

#[test]
fn test_provider_and_test_result_roundtrip() {
    for capability in [Capability::Chat, Capability::Speech, Capability::Transcribe] {
        roundtrip_command(Command::TestProvider { capability });
    }
    // A success with a measured latency and the short summary of what answered.
    roundtrip_event(Event::TestResult {
        capability: Capability::Chat,
        ok: true,
        latency_ms: 1234,
        message: "1.2 s, 412 bytes".into(),
    });
    // A refusal with no latency at all: `ok = false` and the provider's own words, which may not
    // be ASCII.
    roundtrip_event(Event::TestResult {
        capability: Capability::Transcribe,
        ok: false,
        latency_ms: 0,
        message: "no key named openrouter — set TIPSY_KEY_OPENROUTER".into(),
    });
}

#[test]
fn test_pair_json_shape_is_tagged_snake_case() {
    for (capability, json) in
        [(Capability::Chat, "chat"), (Capability::Speech, "speech"), (Capability::Transcribe, "transcribe")]
    {
        assert_eq!(serde_json::to_string(&capability).unwrap(), format!("\"{json}\""));
    }

    let json = serde_json::to_string(&Command::TestProvider { capability: Capability::Speech }).unwrap();
    assert_eq!(json, r#"{"type":"test_provider","capability":"speech"}"#);

    let json = serde_json::to_string(&Event::TestResult {
        capability: Capability::Transcribe,
        ok: false,
        latency_ms: 0,
        message: "no key named openrouter".into(),
    })
    .unwrap();
    assert_eq!(
        json,
        r#"{"type":"test_result","capability":"transcribe","ok":false,"latency_ms":0,"message":"no key named openrouter"}"#
    );
}

/// `Capability` is the contract Plan 2B's `Command::TestProvider` and `Event::TestResult` consume,
/// so its serde spelling is pinned here rather than left to plan 2B's own tests.
#[test]
fn capability_serde_spelling_is_snake_case() {
    for (capability, json) in
        [(Capability::Chat, "chat"), (Capability::Speech, "speech"), (Capability::Transcribe, "transcribe")]
    {
        assert_eq!(serde_json::to_string(&capability).unwrap(), format!("\"{json}\""));
        assert_eq!(serde_json::from_str::<Capability>(&format!("\"{json}\"")).unwrap(), capability);
    }
}

#[test]
fn frames_roundtrip() {
    let mic = Frame::Mic(vec![0, 1, -1, i16::MAX, i16::MIN]);
    assert_eq!(Frame::decode(&mic.encode()).unwrap(), mic);
    let speech = Frame::Speech { seq: 42, samples: vec![5, -5] };
    let bytes = speech.encode();
    assert_eq!(bytes[0], 0x02);
    assert_eq!(&bytes[1..5], &42u32.to_le_bytes());
    assert_eq!(Frame::decode(&bytes).unwrap(), speech);
}

#[test]
fn frames_reject_garbage() {
    assert!(Frame::decode(&[]).is_err());
    assert!(Frame::decode(&[0x09, 0, 0]).is_err());
    assert!(Frame::decode(&[0x01, 0x00]).is_err(), "mic payload must have an even byte count");
    assert_eq!(Frame::decode(&[0x01]).unwrap(), Frame::Mic(vec![]));
}

#[test]
fn language_mapping() {
    assert_eq!(UiLanguage::from_bcp47("es-ES"), UiLanguage::Es);
    assert_eq!(UiLanguage::from_bcp47("es"), UiLanguage::Es);
    assert_eq!(UiLanguage::from_bcp47("ES-mx"), UiLanguage::Es);
    assert_eq!(UiLanguage::from_bcp47("en-AU"), UiLanguage::En);
    assert_eq!(UiLanguage::from_bcp47(""), UiLanguage::En);
}

/// The login screen's three commands and two events (spec §4).
#[test]
fn login_commands_and_events_roundtrip() {
    roundtrip_command(Command::LoginStart);
    roundtrip_command(Command::LoginStop);
    roundtrip_command(Command::LoginAnswer { text: "yo soy Huésped con acceso autorizado".into() });
    roundtrip_event(Event::LoginHeard { text: "yo soy Huésped con acceso autorizado".into() });
    roundtrip_event(Event::LoginVerdict { ok: true, name: Some("Huésped".into()) });
    roundtrip_event(Event::LoginVerdict { ok: false, name: None });
}

/// The wire spellings are a contract: a client and a server that disagree about the tag do not fail
/// to compile, they fail to connect.
#[test]
fn the_login_messages_spell_their_tags_as_the_spec_says() {
    let json = serde_json::to_string(&Command::LoginAnswer { text: "huesped".into() }).unwrap();
    assert!(json.contains(r#""type":"login_answer""#), "{json}");
    let json = serde_json::to_string(&Event::LoginVerdict { ok: false, name: None }).unwrap();
    assert!(json.contains(r#""type":"login_verdict""#), "{json}");
    assert!(json.contains(r#""ok":false"#), "{json}");
    assert!(json.contains(r#""name":null"#), "{json}");
}

/// Both directions of compatibility in one test: a server that predates the field sends a `Welcome`
/// without it, and a client that knows it must still parse the event (that is `#[serde(default)]`);
/// serde ignores unknown fields, which is what makes the other direction work.
#[test]
fn a_welcome_without_the_login_field_parses_as_no_login_screen() {
    let json = r#"{"type":"welcome","session_id":1,"profile":{"id":"compae","name":"El Compae","language":"es-ES"},"ui_language":"es","listening_mode":"push","web_bundle":false}"#;
    match serde_json::from_str::<Event>(json).expect("an older server's Welcome must parse") {
        Event::Welcome { login, .. } => assert_eq!(login, None, "an absent field is no screen"),
        other => panic!("expected a Welcome, got {other:?}"),
    }
    roundtrip_event(Event::Welcome {
        session_id: 1,
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: UiLanguage::Es,
        listening_mode: ListeningMode::Push,
        web_bundle: false,
        login: Some(LoginView { voice: true }),
    });
}

/// The whole editable configuration as the screen edits it, populated so that nothing round-trips by
/// accident: three connectors of two kinds, both `Option` shapes, a carried token, both carried
/// sections and a profile with a voices table.
fn config_view() -> ConfigView {
    ConfigView {
        active_profile: "compae".into(),
        providers: vec![
            ProviderView {
                capability: Capability::Chat,
                kind: "openai_compat".into(),
                base_url: "https://openrouter.ai/api/v1".into(),
                model: "google/gemini-2.5-flash".into(),
                key_name: "openrouter".into(),
            },
            ProviderView {
                capability: Capability::Speech,
                kind: "elevenlabs".into(),
                base_url: "https://api.elevenlabs.io".into(),
                model: "eleven_flash_v2_5".into(),
                key_name: "elevenlabs".into(),
            },
            ProviderView {
                capability: Capability::Transcribe,
                kind: "openai_compat".into(),
                base_url: "http://localhost:11434/v1".into(),
                model: "openai/whisper-large-v3".into(),
                key_name: String::new(),
            },
        ],
        listening: ListeningView {
            mode: ListeningMode::Auto,
            silence_ms: "1000".into(),
            mic_gain: "12".into(),
            mic_device: Some("Rode NT-USB".into()),
        },
        realtime: RealtimeView {
            enabled: true,
            model: "gpt-realtime-2.1-mini".into(),
            key_name: "openai-realtime".into(),
            voice: "cedar".into(),
            max_minutes: 25,
        },
        login: LoginSettingsView {
            enabled: true,
            voice: false,
            allowed: vec![
                AllowedPersonView { name: "Huésped".into(), keywords: vec!["huesped".into(), "core".into()] },
                AllowedPersonView { name: "María".into(), keywords: vec!["maria".into()] },
            ],
        },
        profiles: vec![ProfileEntryView {
            id: "compae".into(),
            name: "El Compae".into(),
            language: "es-ES".into(),
            prompt: "Habla como un compae.".into(),
            voice_instructions: "andaluz, tranquilo".into(),
            greeting: "¿Qué pasa, quillo?".into(),
            wake_word: "oye compae".into(),
            tone_floor: "Tono de prueba.".into(),
            voices: vec![
                ("elevenlabs".to_string(), "george".to_string()),
                ("openai_compat".to_string(), "alloy".to_string()),
            ],
        }],
        server: ServerView {
            bind: "127.0.0.1:7070".into(),
            lan: false,
            token: "0123456789abcdef0123456789abcdef".into(),
        },
    }
}

/// The three commands this tier adds, each in both directions, with a fully populated `ConfigView`.
/// `UpdateConfig` carries the **client's** shape, built here from the full one with the same
/// conversion the native client uses.
#[test]
fn the_configuration_commands_roundtrip() {
    roundtrip_command(Command::GetConfig);
    roundtrip_command(Command::UpdateConfig { config: Box::new(ClientConfigView::from(&config_view())) });
    roundtrip_command(Command::SetKey { name: "openrouter".into(), value: "sk-or-v1-…a1b2".into() });
}

/// The three events, each in both directions. `ProfileChanged` carries the same `ProfileView`
/// `Welcome` carries, which is why it is built from the type that already exists.
#[test]
fn the_configuration_events_roundtrip() {
    roundtrip_event(Event::Config { config: Box::new(config_view()) });
    roundtrip_event(Event::ProfileChanged {
        profile: ProfileView { id: "senyor".into(), name: "O Senyor".into(), language: "gl-ES".into() },
    });
    roundtrip_event(Event::ConfigChanged);
}

/// §4.2 and §4.4's JSON exactly: the tag is the variant's snake_case name, `UpdateConfig` nests the
/// whole view under `config`, and `ConfigChanged` deliberately carries no payload (spec §4.4).
#[test]
fn the_configuration_messages_spell_their_tags_as_the_spec_says() {
    assert_eq!(serde_json::to_string(&Command::GetConfig).unwrap(), r#"{"type":"get_config"}"#);
    assert_eq!(serde_json::from_str::<Command>(r#"{"type":"get_config"}"#).unwrap(), Command::GetConfig);

    assert_eq!(serde_json::to_string(&Event::ConfigChanged).unwrap(), r#"{"type":"config_changed"}"#);
    assert_eq!(serde_json::from_str::<Event>(r#"{"type":"config_changed"}"#).unwrap(), Event::ConfigChanged);

    let json = serde_json::to_string(&Command::SetKey { name: "openrouter".into(), value: "v".into() }).unwrap();
    assert_eq!(json, r#"{"type":"set_key","name":"openrouter","value":"v"}"#);

    let json = serde_json::to_string(&Event::ProfileChanged {
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
    })
    .unwrap();
    assert_eq!(json, r#"{"type":"profile_changed","profile":{"id":"compae","name":"El Compae","language":"es-ES"}}"#);

    // `UpdateConfig` is the one shape whose whole payload is nested, so it is read apart in full:
    // the field name is `config`, and the view's own field names are the form's, unchanged.
    //
    // It is the **client's** shape, and what is missing from it is the point: no provider carries a
    // `base_url` or a `key_name` and the server carries no `token`, so a save from a client cannot
    // move where the server sends its key, which key it spends, or its door password (spec §4.3,
    // §13.6). Every field a client *may* move is asserted present.
    let json =
        serde_json::to_string(&Command::UpdateConfig { config: Box::new(ClientConfigView::from(&config_view())) })
            .unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["type"], "update_config");
    assert_eq!(value["config"]["active_profile"], "compae");
    assert_eq!(value["config"]["listening"]["mode"], "auto");
    assert_eq!(value["config"]["providers"][0]["capability"], "chat");
    assert_eq!(value["config"]["providers"][0]["kind"], "openai_compat");
    assert_eq!(value["config"]["providers"][0]["model"], "google/gemini-2.5-flash");
    assert_eq!(value["config"]["server"]["bind"], "127.0.0.1:7070");
    assert_eq!(value["config"]["server"]["lan"], false);
    assert_eq!(value["config"]["realtime"]["key_name"], "openai-realtime");
    assert_eq!(value["config"]["login"]["allowed"][0]["name"], "Huésped");
    assert_eq!(value["config"]["profiles"][0]["voices"][0][0], "elevenlabs");

    for row in value["config"]["providers"].as_array().expect("the providers are a list") {
        assert!(row.get("base_url").is_none(), "a client may not move a provider's address: {row}");
        assert!(row.get("key_name").is_none(), "a client may not choose which key the server spends: {row}");
    }
    assert!(
        value["config"]["server"].get("token").is_none(),
        "a client may not move the door password: {}",
        value["config"]["server"]
    );

    let json = serde_json::to_string(&Event::Config { config: Box::new(config_view()) }).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["type"], "config");
    assert_eq!(value["config"]["server"]["token"], "0123456789abcdef0123456789abcdef");
    assert_eq!(value["config"]["providers"][0]["base_url"], "https://openrouter.ai/api/v1");
    assert_eq!(value["config"]["providers"][0]["key_name"], "openrouter");
}

/// The five `ErrorCode` variants this tier adds. The spelling is the point and the reason they exist
/// at all: a client branches on `code`, and v1 spec §4.2 makes `Event::Error.message` human text
/// localized to the profile's language, so a code parked in the message would be unreadable to it.
#[test]
fn the_new_error_codes_spell_their_tags_as_the_spec_says() {
    for (code, spelling) in [
        (ErrorCode::ProtocolVersion, "protocol_version"),
        (ErrorCode::TokenRequired, "token_required"),
        (ErrorCode::TokenInvalid, "token_invalid"),
        (ErrorCode::BadMessage, "bad_message"),
        (ErrorCode::InsecureTransport, "insecure_transport"),
    ] {
        assert_eq!(serde_json::to_value(code).unwrap(), serde_json::json!(spelling));
        assert_eq!(serde_json::from_value::<ErrorCode>(serde_json::json!(spelling)).unwrap(), code);
    }

    // The variants are additive: the ten that were there keep their spelling byte for byte.
    for (code, spelling) in [
        (ErrorCode::Auth, "auth"),
        (ErrorCode::RateLimited, "rate_limited"),
        (ErrorCode::Timeout, "timeout"),
        (ErrorCode::Network, "network"),
        (ErrorCode::BadResponse, "bad_response"),
        (ErrorCode::NoKey, "no_key"),
        (ErrorCode::Mic, "mic"),
        (ErrorCode::Config, "config"),
        (ErrorCode::NothingHeard, "nothing_heard"),
        (ErrorCode::Internal, "internal"),
    ] {
        assert_eq!(serde_json::to_value(code).unwrap(), serde_json::json!(spelling));
    }

    // …and they travel inside `Event::Error` unchanged, which is where a client reads them.
    roundtrip_event(Event::Error {
        code: ErrorCode::TokenRequired,
        message: "Este servidor pide un token: pásalo con --token.".into(),
        recoverable: false,
    });
    let json = serde_json::to_string(&Event::Error {
        code: ErrorCode::InsecureTransport,
        message: "Esa conexión no va cifrada.".into(),
        recoverable: true,
    })
    .unwrap();
    assert_eq!(
        json,
        r#"{"type":"error","code":"insecure_transport","message":"Esa conexión no va cifrada.","recoverable":true}"#
    );
}
