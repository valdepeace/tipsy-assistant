//! Connector tests for the native Gemini API (spec §13: `wiremock` with recorded responses).

use futures::StreamExt;
use serde_json::{json, Value};
use tipsy_compadre::providers::gemini::GeminiChat;
use tipsy_compadre::providers::{ChatMessage, ChatProvider, ChatRequest, ProviderError};
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
            Err(ProviderError::BadResponse(
                "Failed to get completed interaction: Result not found.".to_string()
            )),
        ]
    );
    assert!(out.last().unwrap().is_err(), "the error must end the reply, not complete it");
}
