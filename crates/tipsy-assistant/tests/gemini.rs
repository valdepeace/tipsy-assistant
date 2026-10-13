//! Connector tests for the native Gemini API (spec §13: `wiremock` with recorded responses).

use futures::StreamExt;
use serde_json::{Value, json};
use tipsy_assistant::providers::gemini::{GeminiChat, GeminiSpeech};
use tipsy_assistant::providers::{
    ChatMessage, ChatProvider, ChatRequest, ProviderError, SpeechProvider, SpeechRequest,
};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A recorded Interactions stream: a `model_output` step with two text deltas, an audio delta and
/// an unknown event type (both ignored by the reader), then the terminator. Every `data:` line is
/// preceded by the `event:` line that repeats its type, exactly as the API frames it.
const STREAM: &str = concat!(
    "event: step.start\n",
    "data: {\"index\":1,\"step\":{\"type\":\"model_output\"},\"event_type\":\"step.start\"}\n\n",
    "event: step.delta\n",
    "data: {\"index\":1,\"delta\":{\"text\":\"1, 2, 3, 4, 5, 6, \",\"type\":\"text\"},\"event_type\":\"step.delta\"}\n\n",
    "event: step.delta\n",
    "data: {\"index\":1,\"delta\":{\"text\":\"¡Ozú, ñandú!\",\"type\":\"text\"},\"event_type\":\"step.delta\"}\n\n",
    "event: step.stop\n",
    "data: {\"index\":1,\"event_type\":\"step.stop\"}\n\n",
    "event: step.delta\n",
    "data: {\"index\":2,\"delta\":{\"data\":\"AAAA\",\"mime_type\":\"audio/l16\",\"type\":\"audio\"},\"event_type\":\"step.delta\"}\n\n",
    "event: interaction.created\n",
    "data: {\"interaction\":{\"id\":\"v1_abc\"},\"event_type\":\"interaction.created\"}\n\n",
    "event: interaction.completed\n",
    "data: {\"interaction\":{\"id\":\"v1_abc\",\"status\":\"completed\"},\"event_type\":\"interaction.completed\"}\n\n",
    "event: done\n",
    "data: [DONE]\n",
);

fn simple() -> ChatRequest {
    ChatRequest { messages: vec![ChatMessage::system("sys"), ChatMessage::user("hola")] }
}

async fn items(provider: &GeminiChat, req: ChatRequest) -> Vec<Result<String, ProviderError>> {
    let mut stream = provider.stream(req).await.expect("the request must be accepted");
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        out.push(item);
    }
    out
}

async fn collect(provider: &GeminiChat, req: ChatRequest) -> Result<Vec<String>, ProviderError> {
    let mut out = Vec::new();
    for item in items(provider, req).await {
        out.push(item?);
    }
    Ok(out)
}

async fn stream_error(provider: &GeminiChat, req: ChatRequest) -> ProviderError {
    match provider.stream(req).await {
        Ok(_) => panic!("expected the request to fail"),
        Err(e) => e,
    }
}

/// Base64 encoder for fixtures. The API answers with base64 audio and the connector decodes it
/// with its own decoder (this plan adds no dependency), so the test needs the inverse.
fn b64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = ((chunk[0] as u32) << 16)
            | ((*chunk.get(1).unwrap_or(&0) as u32) << 8)
            | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/// A minimal 16-bit mono RIFF/WAVE container at the rate the test asks for. The production code
/// has no generic WAV encoder (`encode_wav_16k` always resamples to 16 kHz) and this task must put
/// the container's rate wherever Review Focus 3 needs it.
fn wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

fn tone(freq: f32, rate: u32, ms: u32) -> Vec<i16> {
    let n = (rate as u64 * ms as u64 / 1000) as usize;
    (0..n).map(|i| ((i as f32 * freq * std::f32::consts::TAU / rate as f32).sin() * 16_000.0) as i16).collect()
}

/// The real `compae` voice brief's first two sentences, verbatim from the profile card.
const BRIEF: &str = concat!(
    "Voz masculina adulta, medio-grave y cálida, limpia y con apenas un leve toque rasposo, nada ronca. ",
    "Acento andaluz natural y bien marcado: seseo, eses aspiradas, finales relajados y musicalidad andaluza."
);
/// The plan's decision: the first sentence of `voice_instructions` is the `style` sent to the API.
const STYLE: &str =
    "Voz masculina adulta, medio-grave y cálida, limpia y con apenas un leve toque rasposo, nada ronca.";

fn speech_request(text: &str, instructions: &str, voice: Option<&str>) -> SpeechRequest {
    SpeechRequest { text: text.into(), instructions: instructions.into(), voice: voice.map(str::to_string) }
}

/// A unary TTS interaction as the note records it: one `model_output` step whose content block is
/// the audio, with `mime_type`, `channels`, `sample_rate` and base64 `data`.
fn audio_response(mime: &str, sample_rate: u32, bytes: &[u8]) -> Value {
    json!({
        "id": "v1_tts",
        "status": "completed",
        "model": "gemini-3.8-flash-tts",
        "object": "interaction",
        "steps": [{
            "type": "model_output",
            "content": [{
                "type": "audio",
                "mime_type": mime,
                "data": b64(bytes),
                "channels": 1,
                "sample_rate": sample_rate
            }]
        }]
    })
}

#[tokio::test]
async fn chat_streams_step_deltas_and_sends_the_documented_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .and(header("x-goog-api-key", "k123"))
        .and(header("content-type", "application/json"))
        .and(body_partial_json(json!({
            "model": "gemini-3.8-flash",
            "system_instruction": "sys",
            "input": "hola",
            "stream": true,
            "store": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(STREAM, "text/event-stream"))
        .mount(&server)
        .await;
    // A trailing slash is what a user pastes into `config.toml`; the endpoint must not double it.
    let p = GeminiChat::new(format!("{}/", server.uri()), "gemini-3.8-flash", Some("k123".into()));
    assert_eq!(
        collect(&p, simple()).await.unwrap(),
        vec!["1, 2, 3, 4, 5, 6, ".to_string(), "¡Ozú, ñandú!".to_string()]
    );
    let sent = &server.received_requests().await.unwrap()[0];
    // Native Gemini authenticates with the API-key header only; `Bearer` is the OpenAI-compat
    // layer's style and would be wrong here.
    assert!(sent.headers.get("authorization").is_none(), "unexpected authorization header");
}

#[tokio::test]
async fn chat_sends_the_full_history_as_inline_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(STREAM, "text/event-stream"))
        .mount(&server)
        .await;
    let p = GeminiChat::new(server.uri(), "gemini-3.8-flash", None);
    let req = ChatRequest {
        messages: vec![
            ChatMessage::system("sys"),
            ChatMessage::user("pregunta"),
            ChatMessage::assistant("respuesta"),
            ChatMessage::user("y ahora"),
        ],
    };
    collect(&p, req).await.unwrap();

    let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["system_instruction"], "sys");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    // The note prints the `user_input` block verbatim and says history travels inline as
    // `[{type:"user_input",...},{type:"model_output",...}, ...]`, but it prints no `model_output`
    // block. This pins the symmetric shape; if the API rejects it, this fixture is what changes.
    assert_eq!(
        body["input"],
        json!([
            {"type": "user_input", "content": [{"type": "text", "text": "pregunta"}]},
            {"type": "model_output", "content": [{"type": "text", "text": "respuesta"}]},
            {"type": "user_input", "content": [{"type": "text", "text": "y ahora"}]},
        ])
    );
}

#[tokio::test]
async fn chat_omits_system_instruction_when_there_is_none() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(STREAM, "text/event-stream"))
        .mount(&server)
        .await;
    let p = GeminiChat::new(server.uri(), "m", None);
    let req = ChatRequest { messages: vec![ChatMessage::user("hola")] };
    collect(&p, req).await.unwrap();

    let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    // A single user turn uses the documented string form, and an absent system prompt is an
    // absent field, never an empty string.
    assert_eq!(body["input"], "hola");
    assert!(body.get("system_instruction").is_none(), "no system prompt must send no field: {body}");
}

/// Review Focus 1: 401, 429 and 5xx with a JSON body keep the provider's own message (never
/// empty). The note records no message text for these three statuses, so the fixture strings are
/// arbitrary; only the envelope `{"error":{"code","message"}}` is quoted from it.
#[tokio::test]
async fn chat_maps_auth_rate_limit_and_server_errors_with_the_provider_message() {
    let cases = [
        (
            401u16,
            "authentication",
            "API key not valid. Please pass a valid API key.",
            ProviderError::Auth("API key not valid. Please pass a valid API key.".to_string()),
        ),
        (
            429u16,
            "rate_limit_exceeded",
            "Quota exceeded for quota metric 'generate requests'.",
            ProviderError::RateLimited("Quota exceeded for quota metric 'generate requests'.".to_string()),
        ),
        (
            500u16,
            "api_error",
            "Internal error encountered.",
            ProviderError::BadResponse("HTTP 500: Internal error encountered.".to_string()),
        ),
    ];
    for (status, code, message, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/interactions"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string(json!({"error": {"code": code, "message": message}}).to_string()),
            )
            .mount(&server)
            .await;
        let p = GeminiChat::new(server.uri(), "gemini-3.8-flash", Some("nope".into()));
        let err = stream_error(&p, simple()).await;
        assert_eq!(err, expected, "status {status}");

        let carried = match &err {
            ProviderError::Auth(m) | ProviderError::RateLimited(m) | ProviderError::BadResponse(m) => m,
            other => panic!("status {status} produced {other:?}"),
        };
        assert!(!carried.trim().is_empty(), "status {status} carried an empty message");

        // The note publishes no `Retry-After` and no per-model limits; the connector must hand the
        // 429 straight up, because the single retry with backoff lives in the brain.
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "status {status} was retried");
    }
}

/// The consuming task (Task 5) asserts the request path ends with `/v1beta/interactions`, and a
/// user may paste `base_url` either with or without the version segment. Both forms must reach the
/// same path: never `/v1beta/v1beta/interactions`.
#[tokio::test]
async fn chat_accepts_a_base_url_that_already_carries_the_version() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(STREAM, "text/event-stream"))
        .mount(&server)
        .await;
    // If the path doubled, wiremock matches nothing and answers 404, so `collect` fails here.
    let p = GeminiChat::new(format!("{}/v1beta", server.uri()), "gemini-3.8-flash", None);
    assert_eq!(collect(&p, simple()).await.unwrap().len(), 2);
}

/// The mid-stream failure the note records: HTTP stays 200 and the stream simply carries an
/// `error` event, so a truncated stream must not be mistaken for a finished reply.
#[tokio::test]
async fn chat_surfaces_a_mid_stream_error_that_arrived_with_http_200() {
    let server = MockServer::start().await;
    let body = concat!(
        "event: step.delta\n",
        "data: {\"index\":1,\"delta\":{\"text\":\"Ozú\",\"type\":\"text\"},\"event_type\":\"step.delta\"}\n\n",
        "event: error\n",
        "data: {\"event_type\":\"error\",\"error\":{\"code\":\"not_found\",\"message\":\"Failed to get completed interaction: Result not found.\"}}\n\n",
        "event: done\n",
        "data: [DONE]\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;
    let p = GeminiChat::new(server.uri(), "gemini-3.8-flash", None);

    let out = items(&p, simple()).await;
    assert_eq!(
        out,
        vec![
            Ok("Ozú".to_string()),
            Err(ProviderError::BadResponse("Failed to get completed interaction: Result not found.".to_string())),
        ]
    );
    assert!(out.last().unwrap().is_err(), "the error must end the reply, not complete it");
}

/// The other half of the truncation story: HTTP/2 delivers the whole body and then closes the
/// stream cleanly (`END_STREAM`), so a body that simply stops after a delta is a clean HTTP
/// success with no transport error anywhere. A proxy or a cut network that truncates the reply
/// this way must not be spoken as a complete one: the reader yields the deltas it read and then
/// one error, and the consumer has an error in hand — never a normal end.
#[tokio::test]
async fn chat_reports_a_stream_that_ends_without_its_completion_event() {
    let server = MockServer::start().await;
    let body = concat!(
        "event: step.delta\n",
        "data: {\"index\":1,\"delta\":{\"text\":\"Ozú, quillo\",\"type\":\"text\"},\"event_type\":\"step.delta\"}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;
    let p = GeminiChat::new(server.uri(), "gemini-3.8-flash", None);

    let out = items(&p, simple()).await;
    assert_eq!(out.first(), Some(&Ok("Ozú, quillo".to_string())), "the deltas that did arrive are kept");
    match out.last() {
        Some(Err(ProviderError::BadResponse(message))) => {
            assert!(message.contains("before the completion event"), "unhelpful message: {message}")
        }
        other => panic!("a truncated reply must end in an error, not a clean end: {other:?}"),
    }
    assert!(!out.iter().all(|item| item.is_ok()), "the half-reply must never be presented as a complete one: {out:?}");
}

#[tokio::test]
async fn speech_sends_the_style_the_profile_voice_and_the_audio_format() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .and(header("x-goog-api-key", "k123"))
        .and(body_partial_json(json!({
            "model": "gemini-3.8-flash-tts",
            "input": [{ "type": "user_input", "content": [{
                "type": "text",
                "text": "Ozú, quillo.",
                "annotations": [{ "type": "speech_metadata", "style": STYLE }]
            }]}],
            "response_format": { "type": "audio" },
            "generation_config": { "speech_config": [{ "voice": "Puck" }] },
            "stream": false,
            "store": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(audio_response(
            "audio/wav",
            24_000,
            &wav(&tone(440.0, 24_000, 10), 24_000),
        )))
        .mount(&server)
        .await;
    let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", Some("k123".into()));
    let pcm = p.synthesize(speech_request("Ozú, quillo.", BRIEF, Some("Puck"))).await.unwrap();

    assert_eq!(pcm.sample_rate, 24_000);
    assert_eq!(pcm.samples.len(), 240);

    let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["input"][0]["content"][0]["text"], "Ozú, quillo.");
    // The note's uncertainty list leaves it open whether a TTS interaction accepts
    // `system_instruction`, so a speech request must never carry one.
    assert!(body.get("system_instruction").is_none(), "speech must not send a system instruction: {body}");
    // `text` is the verbatim transcript: the connector never adds `<sigh>`-style tags of its own.
    assert!(!body["input"][0]["content"][0]["text"].as_str().unwrap().contains('<'));
}

/// Review Focus 5: `voices` is a recommendation table. A profile with no `gemini` entry still
/// speaks, with the documented default voice instead of a failure.
#[tokio::test]
async fn speech_sends_kore_when_the_profile_has_no_gemini_voice() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(audio_response(
            "audio/wav",
            24_000,
            &wav(&tone(440.0, 24_000, 10), 24_000),
        )))
        .mount(&server)
        .await;
    // The `voices` table of this profile is empty, so `session.rs` resolves `voice: None`.
    let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", None);
    let pcm = p.synthesize(speech_request("Ozú.", "", None)).await.unwrap();
    assert_eq!(pcm.sample_rate, 24_000);

    let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["generation_config"]["speech_config"][0]["voice"], "Kore");
    // With no delivery instructions there is no style to send, so the annotations array is absent
    // rather than an empty string the API would have to interpret.
    assert!(body["input"][0]["content"][0].get("annotations").is_none(), "{body}");
}

/// Streaming audio is headerless `audio/l16` (raw signed 16-bit LE) and must never be handed to the
/// WAV decoder; the rate comes from the block.
#[tokio::test]
async fn speech_decodes_headerless_l16_at_the_rate_the_response_declares() {
    let mut raw = Vec::new();
    for sample in [16_384i16, -16_384i16] {
        raw.extend_from_slice(&sample.to_le_bytes());
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(audio_response("audio/l16", 24_000, &raw)))
        .mount(&server)
        .await;
    let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", None);
    let pcm = p.synthesize(speech_request("Ozú.", "", None)).await.unwrap();

    assert_eq!(pcm.sample_rate, 24_000);
    assert_eq!(pcm.samples.len(), 2);
    assert!((pcm.samples[0] - 0.5).abs() < 0.001 && (pcm.samples[1] + 0.5).abs() < 0.001);
}

/// Review Focus 3: the note lists `audio/mulaw` and `audio/alaw` as accepted `response_format`
/// values, so a response can legitimately arrive in a container this connector cannot read. A
/// wrong container reported is worth more than a wrong container played.
#[tokio::test]
async fn speech_reports_an_unsupported_container_instead_of_playing_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(audio_response(
            "audio/mulaw",
            24_000,
            &wav(&tone(440.0, 24_000, 10), 24_000),
        )))
        .mount(&server)
        .await;
    let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", None);
    let err = p.synthesize(speech_request("Ozú.", BRIEF, None)).await.unwrap_err();

    let ProviderError::BadResponse(message) = &err else { panic!("expected BadResponse, got {err:?}") };
    assert!(message.contains("audio/mulaw"), "the message must name the container: {message}");
}

/// Review Focus 3 again: the container says 16 kHz and the block claims 24 kHz. Playing it at the
/// declared rate would pitch the voice wrong and speed it up; the mismatch is an error.
#[tokio::test]
async fn speech_reports_a_container_rate_that_contradicts_the_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(audio_response(
            "audio/wav",
            24_000,
            &wav(&tone(440.0, 16_000, 10), 16_000),
        )))
        .mount(&server)
        .await;
    let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", None);
    let err = p.synthesize(speech_request("Ozú.", BRIEF, None)).await.unwrap_err();

    let ProviderError::BadResponse(message) = &err else { panic!("expected BadResponse, got {err:?}") };
    assert!(message.contains("24000") && message.contains("16000"), "both rates must appear: {message}");
}

/// Review Focus 1 for the speech path: same mapping as chat, same never-empty message.
#[tokio::test]
async fn speech_maps_auth_rate_limit_and_server_errors_with_the_provider_message() {
    let cases = [
        (
            401u16,
            "authentication",
            "API key not valid. Please pass a valid API key.",
            ProviderError::Auth("API key not valid. Please pass a valid API key.".to_string()),
        ),
        (
            429u16,
            "rate_limit_exceeded",
            "Quota exceeded for quota metric 'generate requests'.",
            ProviderError::RateLimited("Quota exceeded for quota metric 'generate requests'.".to_string()),
        ),
        (
            500u16,
            "api_error",
            "Internal error encountered.",
            ProviderError::BadResponse("HTTP 500: Internal error encountered.".to_string()),
        ),
    ];
    for (status, code, message, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/interactions"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string(json!({"error": {"code": code, "message": message}}).to_string()),
            )
            .mount(&server)
            .await;
        let p = GeminiSpeech::new(server.uri(), "gemini-3.8-flash-tts", Some("nope".into()));
        let err = p.synthesize(speech_request("Ozú.", "", None)).await.unwrap_err();
        assert_eq!(err, expected, "status {status}");

        let carried = match &err {
            ProviderError::Auth(m) | ProviderError::RateLimited(m) | ProviderError::BadResponse(m) => m,
            other => panic!("status {status} produced {other:?}"),
        };
        assert!(!carried.trim().is_empty(), "status {status} carried an empty message");
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "status {status} was retried");
    }
}
