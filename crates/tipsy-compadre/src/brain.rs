//! One conversation turn: transcribe → stream the LLM → split → synthesise in parallel → deliver in order.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tipsy_protocol::{AssistantState, ErrorCode, Event, Frame, ToClient};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::audio::pcm::{f32_to_i16, Pcm};
use crate::providers::{
    ChatMessage, ChatRequest, ProviderError, ProviderSlot, Providers, SpeechProvider, SpeechRequest, TextStream,
};
use crate::text::{clean_for_speech, is_speakable, SentenceSplitter};

#[derive(Clone)]
pub struct BrainConfig {
    pub chat_timeout: Duration,
    pub speech_timeout: Duration,
    pub transcribe_timeout: Duration,
    pub retry_backoff: Duration,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            chat_timeout: Duration::from_secs(30),
            speech_timeout: Duration::from_secs(30),
            transcribe_timeout: Duration::from_secs(60),
            retry_backoff: Duration::from_millis(800),
        }
    }
}

pub struct TurnProfile {
    pub prompt: String,
    pub voice_instructions: String,
    pub voice: Option<String>,
    pub language: String,
}

pub enum TurnInput {
    Ask(String),
    Utterance(Pcm),
    Say(String),
}

pub struct Brain {
    providers: ProviderSlot,
    profile: TurnProfile,
    config: BrainConfig,
}

impl Brain {
    pub fn new(providers: ProviderSlot, profile: TurnProfile, config: BrainConfig) -> Self {
        Self { providers, profile, config }
    }

    pub async fn run_turn(
        &self,
        history: &mut Vec<ChatMessage>,
        input: TurnInput,
        out: UnboundedSender<ToClient>,
        cancel: CancellationToken,
    ) {
        let started = Instant::now();
        // The stack this turn uses, read once: a save that rebuilds the connectors takes effect on
        // the *next* turn, and never halfway through this one.
        let providers = self.providers.current();
        let (ask, said) = match input {
            TurnInput::Ask(text) => {
                send(&out, status(AssistantState::Thinking));
                (Some(text), None)
            }
            TurnInput::Utterance(pcm) => {
                send(&out, status(AssistantState::Transcribing));
                match self.transcribe(&providers, pcm, &cancel).await {
                    Transcribed::Text(text) if text.trim().is_empty() => {
                        error(&out, ErrorCode::NothingHeard, "nothing was heard", true);
                        send(&out, status(AssistantState::Idle));
                        return;
                    }
                    Transcribed::Text(text) => {
                        send(&out, ToClient::Event(Event::Heard { text: text.clone() }));
                        send(&out, status(AssistantState::Thinking));
                        (Some(text), None)
                    }
                    Transcribed::Failed(failure) => {
                        report(&out, &failure);
                        send(&out, status(AssistantState::Idle));
                        return;
                    }
                }
            }
            TurnInput::Say(text) => (None, Some(text)),
        };

        match (ask, said) {
            (Some(ask), _) => {
                let turn = self.answer(&providers, &ask, history, &out, &cancel, started).await;
                if turn.aborted {
                    // A cancelled reply keeps what was already spoken; a failed one keeps nothing.
                    if !turn.text.is_empty() {
                        record_cancelled(history, &ask, &turn.spoken);
                    }
                    send(&out, status(AssistantState::Idle));
                } else {
                    history.push(ChatMessage::user(ask));
                    history.push(ChatMessage::assistant(turn.text.clone()));
                    send(&out, ToClient::Event(Event::Reply { text: turn.text }));
                    send(&out, ToClient::Event(Event::Done {
                        first_audio_ms: turn.first_audio_ms,
                        total_ms: started.elapsed().as_millis() as u64,
                    }));
                    send(&out, status(AssistantState::Idle));
                }
            }
            (None, Some(text)) => self.say(&providers, &text, &out, &cancel, started).await,
            (None, None) => send(&out, status(AssistantState::Idle)),
        }
    }

    /// One transcription with the turn pipeline's own timeout and error mapping, and **no turn**:
    /// nothing is asked of the chat provider, nothing is spoken, and the history is not touched. The
    /// login screen's voice path is the only caller (spec `2026-10-11-login-screen-design.md`, §4.1).
    /// Reusing `TurnInput::Utterance` instead would have answered the login out loud and written it
    /// into the conversation.
    pub async fn transcribe_once(
        &self,
        pcm: Pcm,
        out: &UnboundedSender<ToClient>,
        cancel: &CancellationToken,
    ) -> Option<String> {
        let providers = self.providers.current();
        match self.transcribe(&providers, pcm, cancel).await {
            // Empty text is a failure the user can act on ("nothing was heard"), not an answer.
            Transcribed::Text(text) if text.trim().is_empty() => {
                error(out, ErrorCode::NothingHeard, "nothing was heard", true);
                None
            }
            Transcribed::Text(text) => Some(text),
            Transcribed::Failed(failure) => {
                report(out, &failure);
                None
            }
        }
    }

    /// `Ask`: stream the reply and speak whole sentences as they arrive, in the order the model
    /// wrote them.
    async fn answer(
        &self,
        providers: &Providers,
        ask: &str,
        history: &[ChatMessage],
        out: &UnboundedSender<ToClient>,
        cancel: &CancellationToken,
        started: Instant,
    ) -> Turn {
        let messages = self.messages(ask, history);
        let stream = match self.chat_with_retry(providers, ChatRequest { messages }, out, cancel).await {
            Ok(stream) => stream,
            Err(failure) => {
                report(out, &failure);
                return Turn { aborted: true, text: String::new(), spoken: Vec::new(), first_audio_ms: None };
            }
        };
        self.speak(providers, stream, out, cancel, started).await
    }

    /// `Say`: the same sentence pipeline over literal text instead of a chat stream.
    async fn say(
        &self,
        providers: &Providers,
        text: &str,
        out: &UnboundedSender<ToClient>,
        cancel: &CancellationToken,
        started: Instant,
    ) {
        let spoken_text = text.to_string();
        let stream: TextStream = Box::pin(futures::stream::once(async move { Ok(spoken_text) }));
        let turn = self.speak(providers, stream, out, cancel, started).await;
        if turn.aborted {
            send(out, status(AssistantState::Idle));
            return;
        }
        send(out, ToClient::Event(Event::Reply { text: turn.text }));
        send(out, ToClient::Event(Event::Done {
            first_audio_ms: turn.first_audio_ms,
            total_ms: started.elapsed().as_millis() as u64,
        }));
        send(out, status(AssistantState::Idle));
    }

    /// Splits a streamed reply into sentences, synthesising them in parallel but delivering them in
    /// sentence order.
    async fn speak(
        &self,
        providers: &Providers,
        stream: TextStream,
        out: &UnboundedSender<ToClient>,
        cancel: &CancellationToken,
        started: Instant,
    ) -> Turn {
        let mut state = Started {
            stream: Some(stream),
            text: String::new(),
            splitter: SentenceSplitter::new(),
            speech: Some(Synthesis {
                provider: Arc::clone(&providers.speech),
                permits: Arc::new(Semaphore::new(providers.speech.max_parallel())),
                timeout: self.config.speech_timeout,
                request: SpeechRequest {
                    text: String::new(),
                    instructions: self.profile.voice_instructions.clone(),
                    voice: self.profile.voice.clone(),
                },
            }),
            pending: VecDeque::new(),
            seq: 0,
            spoken: Vec::new(),
            speaking: false,
            aborted: false,
            first_audio_ms: None,
        };

        loop {
            let mut sentences = Vec::new();
            let finished = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    state.aborted = true;
                    true
                }
                // Only the front handle is awaited, so a sentence that finishes early waits for the
                // ones before it.
                Some(delivery) = async { state.pending.front_mut()?.await.ok().flatten() }, if state.can_deliver() => {
                    state.pending.pop_front();
                    deliver(out, &mut state, delivery, started);
                    false
                }
                chunk = async {
                    match state.stream.as_mut() {
                        Some(stream) => stream.next().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match chunk {
                        Some(Ok(text)) => {
                            send(out, ToClient::Event(Event::ReplyDelta { text: text.clone() }));
                            state.text.push_str(&text);
                            sentences = state.splitter.push(&text);
                            false
                        }
                        Some(Err(failure)) => {
                            report(out, &failure);
                            state.aborted = true;
                            true
                        }
                        None => {
                            state.stream = None;
                            if let Some(rest) = state.splitter.finish() {
                                sentences.push(rest);
                            }
                            false
                        }
                    }
                }
            };

            for sentence in sentences {
                let cleaned = clean_for_speech(&sentence);
                if is_speakable(&cleaned) {
                    state.spawn(cleaned);
                }
            }

            // A cancelled turn is over: it neither waits for the sentences already being
            // synthesised nor plays them, so nothing is heard after the cancellation.
            if state.aborted {
                state.abort_pending();
                return state.turn();
            }

            if finished || state.closed() {
                break;
            }
        }

        // Everything queued was spawned before the stream ended; drain it, still in order.
        while let Some(handle) = state.pending.pop_front() {
            if let Some(delivery) = handle.await.ok().flatten() {
                deliver(out, &mut state, delivery, started);
            }
        }

        state.stop();
        state.turn()
    }

    /// Calls the chat provider once, retrying a rate-limited request once after the backoff.
    async fn chat_with_retry(
        &self,
        providers: &Providers,
        request: ChatRequest,
        out: &UnboundedSender<ToClient>,
        cancel: &CancellationToken,
    ) -> Result<TextStream, ProviderError> {
        match self.chat(providers, request.clone(), cancel).await {
            Err(ProviderError::RateLimited(message)) => {
                error(out, ErrorCode::RateLimited, &message, true);
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Err(ProviderError::Cancelled),
                    () = tokio::time::sleep(self.config.retry_backoff) => {}
                }
                self.chat(providers, request, cancel).await
            }
            other => other,
        }
    }

    /// One chat request, bounded by `chat_timeout` and dropped the moment the turn is cancelled.
    async fn chat(
        &self,
        providers: &Providers,
        request: ChatRequest,
        cancel: &CancellationToken,
    ) -> Result<TextStream, ProviderError> {
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(ProviderError::Cancelled),
            result = tokio::time::timeout(self.config.chat_timeout, providers.chat.stream(request)) => {
                match result {
                    Ok(stream) => stream,
                    Err(_) => Err(ProviderError::Timeout),
                }
            }
        }
    }

    /// Transcription, bounded by `transcribe_timeout` and using the profile's language.
    async fn transcribe(&self, providers: &Providers, pcm: Pcm, cancel: &CancellationToken) -> Transcribed {
        let language = self.profile.language.as_str();
        let call = providers.transcribe.transcribe(pcm, language);
        tokio::select! {
            biased;
            () = cancel.cancelled() => Transcribed::Failed(ProviderError::Cancelled),
            result = tokio::time::timeout(self.config.transcribe_timeout, call) => {
                match result {
                    Ok(Ok(text)) => Transcribed::Text(text),
                    Ok(Err(failure)) => Transcribed::Failed(failure),
                    Err(_) => Transcribed::Failed(ProviderError::Timeout),
                }
            }
        }
    }

    fn messages(&self, ask: &str, history: &[ChatMessage]) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(history.len() + 2);
        messages.push(ChatMessage::system(self.profile.prompt.clone()));
        messages.extend_from_slice(history);
        messages.push(ChatMessage::user(ask));
        messages
    }
}

/// One turn's outcome: what was said, and whether it was cut short.
struct Turn {
    aborted: bool,
    text: String,
    spoken: Vec<String>,
    first_audio_ms: Option<u64>,
}

/// Everything the streaming loop carries between iterations.
struct Started {
    stream: Option<TextStream>,
    text: String,
    splitter: SentenceSplitter,
    speech: Option<Synthesis>,
    /// Spawned sentences in the order they must be delivered; only the front one may be awaited.
    pending: VecDeque<JoinHandle<Option<Delivery>>>,
    seq: u32,
    spoken: Vec<String>,
    speaking: bool,
    aborted: bool,
    first_audio_ms: Option<u64>,
}

/// One finished sentence, tagged with the position it must be delivered at.
struct Delivery {
    seq: u32,
    /// The cleaned sentence, kept for the history when the turn is cancelled mid-reply.
    text: String,
    result: Result<Pcm, ProviderError>,
}

/// Synthesis of whole sentences, at most `max_parallel()` in flight.
struct Synthesis {
    provider: Arc<dyn SpeechProvider>,
    permits: Arc<Semaphore>,
    timeout: Duration,
    request: SpeechRequest,
}

impl Started {
    /// Queues one cleaned sentence for synthesis, giving it the next delivery position.
    fn spawn(&mut self, text: String) {
        let Some(synthesis) = self.speech.as_ref() else {
            // Already stopped: a sentence arriving later must not take a sequence number.
            return;
        };
        let seq = self.seq;
        self.seq += 1;
        let provider = Arc::clone(&synthesis.provider);
        let permits = Arc::clone(&synthesis.permits);
        let timeout = synthesis.timeout;
        let request = SpeechRequest { text: text.clone(), ..synthesis.request.clone() };
        self.pending.push_back(tokio::spawn(async move {
            // Waiting for a permit here is what bounds synthesis to `max_parallel()` at a time.
            if permits.acquire_owned().await.is_err() {
                return None;
            }
            let result = synthesize(provider.as_ref(), request, timeout).await;
            Some(Delivery { seq, text, result })
        }));
    }

    /// True while there is a sentence waiting to be delivered.
    fn can_deliver(&self) -> bool {
        self.speech.is_some() && !self.pending.is_empty()
    }

    /// True when the reply has ended and every queued sentence was delivered.
    fn closed(&self) -> bool {
        self.stream.is_none() && self.pending.is_empty()
    }

    /// Stops speaking: no further synthesis, nothing further delivered.
    fn stop(&mut self) {
        self.speech = None;
        for handle in self.pending.drain(..) {
            handle.abort();
        }
    }

    /// Discards every queued sentence without playing any of it. Used when a turn is cancelled:
    /// a sentence synthesised before the cancellation is audio the user asked not to hear.
    fn abort_pending(&mut self) {
        self.stop();
    }

    /// The outcome of the turn so far.
    fn turn(&self) -> Turn {
        Turn {
            aborted: self.aborted,
            text: clean_for_speech(&self.text),
            spoken: self.spoken.clone(),
            first_audio_ms: self.first_audio_ms,
        }
    }
}

/// Emits one finished sentence: its header, its audio and the bookkeeping around it.
fn deliver(out: &UnboundedSender<ToClient>, state: &mut Started, delivery: Delivery, started: Instant) {
    match delivery.result {
        Ok(pcm) => {
            if !state.speaking {
                state.speaking = true;
                send(out, status(AssistantState::Speaking));
            }
            if state.first_audio_ms.is_none() {
                state.first_audio_ms = Some(started.elapsed().as_millis() as u64);
            }
            send(out, ToClient::Event(Event::SpeechChunk { seq: delivery.seq, sample_rate: pcm.sample_rate }));
            send(out, ToClient::Audio(Frame::Speech { seq: delivery.seq, samples: f32_to_i16(&pcm.samples) }));
            state.seq = state.seq.max(delivery.seq + 1);
            state.spoken.push(delivery.text);
        }
        Err(failure) => {
            report(out, &failure);
            // One failed sentence stops the audio; the text stream is not this function's business.
            state.stop();
        }
    }
}

async fn synthesize(
    provider: &dyn SpeechProvider,
    request: SpeechRequest,
    timeout: Duration,
) -> Result<Pcm, ProviderError> {
    match tokio::time::timeout(timeout, provider.synthesize(request)).await {
        Ok(result) => result,
        Err(_) => Err(ProviderError::Timeout),
    }
}

/// The result of transcribing one utterance.
enum Transcribed {
    Text(String),
    Failed(ProviderError),
}

fn status(state: AssistantState) -> ToClient {
    ToClient::Event(Event::Status { state, detail: None })
}

fn send(out: &UnboundedSender<ToClient>, message: ToClient) {
    let _ = out.send(message);
}

fn report(out: &UnboundedSender<ToClient>, failure: &ProviderError) {
    error(out, failure.code(), &failure.to_string(), true);
}

fn error(out: &UnboundedSender<ToClient>, code: ErrorCode, message: &str, recoverable: bool) {
    send(out, ToClient::Event(Event::Error { code, message: message.to_string(), recoverable }));
}

/// Keeps the sentences that were spoken, so cancelling mid-reply has something to remember.
fn record_cancelled(history: &mut Vec<ChatMessage>, ask: &str, spoken: &[String]) {
    history.push(ChatMessage::user(ask));
    if !spoken.is_empty() {
        history.push(ChatMessage::assistant(spoken.join(" ")));
    }
}
