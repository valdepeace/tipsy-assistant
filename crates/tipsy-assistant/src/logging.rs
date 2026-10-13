//! `tracing` to a file, `--verbose` also to stderr, and the three things that never reach either
//! (spec §9.2, §9.6, and the v1 spec §11's "logs never contain keys or audio").
//!
//! The rules are enforced *here*, on the way out, and not at each call site: a call site that forgets
//! is exactly the accident this has to survive. Every complete line is scrubbed before it reaches a
//! sink, so a secret split across two `write` calls cannot slip through either.
//!
//! **Both sinks go through the same writer**, which is not a detail: `--verbose` copies the log to
//! stderr, and a rule that only held for the file would be a rule with a hole as wide as
//! `serve --verbose 2> run.log`. The layer that writes a terminal and the layer that writes the file
//! are handed the identical [`ScrubWriter`].

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tracing::Subscriber;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Layer;

/// What a redacted value becomes. One token, so a reader can tell "there was something here" from
/// "there was nothing".
pub const REDACTED: &str = "<redacted>";

/// The most digits a line may carry, comma-separated, before it is treated as audio. One microphone
/// frame is 320 samples (`audio/device.rs:20`), so 32 is far below anything legitimate: this is the
/// backstop behind "no call site logs audio", and `a_sample_dump_is_dropped_whole` is what fires it.
const SAMPLE_RUN: usize = 32;

/// The shortest opaque word treated as a secret by shape. A key is long and random; a URL, a model
/// slug and a path are not (they carry `/`, `.` or `:`), so this does not redact diagnostics.
const OPAQUE: usize = 32;

/// The values that must never appear in a log line: the server's token, and every API key value this
/// process has resolved. `keys::KeyResolver::resolve` is what fills the second half, so a value is
/// registered before anything can log it.
#[derive(Clone, Default)]
pub struct Secrets(Arc<Mutex<Vec<String>>>);

impl Secrets {
    /// Registers a value. Empty and very short values are ignored: redacting `""` or `"a"` would
    /// shred every line it appeared in.
    pub fn add(&self, value: &str) {
        let value = value.trim();
        if value.len() < 8 {
            return;
        }
        let mut secrets = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !secrets.iter().any(|known| known == value) {
            secrets.push(value.to_string());
        }
    }

    /// The three rules, applied to one line.
    pub fn scrub(&self, line: &str) -> String {
        if is_a_sample_dump(line) {
            // Audio is never logged, not even redacted: a line that is a sample dump is dropped
            // whole, so nobody can reconstruct a second of the microphone from the file.
            return format!("{REDACTED}\n");
        }
        let mut scrubbed = line.to_string();
        {
            let secrets = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            for secret in secrets.iter() {
                scrubbed = scrubbed.replace(secret.as_str(), REDACTED);
            }
        }
        // And the shapes that are secrets whether or not this process has seen them yet: the
        // `token=` query parameter of a page request (spec §9.2) and any long opaque word — an
        // `Authorization` header's value, or a key that reached a line before its first resolution.
        scrubbed = redact_query_tokens(&scrubbed);
        redact_opaque_words(&scrubbed)
    }
}

/// The process-wide instance, and the one-line form `keys.rs` calls.
pub fn secrets() -> &'static Secrets {
    static SECRETS: OnceLock<Secrets> = OnceLock::new();
    SECRETS.get_or_init(Secrets::default)
}

/// Registers a value that must never reach the log. Called by `KeyResolver::resolve`, and by `main`
/// for the token it loaded and for the one `--token` named.
pub fn hide(value: &str) {
    secrets().add(value);
}

/// Where the log lives: beside `config.toml`, in the directory `tipsy-assistant config path` prints.
pub fn log_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("tipsy-assistant.log")
}

/// The subscriber: the file always, stderr too when `--verbose` (spec §9.6, v1 spec §12).
///
/// The level is `DEBUG` and never `TRACE`, for a reason that is not a preference: `tokio-tungstenite`
/// logs the request line — headers included — at `trace!`, which is where an `Authorization: Bearer`
/// would go (Plan 2C's review carried exactly that).
pub fn subscriber(log_path: &Path, verbose: bool) -> Result<Box<dyn Subscriber + Send + Sync>, String> {
    subscriber_at(log_path, secrets().clone(), verbose)
}

/// The same, with the secrets a test chose, so the rules can be exercised without the global slot —
/// which a process may take only once.
pub fn subscriber_at(
    log_path: &Path,
    secrets: Secrets,
    verbose: bool,
) -> Result<Box<dyn Subscriber + Send + Sync>, String> {
    if let Some(dir) = log_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| format!("{}: {e}", log_path.display()))?;
    Ok(subscriber_with(file, verbose.then(io::stderr), secrets))
}

/// The two layers, over sinks the caller names: the file always, and the `--verbose` copy when there
/// is one.
///
/// **This is the function that decides a sink is scrubbed**, which is why it takes the sinks as
/// parameters rather than building one of them itself: `subscriber_at` only chooses the file and
/// stderr and hands them here, and a test hands two buffers it can read back. A test that built its
/// own `ScrubWriter` would prove the writer scrubs (true, and already covered) and say nothing at all
/// about whether the layer is handed one — which is exactly the mutation the whole-branch review ran
/// against the earlier version of the test below.
fn subscriber_with<F, S>(file: F, verbose: Option<S>, secrets: Secrets) -> Box<dyn Subscriber + Send + Sync>
where
    F: Write + Send + Sync + 'static,
    S: Write + Send + Sync + 'static,
{
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(Mutex::new(ScrubWriter::new(file, secrets.clone())))
        .with_filter(LevelFilter::DEBUG);
    let base = tracing_subscriber::registry().with(layer);
    match verbose {
        None => Box::new(base),
        Some(sink) => {
            // The identical writer over the second sink: `--verbose` output is a log too, and it is
            // the copy a user pastes into a bug report. `with_ansi(false)` is deliberately not
            // repeated here — colour is what a terminal is for, and the scrubber is indifferent to an
            // escape sequence.
            let stderr = tracing_subscriber::fmt::layer()
                .with_writer(Mutex::new(ScrubWriter::new(sink, secrets)))
                .with_filter(LevelFilter::DEBUG);
            Box::new(base.with(stderr))
        }
    }
}

/// Installs the subscriber for the whole process and reports the file it writes. Called from `main`
/// before anything else, so the first event of a run is recorded.
pub fn init(log_path: &Path, verbose: bool) -> Result<PathBuf, String> {
    let subscriber = subscriber(log_path, verbose)?;
    let path = log_path.to_path_buf();
    tracing::subscriber::set_global_default(subscriber).map_err(|e| e.to_string())?;
    Ok(path)
}

/// A sink — the file, or the terminal — with every complete line scrubbed on its way through.
struct ScrubWriter<W: Write> {
    sink: W,
    secrets: Secrets,
    /// What is left of a line whose newline has not arrived yet. The `fmt` layer writes each event
    /// in several calls, so scrubbing per call would let a secret straddle two of them.
    pending: Vec<u8>,
}

impl<W: Write> ScrubWriter<W> {
    fn new(sink: W, secrets: Secrets) -> Self {
        Self { sink, secrets, pending: Vec::new() }
    }

    fn drain_whole_lines(&mut self, last: bool) -> io::Result<()> {
        loop {
            let end = self.pending.iter().position(|byte| *byte == b'\n');
            let line: Vec<u8> = match end {
                Some(end) => self.pending.drain(..=end).collect(),
                None if last && !self.pending.is_empty() => std::mem::take(&mut self.pending),
                None => return Ok(()),
            };
            let text = String::from_utf8_lossy(&line).into_owned();
            let scrubbed = self.secrets.scrub(&text);
            self.sink.write_all(scrubbed.as_bytes())?;
        }
    }
}

impl<W: Write> Write for ScrubWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        self.drain_whole_lines(false)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain_whole_lines(true)?;
        self.sink.flush()
    }
}

impl<W: Write> Drop for ScrubWriter<W> {
    fn drop(&mut self) {
        // A last line without its newline still gets written, scrubbed.
        let _ = self.flush();
    }
}

/// Whether a run of comma-separated numbers says this line is a sample dump rather than a log line.
fn is_a_sample_dump(line: &str) -> bool {
    line.split(',')
        .filter(|part| {
            let digits = part.trim().trim_matches(|c: char| !c.is_ascii_digit() && c != '-');
            digits.parse::<i64>().is_ok()
        })
        .count()
        >= SAMPLE_RUN
}

/// `token=…` in a query string or a request line: the value goes, the parameter stays, so the line
/// still says what happened (spec §9.2).
fn redact_query_tokens(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("token=") {
        let (before, after) = rest.split_at(at + "token=".len());
        out.push_str(before);
        let end = after.find(['&', ' ', '"', '\'', '\n']).unwrap_or(after.len());
        if end > 0 {
            out.push_str(REDACTED);
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// A long word of nothing but key-ish characters: no `/`, `.` or `:` — which is why a URL, a model
/// slug and a path are left alone — and at least [`OPAQUE`] characters. `keys::redact`'s own output
/// (`sk-or-…3456`) never needs this: it is already a fragment, and the `…` is not a character this
/// rule would match in the first place.
fn redact_opaque_words(line: &str) -> String {
    line.split_inclusive([' ', '\t', '"', '\'', ',', '=', ':', ';'])
        .map(|chunk| {
            let (word, tail) = split_trailing(chunk);
            let opaque = word.len() >= OPAQUE
                && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                && word.chars().any(|c| c.is_ascii_digit())
                && word.chars().any(|c| c.is_ascii_alphabetic());
            if opaque { format!("{REDACTED}{tail}") } else { chunk.to_string() }
        })
        .collect()
}

/// Splits the punctuation a word may be carrying from the word itself.
fn split_trailing(chunk: &str) -> (&str, &str) {
    let end = chunk.trim_end_matches([' ', '\t', '"', '\'', ',', '=', ':', ';', '\n', '\r']).len();
    chunk.split_at(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A token of the shape `store::new_token` produces: 32 bytes as lowercase hex (W8).
    const TOKEN: &str = "9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a39281706f5e4d3c2b1a0";
    /// A key of the shape the connectors use. A fixture, not a key: the workspace rule is that keys
    /// appear by name and never in a file, a log or a fixture, and this string exists only to prove
    /// that the value never reaches a log line.
    const KEY: &str = "sk-or-v1-0123456789abcdef0123456789abcdef";

    #[test]
    fn the_token_never_reaches_a_line_even_as_a_query_parameter() {
        let secrets = Secrets::default();
        secrets.add(TOKEN);
        for line in [
            // The request line a proxy or a socket crate would log, token and all.
            format!("GET /ws?token={TOKEN} HTTP/1.1"),
            // The URL a client builds or a user pastes.
            format!("ws://192.168.1.5:7070/ws?token={TOKEN}"),
            // A page request, and a sentence that happens to contain it.
            format!("GET /terminal?token={TOKEN}&lang=es"),
            format!("the presented token {TOKEN} does not match"),
        ] {
            let scrubbed = secrets.scrub(&line);
            assert!(!scrubbed.contains(TOKEN), "the token reached the log: {scrubbed}");
            // Exactly the secret goes, and nothing else: a log that loses the rest of the line is
            // not a log, and one that keeps a fragment of the token is a leak.
            assert_eq!(scrubbed, line.replace(TOKEN, REDACTED), "only the secret may change");
        }
    }

    #[test]
    fn a_key_value_never_reaches_a_line_by_value_or_by_shape() {
        let secrets = Secrets::default();
        // A value this process resolved: exact match, whatever its shape. Short values are not
        // registered at all (`Secrets::add` ignores anything under eight characters), because
        // redacting `"a"` would shred every line it appeared in.
        secrets.add("hunter2-there");
        assert!(!secrets.scrub("no API key named `hunter2-there`").contains("hunter2-there"));
        // A key that reached a line before its first resolution — the one moment it is not in
        // `Secrets` — is still unreadable: a long opaque word is a secret.
        let scrubbed = secrets.scrub(&format!("Authorization: Bearer {KEY}"));
        assert!(!scrubbed.contains(KEY), "{scrubbed}");
        // And this is not a shredder for anything long: a URL, a model slug, a path, and the
        // fingerprint Task 1.8 prints survive. That last one matters — a certificate fingerprint is
        // a 32-byte hash written as `AA:BB:…`, and a redaction rule that ate it would take away the
        // one thing a user compares against a browser's certificate viewer (spec §5.5).
        for line in [
            "https://openrouter.ai/api/v1/chat/completions",
            "model google/gemini-2.5-flash answered",
            "C:\\Users\\me\\AppData\\Roaming\\tipsy-assistant\\config.toml",
            "SHA-256 fingerprint 5B:1F:0C:44:9A:D3:71:E8:2B:6C:0F:55:A4:19:8E:77:B2:C0:3D:6A:91:5E:08:24:CD:7F:13:AB:60:9D:42:E5",
        ] {
            assert_eq!(secrets.scrub(line), line, "redacting this would cost a reader a diagnosis");
        }
    }

    #[test]
    fn a_sample_dump_is_dropped_whole() {
        let secrets = Secrets::default();
        // What `tracing::debug!(?frame)` would print: the one accident this is shaped for.
        // A slice, not a `Vec`: `repeat` is a slice method and clippy says so (`useless_vec`), which is
        // one of the seven-plus-nothing findings this task may not add to.
        let frame = tipsy_protocol::Frame::Mic([0, 1, -1, 32767, -32768].repeat(64));
        let line = format!("mic frame {frame:?}\n");
        assert_eq!(secrets.scrub(&line), format!("{REDACTED}\n"), "audio is never logged, not even redacted");
        // A line with a couple of numbers in it is a log line.
        let ordinary = "connection 2 of 3 up, latency 41 ms\n";
        assert_eq!(secrets.scrub(ordinary), ordinary);
    }

    /// The whole pipeline: the built subscriber, the real writer, and the three rules at once.
    #[test]
    fn a_record_written_through_the_subscriber_lands_scrubbed() {
        let dir = tempfile::tempdir().unwrap();
        let path = log_path(&dir.path().join("config.toml"));
        assert_eq!(path, dir.path().join("tipsy-assistant.log"));
        let secrets = Secrets::default();
        secrets.add(TOKEN);
        secrets.add(KEY);
        let subscriber = subscriber_at(&path, secrets, false).unwrap();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(url = %format!("ws://127.0.0.1:7070/ws?token={TOKEN}"), "connecting");
            tracing::debug!(key = %format!("Bearer {KEY}"), "handshake");
            tracing::debug!(frame = ?tipsy_protocol::Frame::Mic(vec![3; 320]), "a mistake");
        });

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("connecting"), "the log must be a log: {written}");
        assert!(written.contains("handshake"));
        assert!(!written.contains(TOKEN), "the token reached the log file: {written}");
        assert!(!written.contains(KEY), "a key value reached the log file: {written}");
        assert!(!written.contains("Mic([3, 3, 3"), "a sample dump reached the log file: {written}");
    }

    /// **V1, the vacuous test.** The rules belong to the writer, so the `--verbose` copy is scrubbed
    /// exactly as the file is — but the earlier version of this test built its own `ScrubWriter` over
    /// a `Vec<u8>` and never called the subscriber at all, so handing the `--verbose` layer a raw
    /// `io::stderr` left **every** logging test green. That is the exact regression Task 1.9's ruling
    /// exists to prevent, and it is why this test drives [`subscriber_with`] — the one function that
    /// decides a sink is scrubbed — over two sinks it can read back afterwards.
    #[test]
    fn both_sinks_of_the_real_subscriber_are_scrubbed() {
        /// A sink this test keeps a handle on, so it can be handed to the subscriber and read after.
        #[derive(Clone)]
        struct SharedSink(Arc<Mutex<Vec<u8>>>);

        impl Write for SharedSink {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let secrets = Secrets::default();
        secrets.add(TOKEN);
        let file = SharedSink(Arc::new(Mutex::new(Vec::new())));
        let verbose = SharedSink(Arc::new(Mutex::new(Vec::new())));
        let subscriber = subscriber_with(file.clone(), Some(verbose.clone()), secrets);

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(url = %format!("ws://127.0.0.1:7070/ws?token={TOKEN}"), "connecting");
        });

        for (which, sink) in [("file", &file), ("--verbose", &verbose)] {
            let written = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
            assert!(written.contains("connecting"), "the {which} copy must still be a log: {written}");
            assert!(!written.contains(TOKEN), "the token reached the {which} copy: {written}");
            assert!(written.contains(REDACTED), "and it is replaced rather than dropped: {written}");
        }
    }

    /// The level cap, and why it is the rule rather than a setting: `tokio-tungstenite` logs the
    /// **request line, header included, at `trace!`** — Plan 2C carried exactly that to its review
    /// (`.superpowers/sdd/2026-10-10-plan-2c-openai-realtime/progress.md:37`). A subscriber that can
    /// emit `TRACE` puts an `Authorization: Bearer <key>` in the file the day someone raises the
    /// level; this one cannot.
    #[test]
    fn the_subscriber_never_enables_trace() {
        let dir = tempfile::tempdir().unwrap();
        let subscriber = subscriber_at(&dir.path().join("tipsy-assistant.log"), Secrets::default(), true).unwrap();
        assert_eq!(
            subscriber.max_level_hint(),
            Some(tracing_subscriber::filter::LevelFilter::DEBUG),
            "TRACE is where the request line with the key is logged; it must never be enabled"
        );
    }
}
