//! Server-sent events parsing for streamed chat completions.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseItem {
    /// A piece of reply text (`choices[0].delta.content`).
    Delta(String),
    /// `data: [DONE]`
    Done,
    /// An `{"error": {...}}` payload sent inside the stream.
    Error(String),
}

/// Incremental parser: feed it raw bytes as they arrive, get items back.
#[derive(Default)]
pub struct SseParser {
    pending: Vec<u8>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseItem> {
        self.pending.extend_from_slice(bytes);
        let mut items = Vec::new();
        // Only complete lines are parsed; the tail stays pending (it may split a UTF-8 character).
        while let Some(pos) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim_start();
            if data == "[DONE]" {
                items.push(SseItem::Done);
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else { continue };
            if let Some(msg) = value.pointer("/error/message").and_then(|m| m.as_str()) {
                items.push(SseItem::Error(msg.to_string()));
                continue;
            }
            if let Some(text) = value.pointer("/choices/0/delta/content").and_then(|t| t.as_str()) {
                if !text.is_empty() {
                    items.push(SseItem::Delta(text.to_string()));
                }
            }
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_deltas_done_and_ignores_comments() {
        let mut p = SseParser::new();
        let input = concat!(
            ": OPENROUTER PROCESSING\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Ozú\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\", quillo\"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{}}]}\n\n",
            "data: [DONE]\n\n",
        );
        assert_eq!(p.push(input.as_bytes()), vec![
            SseItem::Delta("Ozú".into()),
            SseItem::Delta(", quillo".into()),
            SseItem::Done,
        ]);
    }

    #[test]
    fn handles_chunks_split_mid_line_and_mid_utf8() {
        let mut p = SseParser::new();
        let line = "data: {\"choices\":[{\"delta\":{\"content\":\"ñandú\"}}]}\n".as_bytes();
        let split = line.iter().position(|b| *b == 0xC3).unwrap() + 1; // inside the 'ñ'
        assert!(p.push(&line[..split]).is_empty());
        assert_eq!(p.push(&line[split..]), vec![SseItem::Delta("ñandú".into())]);
    }

    #[test]
    fn surfaces_in_stream_errors() {
        let mut p = SseParser::new();
        let items = p.push(b"data: {\"error\":{\"message\":\"upstream rate limited\"}}\n");
        assert_eq!(items, vec![SseItem::Error("upstream rate limited".into())]);
    }
}
