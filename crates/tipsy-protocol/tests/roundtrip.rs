use tipsy_protocol::*;

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
    roundtrip_command(Command::Hello { token: Some("t".into()), client: ClientKind::Tui, protocol_version: PROTOCOL_VERSION });
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
