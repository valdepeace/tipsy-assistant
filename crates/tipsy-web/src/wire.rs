//! Wire → value and value → wire: the pure half of the browser client.
//!
//! Text frames carry the protocol's own JSON (`Command` out, `Event` in) and binary frames carry
//! `Frame::encode`'s bytes, exactly as the native bridge does (spec §3.3, §4.5). Nothing here touches
//! a socket, a DOM node or a clock, which is what makes every shape this client trusts a string in
//! the tests below — and what lets them run on the host target.

use tipsy_protocol::{Command, Event, Frame, FrameError};

/// The `type` tag of every `Event` this build knows. `event_from_text` refuses anything else as
/// [`WireError::UnknownType`], which the socket turns into silence rather than a notice: a newer
/// server may send more than this client understands, and that must not break a running HUD
/// (v1 spec §12; Plan 2C Task 3's "an unknown `type` is data, not death").
///
/// **Sixteen, not the thirteen the plan wrote**, because Task 1.1 of this same plan added `config`,
/// `profile_changed` and `config_changed` to `Event` after that paragraph was written. The count is
/// not a comment: `the_known_tags_are_exactly_the_events_this_build_decodes` reads this table against
/// a hand-built value of every variant, so a seventeenth event cannot be added without landing here.
pub const KNOWN_EVENT_TAGS: [&str; 16] = [
    "welcome",
    "status",
    "heard",
    "reply_delta",
    "reply",
    "speech_chunk",
    "done",
    "stats",
    "error",
    "test_result",
    "live",
    "login_heard",
    "login_verdict",
    "config",
    "profile_changed",
    "config_changed",
];

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum WireError {
    #[error("the text frame is not JSON: {0}")]
    Json(String),
    #[error("unknown event type `{0}`")]
    UnknownType(String),
    #[error("`{tag}` did not parse: {detail}")]
    Event { tag: String, detail: String },
    #[error("the binary frame is not audio: {0}")]
    Frame(#[from] FrameError),
    /// Nothing is on the wire: the socket is still connecting, or it is closing or closed. `send`
    /// asks rather than letting the browser throw, because both of those are the same fact to a
    /// caller (`socket::Outbox`).
    #[error("the socket is not open")]
    NotOpen,
    /// A [`tipsy_protocol::ToServer`] variant the server talks to itself with. It is not a refusal by
    /// the socket: it never had anywhere to go.
    #[error("`Reconfigure` is the hub talking to its own sessions and never crosses a socket")]
    ServerOnly,
    /// What the browser itself said: a JS exception's text, or an inbound message that was neither a
    /// string nor an `ArrayBuffer`.
    #[error("{0}")]
    Message(String),
}

/// One text frame → one `Event`. The tag is read first so that an event this build does not know is
/// told apart from an event whose payload it cannot read: the first is ignored, the second is shown.
pub fn event_from_text(text: &str) -> Result<Event, WireError> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| WireError::Json(e.to_string()))?;
    let tag = value.get("type").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    if !KNOWN_EVENT_TAGS.contains(&tag.as_str()) {
        return Err(WireError::UnknownType(tag));
    }
    serde_json::from_value(value).map_err(|e| WireError::Event { tag, detail: e.to_string() })
}

/// One binary frame → one `Frame`. The bytes are the protocol's own (`Frame::decode`), so the browser
/// and the native client read the same audio.
pub fn frame_from_bytes(bytes: &[u8]) -> Result<Frame, WireError> {
    Frame::decode(bytes).map_err(WireError::Frame)
}

/// One `Command` → the text frame that carries it.
pub fn command_to_text(command: &Command) -> Result<String, WireError> {
    serde_json::to_string(command).map_err(|e| WireError::Json(e.to_string()))
}

/// One `Frame` → the binary frame that carries it, in the protocol's own bytes.
pub fn frame_to_bytes(frame: &Frame) -> Vec<u8> {
    frame.encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::{
        AllowedPersonView, AssistantState, Capability, ClientKind, ConfigView, ErrorCode, Event, Frame, FrameError,
        ListeningMode, ListeningView, LoginSettingsView, ProfileEntryView, ProfileView, ProviderView, RealtimeView,
        ServerView, UiLanguage, PROTOCOL_VERSION,
    };

    fn profile() -> ProfileView {
        ProfileView { id: "compae".into(), name: "Compae".into(), language: "es-ES".into() }
    }

    /// A whole `ConfigView`, because `Event::Config` carries one and the fixture below has to build
    /// every variant of `Event`. Its contents are arbitrary; what this file cares about is that the
    /// payload survives the wire, not what a configuration means.
    fn config_view() -> ConfigView {
        ConfigView {
            active_profile: "compae".into(),
            providers: vec![ProviderView {
                capability: Capability::Chat,
                kind: "openai_compat".into(),
                base_url: "http://127.0.0.1:17999/v1".into(),
                model: "stub".into(),
                key_name: "STUB".into(),
            }],
            listening: ListeningView {
                mode: ListeningMode::Push,
                silence_ms: "900".into(),
                mic_gain: "1.0".into(),
                mic_device: None,
            },
            realtime: RealtimeView {
                enabled: false,
                model: "gpt-realtime".into(),
                key_name: "STUB".into(),
                voice: "marin".into(),
                max_minutes: 5,
            },
            login: LoginSettingsView {
                enabled: true,
                voice: false,
                allowed: vec![AllowedPersonView { name: "Compae".into(), keywords: vec!["hola".into()] }],
            },
            profiles: vec![ProfileEntryView {
                id: "compae".into(),
                name: "Compae".into(),
                language: "es-ES".into(),
                prompt: "Habla como un compae.".into(),
                voice_instructions: "Cercano.".into(),
                greeting: "Listo, quillo.".into(),
                wake_word: "oye compae".into(),
                voices: vec![("openai_compat".into(), "alloy".into())],
            }],
            server: ServerView { bind: "127.0.0.1:7070".into(), lan: true, token: "no-es-un-secreto-real".into() },
        }
    }

    /// Every variant of `Event`, built by hand. Adding a variant to `tipsy-protocol` means adding it
    /// here — and that is the point: `event_from_text` refuses a tag it does not know, so this list is
    /// what keeps the client from silently ignoring a new event.
    fn every_event() -> Vec<Event> {
        vec![
            Event::Welcome {
                session_id: 7,
                profile: profile(),
                ui_language: UiLanguage::Es,
                listening_mode: ListeningMode::Push,
                web_bundle: true,
                login: None,
            },
            Event::Status { state: AssistantState::Thinking, detail: Some("pensando".into()) },
            Event::Heard { text: "¿y eso?".into() },
            Event::ReplyDelta { text: "Ozú, ".into() },
            Event::Reply { text: "Ozú, quillo.".into() },
            Event::SpeechChunk { seq: 3, sample_rate: 24_000 },
            Event::Done { first_audio_ms: Some(410), total_ms: 1_930 },
            Event::Stats { cpu: 12.5, ram: 41.0, net_in: 1_024.0, net_out: 512.0 },
            Event::Error { code: ErrorCode::Network, message: "sin red".into(), recoverable: true },
            Event::TestResult { capability: Capability::Chat, ok: true, latency_ms: 220, message: "ok".into() },
            Event::Live { active: false },
            Event::LoginHeard { text: "hola".into() },
            Event::LoginVerdict { ok: true, name: Some("Compae".into()) },
            Event::Config { config: Box::new(config_view()) },
            Event::ProfileChanged { profile: profile() },
            Event::ConfigChanged,
        ]
    }

    #[test]
    fn every_event_survives_a_round_trip_through_the_wire() {
        for event in every_event() {
            let text = serde_json::to_string(&event).expect("an event serialises");
            assert_eq!(event_from_text(&text).expect("and parses back"), event, "{text}");
        }
    }

    /// The other half of the fixture above, and the reason it cannot silently rot: every variant this
    /// build knows is in `KNOWN_EVENT_TAGS`, so a version bump that adds a variant fails here until the
    /// tag table and the fixture are both extended.
    #[test]
    fn the_known_tags_are_exactly_the_events_this_build_decodes() {
        let mut tags: Vec<String> = every_event()
            .iter()
            .map(|event| {
                let value: serde_json::Value = serde_json::to_value(event).expect("an event serialises");
                let tag = value.get("type").and_then(serde_json::Value::as_str).expect("every event is tagged");
                assert!(KNOWN_EVENT_TAGS.contains(&tag), "`{tag}` is missing from KNOWN_EVENT_TAGS");
                tag.to_string()
            })
            .collect();
        tags.sort();
        tags.dedup();
        assert_eq!(tags.len(), KNOWN_EVENT_TAGS.len(), "KNOWN_EVENT_TAGS has a tag no event produces");
    }

    #[test]
    fn an_unknown_event_type_is_a_tag_not_a_crash() {
        // A newer server's event, which this build has never heard of: Plan 2C's Task 3 rule — data,
        // not death (docs/superpowers/plans/2026-10-10-plan-2c-openai-realtime.md:1732).
        assert_eq!(
            event_from_text(r#"{"type":"volume_changed","level":3}"#),
            Err(WireError::UnknownType("volume_changed".into()))
        );
        assert_eq!(event_from_text(r#"{"level":3}"#), Err(WireError::UnknownType(String::new())));
    }

    #[test]
    fn a_known_event_with_a_broken_payload_names_its_tag() {
        let error = event_from_text(r#"{"type":"speech_chunk"}"#).unwrap_err();
        match error {
            WireError::Event { tag, detail } => {
                assert_eq!(tag, "speech_chunk");
                assert!(detail.contains("seq"), "{detail}");
            }
            other => panic!("expected a named event error, got {other:?}"),
        }
    }

    #[test]
    fn a_text_frame_that_is_not_json_is_an_error_not_a_panic() {
        assert!(matches!(event_from_text("not json at all"), Err(WireError::Json(_))));
    }

    #[test]
    fn the_text_of_a_command_is_the_protocols_json() {
        // Field order is the derive's, and `token: None` is `null` — pinned so a protocol change
        // cannot pass unnoticed.
        assert_eq!(
            command_to_text(&Command::Hello {
                token: None,
                client: ClientKind::Web,
                protocol_version: PROTOCOL_VERSION,
            })
            .unwrap(),
            r#"{"type":"hello","token":null,"client":"web","protocol_version":1}"#
        );
        assert_eq!(command_to_text(&Command::ListenStart).unwrap(), r#"{"type":"listen_start"}"#);
        assert_eq!(
            command_to_text(&Command::Ask { text: "¿y eso?".into() }).unwrap(),
            r#"{"type":"ask","text":"¿y eso?"}"#
        );
    }

    #[test]
    fn binary_frames_are_the_protocols_own_bytes() {
        // `audio/device.rs:18-20`'s 320-sample frame, little-endian, behind the 0x01 kind byte
        // (`tipsy-protocol/src/lib.rs:463-464`, `:479-496`).
        assert_eq!(frame_to_bytes(&Frame::Mic(vec![1, -2])), vec![0x01, 0x01, 0x00, 0xFE, 0xFF]);
        assert_eq!(frame_from_bytes(&[0x01, 0x01, 0x00]).unwrap(), Frame::Mic(vec![1]));
        // The downward direction, which is a different shape: `0x02`, a `seq`, then the samples of one
        // whole spoken sentence (`tipsy-protocol/src/lib.rs:488-494`).
        assert_eq!(
            frame_to_bytes(&Frame::Speech { seq: 3, samples: vec![1, -2] }),
            vec![0x02, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0xFE, 0xFF]
        );
        assert_eq!(
            frame_from_bytes(&[0x02, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00]).unwrap(),
            Frame::Speech { seq: 3, samples: vec![1] }
        );
    }

    #[test]
    fn a_binary_frame_that_is_not_audio_is_an_error() {
        assert_eq!(frame_from_bytes(&[]), Err(WireError::Frame(FrameError::Empty)));
        assert_eq!(frame_from_bytes(&[0x09, 0, 0]), Err(WireError::Frame(FrameError::UnknownKind(9))));
        assert_eq!(frame_from_bytes(&[0x01, 0x00]), Err(WireError::Frame(FrameError::Malformed)));
    }
}
