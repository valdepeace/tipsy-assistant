//! Cloud provider abstractions: one trait per capability.

pub mod openai_compat;
pub mod sse;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tipsy_protocol::ErrorCode;

use crate::audio::pcm::Pcm;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeechRequest {
    pub text: String,
    /// Delivery instructions (the persona's `voice_instructions`); ignored by providers without support.
    pub instructions: String,
    pub voice: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProviderError {
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("timed out")]
    Timeout,
    #[error("network error: {0}")]
    Network(String),
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("cancelled")]
    Cancelled,
}

impl ProviderError {
    /// Maps an HTTP error status and its body to an error, extracting `error.message` from a JSON
    /// body when present.
    pub fn from_status(status: u16, body: &str) -> Self {
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .or_else(|| v.pointer("/message"))
                    .or_else(|| v.pointer("/detail/message"))
                    .and_then(|m| m.as_str().map(str::to_string))
            })
            .unwrap_or_else(|| {
                let trimmed: String = body.chars().take(300).collect();
                if trimmed.trim().is_empty() { format!("HTTP {status}") } else { trimmed }
            });
        match status {
            401 | 403 => ProviderError::Auth(message),
            429 => ProviderError::RateLimited(message),
            408 | 504 => ProviderError::Timeout,
            _ => ProviderError::BadResponse(format!("HTTP {status}: {message}")),
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            ProviderError::Auth(_) => ErrorCode::Auth,
            ProviderError::RateLimited(_) => ErrorCode::RateLimited,
            ProviderError::Timeout => ErrorCode::Timeout,
            ProviderError::Network(_) => ErrorCode::Network,
            ProviderError::BadResponse(_) => ErrorCode::BadResponse,
            ProviderError::Cancelled => ErrorCode::Internal,
        }
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            ProviderError::Timeout
        } else {
            ProviderError::Network(e.to_string())
        }
    }
}

pub type TextStream = futures::stream::BoxStream<'static, Result<String, ProviderError>>;

#[async_trait]
pub trait ChatProvider: Send + Sync {
    /// Streams the reply text in pieces as the model produces it.
    async fn stream(&self, req: ChatRequest) -> Result<TextStream, ProviderError>;
}

#[async_trait]
pub trait SpeechProvider: Send + Sync {
    async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError>;
    /// How many sentences may be synthesised at the same time.
    fn max_parallel(&self) -> usize {
        4
    }
}

#[async_trait]
pub trait TranscribeProvider: Send + Sync {
    /// `language` is a BCP 47 tag such as `es-ES`; providers may use only the primary subtag.
    async fn transcribe(&self, pcm: Pcm, language: &str) -> Result<String, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_extracts_messages() {
        assert_eq!(
            ProviderError::from_status(401, r#"{"error":{"message":"Missing Authentication header","code":401}}"#),
            ProviderError::Auth("Missing Authentication header".into())
        );
        assert_eq!(
            ProviderError::from_status(429, r#"{"error":{"message":"Provider returned error"}}"#),
            ProviderError::RateLimited("Provider returned error".into())
        );
        assert_eq!(ProviderError::from_status(403, "forbidden"), ProviderError::Auth("forbidden".into()));
        assert_eq!(ProviderError::from_status(504, ""), ProviderError::Timeout);
        assert_eq!(
            ProviderError::from_status(500, ""),
            ProviderError::BadResponse("HTTP 500: HTTP 500".into())
        );
        assert_eq!(
            ProviderError::from_status(400, r#"{"message":"Gemini TTS only supports pcm"}"#),
            ProviderError::BadResponse("HTTP 400: Gemini TTS only supports pcm".into())
        );
    }

    #[test]
    fn long_non_json_bodies_are_truncated_on_char_boundaries() {
        let body = "ñ".repeat(1000);
        match ProviderError::from_status(502, &body) {
            ProviderError::BadResponse(m) => assert_eq!(m.chars().filter(|c| *c == 'ñ').count(), 300),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn error_codes() {
        assert_eq!(ProviderError::Timeout.code(), ErrorCode::Timeout);
        assert_eq!(ProviderError::Auth("x".into()).code(), ErrorCode::Auth);
    }
}
