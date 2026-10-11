//! Wire → action: the pure half of the realtime module.
//!
//! `map` reads one JSON text frame from the Realtime socket and says what the rest of the module
//! should do about it. It is pure on purpose: it touches no socket, no clock, no file and no
//! configuration, so every shape this plan trusts is pinned by a string in its tests — including the
//! two payloads the research note marks inferred.
//!
//! This task also owns the outbound side of the wire, because it is strings rather than sends:
//! `append_audio_message` and the two base64 helpers the whole module shares, and — added by Task 5
//! — the two interruption messages, `truncate_message` and `cancel_message`.

use serde_json::Value;
use tipsy_protocol::ErrorCode;

use crate::providers::gemini::decode_base64;

/// What one server frame asks the rest of the module to do. Task 4 turns these into protocol events,
/// which is why they are named after the events they become (`Heard`, `ReplyDelta`) rather than after
/// the wire's names.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// `session.created` / `session.updated`: the session is configured and the call is live. Task 4
    /// raises `Event::Live { active: true }` for it — Task 1's event — for the whole of the call, and
    /// `Event::Live { active: false }` when the call ends.
    SessionConfigured { session_id: Option<String> },
    /// `response.output_audio.delta`: PCM16 little-endian mono at 24 kHz (`socket::PCM_RATE`),
    /// base64-decoded and ready for the `Player`, carrying the two fields that name *which* item's
    /// audio this is.
    ///
    /// Task 5 needs the name: `conversation.item.truncate` may only cut an assistant message item,
    /// and the item it must cut is the one the device is playing — never the user's item that
    /// `input_audio_buffer.speech_started` names (research note, Interruption 3). The name is only
    /// knowable from the delta itself, because the item-scaffolding events
    /// (`response.output_item.added`, `conversation.item.added`) are not mapped.
    ///
    /// `item_id` is optional because the note marks the *payload fields* of this event as inferred
    /// rather than verified (its Response-section schemas could not be fetched): a delta without a
    /// readable name must not end the call, it must simply leave nothing safe to truncate.
    /// `content_index` is the note's own fixed `0` when the frame does not carry one.
    AudioDelta { item_id: Option<String>, content_index: u32, pcm: Vec<u8> },
    /// `conversation.item.input_audio_transcription.delta`: what the server heard, as it transcribes
    /// it. `Event::Heard` is one log line, so the deltas are accumulated and the *completed* event is
    /// deliberately not mapped: it would show the same sentence twice.
    Heard { text: String },
    /// `response.output_audio_transcript.delta`: the assistant's words, as it speaks them.
    ReplyDelta { text: String },
    /// `input_audio_buffer.speech_started`: the user talked over the model, so playback stops now
    /// (the note's interruption section: "The WebSocket client MUST stop playback locally"). The
    /// item-id bookkeeping and the truncate arithmetic are Task 5's.
    SpeechStarted,
    /// `input_audio_buffer.speech_stopped`: the server's VAD decided the user has finished. It is a
    /// *known* type with no action of its own — the server commits the item and starts the response
    /// (the note's steps 6–7) — so Task 4 does nothing with it. It is carried as its own variant
    /// rather than left to `Ignored`, because a test can then tell "the turn ended" apart from "this
    /// build has never heard of that event".
    SpeechStopped,
    /// `response.done`: the turn is over, with the server's own status (`completed`/`cancelled`).
    ResponseDone { status: String },
    /// `error`: the API refused or failed something. The message is the server's own words.
    ServerError { message: String },
    /// A `type` this build does not know — or one whose existence the research note could not
    /// confirm. Ignored on purpose, never fatal: a Realtime API that grows an event we have never
    /// heard of must not end the call.
    Ignored,
}

/// A *known* event whose payload is not the shape this build expects. Every variant's `Display` names
/// the event and what was missing or wrong, so the HUD's log line is readable and the call ends with
/// a reason rather than a panic (Review Focus 4).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventError {
    #[error("the frame is not JSON: {0}")]
    NotJson(String),
    #[error("{kind} carried no `{name}`")]
    MissingField { kind: String, name: &'static str },
    #[error("{kind} carried `{name}` as {found}, not a string")]
    NotAString { kind: String, name: &'static str, found: &'static str },
    #[error("{kind} carried audio that is not base64: {detail}")]
    BadAudio { kind: String, detail: String },
}

impl EventError {
    /// What the HUD shows for a frame we could not read (`Event::Error.code`): always
    /// `BadResponse`, because that is what it is, and the message says what was wrong.
    pub fn code(&self) -> ErrorCode {
        ErrorCode::BadResponse
    }
}

/// The one mapping the module runs on. Unknown types are `Action::Ignored`; a known type with a
/// payload we cannot read is an `EventError`. Note what is *not* here: the `_done` siblings of the
/// audio and transcript deltas, whose existence the note could not confirm, and `rate_limits.updated`
/// / `response.usage`, which nothing in this plan acts on yet.
pub fn map(frame: &str) -> Result<Action, EventError> {
    let value: Value = serde_json::from_str(frame).map_err(|error| EventError::NotJson(error.to_string()))?;
    let kind = value.get("type").and_then(Value::as_str).ok_or_else(|| EventError::MissingField {
        kind: "<frame>".into(),
        name: "type",
    })?;

    match kind {
        // The first server event and the acknowledgement of our own `session.update`.
        "session.created" | "session.updated" => Ok(Action::SessionConfigured {
            session_id: value.pointer("/session/id").and_then(Value::as_str).map(str::to_string),
        }),

        // INFERRED FIELDS (`delta`, `item_id`, `content_index`): the note confirms this `type`
        // verbatim but its payload page truncated, so this arm and its fixture are the single place
        // the guessed shape lives. `delta` is the one the audio cannot be read without; the item's
        // name is optional on purpose (see `Action::AudioDelta`).
        "response.output_audio.delta" => {
            let delta = field(&value, kind, "delta")?;
            let pcm = decode_base64(delta)
                .map_err(|detail| EventError::BadAudio { kind: kind.to_string(), detail })?;
            let item_id = value.get("item_id").and_then(Value::as_str).map(str::to_string);
            // An index the frame does not carry, or one too large for a `u32`, is the note's own
            // fixed `0`: the note says `content_index` is `0`, so there is nothing to guess.
            let content_index = match value.get("content_index").and_then(Value::as_u64) {
                Some(index) => u32::try_from(index).unwrap_or(0),
                None => 0,
            };
            Ok(Action::AudioDelta { item_id, content_index, pcm })
        }

        // INFERRED FIELD (`delta`), exactly as above.
        "response.output_audio_transcript.delta" => {
            Ok(Action::ReplyDelta { text: field(&value, kind, "delta")?.to_string() })
        }

        "conversation.item.input_audio_transcription.delta" => {
            Ok(Action::Heard { text: field(&value, kind, "delta")?.to_string() })
        }

        // Stopping playback needs no field at all: the event *is* the instruction.
        "input_audio_buffer.speech_started" => Ok(Action::SpeechStarted),

        // The turn-end half of the same pair: known, and deliberately nothing to do.
        "input_audio_buffer.speech_stopped" => Ok(Action::SpeechStopped),

        "response.done" => {
            let status = value.pointer("/response/status").and_then(Value::as_str).ok_or_else(|| {
                EventError::MissingField { kind: kind.to_string(), name: "response.status" }
            })?;
            Ok(Action::ResponseDone { status: status.to_string() })
        }

        "error" => {
            let message = value.pointer("/error/message").and_then(Value::as_str).ok_or_else(|| {
                EventError::MissingField { kind: kind.to_string(), name: "error.message" }
            })?;
            Ok(Action::ServerError { message: message.to_string() })
        }

        _ => Ok(Action::Ignored),
    }
}

/// `input_audio_buffer.append`: one chunk of the microphone's audio, base64 inside the note's JSON
/// text frame (the note's "Audio" section: "`input_audio_buffer.append.audio` is base64 … the server
/// does not acknowledge it"). Task 4 sends the string through the socket; nothing here touches one.
pub fn append_audio_message(pcm_s16le: &[u8]) -> String {
    serde_json::json!({
        "type": "input_audio_buffer.append",
        "audio": base64_encode(pcm_s16le),
    })
    .to_string()
}

/// `conversation.item.truncate`: remove the model's unplayed audio and its transcript, so what the
/// session remembers is what the user actually heard (research note, Interruption 3). `audio_end_ms`
/// is inclusive milliseconds of audio played and `content_index` is fixed at 0 by the note; only an
/// assistant message item may be named here, which is why the caller passes the item the *device*
/// was playing and never the item `input_audio_buffer.speech_started` names.
///
/// The string is built by hand rather than with `json!` so the frame is byte for byte the one the
/// note quotes, field for field in its order: `serde_json`'s `Map` is a `BTreeMap` unless the
/// `preserve_order` feature is on, so a macro here would emit the same object with its keys
/// alphabetised and pin a shape in the tests that no reader of the note can recognise.
pub fn truncate_message(item_id: &str, content_index: u32, audio_end_ms: u64) -> String {
    // The item id is a string from the server, so it goes through the encoder rather than into the
    // template raw (`serde_json::to_string` of a `&str` is exactly its JSON string literal).
    let item_id = serde_json::to_string(item_id).expect("a string is always encodable");
    format!(
        r#"{{"type":"conversation.item.truncate","item_id":{item_id},"content_index":{content_index},"audio_end_ms":{audio_end_ms}}}"#
    )
}

/// `response.cancel`: stop the response in progress (research note, Interruption 5). No
/// `response_id` is sent: the client event is `{type, response_id?}` and cancelling nothing is safe —
/// it errors, and the session survives.
pub fn cancel_message() -> String {
    serde_json::json!({ "type": "response.cancel" }).to_string()
}

/// RFC 4648 standard base64, `+`/`/` with `=` padding — the wire's audio encoding in both
/// directions, hand-written because this plan adds exactly one dependency and it is the socket.
///
/// The decoder beside it in `providers::gemini` is Plan 2A's; this encoder is the inverse that plan
/// never needed (Gemini only answers with base64), written here with the vectors in this file's tests
/// rather than a new crate.
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        // The 24-bit group, taken as three separate bytes so that a short final chunk is dealt with
        // by the padding branches below and never by a bit that is not there. `first >> 2` is the
        // first sextet; a one-byte chunk's remaining two bits are padding, not a sextet, which is
        // where a `& 63` over a zero-padded 24-bit group invents a character (and a byte on the way
        // back).
        let first = u32::from(chunk[0]);
        let second = u32::from(*chunk.get(1).unwrap_or(&0));
        let third = u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(first >> 2) as usize] as char);
        out.push(TABLE[(((first & 0b11) << 4) | (second >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(((second & 0b1111) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 { TABLE[(third & 0b11_1111) as usize] as char } else { '=' });
    }
    out
}

/// The borrowed decoder, with this module's own error type: a frame's base64 that does not decode is
/// `BadAudio`, the decoder's words kept whole. `map` calls Plan 2A's function directly so that the
/// event's own `type` can name the failure; this wrapper is for a caller holding a bare string and no
/// event around it.
pub fn base64_decode(text: &str) -> Result<Vec<u8>, EventError> {
    decode_base64(text).map_err(|detail| EventError::BadAudio { kind: "audio".into(), detail })
}

/// A flat string field of a known event, with the two ways it can be wrong kept apart.
fn field<'a>(value: &'a Value, kind: &str, name: &'static str) -> Result<&'a str, EventError> {
    match value.get(name) {
        None => Err(EventError::MissingField { kind: kind.to_string(), name }),
        Some(found) => found.as_str().ok_or_else(|| EventError::NotAString {
            kind: kind.to_string(),
            name,
            found: json_kind(found),
        }),
    }
}

/// What a JSON value is, in words a person reads in the log line.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::gemini::decode_base64;

    // ---- Fixtures -----------------------------------------------------------------------------
    //
    // Verbatim from `.superpowers/sdd/plan-2c-research/openai-realtime.md`, except where marked.

    const SESSION_CREATED: &str = r#"{"type":"session.created","event_id":"evt_1","session":{"id":"sess_fixture","object":"realtime.session","model":"gpt-realtime-2.1","voice":"marin","turn_detection":{"type":"server_vad","silence_duration_ms":1000},"expires_at":1780000000}}"#;
    const SESSION_UPDATED: &str = r#"{"type":"session.updated","event_id":"evt_2","session":{"id":"sess_fixture","object":"realtime.session","model":"gpt-realtime-2.1","voice":"marin","turn_detection":{"type":"server_vad","silence_duration_ms":1500},"expires_at":1780000000}}"#;

    /// **INFERRED PAYLOAD.** The note confirms this `type` verbatim (its Migration list) but the page
    /// that documents the fields truncated, so `delta` and the sibling indices below are inferred
    /// from neighbouring events and are *unverified*. This fixture is where the guess lives: if the
    /// live smoke test shows another field, this string and the one `field(...)` call that reads
    /// `delta` change together, and the rest of the module does not move.
    const AUDIO_DELTA: &str = r#"{"type":"response.output_audio.delta","event_id":"evt_3","response_id":"resp_1","item_id":"item_1","output_index":0,"content_index":0,"delta":"AAECAw=="}"#;

    /// **INFERRED PAYLOAD**, as above: the `type` is confirmed, the fields are not.
    const TRANSCRIPT_DELTA: &str = r#"{"type":"response.output_audio_transcript.delta","event_id":"evt_4","response_id":"resp_1","item_id":"item_1","output_index":0,"content_index":0,"delta":"Ozú, quillo"}"#;

    /// Confirmed: `item_id`, `content_index`, `delta` (the note's event table, step 9). It arrives
    /// only because the session asked for `transcription` (Task 2's `session.update`).
    const INPUT_TRANSCRIPT_DELTA: &str = r#"{"type":"conversation.item.input_audio_transcription.delta","event_id":"evt_5","item_id":"item_0","content_index":0,"delta":"oye compae"}"#;

    /// Confirmed: `item_id`, `audio_start_ms` (step 5). The payload carries more than the mapping
    /// reads, which is the point — stopping playback needs no field at all.
    const SPEECH_STARTED: &str = r#"{"type":"input_audio_buffer.speech_started","event_id":"evt_6","item_id":"item_0","audio_start_ms":1200}"#;

    /// Confirmed: `item_id`, `audio_end_ms` (step 6). The turn-end half of the pair above; the
    /// mapping does not act on it, but it is a *known* type and must be read as one.
    const SPEECH_STOPPED: &str = r#"{"type":"input_audio_buffer.speech_stopped","event_id":"evt_6b","item_id":"item_0","audio_end_ms":2600}"#;

    /// Confirmed: `response.status` (`completed`/`cancelled`), `response.output[]` and `response.usage`
    /// (step 17). The usage is carried by the payload and deliberately not mapped.
    const RESPONSE_DONE: &str = r#"{"type":"response.done","event_id":"evt_7","response":{"id":"resp_1","status":"completed","output":[],"usage":{"input_tokens":42,"output_tokens":7,"input_token_details":{"audio_tokens":30,"text_tokens":12,"cached_tokens":0},"output_token_details":{}}}}"#;

    /// Confirmed: `error{message, type, code, param, event_id}` plus the server's own `event_id`.
    const SERVER_ERROR: &str = r#"{"type":"error","event_id":"evt_8","error":{"message":"Invalid value for 'session.audio.output.voice'","type":"invalid_request_error","code":"invalid_value","param":"session.audio.output.voice","event_id":"evt_3"}}"#;

    /// **UNCONFIRMED EXISTENCE.** The note could not establish whether the `_done` siblings of the
    /// audio and transcript deltas exist (only the `_text_` done events are confirmed). Whichever way
    /// that goes, this build does not map them: the deltas are what the HUD accumulates, and an event
    /// we do not know must never end a call.
    const UNCONFIRMED_TRANSCRIPT_DONE: &str = r#"{"type":"response.output_audio_transcript.done","event_id":"evt_9","response_id":"resp_1","item_id":"item_1","output_index":0,"content_index":0,"transcript":"Ozú, quillo."}"#;

    #[test]
    fn session_events_configure_the_call() {
        assert_eq!(
            map(SESSION_CREATED).unwrap(),
            Action::SessionConfigured { session_id: Some("sess_fixture".into()) }
        );
        assert_eq!(
            map(SESSION_UPDATED).unwrap(),
            Action::SessionConfigured { session_id: Some("sess_fixture".into()) }
        );
        // The session object is optional in the frame: an id we cannot read is not a failure.
        assert_eq!(
            map(r#"{"type":"session.updated"}"#).unwrap(),
            Action::SessionConfigured { session_id: None }
        );
        assert_eq!(
            map(r#"{"type":"session.updated","session":[]}"#).unwrap(),
            Action::SessionConfigured { session_id: None }
        );
    }

    #[test]
    fn the_audio_delta_is_decoded_to_pcm() {
        // `AAECAw==` is the four bytes 0,1,2,3: the audio deltas are base64 inside JSON text frames
        // (the note's "Audio" section), PCM16 little-endian mono at 24 kHz (`Task 2`'s `PCM_RATE`).
        // The item's name travels with the audio: Task 5's truncate may only cut the item the device
        // was playing, and this delta is the only place that name is known (see `Action::AudioDelta`).
        assert_eq!(
            map(AUDIO_DELTA).unwrap(),
            Action::AudioDelta { item_id: Some("item_1".into()), content_index: 0, pcm: vec![0, 1, 2, 3] }
        );
        // The note marks these payload fields as inferred, so a delta that carries no name is still
        // a delta: it decodes, and it simply leaves nothing safe to truncate.
        assert_eq!(
            map(r#"{"type":"response.output_audio.delta","delta":"AAECAw=="}"#).unwrap(),
            Action::AudioDelta { item_id: None, content_index: 0, pcm: vec![0, 1, 2, 3] }
        );
    }

    #[test]
    fn the_output_transcript_delta_is_the_reply_text() {
        assert_eq!(map(TRANSCRIPT_DELTA).unwrap(), Action::ReplyDelta { text: "Ozú, quillo".into() });
    }

    #[test]
    fn the_input_transcript_delta_is_what_was_heard() {
        assert_eq!(map(INPUT_TRANSCRIPT_DELTA).unwrap(), Action::Heard { text: "oye compae".into() });
    }

    #[test]
    fn speech_started_asks_for_playback_to_stop() {
        // The note's interruption section: "The WebSocket client MUST stop playback locally" on this
        // event. The milliseconds and the item id it also carries are Task 5's business.
        assert_eq!(map(SPEECH_STARTED).unwrap(), Action::SpeechStarted);
    }

    #[test]
    fn speech_stopped_is_a_known_type_with_nothing_to_do() {
        // The turn-end half of the VAD pair (step 6). It is *known*, so it is read as itself rather
        // than falling through the unknown arm — the server ends the turn on its own and this build
        // has no action for it yet, the way `rate_limits.updated` has none.
        assert_eq!(map(SPEECH_STOPPED).unwrap(), Action::SpeechStopped);
    }

    #[test]
    fn response_done_reports_the_servers_status() {
        assert_eq!(map(RESPONSE_DONE).unwrap(), Action::ResponseDone { status: "completed".into() });
        // The other status the note names: with `interrupt_response: true` the server cancels an
        // interrupted response itself and says so here.
        let cancelled = r#"{"type":"response.done","response":{"id":"resp_2","status":"cancelled"}}"#;
        assert_eq!(map(cancelled).unwrap(), Action::ResponseDone { status: "cancelled".into() });
    }

    #[test]
    fn a_server_error_carries_its_message() {
        assert_eq!(
            map(SERVER_ERROR).unwrap(),
            Action::ServerError { message: "Invalid value for 'session.audio.output.voice'".into() }
        );
    }

    #[test]
    fn an_unknown_type_is_ignored_never_fatal() {
        // Review Focus 4: an event type this build has never heard of is data, not death.
        assert_eq!(map(r#"{"type":"session.resumed"}"#).unwrap(), Action::Ignored);
        assert_eq!(map(r#"{"type":"rate_limits.updated","rate_limits":[]}"#).unwrap(), Action::Ignored);
    }

    #[test]
    fn the_unconfirmed_done_events_are_ignored_too() {
        assert_eq!(map(UNCONFIRMED_TRANSCRIPT_DONE).unwrap(), Action::Ignored);
    }

    #[test]
    fn a_known_event_with_a_malformed_payload_is_a_typed_error() {
        // The inferred audio payload is the riskiest shape in this plan: the message names the event
        // and the field, so a live mismatch is a readable log line rather than a panic.
        let error = map(r#"{"type":"response.output_audio.delta","response_id":"resp_1","item_id":"item_1"}"#)
            .unwrap_err();
        assert_eq!(
            error,
            EventError::MissingField { kind: "response.output_audio.delta".into(), name: "delta" }
        );
        assert_eq!(error.to_string(), "response.output_audio.delta carried no `delta`");
        assert_eq!(error.code(), ErrorCode::BadResponse);

        // Same event family, field present with the wrong type.
        let error = map(r#"{"type":"response.output_audio_transcript.delta","delta":7}"#).unwrap_err();
        assert_eq!(
            error.to_string(),
            "response.output_audio_transcript.delta carried `delta` as a number, not a string"
        );

        // The completion's status lives one level down, and its absence is just as typed.
        let error = map(r#"{"type":"response.done","response":{"id":"resp_1"}}"#).unwrap_err();
        assert_eq!(error.to_string(), "response.done carried no `response.status`");

        // A malformed error event is still readable: the HUD shows the missing piece, not a blank.
        let error = map(r#"{"type":"error","error":{"type":"server_error"}}"#).unwrap_err();
        assert_eq!(error.to_string(), "error carried no `error.message`");

        // Base64 that does not decode keeps the decoder's own words, prefixed with the event.
        let error = map(r#"{"type":"response.output_audio.delta","delta":"not base64!"}"#).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("response.output_audio.delta carried audio that is not base64: invalid base64 byte"),
            "{error}"
        );
        assert_eq!(error.code(), ErrorCode::BadResponse);
    }

    #[test]
    fn frames_that_are_not_events_at_all_are_typed_errors() {
        for (frame, expected) in [
            ("", "the frame is not JSON"),
            ("not json at all", "the frame is not JSON"),
            ("[]", "carried no `type`"),
            ("null", "carried no `type`"),
            ("0", "carried no `type`"),
            (r#"{"type":42}"#, "carried no `type`"),
            (r#"{"type":"response.done","response":"cancelled"}"#, "carried no `response.status`"),
            (r#"{"type":"response.output_audio.delta","delta":null}"#, "carried `delta` as null"),
            (r#"{"type":"response.output_audio.delta","delta":[]}"#, "carried `delta` as an array"),
            (
                r#"{"type":"response.output_audio.delta","delta":{}}"#,
                "carried `delta` as an object",
            ),
            // A truncated frame: JSON that stops mid-object, which is what a socket cut in half
            // leaves behind.
            (r#"{"type":"response.done","response":{"sta"#, "the frame is not JSON"),
        ] {
            let error = map(frame).expect_err(frame);
            assert!(error.to_string().contains(expected), "{frame} → {error}");
            assert_eq!(error.code(), ErrorCode::BadResponse, "{frame}");
        }
    }

    /// The decoder this module borrows is the one Plan 2A wrote for Gemini; the mapping's behaviour
    /// for a well-formed payload is asserted once, here, so the reuse is visible rather than implied.
    #[test]
    fn the_borrowed_base64_decoder_is_the_one_the_connectors_use() {
        assert_eq!(decode_base64("AAECAw==").unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(decode_base64("T3rDug==").unwrap(), "Ozú".as_bytes());
    }

    /// The borrowed decoder, with this module's own error type: a frame's base64 that does not decode
    /// is `BadAudio`, the decoder's words kept whole.
    #[test]
    fn the_borrowed_decoder_is_available_under_this_modules_error_type() {
        assert_eq!(base64_decode("AAECAw==").unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(base64_decode("//8AAA==").unwrap(), vec![0xff, 0xff, 0x00, 0x00]);
        let error = base64_decode("not base64!").unwrap_err();
        assert!(
            error.to_string().starts_with("audio carried audio that is not base64: invalid base64 byte"),
            "{error}"
        );
        assert_eq!(error.code(), ErrorCode::BadResponse);
    }

    /// The one outbound message this task owns. Task 4 sends it from `mic_to_api`'s output; Task 5's
    /// two interruption messages are pinned by `realtime::tests`, beside the arithmetic they carry.
    #[test]
    fn the_audio_append_message_carries_the_audio_as_base64() {
        // RFC 4648, pinned against a hand-written vector rather than against this file's own encoder:
        // a codec that only agrees with itself is caught here. `-1, 0` as little-endian PCM16 is
        // `ff ff 00 00`.
        let message = append_audio_message(&[0xff, 0xff, 0x00, 0x00]);
        let parsed: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(parsed["type"], "input_audio_buffer.append");
        assert_eq!(parsed["audio"], "//8AAA==");
        // The note: `audio` is base64 bytes, ≤15 MiB per event, and the server does not acknowledge it.
        assert_eq!(
            parsed.as_object().unwrap().len(),
            2,
            "the note's append event carries the type and the audio and nothing else: {message}"
        );
    }

    /// The encoder is this file's own — Gemini only decodes — so it is pinned against vectors a
    /// person can check by hand, including the three padding lengths, and round-tripped through the
    /// borrowed decoder.
    #[test]
    fn the_base64_encoder_is_rfc_4648() {
        assert_eq!(base64_encode(&[]), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // A one-byte group is the case a hand-rolled encoder gets wrong: its last sextet is padding,
        // not `0xff & 63`. `[0xff]` is `/w==` and `[0xff, 0xff, 0xff]` is `////`.
        assert_eq!(base64_encode(&[0xff]), "/w==");
        assert_eq!(base64_encode(&[0xff, 0xff]), "//8=");
        assert_eq!(base64_encode(&[0xff, 0xff, 0xff]), "////");
        // The bytes of "Ozú" are 4f 7a c3 ba: what the borrowed decoder's own test decodes back.
        assert_eq!(base64_encode("Ozú".as_bytes()), "T3rDug==");
        // The byte `0xfb` is sextet 62 (the first character the URL-safe alphabet would replace):
        // three of them are four `+` and nothing else, which is where a table lookup written by hand
        // tends to drift to `/`.
        assert_eq!(base64_encode(&[0xfb, 0xef, 0xbe]), "++++");
        // Every vector above decodes back through Plan 2A's decoder — the one the connectors use, so
        // this is a second, independent opinion and not the encoder agreeing with itself.
        for &(bytes, encoded) in &[
            (&b""[..], ""),
            (&b"f"[..], "Zg=="),
            (&b"fo"[..], "Zm8="),
            (&b"foo"[..], "Zm9v"),
            (&b"foob"[..], "Zm9vYg=="),
            (&b"fooba"[..], "Zm9vYmE="),
            (&b"foobar"[..], "Zm9vYmFy"),
            (&[0xff][..], "/w=="),
            (&[0xff, 0xff][..], "//8="),
            (&[0xff, 0xff, 0xff][..], "////"),
            (&[0xfb, 0xef, 0xbe][..], "++++"),
        ] {
            assert_eq!(base64_encode(bytes), encoded, "{bytes:?}");
            assert_eq!(decode_base64(encoded).unwrap(), bytes, "{encoded}");
        }
    }

    /// Every four characters of base64 stand for three bytes, which is the arithmetic this task's own
    /// test in the plan's Task 4 relies on: 960 bytes of 20 ms of 24 kHz mono PCM16 is 1280
    /// characters.
    #[test]
    fn the_encoder_pads_to_a_multiple_of_four() {
        for length in 0..=64usize {
            let bytes: Vec<u8> = (0..length).map(|i| (i * 37 + 11) as u8).collect();
            let encoded = base64_encode(&bytes);
            assert_eq!(encoded.len() % 4, 0, "{length} bytes encode in fours: {encoded}");
            assert_eq!(encoded.len(), length.div_ceil(3) * 4, "{length} bytes: {encoded}");
            assert_eq!(decode_base64(&encoded).unwrap(), bytes, "{length} bytes round-trip");
        }
    }

    /// An empty frame is not an event and must not panic: the note's "an unanticipated event is data,
    /// not death" applies to a frame with no JSON in it at all.
    #[test]
    fn an_empty_frame_is_an_error_not_a_panic() {
        let error = map("").unwrap_err();
        assert!(error.to_string().contains("the frame is not JSON"), "{error}");
        assert_eq!(error.code(), ErrorCode::BadResponse);
    }
}
