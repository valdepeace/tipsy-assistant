//! `wiremock` tests for the native ElevenLabs connector: speech synthesis (Task 3) and Scribe
//! transcription (Task 4).
//!
//! Every endpoint, header, field name and format string here comes from
//! `.superpowers/sdd/plan-2-research/elevenlabs-api.md`.

use tipsy_assistant::audio::pcm::Pcm;
use tipsy_assistant::providers::elevenlabs::{ElevenLabsSpeech, ElevenLabsTranscribe};
use tipsy_assistant::providers::{ProviderError, SpeechProvider, SpeechRequest, TranscribeProvider};
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// The synthesis model: the note's explicit realtime recommendation.
const MODEL: &str = "eleven_flash_v2_5";

/// What the fixture account's `GET /v1/voices` returns. The note records the endpoint and its
/// `xi-api-key` header but **not** the response body, so the two field names read here are the
/// ones these tests pin.
const VOICE_LIST: &str = r#"{"voices":[
    {"voice_id":"fixture-voice-one","name":"Fixture One","category":"premade"},
    {"voice_id":"fixture-voice-george","name":"George","category":"premade"}
]}"#;

const FIRST_VOICE_ID: &str = "fixture-voice-one";
const GEORGE_VOICE_ID: &str = "fixture-voice-george";

fn provider(server: &MockServer) -> ElevenLabsSpeech {
    // The key reaches the connector already resolved; it only ever travels in `xi-api-key`, and
    // `"k"` is a fixture value, not a key.
    ElevenLabsSpeech::new(server.uri(), MODEL, Some("k".into()))
}

fn request(voice: Option<&str>) -> SpeechRequest {
    SpeechRequest {
        text: "Ozú, mi arma".into(),
        instructions: "voz andaluza y achispada".into(),
        voice: voice.map(str::to_string),
    }
}

fn request_with_text(text: &str, voice: Option<&str>) -> SpeechRequest {
    SpeechRequest { text: text.into(), instructions: "voz andaluza y achispada".into(), voice: voice.map(str::to_string) }
}

async fn mount_voice_list(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/voices"))
        .and(header("xi-api-key", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_string(VOICE_LIST))
        .mount(server)
        .await;
}

/// The synthesis call for one already-resolved voice id, answered with two PCM samples:
/// `00 40` = +0.5 and `00 C0` = −0.5 as signed 16-bit little-endian.
async fn mount_synthesis(server: &MockServer, resolved_id: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/v1/text-to-speech/{resolved_id}")))
        .and(query_param("output_format", "pcm_24000"))
        .and(header("xi-api-key", "k"))
        .and(body_partial_json(serde_json::json!({"text": "Ozú, mi arma", "model_id": MODEL})))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x00, 0x40, 0x00, 0xC0]))
        .mount(server)
        .await;
}

async fn requests(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap()
}

fn paths(reqs: &[Request]) -> Vec<String> {
    reqs.iter().map(|r| r.url.path().to_string()).collect()
}

async fn count(server: &MockServer, path: &str) -> usize {
    requests(server).await.iter().filter(|r| r.url.path() == path).count()
}

/// A `voices` entry that already is a `voice_id` is used verbatim in the path, and the request
/// carries exactly the two documented body fields with `output_format=pcm_24000` — no
/// `instructions` (the note records no general style field), no `language_code` (`SpeechRequest`
/// carries none) and no `enable_logging` (the note marks zero retention Enterprise-only).
#[tokio::test]
async fn speech_sends_pcm_24000_and_uses_a_voice_id_verbatim() {
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    mount_synthesis(&server, GEORGE_VOICE_ID).await;

    let provider = provider(&server);
    let pcm = provider.synthesize(request(Some(GEORGE_VOICE_ID))).await.unwrap();

    assert_eq!(pcm.sample_rate, 24_000);
    assert_eq!(pcm.samples.len(), 2);
    assert!((pcm.samples[0] - 0.5).abs() < 0.001 && (pcm.samples[1] + 0.5).abs() < 0.001);
    assert_eq!(
        provider.max_parallel(),
        4,
        "the trait default matches the note's Free-tier Flash concurrency (4)"
    );

    let reqs = requests(&server).await;
    let got = paths(&reqs);
    assert_eq!(got, vec!["/v1/voices", "/v1/text-to-speech/fixture-voice-george"], "{got:?}");
    let post = reqs.iter().find(|r| r.method == "POST").unwrap();
    let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(body, serde_json::json!({"text": "Ozú, mi arma", "model_id": MODEL}));
}

/// El Compae ships `elevenlabs = "george"` — the stock voice's *name*. Sending the name in the
/// path would 404 on the first sentence, so the entry is resolved against the account's voices
/// (name match, case-insensitive) and the list is still fetched only once.
#[tokio::test]
async fn speech_resolves_a_voice_name_to_its_id() {
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    mount_synthesis(&server, GEORGE_VOICE_ID).await;

    let provider = provider(&server);
    provider.synthesize(request(Some("George"))).await.unwrap();
    provider.synthesize(request(Some("george"))).await.unwrap();

    let reqs = requests(&server).await;
    assert_eq!(count(&server, "/v1/text-to-speech/fixture-voice-george").await, 2, "{:?}", paths(&reqs));
    assert_eq!(count(&server, "/v1/voices").await, 1, "fetched once per connector: {:?}", paths(&reqs));
}

/// Review Focus 5 — a profile with no `elevenlabs` entry still speaks: the connector asks the
/// account once and reuses its first voice, including when several sentences are in flight at the
/// same time.
#[tokio::test]
async fn speech_without_a_voice_entry_uses_the_first_voice_and_fetches_the_list_once() {
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    mount_synthesis(&server, FIRST_VOICE_ID).await;

    let provider = provider(&server);
    let first = provider.synthesize(request(None)).await.unwrap();
    let (second, third) = tokio::join!(provider.synthesize(request(None)), provider.synthesize(request(None)));

    assert_eq!(first.sample_rate, 24_000);
    assert_eq!(first.samples.len(), 2);
    assert!(second.is_ok() && third.is_ok(), "{second:?} {third:?}");

    let reqs = requests(&server).await;
    assert_eq!(count(&server, "/v1/voices").await, 1, "one voice list for the whole connector: {:?}", paths(&reqs));
    assert_eq!(count(&server, "/v1/text-to-speech/fixture-voice-one").await, 3, "{:?}", paths(&reqs));
}

/// A value that matches no `voice_id` and no `name` — a stale id, a name from another account —
/// falls back to the account's first voice instead of failing every sentence, and still fetches
/// the list once.
#[tokio::test]
async fn speech_with_an_unknown_voice_falls_back_to_the_first_voice() {
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    mount_synthesis(&server, FIRST_VOICE_ID).await;

    let provider = provider(&server);
    provider.synthesize(request(Some("no-such-voice"))).await.unwrap();
    provider.synthesize(request(Some("no-such-voice"))).await.unwrap();

    let reqs = requests(&server).await;
    assert_eq!(count(&server, "/v1/text-to-speech/fixture-voice-one").await, 2, "{:?}", paths(&reqs));
    assert_eq!(count(&server, "/v1/voices").await, 1, "{:?}", paths(&reqs));
}

/// The note records `GET /v1/voices` but not the shape of its body, so a body this connector
/// cannot read is a `BadResponse` with a readable message — never a panic, and never a synthesis
/// request carrying an empty voice id.
#[tokio::test]
async fn speech_reports_a_voice_list_it_cannot_use() {
    for body in [r#"{"voices":[]}"#, r#"{"detail":{"message":"no voices for this key"}}"#] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/voices"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let err = provider(&server).synthesize(request(None)).await.unwrap_err();
        assert!(
            matches!(&err, ProviderError::BadResponse(m) if !m.trim().is_empty()),
            "{body} produced {err:?}"
        );
        assert_eq!(requests(&server).await.len(), 1, "{body} must not reach synthesis");
    }
}

/// Review Focus 1 — a non-2xx response reaches the user as a typed error carrying the API's own
/// body, never an empty message and never a panic. The note documents the 422 body as `detail[]`
/// and does not record the 401/429 bodies, so those two are pinned through the `detail.message`
/// shape `ProviderError::from_status` already reads.
#[tokio::test]
async fn speech_maps_api_errors_to_typed_errors() {
    let cases: [(u16, &str, &str); 3] = [
        (401, r#"{"detail":{"message":"invalid api key"}}"#, "invalid api key"),
        (429, r#"{"detail":{"message":"rate limit exceeded"}}"#, "rate limit exceeded"),
        (
            422,
            r#"{"detail":[{"type":"invalid_request","code":"voice_not_found","msg":"voice not found"}]}"#,
            "voice not found",
        ),
    ];
    for (status, body, provider_text) in cases {
        let server = MockServer::start().await;
        mount_voice_list(&server).await;
        Mock::given(method("POST"))
            .and(path("/v1/text-to-speech/fixture-voice-george"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;

        let err = provider(&server).synthesize(request(Some(GEORGE_VOICE_ID))).await.unwrap_err();
        let right_variant = match status {
            401 => matches!(&err, ProviderError::Auth(_)),
            429 => matches!(&err, ProviderError::RateLimited(_)),
            _ => matches!(&err, ProviderError::BadResponse(_)),
        };
        assert!(right_variant, "status {status} produced {err:?}");
        let carried = match &err {
            ProviderError::Auth(m) | ProviderError::RateLimited(m) | ProviderError::BadResponse(m) => m,
            other => panic!("status {status} produced {other:?}"),
        };
        assert!(!carried.trim().is_empty(), "status {status} carried an empty message");
        assert!(err.to_string().contains(provider_text), "status {status}: {err}");
    }
}

/// Review Focus 3 — the requested format is raw `pcm_24000`, which announces nothing about
/// itself, so the connector rejects what does announce something else: a JSON body served with
/// 200, a WAV container, a body too short to hold one sample. The note marks the PCM bit depth,
/// signedness and channel count for TTS output **unconfirmed**, which is why this check exists.
#[tokio::test]
async fn speech_rejects_bodies_that_are_not_pcm_24000() {
    let wav = tipsy_assistant::audio::pcm::encode_wav_16k(&Pcm { samples: vec![0.0; 1600], sample_rate: 16_000 });
    let cases: [(&str, Vec<u8>, &str); 3] = [
        ("application/json", br#"{"detail":[{"msg":"voice not found"}]}"#.to_vec(), "voice not found"),
        ("audio/wav", wav, "RIFF"),
        ("application/octet-stream", vec![0x00], "byte"),
    ];
    for (content_type, body, expected) in cases {
        let server = MockServer::start().await;
        mount_voice_list(&server).await;
        Mock::given(method("POST"))
            .and(path("/v1/text-to-speech/fixture-voice-george"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, content_type))
            .mount(&server)
            .await;

        let err = provider(&server).synthesize(request(Some(GEORGE_VOICE_ID))).await.unwrap_err();
        match &err {
            ProviderError::BadResponse(m) => {
                assert!(!m.trim().is_empty(), "{content_type} carried an empty message");
                assert!(m.contains(expected), "{content_type}: {m}");
            }
            other => panic!("{content_type} produced {other:?}"),
        }
    }
}

/// The same guard from two more directions: raw PCM announces nothing, so a payload that starts
/// with `[` (a JSON array) or with a UTF-8 BOM in front of an object is still JSON and must be
/// refused rather than played as noise. The `content-type` is the other announcement the API
/// makes, and a 200 that calls itself JSON is refused even when its bytes would pass the byte
/// checks.
#[tokio::test]
async fn speech_rejects_json_arrays_boms_and_a_json_content_type() {
    let bom = [b"\xEF\xBB\xBF".as_slice(), br#"{"detail":[{"msg":"voice not found"}]}"#.as_slice()].concat();
    let pcm = vec![0x00, 0x40, 0x00, 0xC0];
    let cases: [(&str, Vec<u8>, &str); 3] = [
        ("application/json", br#"[{"msg":"voice not found"}]"#.to_vec(), "voice not found"),
        ("application/json", bom, "voice not found"),
        // Well-formed raw PCM bytes, but the header says JSON: only the content-type check can
        // catch this one, so it proves that check is not dead code.
        ("application/json", pcm, "JSON"),
    ];
    for (content_type, body, expected) in cases {
        let server = MockServer::start().await;
        mount_voice_list(&server).await;
        Mock::given(method("POST"))
            .and(path("/v1/text-to-speech/fixture-voice-george"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body.clone(), content_type))
            .mount(&server)
            .await;

        let err = provider(&server).synthesize(request(Some(GEORGE_VOICE_ID))).await.unwrap_err();
        match &err {
            ProviderError::BadResponse(m) => {
                assert!(!m.trim().is_empty(), "{content_type} carried an empty message");
                assert!(m.contains(expected), "{content_type}, body {body:?}: {m}");
            }
            other => panic!("{content_type}, body {body:?} produced {other:?}"),
        }
    }
}

/// A `GET /v1/voices` that fails must be remembered for the turn: several sentences are
/// synthesised in parallel (up to four), and without a negative cache each one re-lists the
/// voices behind the same mutex, turning one flaky call into four extra requests and a brain
/// timeout instead of the list's own error. The count is the assertion: exactly one request.
#[tokio::test]
async fn speech_failing_voice_list_is_fetched_once_for_parallel_sentences() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/voices"))
        .respond_with(ResponseTemplate::new(500).set_body_string(r#"{"detail":[{"msg":"voices are unwell"}]}"#))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let (a, b, c) = tokio::join!(
        provider.synthesize(request(None)),
        provider.synthesize(request(None)),
        provider.synthesize(request(None))
    );

    for (label, result) in [("first", a), ("second", b), ("third", c)] {
        let err = result.expect_err(&format!("{label} sentence must report the list's failure"));
        assert!(
            matches!(&err, ProviderError::BadResponse(m) if m.contains("voices are unwell")),
            "{label} sentence produced {err:?}"
        );
    }
    assert_eq!(
        count(&server, "/v1/voices").await,
        1,
        "a failed list must be shared by the turn, not retried per sentence: {:?}",
        paths(&requests(&server).await)
    );
}

/// Review Focus 2 for this connector: the audio is raw bytes and `reqwest` hands `serde_json`
/// whole bodies, so no parse can be split mid-character here — the UTF-8 risk lives in the
/// request body and in the preview the connector builds from a body it cannot use. Both are
/// pinned: multi-byte text survives the round trip, and a long multi-byte body that is not JSON
/// is truncated by `chars()`, never by bytes.
#[tokio::test]
async fn speech_keeps_multi_byte_text_and_error_previews_intact() {
    let sentence = "Ozú, ¿qué pasa? ¡Mi arma! 😅";
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/text-to-speech/fixture-voice-one"))
        .and(body_partial_json(serde_json::json!({"text": sentence, "model_id": MODEL})))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x00, 0x40]))
        .mount(&server)
        .await;

    provider(&server).synthesize(request_with_text(sentence, None)).await.unwrap();
    let reqs = requests(&server).await;
    let post = reqs.iter().find(|r| r.method == "POST").unwrap();
    let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(body["text"], sentence);

    // A 1000-character multi-byte body that is not JSON: the message must hold 300 `ñ`, not a
    // byte slice that panics or corrupts.
    let server = MockServer::start().await;
    mount_voice_list(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/text-to-speech/fixture-voice-one"))
        .respond_with(ResponseTemplate::new(502).set_body_string("ñ".repeat(1000)))
        .mount(&server)
        .await;
    let err = provider(&server).synthesize(request_with_text("hola", None)).await.unwrap_err();
    match err {
        ProviderError::BadResponse(m) => assert_eq!(m.chars().filter(|c| *c == 'ñ').count(), 300, "{m}"),
        other => panic!("unexpected {other:?}"),
    }
}

/// A transcript in the note's response shape, with the whole `words[]` array it documents.
fn transcript_body() -> serde_json::Value {
    serde_json::json!({
        "language_code": "es",
        "language_probability": 0.98,
        "text": "  ¿y el ticket de Jira?  ",
        "words": [
            {"text": "¿y", "start": 0.0, "end": 0.1, "type": "word", "logprob": -0.124, "speaker_id": "speaker_1"},
            {"text": " ", "start": 0.1, "end": 0.2, "type": "spacing"},
            {"text": "el", "start": 0.2, "end": 0.3, "type": "word", "logprob": -0.31}
        ],
        "audio_duration_secs": 1.4,
        "transcription_id": "fixture-transcription"
    })
}

fn transcribe_model() -> &'static str {
    "scribe_v2"
}

fn upload() -> Pcm {
    // 100 ms of a 440 Hz tone at 48 kHz: `encode_wav_16k` resamples it to 16 kHz.
    let samples: Vec<f32> =
        (0..4800).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5).collect();
    Pcm { samples, sample_rate: 48_000 }
}

/// Scribe's multipart fields, and the `language_code` hint as the primary subtag of the profile's
/// BCP 47 tag (`es-ES` → `es`, the plan's decision). The multipart body is a real form: the
/// `file` part is a WAV that `encode_wav_16k` wrote at 16 kHz, checked from the raw body.
#[tokio::test]
async fn transcription_uploads_a_wav_with_the_primary_subtag() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/speech-to-text"))
        .and(header("xi-api-key", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_json(transcript_body()))
        .mount(&server)
        .await;

    let provider = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()));
    assert_eq!(provider.transcribe(upload(), "es-ES").await.unwrap(), "¿y el ticket de Jira?");

    let reqs = requests(&server).await;
    let req = &reqs[0];
    let content_type = req.headers.get("content-type").unwrap().to_str().unwrap().to_string();
    assert!(content_type.starts_with("multipart/form-data"), "{content_type}");
    let form = String::from_utf8_lossy(&req.body).to_string();
    assert!(form.contains("name=\"model_id\"") && form.contains("scribe_v2"), "{form}");
    assert!(form.contains("name=\"language_code\"") && form.contains("\r\n\r\nes\r\n"), "{form}");
    assert!(form.contains("filename=\"audio.wav\"") && form.contains("RIFF"), "{form}");
    assert!(
        req.body.windows(4).any(|w| w == [0x80, 0x3e, 0x00, 0x00]),
        "the upload must be 16 kHz (16000 = 0x3E80 little-endian)"
    );
}

/// The hint is the primary subtag only, lowercased; a profile that declares no language sends no
/// `language_code` at all, because the note says omitted means auto-detect.
#[tokio::test]
async fn transcription_language_hint_is_the_primary_subtag_or_omitted() {
    let cases: [(&str, Option<&str>); 4] =
        [("es-ES", Some("es")), ("ES_419", Some("es")), ("EN", Some("en")), ("", None)];
    for (language, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/speech-to-text"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"text": "hola"})))
            .mount(&server)
            .await;

        let provider = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()));
        assert_eq!(provider.transcribe(upload(), language).await.unwrap(), "hola");
        let form = String::from_utf8_lossy(&requests(&server).await[0].body).to_string();
        match expected {
            Some(tag) => assert!(form.contains(&format!("\r\n\r\n{tag}\r\n")), "{language}: {form}"),
            None => assert!(!form.contains("name=\"language_code\""), "{language}: {form}"),
        }
    }
}

/// Review Focus 1 for the Scribe path: typed error, the API's own text, never empty.
#[tokio::test]
async fn transcription_maps_api_errors_to_typed_errors() {
    let cases: [(u16, &str, &str); 3] = [
        (401, r#"{"detail":{"message":"invalid api key"}}"#, "invalid api key"),
        (429, r#"{"detail":{"message":"rate limit exceeded"}}"#, "rate limit exceeded"),
        (500, r#"{"detail":[{"type":"server_error","code":"boom","msg":"scribe is unwell"}]}"#, "scribe is unwell"),
    ];
    for (status, body, provider_text) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/speech-to-text"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;

        let err = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()))
            .transcribe(upload(), "es-ES")
            .await
            .unwrap_err();
        let right_variant = match status {
            401 => matches!(&err, ProviderError::Auth(_)),
            429 => matches!(&err, ProviderError::RateLimited(_)),
            _ => matches!(&err, ProviderError::BadResponse(_)),
        };
        assert!(right_variant, "status {status} produced {err:?}");
        assert!(err.to_string().contains(provider_text), "status {status}: {err}");
    }
}

/// A 200 without a readable `text` is `BadResponse` with a readable message — never an empty
/// transcript handed to the brain as if it were silence.
#[tokio::test]
async fn transcription_rejects_responses_without_text() {
    for body in [serde_json::json!({"language_code": "es", "words": []}), serde_json::json!({"text": 42})] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/speech-to-text"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
            .mount(&server)
            .await;

        let err = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()))
            .transcribe(upload(), "es-ES")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ProviderError::BadResponse(m) if !m.trim().is_empty() && m.contains("text")),
            "{body} produced {err:?}"
        );
    }
}

/// Review Focus 2 inside the JSON parse: accents, inverted punctuation and an emoji survive the
/// round trip, and a long multi-byte error body is truncated by `chars()` (300 characters, the
/// `ProviderError::from_status` rule), never by bytes.
#[tokio::test]
async fn transcription_keeps_multi_byte_text_intact() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/speech-to-text"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"text": "  ¿Y el ticket? ¡Ozú, mi arma! 😅  "})),
        )
        .mount(&server)
        .await;
    let provider = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()));
    assert_eq!(provider.transcribe(upload(), "es-ES").await.unwrap(), "¿Y el ticket? ¡Ozú, mi arma! 😅");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/speech-to-text"))
        .respond_with(ResponseTemplate::new(400).set_body_string("ñ".repeat(1000)))
        .mount(&server)
        .await;
    let err = ElevenLabsTranscribe::new(server.uri(), transcribe_model(), Some("k".into()))
        .transcribe(upload(), "es-ES")
        .await
        .unwrap_err();
    match err {
        ProviderError::BadResponse(m) => assert_eq!(m.chars().filter(|c| *c == 'ñ').count(), 300, "{m}"),
        other => panic!("unexpected {other:?}"),
    }
}
