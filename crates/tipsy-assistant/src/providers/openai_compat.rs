//! OpenAI-compatible connector (OpenRouter, Ollama, LM Studio, Groq, OpenAI…).

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;

use super::sse::{SseItem, SseParser};
use super::{ChatProvider, ChatRequest, ProviderError, SpeechProvider, SpeechRequest, TextStream, TranscribeProvider};
use crate::audio::pcm::{Pcm, decode_audio, encode_wav_16k, pcm_s16le_to_pcm};

const CHAT_TIMEOUT: Duration = Duration::from_secs(30);
const SPEECH_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(60);

pub struct OpenAiCompat {
    client: reqwest::Client,
    base_url: String,
    model: String,
    key: Option<String>,
}

impl OpenAiCompat {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: Option<String>) -> Self {
        let base_url: String = base_url.into();
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.into(),
            key,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path)
    }

    fn authed(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.key {
            Some(k) => rb.bearer_auth(k),
            None => rb,
        }
    }

    /// OpenRouter and Ollama accept `reasoning_effort: "none"`, which skips slow "thinking".
    fn wants_no_reasoning(&self) -> bool {
        self.base_url.contains("openrouter.ai") || self.base_url.contains(":11434")
    }

    async fn check(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(ProviderError::from_status(status.as_u16(), &body))
    }
}

#[async_trait]
impl ChatProvider for OpenAiCompat {
    async fn stream(&self, req: ChatRequest) -> Result<TextStream, ProviderError> {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": req.messages,
            "stream": true,
        });
        if self.wants_no_reasoning() {
            body["reasoning_effort"] = serde_json::json!("none");
        }
        let send = self.authed(self.client.post(self.url("chat/completions"))).json(&body).send();
        let resp = tokio::time::timeout(CHAT_TIMEOUT, send).await.map_err(|_| ProviderError::Timeout)??;
        let resp = Self::check(resp).await?;

        let mut parser = SseParser::new();
        let mut finished = false;
        let stream = resp
            .bytes_stream()
            .map(move |chunk| -> Vec<Result<String, ProviderError>> {
                if finished {
                    return Vec::new();
                }
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => return vec![Err(ProviderError::from(e))],
                };
                let mut out = Vec::new();
                for item in parser.push(&bytes) {
                    match item {
                        SseItem::Delta(t) => out.push(Ok(t)),
                        SseItem::Done => {
                            finished = true;
                            break;
                        }
                        SseItem::Error(m) => {
                            finished = true;
                            out.push(Err(ProviderError::BadResponse(m)));
                            break;
                        }
                    }
                }
                out
            })
            .flat_map(futures::stream::iter);
        Ok(stream.boxed())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechFormat {
    Mp3,
    /// Raw signed 16-bit little-endian mono at 24 kHz (Gemini only accepts this).
    Pcm24k,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechQuirks {
    pub format: SpeechFormat,
    pub instructions: bool,
}

pub fn speech_quirks(model: &str) -> SpeechQuirks {
    if model.starts_with("google/") {
        SpeechQuirks { format: SpeechFormat::Pcm24k, instructions: true }
    } else if model.starts_with("openai/") {
        SpeechQuirks { format: SpeechFormat::Mp3, instructions: true }
    } else {
        SpeechQuirks { format: SpeechFormat::Mp3, instructions: false }
    }
}

#[async_trait]
impl SpeechProvider for OpenAiCompat {
    async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError> {
        let quirks = speech_quirks(&self.model);
        let mut body = serde_json::json!({
            "model": self.model,
            "input": req.text,
            "response_format": match quirks.format { SpeechFormat::Mp3 => "mp3", SpeechFormat::Pcm24k => "pcm" },
        });
        if let Some(voice) = &req.voice {
            body["voice"] = serde_json::json!(voice);
        }
        if quirks.instructions && !req.instructions.is_empty() {
            body["instructions"] = serde_json::json!(req.instructions);
        }
        let resp =
            self.authed(self.client.post(self.url("audio/speech"))).timeout(SPEECH_TIMEOUT).json(&body).send().await?;
        let bytes = Self::check(resp).await?.bytes().await?;
        match quirks.format {
            SpeechFormat::Pcm24k => Ok(pcm_s16le_to_pcm(&bytes, 24000)),
            SpeechFormat::Mp3 => decode_audio(bytes.to_vec()).map_err(ProviderError::BadResponse),
        }
    }
}

#[async_trait]
impl TranscribeProvider for OpenAiCompat {
    async fn transcribe(&self, pcm: Pcm, language: &str) -> Result<String, ProviderError> {
        let primary = language.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        let file = reqwest::multipart::Part::bytes(encode_wav_16k(&pcm))
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| ProviderError::BadResponse(e.to_string()))?;
        let mut form = reqwest::multipart::Form::new().text("model", self.model.clone()).part("file", file);
        if !primary.is_empty() {
            form = form.text("language", primary);
        }
        let resp = self
            .authed(self.client.post(self.url("audio/transcriptions")))
            .timeout(TRANSCRIBE_TIMEOUT)
            .multipart(form)
            .send()
            .await?;
        let value: serde_json::Value = Self::check(resp).await?.json().await?;
        value
            .get("text")
            .and_then(|t| t.as_str())
            .map(|t| t.trim().to_string())
            .ok_or_else(|| ProviderError::BadResponse(format!("no text in transcription response: {value}")))
    }
}
