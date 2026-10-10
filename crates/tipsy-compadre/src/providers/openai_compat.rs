//! OpenAI-compatible connector (OpenRouter, Ollama, LM Studio, Groq, OpenAI…).

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;

use super::sse::{SseItem, SseParser};
use super::{ChatProvider, ChatRequest, ProviderError, TextStream};

const CHAT_TIMEOUT: Duration = Duration::from_secs(30);

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
