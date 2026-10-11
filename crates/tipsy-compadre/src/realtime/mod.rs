//! Realtime voice: one duplex WebSocket session with OpenAI's Realtime API (spec §6, plan 2C).
//!
//! `socket` is the transport — connect, configure, read, keep alive, reopen, close. `events` (Task
//! 3) is the pure mapping from the wire's JSON to the actions the rest of the module performs.
//!
//! This file is the call façade that ties both to the session, the microphone and the `Player`.
//! While a call is live the microphone goes up as mono 24 kHz PCM16 and the model's audio comes down
//! as the same `SpeechChunk`/`Frame::Speech` pair a spoken sentence arrives in, so the client plays
//! it with the `Player` it already has and the HUD logs the same transcript events the text pipeline
//! sends. What this module owns is the call's own bookkeeping: the socket task, the frames in, the
//! events out, the item the device is playing and the call's deadline.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tipsy_protocol::{AssistantState, ErrorCode, Event, Frame, ToClient};
use tokio::sync::mpsc::{Receiver, Sender, UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::audio::device::MIC_SAMPLE_RATE;
use crate::audio::pcm::{f32_to_i16, i16_to_f32, resample_linear};
use crate::config::{Config, RealtimeConfig};
use crate::keys::KeyResolver;
use crate::realtime::events::Action;
use crate::realtime::socket::{CallOptions, CallSpec, RealtimeSocket, SocketEvent};

pub mod events;
pub mod socket;

/// The rate the Realtime API takes and returns audio at: mono PCM16, 24 kHz, and the only rate its
/// `audio/pcm` format accepts (research note, Audio). It is the socket's own `PCM_RATE` under the
/// name this module's callers read, and `the_mic_rate_the_api_rate_and_the_format_agree` keeps the
/// two names from drifting apart.
pub const API_SAMPLE_RATE: u32 = socket::PCM_RATE;

/// Microphone frames that may wait for the socket before the oldest are dropped: about a fifth of a
/// second, the same trade the client's own microphone bridge makes.
const MIC_QUEUE: usize = 16;

/// The endpoint one call opens: the model is a query parameter, not part of the path (research note,
/// Endpoints) — `wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1`.
pub fn url(model: &str) -> String {
    socket::realtime_url(socket::REALTIME_URL, model)
}

/// One call, as the client asks for it.
///
/// `[realtime]` supplies the model, the voice and the key *name*; the profile supplies the
/// instructions and the language; the caller supplies the resolver, which is the only thing that
/// ever holds a key value. `CallSetup` is deliberately not `Debug` and holds no key: it is the
/// thing a log, an event or a panic message would print, and it must have nothing to leak.
pub struct CallSetup {
    /// Where this call connects. The tests point it at a loopback socket; `from_config` builds the
    /// note's endpoint from the model.
    pub url: String,
    /// The two deadlines the API leaves to us (the socket's `CallOptions`): the open timeout and the
    /// 60-minute cap at which a long call reopens rather than dying.
    pub options: CallOptions,
    /// The `[realtime]` section as configured: the model, the voice and the key *name*.
    pub config: RealtimeConfig,
    /// The resolver every key lookup goes through — the same one the providers use. Shared rather
    /// than cloned because `KeyResolver` has no `Clone`: it is an `Arc` for the repeated resolution
    /// a reopen needs.
    pub resolver: Arc<KeyResolver>,
    pub model: String,
    pub instructions: String,
    pub voice: String,
    /// `[listening] silence_ms`: turn detection reuses the listening silence, so two numbers never
    /// describe one behaviour.
    pub silence_ms: u32,
    pub language: String,
    /// The call's own deadline, from `[realtime] max_minutes`: a call ends by itself here, whatever
    /// the socket is doing (Review Focus 3).
    pub limit: Duration,
}

impl CallSetup {
    /// From the configuration and the active profile. The *voice* deliberately comes from
    /// `[realtime] voice` and not from `Profile::voices`, which maps speech connectors to voices and
    /// this mode is not one of them. `key` is the value the caller already resolved, and it is used
    /// for nothing here: the handshake resolves the key *name* again when it opens (and again when
    /// it reopens), so keeping the value would be keeping a secret for no reason. What this argument
    /// is for is the failure it carries — a key name nobody holds is reported before any socket is
    /// touched, which is where the `String` type still earns its place.
    pub fn from_config(config: &Config, resolver: Arc<KeyResolver>, key: String) -> CallSetup {
        let realtime = &config.realtime;
        // The resolved value is dropped, deliberately not stored. See the doc comment above.
        drop(key);
        CallSetup {
            url: url(&realtime.model),
            options: CallOptions::default(),
            config: realtime.clone(),
            model: realtime.model.clone(),
            instructions: config.active().prompt.clone(),
            voice: realtime.voice.clone(),
            silence_ms: config.listening.silence_ms,
            language: config.active().language.clone(),
            limit: Duration::from_secs(u64::from(realtime.max_minutes) * 60),
            resolver,
        }
    }
}

/// One microphone frame as the API takes it: mono 16 kHz in, mono 24 kHz out, PCM16 either way.
///
/// The microphone delivers 16 kHz (`device.rs:17`) and the API's `audio/pcm` takes 24 kHz and
/// nothing else, so the conversion happens here, once, with the workspace's own resampler over its
/// own i16 conversion.
pub fn mic_to_api(frame: &[i16]) -> Vec<i16> {
    f32_to_i16(&resample_linear(&i16_to_f32(frame), MIC_SAMPLE_RATE, API_SAMPLE_RATE))
}

/// The API's audio as samples: little-endian PCM16, one sample per two bytes, in order.
pub fn pcm16_le(bytes: &[u8]) -> Vec<i16> {
    bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}

/// The little-endian bytes of one frame of samples — what `input_audio_buffer.append` carries,
/// base64 inside its JSON.
pub fn pcm16_bytes(samples: &[i16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// What the call knows about the audio the device is playing.
///
/// This task keeps the item it belongs to — the id and content index a truncate would have to name.
/// Task 5 adds the count of what has actually been heard and the handshake with the client's frame
/// loop, which is the only place the device's own queue is visible.
#[derive(Debug, Default)]
pub struct Heard {
    item: Mutex<Option<(String, u32)>>,
}

impl Heard {
    pub fn new() -> Arc<Heard> {
        Arc::new(Heard::default())
    }

    /// The first audio delta of a response names the item the device is playing.
    pub fn begin(&self, item_id: &str, content_index: u32) {
        *self.item.lock().expect("the item lock is never held across a panic") =
            Some((item_id.to_string(), content_index));
    }

    /// The item the device is playing, if one is.
    pub fn item(&self) -> Option<(String, u32)> {
        self.item.lock().expect("the item lock is never held across a panic").clone()
    }

    /// Forgets the item: the call ended, or a barge-in took it away.
    pub fn clear(&self) {
        *self.item.lock().expect("the item lock is never held across a panic") = None;
    }
}

/// One call's handle, held by the client.
pub struct Call {
    mic: Sender<Vec<i16>>,
    heard: Arc<Heard>,
    cancel: CancellationToken,
}

impl Call {
    /// Opens one call.
    ///
    /// The socket is opened on a task of its own, because the client's frame loop must never wait
    /// for a handshake, and everything the model produces is reported through `out`, exactly as a
    /// session reports its own.
    pub fn open(setup: CallSetup, heard: Arc<Heard>, out: UnboundedSender<ToClient>) -> Call {
        let (mic, frames) = tokio::sync::mpsc::channel(MIC_QUEUE);
        let cancel = CancellationToken::new();
        tokio::spawn(run(setup, Arc::clone(&heard), frames, out, cancel.clone()));
        Call { mic, heard, cancel }
    }

    /// Queues one microphone frame for the socket. Bounded and lossy: a busy client drops a frame
    /// rather than stalling the device.
    pub fn push_mic(&self, samples: Vec<i16>) {
        let _ = self.mic.try_send(samples);
    }

    /// The bookkeeping the client fills in as it plays: Task 5's barge-in count.
    pub fn heard(&self) -> &Arc<Heard> {
        &self.heard
    }

    /// Ends the call: the socket is closed and the task finishes. Dropping the handle does the same,
    /// which is how `Esc` ends a call by ending the client.
    pub fn stop(self) {
        self.cancel.cancel();
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// One response in flight: what the HUD is told when it ends.
#[derive(Default)]
struct Turn {
    started: Option<Instant>,
    first_audio_ms: Option<u64>,
    text: String,
    /// True once any of this response's audio reached the device.
    spoke: bool,
    /// True once the HUD was told the assistant is speaking.
    speaking: bool,
    /// The sequence number of the next `SpeechChunk`/`Frame::Speech` pair.
    seq: u32,
}

/// What one server event did to the call.
enum Step {
    Continue,
    Stop,
}

/// The sentence the user reads when the socket goes quiet, on whichever side it happened.
const SOCKET_CLOSED: &str = "the call's socket closed while the model was speaking";

async fn run(
    setup: CallSetup,
    heard: Arc<Heard>,
    mut frames: Receiver<Vec<i16>>,
    out: UnboundedSender<ToClient>,
    cancel: CancellationToken,
) {
    let spec = CallSpec {
        instructions: setup.instructions.clone(),
        language: setup.language.clone(),
        silence_ms: setup.silence_ms,
    };
    let mut socket =
        match RealtimeSocket::connect_with(setup.options.clone(), &setup.resolver, &setup.config, &spec).await
        {
            Ok(socket) => socket,
            Err(e) => {
                // Nothing was ever live, and the client is told why; `Live { active: false }` still
                // goes out so a client holding the handle lets go of it.
                report(&out, e.code(), &format!("the call could not open: {e}"));
                ended(&out);
                return;
            }
        };
    // The key has done its job: it went into the handshake and the socket kept no copy of it.
    send(&out, ToClient::Event(Event::Live { active: true }));
    send(&out, status(AssistantState::Listening));

    // `[realtime] max_minutes`: a call ends by itself here (Review Focus 3), through the same exit
    // as every other ending.
    let deadline = tokio::time::sleep(setup.limit);
    tokio::pin!(deadline);
    // The keepalive: the note documents no idle close, so the client proves the socket is alive
    // every `KEEPALIVE` rather than discovering a dead one on the next question.
    let mut keepalive = tokio::time::interval(socket::KEEPALIVE);
    // The first tick is immediate; the socket was just configured, so it is skipped.
    keepalive.tick().await;
    let mut turn = Turn::default();

    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            () = &mut deadline => {
                report(&out, ErrorCode::Timeout, "the call reached its limit ([realtime] max_minutes)");
                break;
            }
            frame = frames.recv() => {
                let Some(frame) = frame else { break };
                let samples = mic_to_api(&frame);
                if samples.is_empty() {
                    continue;
                }
                let message = events::append_audio_message(&pcm16_bytes(&samples));
                if let Err(e) = socket.send(&serde_json::from_str(&message).expect("our own JSON")).await {
                    report(&out, e.code(), SOCKET_CLOSED);
                    break;
                }
            }
            _ = keepalive.tick() => {
                // A ping that cannot be sent is a dead socket; at the API's own cap it is the
                // session ending, which reopens instead of ending the call.
                let Err(e) = socket.ping().await else { continue };
                if !at_cap(&mut socket, &setup, &out).await {
                    report(&out, e.code(), &format!("the call's socket closed on its keepalive: {e}"));
                    break;
                }
            }
            event = socket.recv() => {
                match event {
                    Ok(SocketEvent::Text(text)) => {
                        if let Step::Stop = on_event(&text, &mut turn, &heard, &out) {
                            break;
                        }
                    }
                    // A close, clean or not, ends the call: the note marks close codes and reconnect
                    // semantics as undocumented, so no code is read. A session at its own cap is the
                    // one ending that reopens instead — and only when the cap has really elapsed.
                    Ok(SocketEvent::Closed { .. }) => {
                        if !at_cap(&mut socket, &setup, &out).await {
                            report(&out, ErrorCode::Network, SOCKET_CLOSED);
                            break;
                        }
                    }
                    Err(e) => {
                        // `e`'s own words, prefixed: the user has to be able to read which side
                        // went quiet (Review Focus 1).
                        if !at_cap(&mut socket, &setup, &out).await {
                            report(&out, e.code(), &format!("the call's socket failed: {e}"));
                            break;
                        }
                    }
                }
            }
        }
    }
    let _ = socket.close().await;
    ended(&out);
}

/// Reopens the socket when the session has reached the API's own 60-minute cap, and answers whether
/// it did. The note documents no close code for the cap, so the decision is `session_expired`'s —
/// never a closed socket's code — and a call whose `max_minutes` is shorter than the cap (the
/// default) never gets here at all. A reopen that fails is not a second ending: the caller reports
/// the failure it already has.
async fn at_cap(socket: &mut RealtimeSocket, setup: &CallSetup, out: &UnboundedSender<ToClient>) -> bool {
    if !socket.session_expired() {
        return false;
    }
    match socket.reopen(&setup.resolver).await {
        Ok(()) => true,
        Err(e) => {
            report(out, e.code(), &format!("the call's socket could not be reopened: {e}"));
            false
        }
    }
}

/// One text frame from the server.
fn on_event(
    text: &str,
    turn: &mut Turn,
    heard: &Arc<Heard>,
    out: &UnboundedSender<ToClient>,
) -> Step {
    // An event this crate does not know is data, not death (Review Focus 4): `map` answers
    // `Action::Ignored` for a type we have never seen. Only a *known* event with a malformed payload
    // is an error, and then it is a typed one that ends the call in words the user can read.
    let action = match events::map(text) {
        Ok(action) => action,
        Err(e) => {
            report(out, e.code(), &format!("the call sent an event we cannot read: {e}"));
            return Step::Stop;
        }
    };
    match action {
        // The session is configured and the call is already live: `run` raised the flag when the
        // socket came up.
        Action::SessionConfigured { .. } => {}
        // The user talked over the model. Task 5 acts on this; the plumbing that matters here is
        // that the item the device is playing is the call's own bookkeeping, not the event's.
        Action::SpeechStarted => {
            heard.clear();
        }
        Action::AudioDelta { pcm } => {
            let samples = pcm16_le(&pcm);
            if samples.is_empty() {
                return Step::Continue;
            }
            // The first delta of a response is the one that reaches the device. A delta after it
            // would be a second response speaking over the first — a cancelled one still arriving —
            // and its bytes must reach neither the device nor the count a later truncate is derived
            // from, so exactly one delta per response is played.
            if turn.spoke {
                return Step::Continue;
            }
            turn.spoke = true;
            if turn.first_audio_ms.is_none() {
                turn.first_audio_ms = Some(elapsed_ms(turn.started));
            }
            if !turn.speaking {
                turn.speaking = true;
                send(out, status(AssistantState::Speaking));
            }
            // The rate header and the audio itself: the same pair a spoken sentence arrives as, so
            // the client hands the device the rate that actually arrived (Review Focus 5).
            send(out, ToClient::Event(Event::SpeechChunk { seq: turn.seq, sample_rate: API_SAMPLE_RATE }));
            send(out, ToClient::Audio(Frame::Speech { seq: turn.seq, samples }));
            turn.seq += 1;
        }
        Action::ReplyDelta { text } => {
            turn.text.push_str(&text);
            send(out, ToClient::Event(Event::ReplyDelta { text }));
        }
        // What the user said. The mapping accumulates the growing
        // `conversation.item.input_audio_transcription.delta` into this one whole sentence, because
        // `Event::Heard` inserts a log row per event. It is also where a turn begins: the call has
        // no key press to start one, and the HUD's timing is measured from the utterance that
        // started it — `first_audio_ms` to the first spoken frame, `total_ms` to the response's
        // end, exactly as the brain measures a text turn.
        Action::Heard { text } => {
            *turn = Turn { started: Some(Instant::now()), ..Turn::default() };
            send(out, ToClient::Event(Event::Heard { text }));
        }
        Action::ResponseDone { .. } => {
            finish(turn, out);
            // The call is still live and the microphone is still open: the resting state of a call
            // is listening, never idle.
            send(out, status(AssistantState::Listening));
        }
        Action::ServerError { message } => {
            report(out, ErrorCode::BadResponse, &message);
            return Step::Stop;
        }
        // Known types with nothing to do here: the acknowledgement of our own `session.update`, the
        // turn-end half of the VAD pair (the server commits the item and starts the response on its
        // own) and a type this build has never seen (Review Focus 4).
        Action::SpeechStopped | Action::Ignored => {}
    }
    Step::Continue
}

/// Says what a response said, in the events the text pipeline also sends, so the HUD's entry stops
/// waiting and keeps its timing. A response that neither spoke nor transcribed leaves the log alone.
fn finish(turn: &mut Turn, out: &UnboundedSender<ToClient>) {
    let total_ms = elapsed_ms(turn.started);
    if !turn.spoke && turn.text.is_empty() {
        return;
    }
    send(out, ToClient::Event(Event::Reply { text: std::mem::take(&mut turn.text) }));
    send(out, ToClient::Event(Event::Done { first_audio_ms: turn.first_audio_ms, total_ms }));
}

fn elapsed_ms(started: Option<Instant>) -> u64 {
    started.map_or(0, |at| at.elapsed().as_millis() as u64)
}

/// The call is over, whatever ended it: a clean close, an error event, a transport failure, the
/// deadline or the key. The HUD leaves its live state in every one of those cases (Review Focus 1).
fn ended(out: &UnboundedSender<ToClient>) {
    send(out, ToClient::Event(Event::Live { active: false }));
    send(out, status(AssistantState::Idle));
}

fn report(out: &UnboundedSender<ToClient>, code: ErrorCode, message: &str) {
    send(out, ToClient::Event(Event::Error { code, message: message.to_string(), recoverable: true }));
}

fn status(state: AssistantState) -> ToClient {
    ToClient::Event(Event::Status { state, detail: None })
}

fn send(out: &UnboundedSender<ToClient>, message: ToClient) {
    let _ = out.send(message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::pcm::{Pcm, i16_to_f32};
    use crate::keys::MemoryStore;

    #[test]
    fn the_endpoint_carries_the_model_as_a_query() {
        // The research note's Endpoints section, verbatim: the socket a native client opens, with
        // the model as a query parameter. The id is Task 1's default for `[realtime] model`.
        assert_eq!(url("gpt-realtime-2.1"), "wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1");
    }

    #[test]
    fn the_mic_rate_the_api_rate_and_the_format_agree() {
        // Review Focus 5 in one place: the microphone's rate, the API's rate and the rate the
        // `session.update` pins are three names for two numbers, and the microphone's is not one of
        // them. `socket::PCM_RATE` is what the wire carries; this is what the module promises.
        assert_eq!(MIC_SAMPLE_RATE, 16_000);
        assert_eq!(API_SAMPLE_RATE, 24_000);
        assert_eq!(socket::PCM_RATE, API_SAMPLE_RATE, "the wire's rate and the module's are one value");
        assert_ne!(MIC_SAMPLE_RATE, API_SAMPLE_RATE, "a resample that does nothing is the bug this catches");
    }

    #[test]
    fn a_microphone_frame_becomes_one_and_a_half_times_as_many_samples() {
        // 20 ms of 16 kHz mono is 320 samples; the same 20 ms at 24 kHz is 480. Review Focus 5:
        // nothing may be sent at the wrong rate, and a missed resample is a 1.0 ratio.
        let frame = vec![1000i16; 320];
        let up = mic_to_api(&frame);
        assert_eq!(up.len(), 480, "16 kHz → 24 kHz is 3/2 the samples");
        assert_eq!(Pcm { samples: i16_to_f32(&up), sample_rate: API_SAMPLE_RATE }.duration_ms(), 20);
        // A constant stays constant within one least-significant bit (the f32 conversion rounds).
        for sample in &up {
            assert!((i32::from(*sample) - 1000).abs() <= 1, "the resampler invented a signal: {sample}");
        }
        assert!(mic_to_api(&[]).is_empty());
    }

    #[test]
    fn the_apis_audio_is_sixteen_bit_little_endian() {
        // The API's PCM16, decoded without the f32 round trip `pcm_s16le_to_pcm` takes
        // (`audio/pcm.rs:87-90`), because the samples the device is handed must be the ones the
        // model sent.
        assert_eq!(pcm16_le(&[0x00, 0x40, 0x00, 0xC0]), vec![16384, -16384]);
        assert_eq!(pcm16_le(&[1, 2, 3]), vec![513], "an odd trailing byte is not a sample");
        assert!(pcm16_le(&[]).is_empty());
        // The outbound half is the exact inverse, so the two never disagree about the byte order.
        assert_eq!(pcm16_bytes(&[16384, -16384]), vec![0x00, 0x40, 0x00, 0xC0]);
        assert!(pcm16_bytes(&[]).is_empty());
    }

    #[test]
    fn the_audio_the_client_appends_is_base64_of_little_endian_samples() {
        // RFC 4648, pinned against a hand-written vector rather than against the crate's own
        // encoder: a codec that only agrees with itself is caught here. `-1, 0` as little-endian
        // PCM16 is `ff ff 00 00`.
        assert_eq!(events::base64_encode(&[0xff, 0xff, 0x00, 0x00]), "//8AAA==");
        assert_eq!(events::base64_decode("//8AAA==").unwrap(), vec![0xff, 0xff, 0x00, 0x00]);
        assert_eq!(events::base64_encode(&pcm16_bytes(&[-1, 0])), "//8AAA==");
    }

    #[test]
    fn the_call_asks_for_the_configured_model_voice_and_prompt() {
        let config = Config::default();
        let keys = Arc::new(KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None)));
        let wanted = CallSetup::from_config(&config, keys, "not-a-key".into());
        assert_eq!(wanted.url, "wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1");
        // The profile's own prompt is the session's `instructions`, exactly as the brain sends it as
        // the system message.
        assert_eq!(wanted.instructions, config.active().prompt);
        // The voice comes from `[realtime] voice`, deliberately *not* from the profile's `voices`
        // table, which maps speech connectors to voices and this mode is not one of them.
        assert_eq!(wanted.voice, "marin");
        // Turn detection reuses `[listening] silence_ms`: `[realtime]` has no number of its own.
        assert_eq!(wanted.silence_ms, config.listening.silence_ms);
        assert_eq!(wanted.language, config.active().language, "input transcription in the profile's language");
        assert_eq!(wanted.limit, Duration::from_secs(600), "`[realtime] max_minutes`, 10 by default");
        // The setup holds the key *name* and the resolver, never a key value: the handshake resolves
        // the name when it opens.
        assert_eq!(wanted.config.key, "openai");
    }

    #[test]
    fn the_item_a_truncate_would_name_is_the_calls_own_bookkeeping() {
        // Task 5 derives `conversation.item.truncate` from this: the item whose audio the device is
        // playing, known here and nowhere else. A barge-in and the end of a call both forget it.
        let heard = Heard::new();
        assert_eq!(heard.item(), None, "nothing has been played yet");
        heard.begin("item_a", 0);
        assert_eq!(heard.item(), Some(("item_a".to_string(), 0)));
        // The first delta of a response names the item; a later one does not replace it.
        heard.begin("item_a", 0);
        assert_eq!(heard.item(), Some(("item_a".to_string(), 0)));
        heard.clear();
        assert_eq!(heard.item(), None, "a barge-in takes the item away");
    }
}
