use futures::StreamExt;
use tipsy_compadre::providers::openai_compat::OpenAiCompat;
use tipsy_compadre::providers::{ChatMessage, ChatProvider, ChatRequest, ProviderError};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse_body(parts: &[&str]) -> String {
    let mut s = String::new();
    for p in parts {
        s.push_str(&format!("data: {}\n\n", serde_json::json!({"choices": [{"delta": {"content": p}}]})));
    }
    s.push_str("data: [DONE]\n\n");
    s
}

async fn collect(provider: &OpenAiCompat) -> Result<Vec<String>, ProviderError> {
    let req = ChatRequest { messages: vec![ChatMessage::system("sys"), ChatMessage::user("hola")] };
    let mut stream = provider.stream(req).await?;
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        out.push(item?);
    }
    Ok(out)
}

#[tokio::test]
async fn chat_streams_deltas() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer k123"))
        .and(body_partial_json(serde_json::json!({
            "model": "m", "stream": true,
            "messages": [{"role": "system", "content": "sys"}, {"role": "user", "content": "hola"}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse_body(&["Ozú", ", quillo."]), "text/event-stream"))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", Some("k123".into()));
    assert_eq!(collect(&p).await.unwrap(), vec!["Ozú".to_string(), ", quillo.".to_string()]);
}

#[tokio::test]
async fn chat_maps_http_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"error":{"message":"bad key"}}"#))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", Some("nope".into()));
    assert_eq!(collect(&p).await.unwrap_err(), ProviderError::Auth("bad key".into()));
}

#[tokio::test]
async fn chat_surfaces_errors_inside_the_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hola\"}}]}\n\ndata: {\"error\":{\"message\":\"boom\"}}\n\n",
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", None);
    let err = collect(&p).await.unwrap_err();
    assert_eq!(err, ProviderError::BadResponse("boom".into()));
}

/// Review focus 1: 403, 429 and 5xx with a JSON body keep the provider's own message (never empty).
#[tokio::test]
async fn chat_maps_auth_rate_limit_and_server_errors_with_the_provider_message() {
    let cases = [
        (403u16, "forbidden", ProviderError::Auth("forbidden".to_string())),
        (429u16, "slow down", ProviderError::RateLimited("slow down".to_string())),
        (500u16, "boom", ProviderError::BadResponse("HTTP 500: boom".to_string())),
    ];
    for (status, message, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string(serde_json::json!({"error": {"message": message}}).to_string()),
            )
            .mount(&server)
            .await;
        let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", Some("k".into()));
        let err = collect(&p).await.unwrap_err();
        assert_eq!(err, expected, "status {status}");
        let carried = match &err {
            ProviderError::Auth(m) | ProviderError::RateLimited(m) | ProviderError::BadResponse(m) => m,
            other => panic!("status {status} produced {other:?}"),
        };
        assert!(!carried.trim().is_empty(), "status {status} carried an empty message");
    }
}

/// A stream that ends without a final newline and without `[DONE]` ends cleanly: the complete
/// event is delivered, the unterminated tail is dropped, no panic and no error.
#[tokio::test]
async fn truncated_stream_without_final_newline_or_done_finishes_cleanly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Ozú\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\" y ya\"}}]}",
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", None);
    assert_eq!(collect(&p).await.unwrap(), vec!["Ozú".to_string()]);
}

#[tokio::test]
async fn trailing_slash_in_base_url_is_ignored() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse_body(&["ok"]), "text/event-stream"))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1/", server.uri()), "m", None);
    assert_eq!(collect(&p).await.unwrap(), vec!["ok".to_string()]);
}
