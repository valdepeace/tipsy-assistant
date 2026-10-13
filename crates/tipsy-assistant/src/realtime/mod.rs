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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

/// Bytes of one millisecond of the API's audio: 24 kHz mono PCM16 is 48 000 bytes per second
/// (research note, Interruption 4). The one number a barge-in must not get wrong.
pub const BYTES_PER_MS: u64 = 48;

/// Whole milliseconds in `bytes`. Flooring, never rounding: a partial millisecond is a claim about
/// audio the user has not heard.
pub fn ms_of(bytes: u64) -> u64 {
    bytes / BYTES_PER_MS
}

/// Bytes of an item's audio that have left the playback buffer.
///
/// `appended` is every chunk handed to the device for that item, in order; `remaining` is how many
/// of them the device still holds (`Player::progress`); `pos_ms` is the position inside the one
/// playing. The chunks that have finished are the first `appended.len() - remaining` of them, and
/// the one playing contributes only what it has actually played, never more than its own length.
///
/// Every uncertain term here resolves downwards: a stale position is not counted once nothing is
/// queued, and a chunk of something else in the sink can only make `remaining` too large, which
/// understates. Overstating is the direction that corrupts the conversation (Review Focus 2).
pub fn played_bytes(appended: &[usize], remaining: usize, pos_ms: u64) -> u64 {
    // A device that holds more chunks than were ever appended to *this* item means the queue holds
    // something else too: the finished set is empty and only the playing chunk counts, which is the
    // floor rather than a guess about what those extra chunks were. Nothing here may grow the count.
    let finished = appended.len().saturating_sub(remaining);
    let done: u64 = appended[..finished].iter().map(|bytes| *bytes as u64).sum();
    let playing = match appended.get(finished) {
        Some(length) if remaining > 0 => (pos_ms * BYTES_PER_MS).min(*length as u64),
        _ => 0,
    };
    done + playing
}

/// What the call knows about the audio the device is playing.
///
/// Two threads touch it, and neither waits for the other: the client's frame loop writes what has
/// actually left the playback buffer — the device's queue is visible nowhere else — and the socket
/// task reads it when the server reports that the user started speaking. No lock here is ever held
/// across an `await`, and none of this touches the audio thread.
#[derive(Debug, Default)]
pub struct Heard {
    item: Mutex<Option<(String, u32)>>,
    /// Every chunk handed to the device for the current item, in order, in bytes.
    appended: Mutex<Vec<usize>>,
    /// How much of the current item has been heard, in bytes: the client publishes it from
    /// `Player::progress` after each chunk, and the socket task reads it at a barge-in.
    played: AtomicU64,
    /// Set by a barge-in: the client's next frame silences the device and clears the count.
    interrupted: AtomicBool,
}

impl Heard {
    pub fn new() -> Arc<Heard> {
        Arc::new(Heard::default())
    }

    /// The first audio delta of a response names the item the device is playing, and the count
    /// starts at nothing.
    pub fn begin(&self, item_id: &str, content_index: u32) {
        *self.item.lock().expect("the item lock is never held across a panic") =
            Some((item_id.to_string(), content_index));
        self.forget();
    }

    /// One chunk handed to the device, and what the device says about the queue afterwards.
    pub fn push(&self, chunk_bytes: usize, remaining: usize, pos_ms: u64) {
        let played = {
            let mut appended = self.appended.lock().expect("the chunk lock is never held across a panic");
            appended.push(chunk_bytes);
            played_bytes(&appended, remaining, pos_ms)
        };
        self.played.store(played, Ordering::Relaxed);
    }

    /// The milliseconds of the current item the user has actually heard. `None` while none of it has
    /// been played: a partial millisecond is not rounded up to one, and nothing is claimed about an
    /// item the device has not started.
    pub fn heard_ms(&self) -> Option<u64> {
        if self.item.lock().ok()?.is_none() {
            return None;
        }
        match ms_of(self.played.load(Ordering::Relaxed)) {
            0 => None,
            ms => Some(ms),
        }
    }

    /// What a barge-in must tell the model: the item the device is playing, its content index and
    /// the milliseconds of it the user heard — or `None` when there is nothing safe to send.
    ///
    /// The item comes from the bookkeeping and not from the event: `speech_started` names the user's
    /// input item (research note, Interruption 1), and a truncate cuts the transcript of whatever
    /// item it names (Interruption 3), so naming the wrong one is the failure this task exists to
    /// prevent.
    ///
    /// The count is read *before* the item, deliberately. The client's frame loop writes both, and a
    /// chunk it publishes between the two reads would otherwise pair an older item with a newer
    /// count — the direction that overstates. Reading the count first can only pair a count that is
    /// at or below the truth for the item, which is the safe direction.
    pub fn truncation(&self) -> Option<(String, u32, u64)> {
        let heard_ms = self.heard_ms()?;
        let (item_id, content_index) = self.item()?;
        Some((item_id, content_index, heard_ms))
    }

    /// The item the device is playing, if one is.
    pub fn item(&self) -> Option<(String, u32)> {
        self.item.lock().expect("the item lock is never held across a panic").clone()
    }

    /// Asks the client's frame loop to silence the device: only the client owns the `Player`.
    pub fn interrupt(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }

    /// True exactly once per barge-in, for the client that acts on it.
    pub fn take_interrupted(&self) -> bool {
        self.interrupted.swap(false, Ordering::SeqCst)
    }

    /// Forgets the item and everything counted for it: the call ended, a barge-in took it away, or a
    /// new item began. What is left is the interrupted flag, which belongs to the frame loop that
    /// has not acted on it yet.
    pub fn clear(&self) {
        *self.item.lock().expect("the item lock is never held across a panic") = None;
        self.forget();
    }

    fn forget(&self) {
        self.appended.lock().expect("the chunk lock is never held across a panic").clear();
        self.played.store(0, Ordering::Relaxed);
    }
}

/// What the client asks of a call while it lasts.
enum Control {
    /// The user is taking the floor by hand: the response is cancelled rather than truncated.
    Interrupt,
}

/// One call's handle, held by the client.
pub struct Call {
    mic: Sender<Vec<i16>>,
    control: UnboundedSender<Control>,
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
        let (control, orders) = tokio::sync::mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        tokio::spawn(run(setup, Arc::clone(&heard), frames, orders, out, cancel.clone()));
        Call { mic, control, heard, cancel }
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

    /// The user is taking the floor by hand: the server is told to cancel the response, and the
    /// device is silenced on the client's next frame.
    ///
    /// A local interrupt cancels instead of truncating (research note, Interruption 5): the number a
    /// truncate carries comes from the server's own report of what the user heard, and a key press
    /// has no such report to give.
    pub fn interrupt(&self) {
        let _ = self.control.send(Control::Interrupt);
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
    /// The item this response is playing, and its content index — the pair a truncate names. It is
    /// fixed by the response's first *named* audio delta and never replaced, so a later delta of
    /// another item is not this response's audio. A delta that carries no name leaves this `None`:
    /// there is nothing safe to truncate yet, and a sentinel no later delta could match would gate
    /// the rest of the response's audio away.
    item: Option<(String, u32)>,
    /// True once the HUD was told the assistant is speaking.
    speaking: bool,
    /// True from a barge-in until the next turn: a response the user talked over must not be played
    /// even if its deltas are still in flight (research note, Interruption 1).
    silenced: bool,
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
    orders: tokio::sync::mpsc::UnboundedReceiver<Control>,
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
    // The client's orders, as an `Option` so that the day the client drops its handle the arm is
    // disabled instead of firing on a closed channel for ever. `None` is not an ending: the call
    // ends through the token, exactly as it does when the handle is dropped.
    let mut orders = Some(orders);

    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            () = &mut deadline => {
                report(&out, ErrorCode::Timeout, "the call reached its limit ([realtime] max_minutes)");
                break;
            }
            // The user's own key press, which only the client can see. Cancelling nothing is safe:
            // the research note says the cancel errors on the server and the session survives, so it
            // is sent without checking what is running.
            Some(order) = recv_order(&mut orders) => match order {
                None => orders = None,
                Some(Control::Interrupt) => {
                    let message = events::cancel_message();
                    if socket.send(&serde_json::from_str(&message).expect("our own JSON")).await.is_err() {
                        report(&out, ErrorCode::Network, SOCKET_CLOSED);
                        break;
                    }
                    heard.interrupt();
                }
            },
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
                let event = match event {
                    Ok(event) => event,
                    Err(e) => {
                        // `e`'s own words, prefixed: the user has to be able to read which side
                        // went quiet (Review Focus 1).
                        if !at_cap(&mut socket, &setup, &out).await {
                            report(&out, e.code(), &format!("the call's socket failed: {e}"));
                            break;
                        }
                        continue;
                    }
                };
                match event {
                    // A text frame cannot be acted on while `recv` still holds the socket — a
                    // `select!` can stop polling the read but the value it produced borrows the
                    // socket until the end of the arm — so `on_event` is the whole of the arm's
                    // work, as it already was, and it answers with the one frame it wants sent.
                    // The truncate goes out here, after the read. Losing the socket between the two
                    // costs the truncate and the call, which is the ending that was already coming.
                    SocketEvent::Text(text) => {
                        let (step, truncate) = on_event(&text, &mut turn, &heard, &out);
                        if let Some(message) = truncate
                            && socket.send(&serde_json::from_str(&message).expect("our own JSON")).await.is_err()
                        {
                            report(&out, ErrorCode::Network, SOCKET_CLOSED);
                            break;
                        }
                        if let Step::Stop = step {
                            break;
                        }
                    }
                    // A close, clean or not, ends the call: the note marks close codes and reconnect
                    // semantics as undocumented, so no code is read. A session at its own cap is the
                    // one ending that reopens instead — and only when the cap has really elapsed.
                    SocketEvent::Closed { .. } => {
                        if !at_cap(&mut socket, &setup, &out).await {
                            report(&out, ErrorCode::Network, SOCKET_CLOSED);
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

/// The next order the client sent, or `None` when it has dropped its handle.
///
/// `orders` is taken away once that happens, which is what keeps a closed channel from being a busy
/// loop: a channel with no senders is ready for ever, and an arm that is ready for ever spins the
/// whole `select!`. The call itself ends when the token is cancelled, not here.
async fn recv_order(orders: &mut Option<tokio::sync::mpsc::UnboundedReceiver<Control>>) -> Option<Option<Control>> {
    match orders {
        Some(receiver) => Some(receiver.recv().await),
        // Already disabled: a future that is never ready, so the arm cannot fire.
        None => std::future::pending().await,
    }
}

/// The sentence the user reads when a call outlives the API's own session cap. The reopen is not an
/// ending — the call carries on and the microphone stays open — but it *is* a fresh session: the new
/// socket is configured from `session.update` alone, with no `conversation.item.create` replay, so
/// the assistant remembers nothing said before it. Saying so is the difference between a call that
/// quietly forgot and one the user knows has started over.
const REOPENED: &str =
    "the call reached the API's 60-minute cap: reopened a fresh session, which remembers nothing before it";

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
        Ok(()) => {
            // The call is still live, but the conversation in it is not: the log gets the notice
            // rather than the user discovering the hole when the model answers as a stranger.
            report(out, ErrorCode::Timeout, REOPENED);
            true
        }
        Err(e) => {
            report(out, e.code(), &format!("the call's socket could not be reopened: {e}"));
            false
        }
    }
}

/// One text frame from the server.
///
/// The answer is deliberately not carried out through `socket`: a `select!` arm's `recv` keeps the
/// socket borrowed for as long as the arm's binds live, so this function cannot send and must not
/// try. It says what the call should do, and the one frame it wants sent goes back with it.
fn on_event(
    text: &str,
    turn: &mut Turn,
    heard: &Arc<Heard>,
    out: &UnboundedSender<ToClient>,
) -> (Step, Option<String>) {
    // An event this crate does not know is data, not death (Review Focus 4): `map` answers
    // `Action::Ignored` for a type we have never seen. Only a *known* event with a malformed payload
    // is an error, and then it is a typed one that ends the call in words the user can read.
    let action = match events::map(text) {
        Ok(action) => action,
        Err(e) => {
            report(out, e.code(), &format!("the call sent an event we cannot read: {e}"));
            return (Step::Stop, None);
        }
    };
    match action {
        // The session is configured and the call is already live: `run` raised the flag when the
        // socket came up.
        Action::SessionConfigured { .. } => {}
        // The user talked over the model. Playback stops on the client's next frame, and the model
        // is told how much of its own speech was actually heard, so its memory matches the user's
        // (research note, Interruption 2 and 3). The truncate is asked for, not sent: the caller
        // owns the socket by the time the arm is over.
        Action::SpeechStarted => {
            let truncate = heard
                .truncation()
                .map(|(item_id, content_index, audio_end_ms)| {
                    events::truncate_message(&item_id, content_index, audio_end_ms)
                });
            // The count is spent: a second barge-in counts from the next item, and nothing of this
            // one can be added to it by a delta that is still arriving.
            heard.clear();
            heard.interrupt();
            turn.silenced = true;
            return (Step::Continue, truncate);
        }
        Action::AudioDelta { item_id, content_index, pcm } => {
            // The tail of a response the user talked over is not played, and is not counted: it is
            // audio nobody asked to hear any more.
            if turn.silenced {
                return (Step::Continue, None);
            }
            let samples = pcm16_le(&pcm);
            if samples.is_empty() {
                return (Step::Continue, None);
            }
            // The first *named* delta of the response names the item the device is playing, and
            // `Heard` starts its count there. The count is per item and is fed one chunk per delta, so
            // it is that first delta and no later one that calls `begin`: calling it again would throw
            // the chunks already counted away and understate the audio the user has heard.
            //
            // A delta whose frame carries no name (the note marks that field inferred) is still played
            // — it is the model's audio — but it leaves nothing safe to truncate later: it names no
            // item, so the bookkeeping keeps waiting for a name instead of storing a sentinel that no
            // later delta could ever match.
            if turn.item.is_none() && let Some(named) = &item_id {
                turn.item = Some((named.clone(), content_index));
                heard.begin(named, content_index);
            }
            // A delta of an item this response is not playing is not heard at all. It is what a
            // cancelled response still arriving looks like, and its bytes must reach neither the
            // device nor the count a later truncate is derived from.
            //
            // The gate fires on a *known* mismatch, never on a missing name: an unnamed delta is not
            // known to disagree with anything, and reading its absence as one would drop every later
            // delta of the response — named ones included — before any of them reached the device.
            if let (Some((playing, _)), Some(named)) = (&turn.item, &item_id) && playing != named {
                return (Step::Continue, None);
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
            return (Step::Stop, None);
        }
        // Known types with nothing to do here: the acknowledgement of our own `session.update`, the
        // turn-end half of the VAD pair (the server commits the item and starts the response on its
        // own) and a type this build has never seen (Review Focus 4).
        Action::SpeechStopped | Action::Ignored => {}
    }
    (Step::Continue, None)
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

    #[test]
    fn the_bytes_per_millisecond_are_the_notes_forty_eight() {
        // The research note's Interruption 4: 24 kHz mono PCM16 is 24000 samples × 2 bytes per
        // second, i.e. 48 bytes per millisecond. An off-by-48 error is a whole millisecond of the
        // model's memory, every barge-in (Review Focus 2).
        assert_eq!(BYTES_PER_MS, u64::from(API_SAMPLE_RATE) * 2 / 1000);
        assert_eq!(BYTES_PER_MS, 48);
        assert_eq!(ms_of(960), 20);
        assert_eq!(ms_of(47), 0, "a partial millisecond is never rounded up");
    }

    #[test]
    fn nothing_played_yet_is_nothing_to_truncate() {
        let heard = Heard::new();
        assert_eq!(played_bytes(&[], 0, 0), 0);
        assert_eq!(heard.heard_ms(), None);
        assert_eq!(heard.truncation(), None, "no item, no truncate");

        heard.begin("item_a", 0);
        assert_eq!(heard.heard_ms(), None, "an item the user has not heard is not a truncation");
        assert_eq!(heard.truncation(), None);

        // A chunk that is still entirely in the queue has not been heard. The note does not say
        // whether `audio_end_ms: 0` is legal, so nothing is sent rather than a zero that claims
        // something about an item nobody heard.
        heard.push(960, 1, 0);
        assert_eq!(heard.heard_ms(), None, "queued is not played");
        assert_eq!(heard.truncation(), None);
    }

    // ---- Where the brief's numbers and its own arithmetic disagree ---------------------------
    //
    // Two of the six pure tests below do not assert every value the brief prints, and the reason is
    // arithmetic rather than preference: the brief's expected values cannot be produced by the
    // `played_bytes` that same brief prescribes, and one of them claims audio nobody heard. The
    // implementation follows the prescribed function exactly, and the brief's own hand-stated
    // client side of the integration test is reproduced by it byte for byte — three 20 ms chunks
    // with `remaining` 3 → 2 → 1 and a 12 ms position give 2496 bytes, i.e. the 52 ms that
    // `a_barge_in_truncates_the_item_at_the_milliseconds_that_were_heard` asserts. The full table,
    // `appended` being one entry longer per `push`:
    //
    //   push         appended      finished  playing          truth    brief says
    //   (960,1,12)   [960]         0         12 ms            12 ms    Some(12)  ✓
    //   (960,1,20)   [960,960]     1         20 ms            40 ms    Some(20)  ✗
    //   (960,1,100)  [960;3]       2         960 B (capped)  60 ms    Some(40)  ✗
    //
    //   (960,3,0)    [960]         0          0              None     None      ✓
    //   (960,2,0)    [960,960]     0          0              None     Some(20)  ✗
    //   (960,1,12)   [960;3]       1         12 ms            52 ms    Some(52)  ✓
    //   (960,1,20)   [960;4]       2         20 ms            80 ms    Some(60)  ✗
    //   (960,0,999)  [960;5]       5         none (empty)   100 ms    Some(80)  ✗
    //
    // The brief is internally inconsistent as well: its third line expects the *first* chunk plus a
    // 12 ms position to be 52 ms, and its fourth expects "three finished" to be 60 ms — one line
    // after the same three chunks were 52 ms. Every value below is the arithmetic's. Where the two
    // disagree the arithmetic wins, because the truncate that overstates is the one failure this
    // task exists to prevent. **Numbered defect D1 in `task-5-report.md`; if the controller prefers
    // the brief's numbers, `played_bytes` is the single function to change and these are the two
    // tests that pin it.**

    #[test]
    fn a_partially_played_chunk_counts_only_what_left_the_buffer() {
        let heard = Heard::new();
        heard.begin("item_a", 0);
        heard.push(960, 1, 12);
        assert_eq!(heard.heard_ms(), Some(12), "12 × 48 of the chunk's 960 bytes");
        heard.push(960, 1, 20);
        assert_eq!(heard.heard_ms(), Some(40), "the chunk that finished plus the one playing");
        heard.push(960, 1, 100);
        assert_eq!(heard.heard_ms(), Some(60), "a position cannot report more than its own chunk");
    }

    #[test]
    fn the_chunks_that_finished_are_counted_and_the_one_playing_is_added() {
        let heard = Heard::new();
        heard.begin("item_a", 0);
        heard.push(960, 3, 0);
        assert_eq!(heard.heard_ms(), None, "all three are still queued");
        heard.push(960, 2, 0);
        assert_eq!(heard.heard_ms(), None, "the device still holds everything it was handed");
        heard.push(960, 1, 12);
        assert_eq!(heard.heard_ms(), Some(52), "the first has finished, plus 12 ms of the third");
        heard.push(960, 1, 20);
        assert_eq!(heard.heard_ms(), Some(80), "two finished plus 20 ms of the third");
        heard.push(960, 0, 999);
        // An empty queue is the boundary that matters most: nothing is playing, so the stale `999`
        // contributes nothing. Five 20 ms chunks are 100 ms; adding the stale position would claim
        // audio the user has not heard (Review Focus 2).
        assert_eq!(heard.heard_ms(), Some(100), "an empty queue: the stale position is not added");
        assert_eq!(heard.truncation(), Some(("item_a".to_string(), 0, 100)));
    }

    #[test]
    fn a_delta_for_another_item_never_counts_towards_this_one() {
        let heard = Heard::new();
        heard.begin("item_a", 0);
        heard.push(960, 1, 20);
        assert_eq!(heard.truncation(), Some(("item_a".to_string(), 0, 20)));

        // The next response's item takes the count's place: the old item's bytes were never played
        // as part of it, and truncating the wrong item's transcript is the one failure this whole
        // task exists to prevent. (The socket-level half of the same rule is
        // `a_delta_for_another_item_is_not_played` in `tests/realtime.rs`.)
        heard.begin("item_b", 7);
        assert_eq!(heard.heard_ms(), None);
        assert_eq!(heard.truncation(), None);
        heard.push(960, 1, 20);
        assert_eq!(heard.truncation(), Some(("item_b".to_string(), 7, 20)));

        // A second barge-in counts from the new item only: `clear` is what the task sends before it
        // raises the interrupt flag, so nothing of the first one survives into the second.
        heard.clear();
        assert_eq!(heard.heard_ms(), None);
        heard.begin("item_c", 0);
        heard.push(960, 1, 20);
        assert_eq!(heard.truncation(), Some(("item_c".to_string(), 0, 20)));
    }

    #[test]
    fn the_truncate_and_cancel_messages_are_the_ones_the_note_quotes() {
        // Research note, Interruption 3 and 5, verbatim.
        assert_eq!(
            events::truncate_message("item_a", 0, 1500),
            r#"{"type":"conversation.item.truncate","item_id":"item_a","content_index":0,"audio_end_ms":1500}"#
        );
        assert_eq!(events::cancel_message(), r#"{"type":"response.cancel"}"#);
    }
}
