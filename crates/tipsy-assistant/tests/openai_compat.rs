use futures::StreamExt;
use tipsy_assistant::audio::pcm::Pcm;
use tipsy_assistant::providers::openai_compat::{OpenAiCompat, SpeechFormat, speech_quirks};
use tipsy_assistant::providers::{
    ChatMessage, ChatProvider, ChatRequest, ProviderError, SpeechProvider, SpeechRequest, TranscribeProvider,
};
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

fn wav_bytes() -> Vec<u8> {
    // 100 ms of a 440 Hz tone at 16 kHz, encoded as WAV (the decoder accepts WAV like MP3).
    let samples: Vec<f32> =
        (0..1600).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 16000.0).sin() * 0.5).collect();
    tipsy_assistant::audio::pcm::encode_wav_16k(&tipsy_assistant::audio::pcm::Pcm { samples, sample_rate: 16000 })
}

#[test]
fn quirks_by_model() {
    assert_eq!(speech_quirks("google/gemini-3.8-flash-tts").format, SpeechFormat::Pcm24k);
    assert!(speech_quirks("google/gemini-3.8-flash-tts").instructions);
    assert_eq!(speech_quirks("openai/gpt-4o-mini-tts").format, SpeechFormat::Mp3);
    assert!(speech_quirks("openai/gpt-4o-mini-tts").instructions);
    assert_eq!(speech_quirks("elevenlabs/eleven-flash-v2.5").format, SpeechFormat::Mp3);
    assert!(!speech_quirks("elevenlabs/eleven-flash-v2.5").instructions);
}

#[tokio::test]
async fn speech_gemini_sends_pcm_format_and_instructions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .and(body_partial_json(serde_json::json!({
            "model": "google/gemini-3.8-flash-tts", "input": "Ozú", "voice": "Charon",
            "instructions": "andaluz", "response_format": "pcm"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x00, 0x40, 0x00, 0xC0]))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "google/gemini-3.8-flash-tts", Some("k".into()));
    let pcm = p
        .synthesize(SpeechRequest { text: "Ozú".into(), instructions: "andaluz".into(), voice: Some("Charon".into()) })
        .await
        .unwrap();
    assert_eq!(pcm.sample_rate, 24000);
    assert_eq!(pcm.samples.len(), 2);
}

#[tokio::test]
async fn speech_mp3_models_decode_and_skip_instructions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .and(body_partial_json(
            serde_json::json!({"model": "elevenlabs/eleven-flash-v2.5", "response_format": "mp3", "voice": "george"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wav_bytes()))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "elevenlabs/eleven-flash-v2.5", None);
    let pcm = p
        .synthesize(SpeechRequest { text: "hola".into(), instructions: "ignored".into(), voice: Some("george".into()) })
        .await
        .unwrap();
    assert_eq!(pcm.sample_rate, 16000);
    assert_eq!(pcm.samples.len(), 1600);
    let received = &server.received_requests().await.unwrap()[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    assert!(body.get("instructions").is_none(), "instructions must not be sent: {body}");
}

#[tokio::test]
async fn speech_without_voice_omits_the_field() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wav_bytes()))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "x/y", None);
    p.synthesize(SpeechRequest { text: "hola".into(), instructions: String::new(), voice: None }).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert!(body.get("voice").is_none());
}

#[tokio::test]
async fn speech_errors_are_mapped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(r#"{"error":{"message":"Gemini TTS only supports response_format=\"pcm\"."}}"#),
        )
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "x/y", None);
    let err =
        p.synthesize(SpeechRequest { text: "a".into(), instructions: String::new(), voice: None }).await.unwrap_err();
    assert!(matches!(err, ProviderError::BadResponse(m) if m.contains("only supports")));
}

#[tokio::test]
async fn transcription_sends_multipart_wav_and_language() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"text": "  ¿y el ticket?  "})))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "openai/whisper-large-v3", Some("k".into()));
    let pcm = Pcm { samples: vec![0.1; 48000], sample_rate: 48000 };
    assert_eq!(p.transcribe(pcm, "es-ES").await.unwrap(), "¿y el ticket?");

    let req = &server.received_requests().await.unwrap()[0];
    let content_type = req.headers.get("content-type").unwrap().to_str().unwrap().to_string();
    assert!(content_type.starts_with("multipart/form-data"), "{content_type}");
    let body = String::from_utf8_lossy(&req.body);
    assert!(body.contains("name=\"model\"") && body.contains("openai/whisper-large-v3"));
    assert!(body.contains("name=\"language\"") && body.contains("\r\n\r\nes\r\n"));
    assert!(body.contains("filename=\"audio.wav\"") && body.contains("RIFF"));
}

#[tokio::test]
async fn transcription_errors_are_mapped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(429).set_body_string(r#"{"error":{"message":"slow down"}}"#))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", None);
    let err = p.transcribe(Pcm { samples: vec![0.0; 160], sample_rate: 16000 }, "en-GB").await.unwrap_err();
    assert_eq!(err, ProviderError::RateLimited("slow down".into()));
}

#[tokio::test]
async fn transcription_rejects_responses_without_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"nope": 1})))
        .mount(&server)
        .await;
    let p = OpenAiCompat::new(format!("{}/v1", server.uri()), "m", None);
    let err = p.transcribe(Pcm { samples: vec![0.0; 160], sample_rate: 16000 }, "es").await.unwrap_err();
    assert!(matches!(err, ProviderError::BadResponse(_)));
}
