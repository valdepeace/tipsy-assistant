//! The browser microphone: WebAudio capture into the protocol's 320-sample 16 kHz frames.
//!
//! The browser has to do in WebAudio what `crates/tipsy-compadre/src/audio/device.rs` does in cpal:
//! downmix the channels, resample to 16 kHz, clamp to `i16`, and emit exactly
//! `MIC_FRAME_SAMPLES = 320` samples — 20 ms — per frame, because that is the frame the server's own
//! `audio/device.rs:18-20` and `session.rs` are built around. The frames leave as the **existing**
//! `0x01` binary frames inside `ToServer::Audio(Frame::Mic(..))`: no protocol change, no new frame
//! kind.
//!
//! **The pure half is at the top and the WebAudio wiring is below it, and the split is not
//! decoration.** There is no browser in the machine that runs this suite, so everything that is a
//! decision rather than a DOM call — the eight `getUserMedia` rejection names mapped to the states the
//! gate shows ([`MicState`]), the capability answer those states make ([`MicAvailability`], whose
//! reason is always a value out of `tipsy-ui`'s two tables), the 320-sample boundary and the carry of
//! a partial block ([`FrameAssembler`]), the resampler and the sample conversion
//! ([`resample_linear`], [`f32_to_i16`], [`i16_to_f32`], [`rms`]) — is a plain function over plain
//! values with a host test under it. What is left for `web-sys` is creating a context, asking for a
//! stream, loading one worklet module and installing one port handler.
//!
//! **320 is not a multiple of the render quantum, and that is why the boundary is here.** The quantum
//! is 128 frames (320 = 2 × 128 + 64) and MDN warns that the size must be *checked* rather than
//! assumed (`ratzilla-and-wasm-audio.md` §4.5), so the worklet only forwards whatever it was handed
//! and [`FrameAssembler`] owns the accumulator. The prior art's core/presenter sentence
//! (`codewhale-audio-prior-art.md` §3) has two invariants and both hold here: **calling at any cadence
//! produces the same frames** — the assembler is driven by sample counts and never by the browser's
//! paint rate, so a 128-frame quantum, a 400-sample block and a 480-sample 48 kHz block all land in
//! the same 320-sample output — and **a repeated input cannot retrigger**, because there are no
//! sequence numbers and no timers here: the assembler appends and emits.
//!
//! **Which gate is whose, because there are two and only one of them is this file's.** This file's
//! gate is the *capability and gesture* gate: the page finds out whether it can record at all before
//! it offers to, and the one click that opens the capture is the user gesture both `getUserMedia` and
//! the `AudioContext` need. The **login door** — the gate the client inherits from the login screen —
//! is `login_door.rs`'s: `Door::sends_frames` reads `LoginState { voice, open }` while a screen is up
//! and the HUD's own `mic_open` while none is, and [`drain_capture`] takes the door and calls
//! `Outbox::send` behind it and behind nothing else. **The port handler sends nothing**: it downmixes,
//! resamples, assembles and appends to [`Capture`]'s queue, and the one function that may reach the
//! socket is [`drain_capture`], called once per frame. There is one send site, and the door's
//! predicate is what guards it.
//!
//! **What the door is *not* on this client, said plainly because the phase's own phrase got it wrong.**
//! The login spec's promise to the provider is "nothing reaches the provider until the local detector
//! confirms speech", and **this crate has no detector at all**: every 20 ms frame the page captures
//! while its capture is open is uploaded to `/ws`, silence and room tone included, exactly as
//! push-to-talk uploads them. What keeps the silence away from the provider is the **server's**
//! `login_vad` (`crates/tipsy-compadre/src/session.rs:935-962`), which runs in front of the transcriber
//! on the login capture the way `auto` mode runs it. The browser's half of the promise is narrower and
//! is what [`drain_capture`] enforces: no frame leaves the page while no capture is open, and a frame
//! the door refuses is dropped rather than kept. A later task that wants the browser to filter its own
//! uploads has to bring a detector with it.
//!
//! **What only a browser can check, named rather than implied.** Whether `getUserMedia` is reachable
//! on this origin, whether the worklet module loads, whether the context really runs at the rate it
//! answers, whether a real `MessageEvent`'s `data()` is the array of `Float32Array`s the worklet
//! posts: no host test and no script can produce any of those. The live check in this task's ledger
//! drives the *bytes* a real client's frames are, through a real socket, against a real server; the
//! browser half of Step 8 is recorded as unverified.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use tipsy_protocol::{Command, Frame, ToServer};
use tipsy_ui::{LoginLine, UiState};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::socket::Outbox;

/// 20 ms at 16 kHz, the frame the protocol promises (`audio/device.rs:18-20`).
pub const FRAME_SAMPLES: usize = 320;

/// The rate the project wants, and the rate the resampler brings every block to.
pub const MIC_RATE: u32 = 16_000;

/// How many assembled frames may wait for one turn of the render loop before the newest are lost.
///
/// **The twin of the native capture's bound**: `crates/tipsy-compadre/src/client.rs:53` is
/// `MIC_QUEUE = 16` on a `sync_channel`, and its `try_send` drops the frame rather than stalling the
/// device. Sixteen frames is 320 ms of speech, which is longer than any stall a frame loop this shape
/// can have and short enough that a forgotten queue cannot grow without bound.
pub const MAX_QUEUED_FRAMES: usize = 16;

/// The worklet module, as the route that serves it spells the path.
///
/// **`/assets/client/mic-worklet.js`, not the plan's `/assets/mic-worklet.js`.** The bundle is the
/// repository's `web/` directory served under `/assets/` (`server/assets.rs`'s `#[folder =
/// "../../web/"]` and `server/mod.rs`'s `/assets/{*path}` route), and this task's `Files` list creates
/// the file at `web/client/mic-worklet.js` — so the embed serves it at `/assets/client/mic-worklet.js`
/// and the plan's constant names `web/mic-worklet.js`, which no task creates. The two lines cannot
/// both be satisfied and the file's location wins, because it is the one Task 3.6's `dist` does not
/// have to move: `test_the_worklet_is_where_the_url_says_it_is` reads the constant back against the
/// file on disk, so a later move of either half fails the suite instead of a 404 in a browser.
pub const WORKLET_URL: &str = "/assets/client/mic-worklet.js";

/// The gate's element id, which is also how the click handler finds the gate to take it down.
const GATE_ID: &str = "tipsy-gate";

/// The three permission states plus the four ways a device can fail, as the gate shows them. The v1
/// spec §12 rule is one sentence: a microphone that will not open disables listening with a warning
/// and typing still works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicState {
    /// `PermissionStatus.state == "prompt"`: show "press to start".
    Prompt,
    Granted,
    /// The user said no, or the context is not secure.
    Denied,
    NoDevice,
    Busy,
    /// The browser ignored our constraints, or there is no `navigator.mediaDevices` at all.
    Unsupported,
    /// Everything else, including the two transient rejections.
    Failed,
}

/// One of `getUserMedia`'s documented rejection names, as the state the HUD shows.
///
/// The list is MDN's, quoted at `ratzilla-and-wasm-audio.md` §4.1: `AbortError`,
/// `InvalidStateError`, `NotAllowedError`, `NotFoundError`, `NotReadableError`,
/// `OverconstrainedError`, `SecurityError`, `TypeError`. **Only two of them mean the user said no**,
/// and treating the rest as a denial would send the user to the wrong setting — a machine with no
/// input device is not a refused permission. An unknown name is [`MicState::Failed`], which is the
/// honest answer: something went wrong that this build does not have a better word for.
pub fn mic_state_for(rejection: &str) -> MicState {
    match rejection {
        "NotAllowedError" | "SecurityError" => MicState::Denied,
        "NotFoundError" => MicState::NoDevice,
        "NotReadableError" => MicState::Busy,
        "OverconstrainedError" | "TypeError" => MicState::Unsupported,
        _ => MicState::Failed,
    }
}

/// What the gate acts on: can this page offer the microphone, and if not, why — as data.
///
/// The prior art's shape (`codewhale-audio-prior-art.md` §2 and §4: failures "answer `200` with
/// `ok: false` + a machine-readable `reason`", and the browser disables and relabels its control from
/// the data it holds, `pet/public/shared.html:45`), which is the same shape `Welcome.web_bundle`
/// already uses here. The reason is a `&'static str` **out of the table**; there is no constructor on
/// this type that can invent one, which is what keeps the es/en pairing honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicAvailability {
    pub available: bool,
    pub reason: Option<&'static str>,
}

impl MicAvailability {
    /// The answer before anything is pressed. `devices_present` is whether `navigator.mediaDevices`
    /// exists at all — it is `undefined` in an insecure context, which is a different problem from a
    /// refused permission and gets its own reason (MDN, quoted at `ratzilla-and-wasm-audio.md` §4.1).
    pub fn probe(devices_present: bool, permission: MicState, strings: &tipsy_ui::Strings) -> Self {
        let reason = match (devices_present, permission) {
            (false, _) => Some(strings.mic_no_api),
            (true, MicState::Prompt | MicState::Granted) => None,
            (true, MicState::Denied) => Some(strings.mic_denied),
            (true, MicState::NoDevice) => Some(strings.mic_nodevice),
            (true, MicState::Busy) => Some(strings.mic_busy),
            (true, MicState::Unsupported | MicState::Failed) => Some(strings.mic_failed),
        };
        Self { available: reason.is_none(), reason }
    }

    /// The same answer after a `getUserMedia` rejection, from the exception's `name`.
    pub fn from_rejection(name: &str, strings: &tipsy_ui::Strings) -> Self {
        Self::probe(true, mic_state_for(name), strings)
    }
}

/// Whatever the audio thread hands over, in 320-sample frames.
///
/// The worklet forwards each render quantum untouched, because the quantum is 128 frames and 320 is
/// not a multiple of it — and because MDN's warning on `process()` is that the size must be checked
/// rather than assumed. The channels arrive as they came; downmixing is `audio/device.rs:186-189`'s
/// job and happens here so that it has a test.
#[derive(Debug, Default)]
pub struct FrameAssembler {
    pending: Vec<f32>,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self { pending: Vec::with_capacity(FRAME_SAMPLES * 2) }
    }

    /// Adds one block (any channel count, any length) and returns every frame it completed.
    ///
    /// The carry is the whole point: a block that is not a whole number of frames leaves its tail in
    /// `pending` and it is *prepended* to the next block, so no sample is ever dropped and no frame is
    /// ever short. `push_block` is total — any input, including an empty one, is answered with a
    /// `Vec`, never a panic and never a partial frame.
    pub fn push_block(&mut self, channels: &[Vec<f32>]) -> Vec<Vec<i16>> {
        if channels.is_empty() {
            return Vec::new();
        }
        let mono = downmix(channels);
        self.push_mono(&mono)
    }

    /// The same, for samples that are already mono — the shape a resampled block arrives in.
    fn push_mono(&mut self, mono: &[f32]) -> Vec<Vec<i16>> {
        self.pending.extend_from_slice(mono);
        let mut frames = Vec::new();
        while self.pending.len() >= FRAME_SAMPLES {
            let rest = self.pending.split_off(FRAME_SAMPLES);
            let frame = std::mem::replace(&mut self.pending, rest);
            frames.push(f32_to_i16(&frame));
        }
        frames
    }

    /// What is held back. Only the tests and the frame counter need it.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

/// The channels averaged into mono, the way the native capture does it (`audio/device.rs:186-189`).
///
/// The width is the first channel's, and a channel that is shorter than it contributes zero for the
/// samples it does not have: the worklet posts equal-length channels, and a block that is not shaped
/// that way is a broken block rather than a reason to panic. Divided by the channel *count*, not by
/// the number of channels that happened to have a sample there, so a two-channel block is a real
/// average and not a sum.
pub fn downmix(channels: &[Vec<f32>]) -> Vec<f32> {
    let Some(first) = channels.first() else {
        return Vec::new();
    };
    let count = channels.len() as f32;
    (0..first.len())
        .map(|i| channels.iter().map(|channel| channel.get(i).copied().unwrap_or(0.0)).sum::<f32>() / count)
        .collect()
}

/// One block from the worklet, in the 16 kHz mono frames the protocol wants.
///
/// The order is the native capture's: downmix, then resample, then cut into frames
/// (`audio/device.rs:186-192`). **The context's rate is read back, never assumed** — `AudioContext`
/// answers with the rate it actually runs at, and `AudioContextOptions.sampleRate` is only a request
/// (`ratzilla-and-wasm-audio.md` §4.3) — so a 48 kHz context reaches 16 kHz here and a context that
/// already runs at 16 kHz is not resampled at all.
pub fn frames_from_block(assembler: &mut FrameAssembler, rate: u32, channels: &[Vec<f32>]) -> Vec<Vec<i16>> {
    if channels.is_empty() {
        return Vec::new();
    }
    let mono = downmix(channels);
    let mono = if rate == MIC_RATE { mono } else { resample_linear(&mono, rate, MIC_RATE) };
    assembler.push_mono(&mono)
}

/// Adds one completed frame to the queue, or drops it when the queue is full.
///
/// The twin of the native capture's bounded `try_send` (`client.rs:53`, `:95-101`): **the newest frame
/// is the one that is lost**, because a device that blocked on a client that is behind is worse than
/// a client that misses a word, and because that is the direction the native side already chose.
pub fn enqueue_frame(queue: &mut VecDeque<Vec<i16>>, frame: Vec<i16>) {
    if queue.len() < MAX_QUEUED_FRAMES {
        queue.push_back(frame);
    }
}

/// Linear interpolation to 16 kHz. The twin of `crates/tipsy-compadre/src/audio/pcm.rs`'s
/// `resample_linear`, copied because this crate must not depend on `tipsy-compadre` (its closure holds
/// tokio, cpal and rodio); the test below pins the same arithmetic, because a twin that rounds
/// differently is a pitch shift the server would hear.
pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if samples.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    if from == to {
        return samples.to_vec();
    }
    let out_len = (samples.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * step;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = samples[idx.min(samples.len() - 1)];
            let b = samples[(idx + 1).min(samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

/// The twin of `pcm.rs`'s `f32_to_i16`, which is also the expression inlined at
/// `audio/device.rs:191`: clamp, scale by 32767 and truncate. Nothing here rounds differently.
pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect()
}

/// The twin of `pcm.rs`'s `i16_to_f32`, and the conversion the HUD's envelope is taken through.
///
/// **It divides by 32768, where `f32_to_i16` multiplies by 32767**, and that asymmetry is the
/// native code's, verbatim: the plan's Task 3.7 sketch writes this conversion a third time as
/// `*s as f32 / 32767.0`, which is neither twin and would make a full-scale sample read `1.0` here
/// and `0.99997` natively.
pub fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|s| *s as f32 / 32768.0).collect()
}

/// The twin of `pcm.rs`'s `rms`, for the HUD's red envelope.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// The loudness one frame shows in the HUD, from its own samples and **before the configured gain**.
///
/// The native client's `state.push_mic_level(rms(&i16_to_f32(&samples)) * self.mic_gain)`
/// (`crates/tipsy-compadre/src/client/mic.rs:44`): this is the `rms` half of that expression, and the
/// gain is [`MicGain`]'s, applied by [`drain_capture`]. The split is what lets the unscaled twin be
/// pinned against `pcm.rs`'s own conversion in a test with no configuration anywhere near it.
pub fn levels_from_samples(samples: &[i16]) -> f32 {
    rms(&i16_to_f32(samples))
}

/// `[listening] mic_gain`, as the configuration this client has been told spells it.
///
/// **Applied, and this is where a browser can get it.** The native client reads the number out of
/// `config.toml` when it starts (`crates/tipsy-compadre/src/client.rs:186`) and multiplies every level
/// by it (`client/mic.rs:44`); a browser has no file, so the only source is `Event::Config` — the
/// answer to the `GetConfig` the configuration screen sends when it opens (`UiAction::OpenConfig`,
/// spec §4.2's "Reading: `GetConfig` on open"). [`configured_gain`] reads it from that answer and the
/// frame loop keeps it for the rest of the run, so a save from the screen applies the new gain the way
/// the native client applies the file. **Before the screen has ever been opened the level is
/// unscaled**, and that is named rather than implied: this page has not been told the configuration,
/// and inventing the file's own default would be a number the server never sent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicGain(f32);

impl Default for MicGain {
    /// The identity: what a client that has not been told a gain applies. `1.0` and not the file's
    /// `12.0`, because the value has to come from the server or from nowhere.
    fn default() -> Self {
        Self(1.0)
    }
}

impl MicGain {
    /// The value the form's `mic_gain` row holds. Anything that is not a finite positive number is the
    /// identity: the row is text the user typed, the form is what validates it before it is saved
    /// (`config_form.rs`'s `MIC_GAIN_MIN..MAX`), and a client that guessed would scale the level by
    /// zero or by a `NaN` and draw nothing at all.
    pub fn of(text: &str) -> Self {
        Self(text.trim().parse::<f32>().ok().filter(|gain| gain.is_finite() && *gain > 0.0).unwrap_or(1.0))
    }

    /// The number itself, for a test and for anyone who ever has to draw it.
    pub fn value(&self) -> f32 {
        self.0
    }

    /// The native client's `* self.mic_gain`, in one place.
    pub fn apply(&self, level: f32) -> f32 {
        level * self.0
    }
}

/// The gain the configuration this client has been given carries, or `None` when it has been told
/// nothing.
///
/// **`filled() && !dirty` is the whole of the rule**, and it is what makes reading it once a frame
/// honest: a *filled* form is the one the server's `Event::Config` produced
/// (`UiState::show_config_form` marks it), so the placeholder the screen opens on — whose rows carry
/// the form's own defaults — is never mistaken for an answer; and a *clean* form is the server's own
/// values, so an edit the user has not saved is never applied. The native client reads the file at
/// startup and applies a new gain when the file is written; this is that promise by the only route a
/// browser has.
pub fn configured_gain(state: &UiState) -> Option<MicGain> {
    let form = state.config_form.as_ref()?;
    (form.filled() && !form.dirty).then(|| MicGain::of(&form.listening.mic_gain))
}

/// What one turn of the render loop owes the socket and the oscilloscope.
#[derive(Debug, Default, PartialEq)]
pub struct Drained {
    /// The frames that may leave the page, in the order they were captured.
    pub send: Vec<Vec<i16>>,
    /// One loudness per frame that was drained — sent or dropped, because the native client's
    /// `push_mic_level` runs for every routed frame whichever way it went (`client.rs:44`).
    pub levels: Vec<f32>,
}

/// Empties the capture's queue: what may leave the page, and what the HUD has to draw.
///
/// **`sending` is the whole gate, and it is passed in rather than read here**, because this is the
/// pure half: [`drain_capture`] answers it with `login_door::Door::sends_frames`, and that function's
/// own tests answer it directly. **A frame the gate refuses is dropped, not kept**: keeping it would
/// send it the moment the gate opened, and the server would transcribe a sentence said to a closed
/// door as the user's answer — the native client's "a frame belongs to a *stream*" (`mic_route` in
/// `tipsy_ui::policy`), on this side of the wire.
pub fn drain_frames(queue: &mut VecDeque<Vec<i16>>, sending: bool) -> Drained {
    let mut drained = Drained::default();
    while let Some(samples) = queue.pop_front() {
        drained.levels.push(levels_from_samples(&samples));
        if sending {
            drained.send.push(samples);
        }
    }
    drained
}

/// The live capture: the stream, the node, and the frames waiting for a frame.
///
/// **The context is held but is no longer the run's own clock**: the one `AudioContext` per run lives
/// in the [`AudioSlot`] the user's press fills, whether or not this capture opened (the phase review's
/// finding 4), and this is a handle on the same context — the graph the stream is plugged into cannot
/// outlive it. The two fields that exist only for their lifetime say so.
pub struct Capture {
    context: web_sys::AudioContext,
    /// The node the port belongs to, and the closure installed on that port. Neither is read again:
    /// the two are held because **dropping them is what detaches the port**, and a `Closure` that
    /// fell out of scope here would be freed memory the browser could still call.
    #[allow(dead_code)]
    node: web_sys::AudioWorkletNode,
    #[allow(dead_code)]
    on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    /// The live stream, held for the same reason: dropping the last handle to it is what stops the
    /// tracks, so this field *is* the capture's lifetime rather than data about it.
    #[allow(dead_code)]
    stream: web_sys::MediaStream,
    /// The frames the port has delivered since the last frame of the render loop.
    frames: Rc<RefCell<VecDeque<Vec<i16>>>>,
}

impl Capture {
    /// The context this capture runs in — the same one the run's playback is scheduled on, because
    /// there is exactly one per press and a second would be a second clock.
    pub fn context(&self) -> &web_sys::AudioContext {
        &self.context
    }
}

/// Where the live capture is kept: one per `start()`, shared by the gate that fills it and the frame
/// loop that drains it. `None` until the user's press opens one, which is also the answer to "is a
/// capture running".
pub type CaptureSlot = Rc<RefCell<Option<Capture>>>;

/// **The one `AudioContext` per run, and it is not the microphone's.** The user's press creates it
/// (that gesture is what `resume()` needs) and the reply is scheduled on it, so **playback does not
/// depend on the capture having succeeded** (the phase review's finding 4): a page whose
/// `getUserMedia` was refused or whose device is busy still hears the greeting and every reply, and
/// the microphone is simply one consumer of a context that exists either way. `None` is "the user has
/// not pressed yet", which is also the only state in which a `Frame::Speech` has nowhere to go.
pub type AudioSlot = Rc<RefCell<Option<web_sys::AudioContext>>>;

/// Why the microphone will not work, once the page knows — the table value the gate showed, kept for
/// the whole run.
///
/// **The fact has to outlive the press that learned it.** The probe runs when the page loads, before
/// `Welcome` and therefore before any screen exists, and a capture can fail on a press that happens
/// while no screen is up either; the login screen that opens afterwards must say `login_no_mic` and
/// must not claim a `● rec` it does not have. `None` is "nothing has refused yet", which is also the
/// state of a page whose microphone works.
pub type MicFailure = Rc<RefCell<Option<&'static str>>>;

/// The one place a refused microphone becomes words the user can read, and the reason the gate can go
/// away.
///
/// **Spec §9's rule, on this client**: with a screen up the line is the screen's own (`login_no_mic`,
/// "type it, that works too"), and with no screen it is the HUD's notice — the sink that was behind an
/// opaque full-viewport gate before the fix wave, which is why the gate only leaves once this has
/// been said. The screen's `voice` switch is turned **off**, which is a decision and not a detail:
/// `voice = false` is the state in which `UiState`'s own key table makes `Tab` do nothing
/// (`state.rs:414`), so the screen cannot raise a `LoginStart` for a device that will not open, and
/// the `● rec` badge (`render_login.rs:212`) stays off. The native client instead retries the device
/// every frame (`crates/tipsy-compadre/src/client/mic.rs:93-124`); a browser cannot — a second
/// `getUserMedia` needs a second gesture, and the gate that had the first one is gone — so the honest
/// answer is the typing-only screen the spec describes.
pub fn record_no_microphone(state: &mut UiState, reason: &'static str) {
    state.mic_open = false;
    if state.login.is_none() {
        state.notify(reason);
    }
    say_no_microphone_on_screen(state);
}

/// The screen's half of [`record_no_microphone`], for a screen that opens **after** the microphone
/// refused: the fact is carried by [`MicFailure`] and read once a frame, so a client whose probe
/// answered at load does not open a login screen claiming a capture it can never have.
///
/// It says nothing while no screen is up — there is no second notice to raise, and the HUD has
/// already been told once — and it is idempotent, so the frame loop may call it every frame.
pub fn carry_no_microphone(state: &mut UiState) {
    say_no_microphone_on_screen(state);
}

/// The screen's line and its two switches, and nothing else: the HUD's notice is only for the moment
/// the failure happens, because a screen that is up is where the user is looking.
fn say_no_microphone_on_screen(state: &mut UiState) {
    // `strings()` hands back a `&'static Strings`, so the borrow of `state` ends with this statement
    // and the borrow below is free of it — the same pattern the native client's failure arm uses.
    let no_mic = state.strings().login_no_mic;
    let Some(login) = state.login.as_mut() else {
        return;
    };
    login.voice = false;
    login.open = false;
    // The line says why once. A refusal, an utterance or a `Transcribing` that lands afterwards is
    // what the user needs next, and `status_line` prefers `Failed` over all of them
    // (`render_login.rs:237-248`) — so this writes over `Prompt` and nothing else.
    if matches!(login.line, LoginLine::Prompt) {
        login.line = LoginLine::Failed(no_mic.to_string());
    }
}

/// How much of the live capture the widget's core pulses on, in frames.
///
/// Twelve 320-sample frames are ≈ 240 ms of history — three of the HUD's own
/// `ENVELOPE_HOP_MS = 50` windows (`crates/tipsy-ui/src/state.rs:16`), which is the unit
/// `UiState::voice_level` indexes an envelope in (`:800-803`). Short enough that the core reads as
/// following the voice rather than the room.
pub const LOGIN_ENVELOPE_POINTS: usize = 12;

/// The one place a captured frame may reach the socket, called once per frame from the render loop.
///
/// It takes `&mut UiState` rather than the `Rc<RefCell<UiState>>` the rest of the client passes
/// around, and that is not a style choice: the render loop already holds the state's mutable borrow
/// for the whole frame, so a second `borrow_mut()` from inside it would panic.
///
/// **The gate is the login door's predicate, and it is the only one.** `door.sends_frames(state)` is
/// the screen's own `voice && open` while a screen is up, and the HUD's `state.mic_open` while none is
/// (`login_door.rs`'s own doc says why both cases are the shared state's) — so a frame is refused
/// whenever the door is shut: a `voice = false` screen, `Tab` off, the third refusal, a failed device,
/// a pass, and the moments before `Welcome` with the HUD's switch off. **A refused frame is dropped,
/// not kept**: keeping it would send it the moment the door opened and the server would transcribe a
/// sentence said to a closed door as the user's answer (the login plan's review finding 1, on this
/// client).
///
/// **The gain is applied here and not in [`levels_from_samples`]**, which is where the native client's
/// `* self.mic_gain` lands too: `[listening] mic_gain` is the configuration's promise and the
/// oscilloscope is what shows it (the phase review's finding 7).
///
/// A send that fails because the socket is down is dropped too: the session has no use for a
/// second-old word, and a queue here would be a delay disguised as reliability.
pub fn drain_capture(
    capture: &Capture,
    door: &crate::login_door::Door,
    state: &mut tipsy_ui::UiState,
    outbox: &Outbox,
    gain: &MicGain,
) {
    // Read once, before the queue is emptied: the door's answer is a function of the state's login
    // fields, which nothing below touches.
    let sending = door.sends_frames(state);
    let capturing = door.capture_open(state);
    let drained = drain_frames(&mut capture.frames.borrow_mut(), sending);
    for level in drained.levels {
        state.push_mic_level(gain.apply(level));
    }
    for samples in drained.send {
        let _ = outbox.send(ToServer::Audio(Frame::Mic(samples)));
    }
    // The core's pulse, for as long as the capture runs. Login spec §6 wants it to follow the voice
    // "while the door is shut", and the widget draws it with the HUD's own `draw_core`, whose whole
    // amplitude is `UiState::voice_level()` — which reads `voice_envelope` and not `mic_levels`
    // (`crates/tipsy-ui/src/state.rs:800-803`). Without this the core stands still for the whole of
    // the door, which is exactly the feedback §6 says the screen needs. A reply's own envelope is
    // left alone: nothing is spoken behind the veil — the greeting is withheld until the verdict
    // (login spec §10).
    if capturing {
        let points = state.mic_levels.len().min(LOGIN_ENVELOPE_POINTS);
        let envelope = state.mic_levels[state.mic_levels.len() - points..].to_vec();
        state.start_voice(envelope);
    }
}

/// The gate's DOM: the button and the line under it.
struct Gate {
    button: web_sys::HtmlButtonElement,
    /// Starts as the hint and becomes the reason when the probe says the microphone cannot work: one
    /// element, so the two can never be on screen together.
    reason: web_sys::Element,
}

/// Builds the gate and puts it on the page. `None` when the page has no body to hang it on, which is
/// the only way this can fail.
///
/// The gate is built from Rust rather than written into the page for the reason the whole task is
/// shaped this way: the *decision* it shows (`MicAvailability`) is a value with tests under it, and
/// Task 3.5 may restyle or move this element without touching that value.
fn gate_elements(document: &web_sys::Document, strings: &tipsy_ui::Strings) -> Option<Gate> {
    let body = document.body()?;
    let overlay = document.create_element("div").ok()?;
    overlay.set_attribute("id", GATE_ID).ok()?;
    // `disabled` lives on the button element and not on `HtmlElement`
    // (`web-sys-0.3.106/src/features/gen_HtmlButtonElement.rs`), which is why this is not a
    // `HtmlElement`.
    let button = document.create_element("button").ok()?.dyn_into::<web_sys::HtmlButtonElement>().ok()?;
    button.set_text_content(Some(strings.mic_start));
    let reason = document.create_element("p").ok()?;
    reason.set_text_content(Some(strings.mic_hint));
    overlay.append_child(&button).ok()?;
    overlay.append_child(&reason).ok()?;
    body.append_child(&overlay).ok()?;
    Some(Gate { button, reason })
}

/// The gate: capability first, the click is the gesture everything audio needs, and **every path out
/// of here takes the gate off the page**.
///
/// The v1 spec requires the press — "In the browser, audio capture requires a user gesture; the web
/// client shows a 'press to start' screen until then" (`2026-10-10-tipsy-compadre-design.md:193-194`)
/// — and the MDN autoplay rule makes it load-bearing for **both** halves: the click must call
/// `getUserMedia` *and* create and resume the `AudioContext` in the same handler, or the user gets a
/// silent client that looks started (`ratzilla-and-wasm-audio.md` §5.2; the prior art's shipped page
/// does exactly this, `pet/src/ui/pet.ts:173`, quoted at `codewhale-audio-prior-art.md` §3).
///
/// **The probe runs first and the answer is data.** When it says the microphone can work, the button
/// becomes the gesture, and `Prompt` and `Granted` both leave the gate up — the button is not only the
/// permission prompt, so a page whose permission is already granted still gets its click.
///
/// **When it says the microphone cannot work, the gate becomes the way past itself** (the phase
/// review's Blocker 2). The reason goes on the gate's own line — the one place in front of the user,
/// because this overlay is opaque and full-viewport — and the button stays **pressable**: the press
/// creates and resumes the audio context (the gesture finding 4 needs), records the failure so the
/// login screen can say `login_no_mic` and the HUD gets its notice, and takes the gate down. What it
/// no longer does is leave a **disabled** button over the page: that is a HUD, a login screen and a
/// message nobody can reach, and the v1 spec's §12 row ("listening disabled with a warning; typing
/// still works") is exactly what the user could not do.
///
/// **A failure after the click is the same shape**: the context has already been created and kept
/// (playback outlives a refused microphone), the reason is recorded for the screen and the HUD, and
/// the gate comes down. The capture is stored in `slot` before the gate goes, so the very next frame
/// the render loop draws already sees it.
///
/// **And the server is told the microphone is open, because the press is the user's own yes.** The
/// press sets `state.mic_open` — the HUD's switch, which is the frame path while no screen is up (the
/// phase review's Blocker 1) — and a switch the server has not been told about is a microphone whose
/// frames it drops: the session ignores `Frame::Mic` while `!listening`
/// (`crates/tipsy-compadre/src/session.rs:534-548`), and `listening` is what `Command::ListenStart`
/// sets. So the press does what `Tab` does when it sets the same flag — the key table raises
/// `ListenStart` with it (`crates/tipsy-ui/src/state.rs:698-705`) — and the browser's push-to-talk is
/// then the native one: the microphone is open, you speak, and `Tab` closes it and sends what it
/// heard. Without this the page uploaded into a void and the user's first `Tab` closed a capture the
/// server had never opened. A socket that is not open yet loses it, like every other command
/// (`Outbox::send`'s `NotOpen`), and the console says so when the failure is anything else.
pub fn press_to_start(
    state: Rc<RefCell<tipsy_ui::UiState>>,
    slot: CaptureSlot,
    audio: AudioSlot,
    failure: MicFailure,
    outbox: Outbox,
) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(gate) = gate_elements(&document, state.borrow().strings()) else {
        return;
    };

    let state_probe = Rc::clone(&state);
    spawn_local(async move {
        // `state`, `document`, `slot`, `audio` and `failure` are this block's own. Each click handler
        // takes a clone of what it needs: `Closure::wrap`'s box is `FnMut` and may run again, so it
        // cannot move out of what the block still owns.
        let availability = probe_availability(&state_probe).await;
        match availability.reason {
            None => {
                let on_click = Closure::wrap(Box::new(move || {
                    let state = Rc::clone(&state);
                    let document = document.clone();
                    let slot = Rc::clone(&slot);
                    let audio = Rc::clone(&audio);
                    let failure = Rc::clone(&failure);
                    let outbox = outbox.clone();
                    spawn_local(async move {
                        // **The context first, on its own** (finding 4): it is playback's clock, and a
                        // `getUserMedia` that is refused below leaves a page that still hears the
                        // greeting and every reply — the microphone is one consumer, not the clock.
                        let context = match open_context().await {
                            Ok(context) => context,
                            Err(name) => {
                                report_no_microphone(&state, &failure, &name, &document);
                                return;
                            }
                        };
                        *audio.borrow_mut() = Some(context.clone());
                        match open_capture(&context).await {
                            Ok(capture) => {
                                let granted = {
                                    let mut state = state.borrow_mut();
                                    // The press is the user's own yes. `mic_open` is the HUD's
                                    // microphone switch — what the `VOZ` panel reads — and a click on
                                    // "press to start" is the same statement `Tab` makes, made with the
                                    // pointer; since the review's Blocker 1 it is also what lets the
                                    // browser's push-to-talk reach `/ws` again once no screen is up.
                                    // Without it the HUD's red oscilloscope would never come on, which
                                    // is why the plan's own Step 8 check ("the HUD's red oscilloscope
                                    // moves when you speak") could not pass as written.
                                    state.mic_open = true;
                                    state.strings().mic_granted
                                };
                                *slot.borrow_mut() = Some(capture);
                                take_gate_down(&document);
                                state.borrow_mut().notify(granted);
                                // The other half of the same statement: the switch the server has to
                                // hear about, or every frame the gate just made possible is dropped
                                // (`session.rs:534-548`). `NotOpen` is the ordinary case on a page
                                // whose socket is still connecting, and the only refusal worth a line
                                // is the other one.
                                if let Err(error) = outbox.send(ToServer::Command(Command::ListenStart))
                                    && !matches!(error, crate::wire::WireError::NotOpen)
                                {
                                    web_sys::console::warn_1(&JsValue::from_str(&format!(
                                        "tipsy-web: {error}"
                                    )));
                                }
                            }
                            Err(name) => report_no_microphone(&state, &failure, &name, &document),
                        }
                    });
                }) as Box<dyn FnMut()>);
                gate.button.set_onclick(Some(on_click.as_ref().unchecked_ref()));
                // The button owns this handler for as long as the page lives: a `Closure` dropped at
                // the end of this block would be freed memory the browser would still call on a click.
                on_click.forget();
            }
            // Not available. The reason is the affordance's own line from here on — one element, so
            // the hint and the reason can never be on screen together — and the press is not "start
            // the microphone" but "start the page without one": the context, the fact, and the way
            // past the gate.
            Some(reason) => {
                gate.reason.set_text_content(Some(reason));
                let on_click = Closure::wrap(Box::new(move || {
                    let state = Rc::clone(&state);
                    let document = document.clone();
                    let audio = Rc::clone(&audio);
                    let failure = Rc::clone(&failure);
                    spawn_local(async move {
                        // Best effort, and deliberately not fatal: the press is the gesture the
                        // context needs, and a page with no context is a page with no audio and a HUD
                        // that still types. The console is where a context that will not start is
                        // readable.
                        match open_context().await {
                            Ok(context) => *audio.borrow_mut() = Some(context),
                            Err(name) => web_sys::console::warn_1(&JsValue::from_str(&format!(
                                "tipsy-web: no audio context ({name})"
                            ))),
                        }
                        *failure.borrow_mut() = Some(reason);
                        record_no_microphone(&mut state.borrow_mut(), reason);
                        take_gate_down(&document);
                    });
                }) as Box<dyn FnMut()>);
                gate.button.set_onclick(Some(on_click.as_ref().unchecked_ref()));
                on_click.forget();
            }
        }
    });
}

/// Takes the gate off the page, and it is the **only** way out of it: the success, the failed press
/// and the press that continues without a microphone all call this and nothing else.
///
/// The overlay is `#tipsy-gate { position: fixed; inset: 0; background: #000 }`
/// (`web/client/index.html:43-46`, the page's own comment calls it the one positioned element), so a
/// gate that stayed up is a HUD, a login screen and a failure message that nobody can see. The phase
/// review's Blocker 2: until this helper existed, the successful capture was the only path that took
/// it down, the probe's refusal left a disabled button over the page, and a refused press wrote its
/// reason into a notice behind the overlay.
fn take_gate_down(document: &web_sys::Document) {
    if let Some(gate) = document.get_element_by_id(GATE_ID) {
        gate.remove();
    }
}

/// A press that could not open the microphone, in the three places that have to know: the fact for the
/// run (so a login screen that opens later says its piece), the state the user is looking at, and the
/// gate, which goes. The exception's own name goes to the console, because a table value is what the
/// user needs and the name is what a reader needs.
fn report_no_microphone(
    state: &Rc<RefCell<tipsy_ui::UiState>>,
    failure: &MicFailure,
    name: &str,
    document: &web_sys::Document,
) {
    web_sys::console::warn_1(&JsValue::from_str(&format!("tipsy-web: microphone unavailable ({name})")));
    // `strings()` hands back a `&'static Strings`, so the borrow of `state` ends with this statement.
    let strings = state.borrow().strings();
    let reason = MicAvailability::from_rejection(name, strings).reason.unwrap_or(strings.mic_failed);
    *failure.borrow_mut() = Some(reason);
    record_no_microphone(&mut state.borrow_mut(), reason);
    take_gate_down(document);
}

/// Reads the two things the capability answer is made of: whether `navigator.mediaDevices` exists at
/// all, and what the microphone's permission state is.
///
/// `PermissionStatus.state` is `granted | denied | prompt`. **The wildcard arm is required, not
/// laziness**: `web_sys::PermissionState`'s three variants are written out in
/// `gen_PermissionState.rs` but the `#[wasm_bindgen]` attribute adds `#[non_exhaustive]` to the
/// generated enum — measured by the compiler, which refuses the three-arm match with "pattern `_` not
/// covered" — so `prompt` and anything a newer browser adds both mean "ask the user". A browser that
/// cannot answer `permissions.query` is not a browser that cannot record, so `navigator.permissions`
/// being absent is `Prompt`, not a refusal.
async fn probe_availability(state: &Rc<RefCell<tipsy_ui::UiState>>) -> MicAvailability {
    // `strings()` returns a `&'static Strings`, so the borrow of `state` ends with this statement and
    // the awaits below hold no borrow at all.
    let strings = state.borrow().strings();
    let Some(window) = web_sys::window() else {
        return MicAvailability::probe(false, MicState::Failed, strings);
    };
    // `media_devices()` is the presence check MDN describes: `undefined` in an insecure context.
    if window.navigator().media_devices().is_err() {
        return MicAvailability::probe(false, MicState::Unsupported, strings);
    }
    let permission = match window.navigator().permissions() {
        Ok(permissions) => match permissions.query(&microphone_descriptor()) {
            Ok(promise) => match JsFuture::from(promise).await {
                Ok(status) => match status.unchecked_into::<web_sys::PermissionStatus>().state() {
                    web_sys::PermissionState::Granted => MicState::Granted,
                    web_sys::PermissionState::Denied => MicState::Denied,
                    web_sys::PermissionState::Prompt | _ => MicState::Prompt,
                },
                // A `TypeError` for a name the browser does not know lands here: a browser that
                // cannot say is asked the old-fashioned way, by pressing the button.
                Err(_) => MicState::Failed,
            },
            Err(_) => MicState::Failed,
        },
        Err(_) => MicState::Prompt,
    };
    MicAvailability::probe(true, permission, strings)
}

/// `{ name: "microphone" }`, built by hand because `web_sys::PermissionName` in 0.3.106 has four
/// variants — `Geolocation`, `Notifications`, `Push`, `PersistentStorage` (`gen_PermissionName.rs`)
/// — with **no `Microphone`**, so the typed descriptor cannot be spelled at all. `Permissions::query`
/// takes a `&js_sys::Object` anyway (`gen_Permissions.rs`).
fn microphone_descriptor() -> js_sys::Object {
    let descriptor = js_sys::Object::new();
    js_sys::Reflect::set(&descriptor, &JsValue::from_str("name"), &JsValue::from_str("microphone"))
        .expect("setting a property on a fresh object cannot fail");
    descriptor
}

/// A rejected promise's `name`, which is what [`MicAvailability::from_rejection`] reads.
///
/// `Reflect::get` and not `web_sys::DomException::name`, for the reason `socket::error_text` gives:
/// the rejected value is not always an `Error`, and the two names that matter (`NotAllowedError`,
/// `NotFoundError`) are plain object properties. A value with no `name` falls back to its message.
fn rejection_name(value: &JsValue) -> String {
    js_sys::Reflect::get(value, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_else(|| crate::socket::error_text(value))
}

/// The one `AudioContext` per run, created and resumed **inside the click handler**, because the
/// gesture is what `resume()` needs.
///
/// It is its own function since the phase review's finding 4, and that is the whole point: the press
/// creates it whether or not a microphone follows, so the reply has a clock even on a page whose
/// `getUserMedia` is refused. `AudioContextOptions.sampleRate` is a *request* — a browser that refuses
/// the rate is not a browser without audio, because the resampler does the work either way — and
/// `set_sample_rate` rather than the plan's `sample_rate`, because the deprecated builder is a new
/// clippy finding (web-sys 0.3.106).
async fn open_context() -> Result<web_sys::AudioContext, String> {
    let options = web_sys::AudioContextOptions::new();
    options.set_sample_rate(MIC_RATE as f32);
    let context = web_sys::AudioContext::new_with_context_options(&options)
        .or_else(|_| web_sys::AudioContext::new())
        .map_err(|error| rejection_name(&error))?;
    // A context created outside a gesture starts suspended; awaiting the resume is what makes it run.
    let resumed = context.resume().map_err(|error| rejection_name(&error))?;
    JsFuture::from(resumed).await.map_err(|error| rejection_name(&error))?;
    Ok(context)
}

/// Opens the capture on a context the press has already made, **inside the click handler**, because
/// `getUserMedia` needs the gesture too.
///
/// Returns the exception's `name` on failure, which is exactly what [`MicAvailability::from_rejection`]
/// consumes, so a failure and a refused probe describe themselves the same way. The context is a
/// parameter rather than a local since the phase review's finding 4: it belongs to the page, not to
/// the microphone, and a capture that fails must not take the reply's clock down with it.
async fn open_capture(context: &web_sys::AudioContext) -> Result<Capture, String> {
    // 1. The stream. Mono, and the browser's own processing left on. **The 16 kHz hint cannot travel
    //    in these constraints**: `web-sys 0.3.106`'s `MediaTrackConstraints` has no `sample_rate`
    //    accessor at all, which matches MDN's "not Baseline" flag on `sampleRate` — so the rate is
    //    settled by the context and the resampler instead.
    let navigator = web_sys::window().ok_or_else(|| "no window".to_string())?.navigator();
    let devices = navigator.media_devices().map_err(|error| rejection_name(&error))?;
    let track = web_sys::MediaTrackConstraints::new();
    track.set_channel_count(&JsValue::from_f64(1.0));
    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio_media_track_constraints(&track);
    let promise = devices
        .get_user_media_with_constraints(&constraints)
        .map_err(|error| rejection_name(&error))?;
    let stream: web_sys::MediaStream =
        JsFuture::from(promise).await.map_err(|error| rejection_name(&error))?.unchecked_into();

    // 2. The worklet module, as a real file at WORKLET_URL rather than a `Blob` URL: the research
    //    note could not verify that `addModule` accepts one (`ratzilla-and-wasm-audio.md` §4.4) and
    //    this plan does not bet on an unverified API.
    let worklet = context.audio_worklet().map_err(|error| rejection_name(&error))?;
    let module = worklet.add_module(WORKLET_URL).map_err(|error| rejection_name(&error))?;
    JsFuture::from(module).await.map_err(|error| rejection_name(&error))?;

    // 3. The node. Zero outputs: nothing is played back into the microphone's own graph.
    let node_options = web_sys::AudioWorkletNodeOptions::new();
    node_options.set_number_of_inputs(1);
    node_options.set_number_of_outputs(0);
    let node = web_sys::AudioWorkletNode::new_with_options(context, "mic-frames", &node_options)
        .map_err(|error| rejection_name(&error))?;
    let source = context.create_media_stream_source(&stream).map_err(|error| rejection_name(&error))?;
    source.connect_with_audio_node(&node).map_err(|error| rejection_name(&error))?;

    // 4. The port. **The handler appends and never sends**: the queue is drained by the render loop,
    //    which is the one place the state may be borrowed and the one place a socket call belongs.
    let frames: Rc<RefCell<VecDeque<Vec<i16>>>> = Rc::new(RefCell::new(VecDeque::new()));
    // Read back, never assumed: this is what the context actually runs at.
    let rate = context.sample_rate() as u32;
    let port = node.port().map_err(|error| rejection_name(&error))?;
    let mut assembler = FrameAssembler::new();
    let on_message = Closure::wrap(Box::new({
        let frames = Rc::clone(&frames);
        move |event: web_sys::MessageEvent| {
            for frame in frames_from_block(&mut assembler, rate, &channels_from(&event.data())) {
                enqueue_frame(&mut frames.borrow_mut(), frame);
            }
        }
    }) as Box<dyn FnMut(web_sys::MessageEvent)>);
    port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    Ok(Capture { context: context.clone(), node, on_message, stream, frames })
}

/// The channels the worklet posted, as plain vectors.
///
/// The worklet posts `channels.map((channel) => channel.slice(0))` — an array of `Float32Array`s —
/// so this is exactly that shape and nothing else: a message that is not an array is an empty block,
/// and the worklet already refuses to post one whose first channel is empty.
fn channels_from(data: &JsValue) -> Vec<Vec<f32>> {
    if !js_sys::Array::is_array(data) {
        return Vec::new();
    }
    let channels = js_sys::Array::from(data);
    (0..channels.length())
        .map(|i| js_sys::Float32Array::new(&channels.get(i)).to_vec())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_render_quantum_is_not_a_frame() {
        // 128 frames = 8 ms at 16 kHz (MDN, quoted at ratzilla-and-wasm-audio.md §4.5).
        let mut assembler = FrameAssembler::new();
        assert!(assembler.push_block(&[vec![0.0; 128]]).is_empty());
        assert_eq!(assembler.pending_len(), 128);
    }

    #[test]
    fn three_quanta_make_one_frame_and_leave_sixty_four_samples() {
        // 320 = 2 × 128 + 64: 320 is not a multiple of the render quantum, which is exactly why the
        // remainder has to be carried.
        let mut assembler = FrameAssembler::new();
        assert!(assembler.push_block(&[vec![0.0; 128]]).is_empty());
        assert!(assembler.push_block(&[vec![0.0; 128]]).is_empty());
        let frames = assembler.push_block(&[vec![0.0; 128]]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 320);
        assert_eq!(assembler.pending_len(), 64);
    }

    #[test]
    fn a_partial_quantum_is_carried_and_never_dropped() {
        let mut assembler = FrameAssembler::new();
        // A block larger than a frame emits a frame and keeps the rest.
        let frames = assembler.push_block(&[vec![0.5; 400]]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 320);
        assert_eq!(assembler.pending_len(), 80);
        // Then 240 more completes the second frame exactly.
        let frames = assembler.push_block(&[vec![0.5; 240]]);
        assert_eq!(frames.len(), 1);
        assert_eq!(assembler.pending_len(), 0);
    }

    #[test]
    fn a_block_that_crosses_two_frames_returns_both() {
        let mut assembler = FrameAssembler::new();
        let frames = assembler.push_block(&[vec![0.0; 640]]);
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|frame| frame.len() == 320));
        assert_eq!(assembler.pending_len(), 0);
    }

    #[test]
    fn two_channels_are_averaged_like_the_native_capture() {
        // `audio/device.rs:186-189` averages the channels into mono before resampling.
        let mut assembler = FrameAssembler::new();
        let frames = assembler.push_block(&[vec![1.0; 320], vec![-1.0; 320]]);
        assert_eq!(frames, vec![vec![0i16; 320]]);
    }

    /// The read-back caveat, and **the arithmetic the plan's own version of this test got wrong**:
    /// it wrote "three 48 kHz blocks are exactly one 16 kHz frame, with nothing left over" and
    /// asserted `pending_len() == 0` — but three blocks of 480 samples are 1440 samples at 48 kHz,
    /// which is 480 at 16 kHz: one 320-sample frame **and 160 samples still to come**. Two blocks are
    /// the exact pair, and the third is what proves the carry survives a resampled block boundary.
    #[test]
    fn the_context_rate_is_resampled_to_sixteen_kilohertz() {
        let block = vec![0.25f32; 480];
        let mono = resample_linear(&block, 48_000, 16_000);
        assert_eq!(mono.len(), 160, "480 samples at 48 kHz are 160 at 16 kHz");

        let mut assembler = FrameAssembler::new();
        let mut frames = Vec::new();
        for _ in 0..2 {
            frames.extend(assembler.push_block(&[resample_linear(&block, 48_000, 16_000)]));
        }
        assert_eq!(frames.len(), 1, "two 48 kHz blocks are exactly one 16 kHz frame");
        assert_eq!(frames[0].len(), 320);
        assert_eq!(assembler.pending_len(), 0);

        // And the third block carries: half a frame now, waiting for the next half.
        frames.extend(assembler.push_block(&[resample_linear(&block, 48_000, 16_000)]));
        assert_eq!(frames.len(), 1);
        assert_eq!(assembler.pending_len(), 160, "the tail of a resampled block is carried too");
    }

    #[test]
    fn the_resampler_matches_the_native_one() {
        // `crates/tipsy-compadre/src/audio/pcm.rs`'s own arithmetic: out_len = 2 * 4 / 2 = 4,
        // step = 0.5.
        assert_eq!(resample_linear(&[0.0, 1.0], 2, 4), vec![0.0, 0.5, 1.0, 1.0]);
        assert_eq!(resample_linear(&[1.0, 2.0], 16_000, 16_000), vec![1.0, 2.0]);
        assert!(resample_linear(&[], 48_000, 16_000).is_empty());
        // A rate of zero is not a rate: nothing is invented and nothing panics.
        assert!(resample_linear(&[1.0], 0, 16_000).is_empty());
        assert!(resample_linear(&[1.0], 16_000, 0).is_empty());
    }

    #[test]
    fn samples_convert_the_way_the_native_capture_converts_them() {
        // `pcm.rs` and `audio/device.rs:191` are the same expression; the browser's copy must not
        // round differently.
        assert_eq!(f32_to_i16(&[0.0, 1.0, -1.0]), vec![0, 32767, -32767]);
        assert_eq!(f32_to_i16(&[2.0, -2.0]), vec![32767, -32767]);
        assert_eq!(rms(&[1.0, -1.0]), 1.0);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn the_eight_rejection_names_map_to_the_states_the_hud_shows() {
        // MDN's documented list, quoted at ratzilla-and-wasm-audio.md §4.1. Only `NotAllowedError`
        // and `SecurityError` mean the user said no; the rest are device or context trouble, and
        // treating them all as a denial would lie in the gate's own message.
        assert_eq!(mic_state_for("NotAllowedError"), MicState::Denied);
        assert_eq!(mic_state_for("SecurityError"), MicState::Denied);
        assert_eq!(mic_state_for("TypeError"), MicState::Unsupported); // no `navigator.mediaDevices`
        assert_eq!(mic_state_for("NotFoundError"), MicState::NoDevice);
        assert_eq!(mic_state_for("NotReadableError"), MicState::Busy);
        assert_eq!(mic_state_for("OverconstrainedError"), MicState::Unsupported);
        assert_eq!(mic_state_for("AbortError"), MicState::Failed);
        assert_eq!(mic_state_for("InvalidStateError"), MicState::Failed);
        assert_eq!(mic_state_for("SomethingNewError"), MicState::Failed);
    }

    #[test]
    fn the_gate_is_disabled_with_a_reason_instead_of_failing_on_first_use() {
        // Capability first: the affordance is offered only when the microphone can actually open, and
        // when it cannot, the reason is a value out of the table (the prior art's shape,
        // codewhale-audio-prior-art.md §4).
        let es = tipsy_ui::strings(tipsy_protocol::UiLanguage::Es);
        let en = tipsy_ui::strings(tipsy_protocol::UiLanguage::En);
        assert_eq!(
            MicAvailability::probe(true, MicState::Prompt, es),
            MicAvailability { available: true, reason: None }
        );
        assert_eq!(
            MicAvailability::probe(true, MicState::Granted, es),
            MicAvailability { available: true, reason: None }
        );
        let denied = MicAvailability::probe(true, MicState::Denied, es);
        assert!(!denied.available);
        assert_eq!(denied.reason, Some(es.mic_denied));
        // No `navigator.mediaDevices` at all is its own reason: MDN's second documented half is that
        // the object is `undefined` in an insecure context, so telling the user "denied" there would
        // send them to the wrong setting (ratzilla-and-wasm-audio.md §4.1).
        let insecure = MicAvailability::probe(false, MicState::Unsupported, en);
        assert!(!insecure.available);
        assert_eq!(insecure.reason, Some(en.mic_no_api));
        // The same shape answers a rejection that arrives later, from the exception's name.
        assert_eq!(
            MicAvailability::from_rejection("NotFoundError", es),
            MicAvailability { available: false, reason: Some(es.mic_nodevice) }
        );
        assert_eq!(
            MicAvailability::from_rejection("NotReadableError", es),
            MicAvailability { available: false, reason: Some(es.mic_busy) }
        );
        // Every reason is one of the table's own values, in both languages: a `&'static str` that is
        // not in either table cannot be produced by these two constructors.
        for availability in [
            MicAvailability::probe(false, MicState::Failed, es),
            MicAvailability::probe(true, MicState::Denied, es),
            MicAvailability::probe(true, MicState::NoDevice, es),
            MicAvailability::probe(true, MicState::Busy, es),
            MicAvailability::probe(true, MicState::Failed, es),
        ] {
            assert!(!availability.available);
            assert!(availability.reason.is_some());
        }
        // A device that is there and a permission that is granted is the only pair that opens.
        assert!(MicAvailability::probe(true, MicState::Granted, en).available);
    }

    /// The frame the server is built around, and the file the module URL names.
    ///
    /// The first half is the protocol constant, twin-checked against the native one's own arithmetic
    /// (`audio/device.rs:18-20`: 16000 / 50). The second half is the drift this task found and fixed:
    /// the plan writes `WORKLET_URL = "/assets/mic-worklet.js"` **and** creates the file at
    /// `web/client/mic-worklet.js`, and the embed serves the repository's `web/` under `/assets/`, so
    /// those two lines cannot both be true. The constant follows the file, and this test is what keeps
    /// them together: it reads the URL back into a path and asserts the file is there, so a later task
    /// that moves one half without the other fails here rather than as a 404 in a browser.
    #[test]
    fn the_frame_is_the_native_captures_and_the_worklet_is_where_the_url_says_it_is() {
        assert_eq!(FRAME_SAMPLES, (MIC_RATE as usize) / 50, "20 ms at 16 kHz");
        assert_eq!(FRAME_SAMPLES, 320);

        let relative = WORKLET_URL.strip_prefix("/assets/").expect("the bundle is served under /assets/");
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web").join(relative);
        let bytes = std::fs::read(&file).unwrap_or_else(|error| {
            panic!("{WORKLET_URL} must be a file in the bundle: {} ({error})", file.display())
        });
        let source = String::from_utf8(bytes).expect("the worklet is a text module");
        assert!(
            source.contains("registerProcessor"),
            "a module the browser loads has to register its processor: {}",
            file.display()
        );
    }

    /// The bound, and which end loses. `enqueue_frame` is the twin of the native capture's bounded
    /// `try_send`: the frame that arrives when the queue is full is the one that goes.
    #[test]
    fn a_queue_that_is_full_drops_the_newest_frame_like_the_native_capture() {
        let mut queue = VecDeque::new();
        for n in 0..MAX_QUEUED_FRAMES {
            enqueue_frame(&mut queue, vec![n as i16; FRAME_SAMPLES]);
        }
        assert_eq!(queue.len(), MAX_QUEUED_FRAMES);
        enqueue_frame(&mut queue, vec![-1; FRAME_SAMPLES]);
        assert_eq!(queue.len(), MAX_QUEUED_FRAMES, "the queue cannot grow past its bound");
        assert_eq!(queue.back().unwrap()[0], (MAX_QUEUED_FRAMES - 1) as i16, "the newest one is the loss");
        assert_eq!(queue.front().unwrap()[0], 0, "and the oldest is still first in line");
    }

    /// The gate, as a value: the drain sends exactly what it was told to and keeps nothing back.
    ///
    /// This is the half of `drain_capture` a host test can reach — the other half is a DOM send — and
    /// it is the half the login plan's review finding 1 was in: **a refused frame is dropped, not
    /// kept**, so a sentence said while the gate was shut can never be sent when it opens.
    #[test]
    fn the_drain_sends_only_while_the_gate_is_open_and_keeps_no_frame_behind() {
        let mut queue: VecDeque<Vec<i16>> = VecDeque::new();
        enqueue_frame(&mut queue, vec![1; FRAME_SAMPLES]);
        enqueue_frame(&mut queue, vec![2; FRAME_SAMPLES]);

        let shut = drain_frames(&mut queue, false);
        assert!(shut.send.is_empty(), "a shut gate sends nothing");
        assert_eq!(shut.levels.len(), 2, "but every frame drained has a level for the oscilloscope");
        assert!(queue.is_empty(), "and nothing is kept for later");

        enqueue_frame(&mut queue, vec![1; FRAME_SAMPLES]);
        enqueue_frame(&mut queue, vec![2; FRAME_SAMPLES]);
        let open = drain_frames(&mut queue, true);
        assert_eq!(open.send, vec![vec![1; FRAME_SAMPLES], vec![2; FRAME_SAMPLES]], "in the order captured");
        assert_eq!(open.levels, shut.levels, "the loudness does not depend on the gate");
        assert!(queue.is_empty());
        // An empty queue is an empty drain, not a panic and not a phantom frame.
        assert_eq!(drain_frames(&mut queue, true), Drained::default());
    }

    /// The loudness is the native client's own conversion, to the digit that shows the difference:
    /// `i16_to_f32` divides by 32768, so full scale is just under one, and a `1.0` here would be the
    /// plan's Task 3.7 sketch (`/ 32767.0`), which is nobody's twin.
    #[test]
    fn the_level_is_the_native_captures_own_conversion() {
        assert_eq!(levels_from_samples(&[]), 0.0);
        assert_eq!(levels_from_samples(&[0; FRAME_SAMPLES]), 0.0);
        let full = levels_from_samples(&[i16::MAX; FRAME_SAMPLES]);
        assert!(full < 1.0 && full > 0.999, "32767 / 32768, and a full-scale sample is not 1.0: {full}");
        assert_eq!(levels_from_samples(&[i16::MIN; FRAME_SAMPLES]), 1.0, "-32768 / 32768 is exactly one");
        assert_eq!(i16_to_f32(&[i16::MAX, i16::MIN]), vec![32767.0 / 32768.0, -1.0]);
    }

    /// The exact bytes the live check puts on the wire, pinned here so the check cannot drift from
    /// the client's own encoder.
    ///
    /// The check's script builds three frames from the integer rule `sample(i) = (i * 37) % 1601 -
    /// 800` and sends them as binary WebSocket messages. This test builds the same three frames and
    /// encodes them with `Frame::encode`, which is what `Outbox::send` does for a `Frame::Mic`
    /// (`crate::socket::Outbox::send` → `wire::frame_to_bytes`) — so the script and the client are
    /// pinned to one literal, and the `-5522` below is the sum the check's stub has to read back out
    /// of the WAV the server sends it.
    #[test]
    fn the_bytes_the_live_check_sends_are_the_clients_own_frame_encoding() {
        let samples: Vec<i16> = (0..FRAME_SAMPLES * 3).map(|i| ((i * 37) % 1601) as i16 - 800).collect();
        let frames: Vec<Vec<i16>> = samples.chunks(FRAME_SAMPLES).map(<[i16]>::to_vec).collect();
        assert_eq!(frames.len(), 3, "three frames of 320");
        assert!(frames.iter().all(|frame| frame.len() == FRAME_SAMPLES));

        let first = tipsy_protocol::Frame::Mic(frames[0].clone()).encode();
        assert_eq!(first.len(), 1 + FRAME_SAMPLES * 2, "one kind byte and 320 little-endian samples");
        assert_eq!(first[0], 0x01, "KIND_MIC, the frame the server reads as the microphone");
        // -800 is 0xFCE0 and -763 is 0xFD05, so the first five bytes are these.
        assert_eq!(&first[..5], &[0x01, 0xE0, 0xFC, 0x05, 0xFD]);
        assert_eq!(i16::from_le_bytes([first[1], first[2]]), -800);
        assert_eq!(i16::from_le_bytes([first[3], first[4]]), -763);
        assert_eq!(i16::from_le_bytes([first[639], first[640]]), -204, "sample 319 is this frame's last");

        assert_eq!(samples[0], -800);
        assert_eq!(samples[320], -167);
        assert_eq!(samples[639], 429);
        assert_eq!(samples[959], -539);
        assert_eq!(samples.iter().map(|s| i64::from(*s)).sum::<i64>(), -5522);
    }

    /// What the *server* does to those samples on the way to its transcriber, which is what the live
    /// check's stub reads back out of the WAV.
    ///
    /// `session.rs` puts the frames into a `Pcm` through `i16_to_f32` (`samples: i16_to_f32(&samples)`,
    /// `sample_rate: MIC_SAMPLE_RATE`) and the transcribe provider encodes that with `encode_wav_16k`,
    /// which resamples 16 kHz to 16 kHz — the identity — and converts back with `f32_to_i16`. **The
    /// round trip is lossy by exactly one unit in magnitude**, because the two twins deliberately use
    /// different constants (÷ 32768 out, × 32767 in): `429 / 32768 * 32767` is `428.999…` and truncates
    /// to `428`, and `-800` truncates to `-799`. Split into 0 and the sign, that is the whole rule, and
    /// the sums below are the numbers the live check compares its stub's WAV against — so a check that
    /// silently lost or reordered samples cannot pass by luck.
    #[test]
    fn the_round_trip_the_server_applies_is_the_number_the_live_check_looks_for() {
        let samples: Vec<i16> = (0..FRAME_SAMPLES * 3).map(|i| ((i * 37) % 1601) as i16 - 800).collect();
        let round_tripped = f32_to_i16(&i16_to_f32(&samples));
        let by_the_rule: Vec<i16> = samples
            .iter()
            .map(|s| match s.cmp(&0) {
                std::cmp::Ordering::Greater => s - 1,
                std::cmp::Ordering::Less => s + 1,
                std::cmp::Ordering::Equal => 0,
            })
            .collect();
        assert_eq!(round_tripped, by_the_rule, "one unit off in magnitude, and never more");
        assert_eq!(round_tripped.len(), 960, "not one sample is lost on the way");
        assert_eq!(round_tripped.iter().map(|s| i64::from(*s)).sum::<i64>(), -5514);
        assert_eq!(&round_tripped[..1], &[-799]);
        assert_eq!(round_tripped[320], -166);
        assert_eq!(round_tripped[639], 428);
        assert_eq!(round_tripped[959], -538);
    }

    /// **`[listening] mic_gain`, applied** (the phase review's finding 7), with the two rules that
    /// make reading it off the wire safe: a form the server has not answered is not an answer, and an
    /// edit the user has not saved is not applied. `1.0` before that, because the number has to come
    /// from the server or from nowhere — a browser has no `config.toml`.
    #[test]
    fn the_configured_gain_is_applied_and_only_from_a_filled_clean_form() {
        // The identity until this client has been told anything, and the native client's arithmetic.
        assert_eq!(MicGain::default().value(), 1.0);
        assert_eq!(MicGain::default().apply(0.25), 0.25);
        assert_eq!(MicGain::of("9.5").apply(0.25), 2.375);
        assert_eq!(MicGain::of(" 12 ").value(), 12.0, "the row's own text, trimmed");
        // A row that is empty or nonsense is the identity rather than a zero or a `NaN`: the form
        // validates what it saves, and this must not draw an oscilloscope at no amplitude.
        for text in ["", "  ", "doce", "0", "-3", "NaN", "inf"] {
            assert_eq!(MicGain::of(text).value(), 1.0, "{text:?} is not a gain");
        }

        let mut state = UiState::new();
        assert_eq!(configured_gain(&state), None, "a closed screen carries no configuration");
        // The gear's own path, as the frame loop sees it: the screen opens on the placeholder.
        assert_eq!(state.on_key(tipsy_ui::KeyInput::F(9)), vec![tipsy_ui::UiAction::OpenConfig]);
        assert!(state.config_form.is_some(), "the screen is up on the placeholder");
        assert_eq!(configured_gain(&state), None, "and a placeholder is not an answer");

        // The server's answer, which `show_config_form` is what marks filled.
        let mut form = tipsy_ui::ConfigForm::default();
        form.listening.mic_gain = "9.5".into();
        state.show_config_form(form);
        assert_eq!(configured_gain(&state), Some(MicGain::of("9.5")));

        // An edit the user has not saved is not the configuration, which is what the form's own
        // `dirty` flag is for.
        {
            let form = state.config_form.as_mut().expect("the screen is up");
            form.listening.mic_gain = "31".into();
            form.dirty = true;
        }
        assert_eq!(configured_gain(&state), None, "an unsaved edit is not applied");
    }

    /// **The refused microphone is said where the user is looking, and a screen that opens afterwards
    /// says it too** (the phase review's Blocker 2). The gate is an opaque full-viewport overlay: before
    /// the fix, the probe's refusal left a disabled button over the page and a failed press wrote its
    /// reason into the HUD's notice *behind* that overlay, so the v1 spec's §12 row ("typing still
    /// works") and login spec §9's own line were both unreachable.
    #[test]
    fn a_refused_microphone_is_said_on_the_screen_and_in_the_huds_notice() {
        let es = tipsy_ui::strings(tipsy_protocol::UiLanguage::Es);

        // No screen: the HUD's notice, which is on top then, and no microphone left claimed.
        let mut state = UiState::new();
        state.mic_open = true;
        record_no_microphone(&mut state, es.mic_denied);
        assert_eq!(state.notice.as_ref().map(|(text, _)| text.as_str()), Some(es.mic_denied));
        assert!(!state.mic_open, "a device that did not open is not a microphone the HUD has");

        // A screen is up: its own line, and never the HUD's notice — the veil is in front of that.
        let mut state = UiState::new();
        state.show_login(true);
        state.login.as_mut().expect("the screen is up").open = true;
        record_no_microphone(&mut state, es.mic_denied);
        let screen = state.login.as_ref().expect("the screen is up");
        assert_eq!(screen.line, tipsy_ui::LoginLine::Failed(es.login_no_mic.to_string()));
        assert!(!screen.voice, "the switch is off, so the screen's `Tab` cannot claim a capture");
        assert!(!screen.open, "and the `● rec` badge goes with it");
        assert!(state.notice.is_none(), "a notice behind the veil is not where this belongs");

        // The carried case: the probe answered at load, before `Welcome` and before any screen.
        let mut state = UiState::new();
        carry_no_microphone(&mut state);
        assert!(state.login.is_none(), "there is nothing to say and no screen to say it on");
        state.show_login(true);
        assert!(state.login.as_ref().expect("the screen is up").voice, "the server's own switch");
        carry_no_microphone(&mut state);
        let screen = state.login.as_ref().expect("the screen is up");
        assert!(!screen.voice && !screen.open, "the screen cannot claim the capture it was given");
        assert_eq!(screen.line, tipsy_ui::LoginLine::Failed(es.login_no_mic.to_string()));

        // And it is idempotent without eating a better line: a refusal or an utterance that lands
        // afterwards is what the user needs next, and `status_line` prefers `Failed` over both.
        state.login.as_mut().expect("the screen is up").line = tipsy_ui::LoginLine::Refused;
        carry_no_microphone(&mut state);
        assert_eq!(
            state.login.as_ref().expect("the screen is up").line,
            tipsy_ui::LoginLine::Refused,
            "the carry may not write over a later line"
        );
    }

    /// **Every way out of the gate takes it down, and no path leaves a disabled button behind.**
    ///
    /// There is no browser on this machine and `press_to_start` takes a real `Document`, so no host
    /// test can click it — the same wall `login_door.rs`'s one-send-site pin hits, and the same
    /// technique: read the committed source and ask about the text. The four paths are the success (the
    /// gate has done its job), the press whose capture failed, the press whose **context** failed, and
    /// the press that continues without a microphone at all; each owes the page a gate-free screen,
    /// because `#tipsy-gate` is `position: fixed; inset: 0` and opaque. The success arm owes the
    /// **server** one thing more — a `ListenStart`, without which the frames the press made possible
    /// are dropped. It is falsifiable: deleting one call, putting `set_disabled` back, or dropping the
    /// command fails it.
    #[test]
    fn every_path_out_of_the_gate_takes_it_down() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("mic.rs");
        let source =
            std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        // Production only: this test's own text names the strings it counts, so the module that holds
        // it has to be cut off first.
        let production = &source[..source.find("#[cfg(test)]").expect("this test's own module follows")];
        assert_eq!(
            production.matches("take_gate_down(").count(),
            4,
            "the helper and the three arms that call it: the success, the failed press (the context \
             failure and the capture failure, both through `report_no_microphone`) and the page whose \
             microphone cannot work at all"
        );
        assert!(
            !production.contains("set_disabled"),
            "a disabled button is the page nobody can get past: the probe's refusal used to leave one \
             over a full-viewport overlay (Blocker 2)"
        );

        let start = source.find("pub fn press_to_start(").expect("`press_to_start` is the gate");
        let rest = &source[start..];
        let end = rest.find("\n}").expect("the function's closing brace");
        let body = &rest[..end];
        assert_eq!(
            body.matches("take_gate_down(").count(),
            2,
            "the two arms inside the gate itself: the capture that opened, and the page that carries \
             on without one:\n{body}"
        );
        assert_eq!(
            body.matches("report_no_microphone(").count(),
            2,
            "and the two failures a press can hit — no context, no capture — both of which record and \
             take the gate down:\n{body}"
        );
        assert_eq!(
            body.matches("record_no_microphone(").count(),
            1,
            "and the probe's own answer, which is recorded when the user presses past it:\n{body}"
        );
        assert!(
            body.contains("ToServer::Command(Command::ListenStart)"),
            "and the success arm tells the server the microphone is open, which is what `Tab` does when \
             it sets the same flag: the session drops every `Frame::Mic` while it is not listening \
             (`session.rs:534-548`), so a press that did not say so would upload into a void:\n{body}"
        );
    }
}
