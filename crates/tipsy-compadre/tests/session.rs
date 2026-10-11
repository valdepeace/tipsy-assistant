// The fake transcriber records what it was handed in a `std::sync::Mutex`, and the tests read those
// records while the session keeps running; holding that guard across an await is deliberate here
// (the alternative is copying the buffer out on every assertion). Test-only, so the lint is allowed
// for this file rather than silenced per test.
#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tipsy_compadre::audio::pcm::Pcm;
use tipsy_compadre::brain::BrainConfig;
use tipsy_compadre::config::Config;
use tipsy_compadre::config::store;
use tipsy_compadre::providers::*;
use tipsy_compadre::session::spawn_session;
use tipsy_compadre::vad::{Earshot, Scorer, VadEvent, VadState};
use tipsy_protocol::{AssistantState, Command, Event, Frame, ListeningMode, ToClient, ToServer, UiLanguage};
use tokio::sync::mpsc;

// ---------- fakes (copied from `tests/brain.rs`) ----------

/// Chat fake: each call pops the next scripted outcome.
struct FakeChat {
    script: Mutex<Vec<Result<Vec<&'static str>, ProviderError>>>,
    requests: Mutex<Vec<ChatRequest>>,
    chunk_delay: Duration,
}

impl FakeChat {
    fn new(script: Vec<Result<Vec<&'static str>, ProviderError>>) -> Arc<Self> {
        Self::slow(script, Duration::ZERO)
    }
    /// The same fake, sleeping `chunk_delay` before each streamed chunk.
    fn slow(script: Vec<Result<Vec<&'static str>, ProviderError>>, chunk_delay: Duration) -> Arc<Self> {
        Arc::new(Self { script: Mutex::new(script), requests: Mutex::new(Vec::new()), chunk_delay })
    }
    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl ChatProvider for FakeChat {
    async fn stream(&self, req: ChatRequest) -> Result<TextStream, ProviderError> {
        self.requests.lock().unwrap().push(req);
        let next = self.script.lock().unwrap().remove(0);
        let chunks = next?;
        let delay = self.chunk_delay;
        Ok(futures::stream::iter(chunks)
            .then(move |c| async move {
                tokio::time::sleep(delay).await;
                Ok(c.to_string())
            })
            .boxed())
    }
}

/// Speech fake: returns one sample per character at 24 kHz; per-text delays and failures.
struct FakeSpeech {
    delays: HashMap<&'static str, Duration>,
    fail_on: Option<&'static str>,
    requests: Mutex<Vec<SpeechRequest>>,
}

impl FakeSpeech {
    fn new() -> Arc<Self> {
        Arc::new(Self { delays: HashMap::new(), fail_on: None, requests: Mutex::new(Vec::new()) })
    }
}

#[async_trait]
impl SpeechProvider for FakeSpeech {
    async fn synthesize(&self, req: SpeechRequest) -> Result<Pcm, ProviderError> {
        self.requests.lock().unwrap().push(req.clone());
        for (prefix, d) in &self.delays {
            if req.text.starts_with(prefix) {
                tokio::time::sleep(*d).await;
            }
        }
        if self.fail_on.is_some_and(|p| req.text.starts_with(p)) {
            return Err(ProviderError::BadResponse("tts exploded".into()));
        }
        Ok(Pcm { samples: vec![0.25; req.text.chars().count()], sample_rate: 24000 })
    }
}

struct FakeTranscribe {
    text: &'static str,
    languages: Mutex<Vec<String>>,
    /// The audio handed to each call, so the hold-to-talk turn can be checked, not just exercised.
    pcms: Mutex<Vec<Pcm>>,
}

#[async_trait]
impl TranscribeProvider for FakeTranscribe {
    async fn transcribe(&self, pcm: Pcm, language: &str) -> Result<String, ProviderError> {
        self.languages.lock().unwrap().push(language.to_string());
        self.pcms.lock().unwrap().push(pcm);
        Ok(self.text.to_string())
    }
}

// ---------- helpers ----------

const S1: &str = "Ozú, quillo, esto es la primera frase.";
const S2: &str = "Y esta es la segunda frase, compadre.";
const S3: &str = "Y la tercera, mi arma.";

/// A real config, so the built-in `compae` profile (and its greeting) is present.
fn config() -> Config {
    let dir = tempfile::tempdir().unwrap();
    store::load_or_create(&dir.path().join("config.toml")).unwrap().config
}

/// Runs a session on fake providers, handing back its ends and the transcribe fake.
fn session(
    chat: Arc<FakeChat>,
    speech: Arc<FakeSpeech>,
    transcript: &'static str,
) -> (mpsc::UnboundedSender<ToServer>, mpsc::UnboundedReceiver<ToClient>, Arc<FakeTranscribe>) {
    let transcribe = Arc::new(FakeTranscribe {
        text: transcript,
        languages: Mutex::new(Vec::new()),
        pcms: Mutex::new(Vec::new()),
    });
    let providers = ProviderSlot::new(Providers { chat, speech, transcribe: transcribe.clone() });
    let (tx, rx) = spawn_session(config(), providers, BrainConfig::default());
    (tx, rx, transcribe)
}

fn send(tx: &mpsc::UnboundedSender<ToServer>, command: Command) {
    tx.send(ToServer::Command(command)).expect("the session must still be listening");
}

fn send_mic(tx: &mpsc::UnboundedSender<ToServer>, samples: Vec<i16>) {
    tx.send(ToServer::Audio(Frame::Mic(samples))).expect("the session must still be listening");
}

/// The next message that matches `pred`, panicking with everything received if the timeout expires.
async fn next_matching(
    rx: &mut mpsc::UnboundedReceiver<ToClient>,
    pred: impl Fn(&ToClient) -> bool,
    timeout_ms: u64,
) -> ToClient {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            panic!("no message matched within {timeout_ms} ms; received {seen:#?}");
        }
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(m)) => {
                if pred(&m) {
                    return m;
                }
                seen.push(m);
            }
            Ok(None) => panic!("the session closed the channel; received {seen:#?}"),
            Err(_) => panic!("no message matched within {timeout_ms} ms; received {seen:#?}"),
        }
    }
}

fn is_speech(m: &ToClient) -> bool {
    matches!(m, ToClient::Audio(Frame::Speech { .. }))
}

fn is_done(m: &ToClient) -> bool {
    matches!(m, ToClient::Event(Event::Done { .. }))
}

fn is_idle(m: &ToClient) -> bool {
    matches!(m, ToClient::Event(Event::Status { state: AssistantState::Idle, .. }))
}

/// Everything received until `wait_ms` has passed without a message, or the session goes quiet.
async fn drain(rx: &mut mpsc::UnboundedReceiver<ToClient>, wait_ms: u64) -> Vec<ToClient> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return out;
        }
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(m)) => out.push(m),
            _ => return out,
        }
    }
}

// ---------- tests ----------

#[tokio::test]
async fn welcome_then_greeting() {
    let config = config();
    let chat = FakeChat::new(vec![]);
    let speech = FakeSpeech::new();
    let (_tx, mut rx, _) = session(chat.clone(), speech.clone(), "");

    let first = next_matching(&mut rx, |_: &ToClient| true, 500).await;
    match first {
        ToClient::Event(Event::Welcome { profile, ui_language, listening_mode, web_bundle, .. }) => {
            assert_eq!(profile.id, "compae");
            assert_eq!(ui_language, UiLanguage::Es);
            assert_eq!(listening_mode, config.listening.mode);
            assert!(!web_bundle);
        }
        other => panic!("the first message must be Welcome, got {other:?}"),
    }

    next_matching(&mut rx, is_speech, 500).await;
    assert_eq!(chat.calls(), 0, "the greeting must not ask the chat provider");
    let spoken = speech.requests.lock().unwrap().clone();
    // The greeting is split into sentences, so the provider may be asked more than once;
    // what matters is that the whole greeting is spoken and the chat provider is untouched.
    let spoken_text = spoken.iter().map(|r| r.text.clone()).collect::<Vec<_>>().join(" ");
    let normalize = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(normalize(&spoken_text), normalize(&config.active().greeting));
}

#[tokio::test]
async fn ask_produces_reply_and_done() {
    let chat = FakeChat::new(vec![Ok(vec![S1])]);
    let (tx, mut rx, _) = session(chat.clone(), FakeSpeech::new(), "");

    // The greeting is a `Say` turn: its `Done` means the session is ready for the next command.
    next_matching(&mut rx, is_done, 500).await;

    send(&tx, Command::Ask { text: "¿qué tal, compadre?".into() });
    let reply = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Reply { .. })), 500).await;
    assert_eq!(reply, ToClient::Event(Event::Reply { text: S1.into() }));
    next_matching(&mut rx, is_done, 500).await;
    assert_eq!(chat.calls(), 1);
}

#[tokio::test]
async fn listen_stop_transcribes_buffered_mic_audio() {
    let chat = FakeChat::new(vec![Ok(vec![S1])]);
    let (tx, mut rx, transcribe) = session(chat, FakeSpeech::new(), "¿y el ticket?");

    next_matching(&mut rx, is_done, 500).await; // the greeting

    send(&tx, Command::ListenStart);
    for _ in 0..10 {
        send_mic(&tx, vec![100; 320]);
    }
    send(&tx, Command::ListenStop);

    let heard = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Heard { .. })), 1000).await;
    assert_eq!(heard, ToClient::Event(Event::Heard { text: "¿y el ticket?".into() }));
    assert_eq!(transcribe.languages.lock().unwrap().as_slice(), ["es-ES".to_string()]);

    let pcms = transcribe.pcms.lock().unwrap();
    assert_eq!(pcms.len(), 1, "ListenStop must transcribe once, and only once: {pcms:#?}");
    assert_eq!(
        pcms[0].samples.len(),
        3200,
        "the ten 320-sample mic frames must reach the transcriber: {pcms:#?}"
    );
    assert_eq!(
        pcms[0].sample_rate,
        16000,
        "mic audio must be resampled to 16 kHz before transcription: {pcms:#?}"
    );
}

#[tokio::test]
async fn reset_clears_history() {
    let chat = FakeChat::new(vec![Ok(vec![S1]), Ok(vec![S2])]);
    let (tx, mut rx, _) = session(chat.clone(), FakeSpeech::new(), "");

    next_matching(&mut rx, is_done, 500).await; // the greeting
    send(&tx, Command::Ask { text: "primera pregunta".into() });
    next_matching(&mut rx, is_done, 500).await;

    send(&tx, Command::Reset);
    send(&tx, Command::Ask { text: "segunda pregunta".into() });
    next_matching(&mut rx, is_done, 500).await;

    let requests = chat.requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "one chat call per Ask");
    let second = requests[1].messages.clone();
    assert_eq!(second.len(), 2, "Reset must drop the earlier turn: {second:#?}");
    assert_eq!(second[0].role, Role::System);
    assert_eq!(second[1], ChatMessage::user("segunda pregunta"));
    assert!(!second.iter().any(|m| m.content == "primera pregunta" || m.content == S1));
}

#[tokio::test]
async fn barge_in_cancels_running_turn() {
    let chat = FakeChat::slow(vec![Ok(vec![S1, " ", S2, " ", S3])], Duration::from_millis(60));
    let (tx, mut rx, _) = session(chat, FakeSpeech::new(), "");

    next_matching(&mut rx, is_done, 500).await; // the greeting
    send(&tx, Command::Ask { text: "una pregunta larga".into() });
    next_matching(&mut rx, is_speech, 1000).await; // the first sentence is being spoken
    send(&tx, Command::Cancel);

    next_matching(&mut rx, is_idle, 500).await;
    let late = drain(&mut rx, 500).await;
    let audio: Vec<&ToClient> = late.iter().filter(|m| is_speech(m)).collect();
    assert!(audio.is_empty(), "no audio may play after Cancel: {audio:#?}");
}

#[tokio::test]
async fn a_long_conversation_keeps_the_prompt_bounded() {
    // Thirty questions is far more than the session keeps, so the prompt has to stop growing:
    // without a cap, the last request would carry every question ever asked and its reply.
    let turns = 30usize;
    let chat = FakeChat::new((0..turns).map(|_| Ok(vec![S1])).collect());
    let (tx, mut rx, _) = session(chat.clone(), FakeSpeech::new(), "");

    next_matching(&mut rx, is_done, 500).await; // the greeting

    for n in 0..turns {
        send(&tx, Command::Ask { text: format!("pregunta {n}") });
        next_matching(&mut rx, is_done, 1000).await;
    }

    let requests = chat.requests.lock().unwrap();
    assert_eq!(requests.len(), turns, "one chat call per Ask");
    let sizes: Vec<usize> = requests.iter().map(|r| r.messages.len()).collect();
    assert!(sizes[0] < sizes[turns - 1], "the prompt must carry the conversation: {sizes:?}");
    // Once the history is full the prompt stops growing: the last ten questions all send the same
    // number of messages, however many were asked before them.
    assert!(
        sizes[turns - 10..].windows(2).all(|w| w[0] == w[1]),
        "the prompt must stop growing: {sizes:?}"
    );

    let last = requests[turns - 1].messages.clone();
    assert_eq!(last[0].role, Role::System, "the profile prompt stays first: {last:#?}");
    assert_eq!(last.last().unwrap(), &ChatMessage::user(format!("pregunta {}", turns - 1)));
}

/// Review Focus 1's live half: the session reads the provider stack when a turn starts, so a
/// save that rebuilds the connectors answers the *next* turn with them — no restart, and the
/// conversation the session was holding is not dropped.
#[tokio::test]
async fn a_rebuilt_provider_stack_answers_the_next_turn() {
    let first = FakeChat::new(vec![Ok(vec!["primera respuesta."])]);
    let second = FakeChat::new(vec![Ok(vec!["segunda respuesta."])]);
    let speech = FakeSpeech::new();
    let transcribe = Arc::new(FakeTranscribe {
        text: "",
        languages: Mutex::new(Vec::new()),
        pcms: Mutex::new(Vec::new()),
    });
    let slot = ProviderSlot::new(Providers {
        chat: first.clone(),
        speech: speech.clone(),
        transcribe: transcribe.clone(),
    });
    let (tx, mut rx) = spawn_session(config(), slot.clone(), BrainConfig::default());
    next_matching(&mut rx, is_done, 500).await; // the greeting

    send(&tx, Command::Ask { text: "primera pregunta".into() });
    let reply = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Reply { .. })), 500).await;
    assert_eq!(reply, ToClient::Event(Event::Reply { text: "primera respuesta.".into() }));
    assert_eq!(first.calls(), 1);

    // The save: the connectors are rebuilt and swapped in place, exactly as the client does it.
    slot.replace(Providers { chat: second.clone(), speech, transcribe });

    send(&tx, Command::Ask { text: "segunda pregunta".into() });
    let reply = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Reply { .. })), 500).await;
    assert_eq!(reply, ToClient::Event(Event::Reply { text: "segunda respuesta.".into() }));
    assert_eq!(second.calls(), 1, "the next turn must use the stack the save installed");
    assert_eq!(first.calls(), 1, "the replaced stack must never be called again");
}

/// The mode the screen saves is the mode the HUD shows: `Welcome` is the only event that carries
/// it, so the session announces the change.
#[tokio::test]
async fn a_new_listening_mode_is_announced_to_the_client() {
    let slot = ProviderSlot::new(Providers {
        chat: FakeChat::new(vec![]),
        speech: FakeSpeech::new(),
        transcribe: Arc::new(FakeTranscribe {
            text: "",
            languages: Mutex::new(Vec::new()),
            pcms: Mutex::new(Vec::new()),
        }),
    });
    let (tx, mut rx) = spawn_session(config(), slot, BrainConfig::default());
    let first = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Welcome { .. })), 500).await;
    assert!(
        matches!(first, ToClient::Event(Event::Welcome { listening_mode: ListeningMode::Push, .. })),
        "{first:?}"
    );

    send(&tx, Command::SetListeningMode { mode: ListeningMode::Auto });

    let again = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Welcome { .. })), 500).await;
    assert!(
        matches!(again, ToClient::Event(Event::Welcome { listening_mode: ListeningMode::Auto, .. })),
        "the header needs a fresh Welcome to show the mode that was just saved: {again:?}"
    );
}

/// The profile half of a save: the switch reaches the running session, so the next turn speaks
/// the new profile's own prompt and the header and the language follow it.
#[tokio::test]
async fn switching_profile_changes_the_prompt_of_the_next_turn() {
    let chat = FakeChat::new(vec![Ok(vec!["Una respuesta suficientemente larga."])]);
    let slot = ProviderSlot::new(Providers {
        chat: chat.clone(),
        speech: FakeSpeech::new(),
        transcribe: Arc::new(FakeTranscribe {
            text: "",
            languages: Mutex::new(Vec::new()),
            pcms: Mutex::new(Vec::new()),
        }),
    });
    let (tx, mut rx) = spawn_session(config(), slot, BrainConfig::default());
    next_matching(&mut rx, is_done, 500).await; // the greeting

    send(&tx, Command::SetProfile { id: "senyor".into() });
    let welcome = next_matching(&mut rx, |m| matches!(m, ToClient::Event(Event::Welcome { .. })), 500).await;
    match welcome {
        ToClient::Event(Event::Welcome { profile, ui_language, .. }) => {
            assert_eq!(profile.id, "senyor");
            assert_eq!(ui_language, UiLanguage::Es);
        }
        other => panic!("the profile switch must be announced in a fresh Welcome, got {other:?}"),
    }

    send(&tx, Command::Ask { text: "¿cuánto cuesta?".into() });
    next_matching(&mut rx, is_done, 500).await;

    let sent = chat.requests.lock().unwrap()[0].messages.clone();
    assert_eq!(sent[0].role, Role::System);
    let senyor = config().profiles.into_iter().find(|p| p.id == "senyor").unwrap().prompt;
    assert_eq!(sent[0].content, senyor, "the next turn must speak as the profile just chosen: {sent:#?}");
}

// ---------- auto mode ----------

/// A real config with the listening mode the test needs; the built-in `compae` profile and its
/// greeting are still present, which is why every test waits for the greeting's `Done` first.
fn auto_config(silence_ms: u32) -> Config {
    let dir = tempfile::tempdir().unwrap();
    let mut config = store::load_or_create(&dir.path().join("config.toml")).unwrap().config;
    config.listening.mode = ListeningMode::Auto;
    config.listening.silence_ms = silence_ms;
    config
}

/// The session ends and the transcribe fake for an `auto` run.
fn auto_session(
    chat: Arc<FakeChat>,
    speech: Arc<FakeSpeech>,
    transcript: &'static str,
    silence_ms: u32,
) -> (mpsc::UnboundedSender<ToServer>, mpsc::UnboundedReceiver<ToClient>, Arc<FakeTranscribe>) {
    let transcribe = Arc::new(FakeTranscribe {
        text: transcript,
        languages: Mutex::new(Vec::new()),
        pcms: Mutex::new(Vec::new()),
    });
    let providers = ProviderSlot::new(Providers { chat, speech, transcribe: transcribe.clone() });
    let (tx, rx) = spawn_session(auto_config(silence_ms), providers, BrainConfig::default());
    (tx, rx, transcribe)
}

/// A microphone frame as the client sends it: 20 ms at 16 kHz (`MIC_FRAME_SAMPLES`).
const MIC_FRAME: usize = 320;

/// The window the detector scores: 256 samples = 16 ms at 16 kHz (`vad.rs`'s own `FRAME`).
const VAD_WINDOW: usize = 256;

/// The rate the client's frames and the detector are both at.
const SAMPLE_RATE: f32 = 16_000.0;

/// The room tone `auto_mode_transcribes_only_the_audio_the_detector_heard` sends before the voice.
/// Twenty frames is a multiple of four, so the detector's 256-sample windows and the client's
/// 320-sample frames line up exactly when the voice starts.
const ROOM_TONE_FRAMES: usize = 20;

/// The same room tone in the detector's windows: 25, exactly, because 6400 divides by 256.
const ROOM_TONE_WINDOWS: usize = ROOM_TONE_FRAMES * MIC_FRAME / VAD_WINDOW;

/// The voiced frames that test sends: just over a second.
const SPOKEN_FRAMES: usize = 52;

/// The `silence_ms` that test builds its session with.
const SILENCE_MS: u32 = 200;

/// `vad.rs`'s onset: three consecutive voiced windows confirm a new utterance.
const ONSET_WINDOWS: usize = 3;

/// Earshot's own documented split: "Scores over 0.5 can generally be considered voice".
const VOICE: f32 = 0.5;

/// How many frames the detector may spend confirming the onset before that test's own bound —
/// `voiced >= (spoken - MAX_ONSET_FRAMES) * frame` — stops holding.
const MAX_ONSET_FRAMES: usize = 6;

/// Full scale for every candidate, and the only amplitude a candidate ever has: `|s| > 0.3` is then
/// true of every kept voiced sample however the waveform happens to run, and false of the room tone.
const FULL_SCALE: i16 = 12_000;

/// One candidate utterance: the sample at absolute index `i` of the audio the microphone sends
/// after the room tone. A plain `fn` so candidates can be listed, and pure so the frames tile the
/// signal without a seam at 320 samples.
type Wave = fn(usize) -> i16;

/// A two-level signal: the clip that keeps every kept sample above the 0.3 the test asserts on.
fn hard_clip(value: f32) -> i16 {
    if value >= 0.0 { FULL_SCALE } else { -FULL_SCALE }
}

/// A resonance at `centre` with bandwidth `bw`, evaluated at `f`: the spectral envelope of a vowel,
/// cheap enough to evaluate once per harmonic.
fn resonance(f: f32, centre: f32, bw: f32) -> f32 {
    let d = (f - centre) / bw;
    1.0 / (1.0 + d * d)
}

/// A deterministic hash of a small integer, in `[0, 1)`: the pseudo-random frequencies and phases
/// of the synthetic signals, with no random-number dependency and no state carried between samples.
fn hash(k: u32) -> f32 {
    let x = k.wrapping_mul(2_654_435_761).wrapping_add(1_013_904_223);
    let x = x ^ (x >> 15);
    (x >> 8) as f32 / (1 << 24) as f32
}

/// A 120 Hz glottal buzz shaped by two resonances that move from `a` towards `e`: a synthetic vowel,
/// hard-limited to full scale.
///
/// The clip is what makes `|s| > 0.3` honest for every kept frame — a vowel waveform drops below a
/// third of full scale at every zero crossing — and it keeps what a detector keys on: the pitch, the
/// harmonic series and the formant envelope, which is what pulse-width-modulated speech synthesis
/// carries too. A little deterministic breath keeps every mel band occupied the way a voice's noise
/// component does: the line spectrum of a held tone has holes in it that speech has not.
fn clipped_vowel(i: usize) -> i16 {
    let t = i as f32 / SAMPLE_RATE;
    // 120 Hz with a few percent of vibrato: a voice, not an oscillator.
    let phase = TAU * 120.0 * t + 0.35 * (TAU * 3.7 * t).sin();
    // Two formants that move at syllable rate over the utterance: the detector scores three context
    // frames at a time, and a vowel that never changes is not what those three frames see in speech.
    let f1 = 700.0 - 250.0 * (TAU * 1.5 * t).sin();
    let f2 = 1_200.0 + 450.0 * (TAU * 1.1 * t).sin();
    let mut voiced = 0.0;
    for k in 1..=60u32 {
        let f = k as f32 * 120.0;
        if f >= 7_600.0 {
            break;
        }
        // A glottal tilt under the two resonances: a strong fundamental for the pitch a detector
        // looks for, and formants over it for the timbre.
        let glottal = 1.0 / (1.0 + (f / 400.0) * (f / 400.0));
        let shape = resonance(f, f1, 90.0) + 0.6 * resonance(f, f2, 130.0) + 0.5 * glottal;
        voiced += (shape / k as f32) * (k as f32 * phase).sin();
    }
    // Just enough breath to dither the zero crossings into the neighbouring bands; not enough to
    // move them, which would turn the vowel into the noise the detector is trained to reject.
    hard_clip(voiced + 0.15 * (hash(i as u32) - 0.5))
}

/// Speech-shaped noise: a sum of sinusoids at pseudo-random frequencies across the band a voice
/// occupies, with amplitudes tilted down with frequency and following the formants, cut into
/// syllables. A fricative has no pitch to find, so this is the candidate that asks whether the
/// detector needs one.
fn clipped_speech_noise(i: usize) -> i16 {
    let t = i as f32 / SAMPLE_RATE;
    let mut noise = 0.0;
    for k in 0..64u32 {
        let r = hash(k);
        let f = 150.0 + 3_600.0 * r * r;
        let phase = TAU * hash(k.wrapping_add(997));
        let shape = 600.0 / (f + 600.0) * (1.0 + resonance(f, 800.0, 600.0));
        noise += shape * (TAU * f * t + phase).sin();
    }
    // Four syllables a second: a held noise is a fan, and a fricative comes in bursts.
    hard_clip(noise * (0.55 + 0.45 * (TAU * 4.0 * t).sin()))
}

/// A chirp: one sweep from 100 Hz to 4 kHz and back, every two seconds. The whole speech band with
/// none of its structure — the control for "the detector wants the band occupied".
fn clipped_chirp(i: usize) -> i16 {
    // Where in the up-and-down sweep this sample falls.
    let s = i as f32 / SAMPLE_RATE % 2.0;
    // The integral of the frequency, so the phase stays continuous at the turn and at the wrap: both
    // ends of the sweep reach a whole number of cycles.
    let cycles = if s < 1.0 {
        100.0 * s + 1_950.0 * s * s
    } else {
        2_050.0 + 7_900.0 * (s - 1.0) - 1_950.0 * (s * s - 1.0)
    };
    hard_clip((TAU * cycles).sin())
}

/// The frame the writer's version used: a 200 Hz square wave, five periods of the 20 ms frame.
fn clipped_square(i: usize) -> i16 {
    if i % 80 < 40 { FULL_SCALE } else { -FULL_SCALE }
}

/// The vowel behind a plosive: 8 ms of broadband noise, then `clipped_vowel`. A syllable that starts
/// with `t` or `p` has that edge, so a detector that needs a spectral step to call an onset gets one
/// at the first window instead of the third.
fn clipped_burst_vowel(i: usize) -> i16 {
    if i < 128 { hard_clip(hash((i / 2) as u32) - 0.5) } else { clipped_vowel(i - 128) }
}

/// The candidate utterances, in the order the test tries them: the four the task names, then a
/// plosive onset.
const CANDIDATES: [(&str, Wave); 5] = [
    ("120 Hz vowel", clipped_vowel),
    ("speech-shaped noise", clipped_speech_noise),
    ("100-4000 Hz chirp", clipped_chirp),
    ("200 Hz square", clipped_square),
    ("plosive + vowel", clipped_burst_vowel),
];

/// The frame with the amplitude of a quiet room.
fn silent_frame() -> Vec<i16> {
    vec![0; MIC_FRAME]
}

/// Frame `frame` of a candidate waveform, in the 320 samples the client sends.
fn candidate_frame(wave: Wave, frame: usize) -> Vec<i16> {
    (0..MIC_FRAME).map(|i| wave(frame * MIC_FRAME + i)).collect()
}

/// The frames of one candidate.
fn candidate_frames(wave: Wave, count: usize) -> Vec<Vec<i16>> {
    (0..count).map(|frame| candidate_frame(wave, frame)).collect()
}

/// The silent microphone frames the session's hangover budget needs: `silence_ms` of 16 ms windows,
/// rounded up to the client's 20 ms frames. The frame that completes the budget is the one that
/// reports the end, and it is part of the utterance.
fn hangover_frames() -> usize {
    (VadState::new(SILENCE_MS).silence_frames() as usize * VAD_WINDOW).div_ceil(MIC_FRAME)
}

/// Every 256-sample window of `frames`, in order, scored by `scorer`.
///
/// The re-chunking is `VadState::push`'s own — whole 256-sample windows, with the remainder carried
/// to the next frame — so these are the scores the session's detector computes for the very same
/// frames.
fn window_scores(scorer: &mut Earshot, frames: &[Vec<i16>]) -> Vec<f32> {
    let mut carry: Vec<i16> = Vec::new();
    let mut scores = Vec::new();
    for frame in frames {
        carry.extend_from_slice(frame);
        let mut offset = 0;
        while carry.len() - offset >= VAD_WINDOW {
            scores.push(scorer.score(&carry[offset..offset + VAD_WINDOW]));
            offset += VAD_WINDOW;
        }
        carry.drain(..offset);
    }
    scores
}

/// Asks the detector itself which candidate it calls voice, on the stream the test really sends.
///
/// A pure tone is not speech and this detector is a trained network, so nothing here may assume a
/// waveform the test likes is scored above [`VOICE`]: the candidates are built, the detector scores
/// them, and the first one it confirms is the utterance the test uses. Each candidate is asked
/// twice — once window by window with [`Earshot::score`], which is what the report and the panic
/// below print, and once through the session's own [`VadState`], which is what the test's
/// assertions need: an onset confirmed within [`MAX_ONSET_FRAMES`] frames of the voice, and the end
/// that arrives after the silence budget.
///
/// It panics, listing every candidate's scores, when none of them is voice. That outcome is a fact
/// about this detector and synthetic audio, not something this test may paper over.
fn voiced_wave() -> Wave {
    let room_tone = vec![vec![0i16; MIC_FRAME]; ROOM_TONE_FRAMES];
    let silence = vec![vec![0i16; MIC_FRAME]; hangover_frames() + 5];
    let mut report = Vec::new();

    for (name, wave) in CANDIDATES {
        let mut stream = room_tone.clone();
        stream.extend(candidate_frames(wave, SPOKEN_FRAMES));
        stream.extend(silence.iter().cloned());

        // What the detector thought, window by window. The first `ROOM_TONE_WINDOWS` scores are the
        // room tone; the voice starts at the next one.
        let mut scorer = Earshot::default();
        let scores = window_scores(&mut scorer, &stream);
        let voice_scores = &scores[ROOM_TONE_WINDOWS..];
        let loudest = voice_scores.iter().cloned().fold(f32::MIN, f32::max);
        let first_run = voice_scores
            .windows(ONSET_WINDOWS)
            .position(|window| window.iter().all(|score| *score >= VOICE));

        // What the session's state machine — the same one the session runs, over the same detector —
        // does with those very frames.
        let mut vad = VadState::new(SILENCE_MS);
        let mut onset = None;
        let mut ended = false;
        for (frame, samples) in stream.iter().enumerate() {
            match vad.push(samples) {
                VadEvent::SpeechStarted if onset.is_none() => onset = Some(frame),
                VadEvent::SpeechEnded => ended = true,
                _ => {}
            }
        }
        // Relative to the voice: `None` when no onset was confirmed at all, and an onset inside the
        // room tone cannot be one.
        let onset = onset.and_then(|frame| frame.checked_sub(ROOM_TONE_FRAMES));

        report.push(format!(
            "{name}: loudest {loudest:.3}, first {}-window run at voice window {}, onset frame \
             {onset:?}, ended {ended}, first voice scores {}",
            ONSET_WINDOWS,
            first_run.map_or("never".to_string(), |window| window.to_string()),
            voice_scores
                .iter()
                .take(8)
                .map(|score| format!("{score:.2}"))
                .collect::<Vec<_>>()
                .join(" "),
        ));

        if onset.is_some_and(|frame| frame <= MAX_ONSET_FRAMES) && ended {
            return wave;
        }
    }

    panic!(
        "no candidate frame is voice to this detector, so `auto` mode cannot end an utterance built \
         from any of them and this test would fail on the detector's verdict rather than on the \
         session. Every score below is the detector's own, one per 256-sample window of the stream \
         this test sends; the room tone is the first {} windows and the voice starts at the next \
         one. {}",
        ROOM_TONE_WINDOWS,
        report.join(" | ")
    );
}

/// Waits out the 300 ms guard the assistant's speech leaves behind, so a `ListenStart` that follows
/// is not swallowed by it.
async fn past_the_guard() {
    tokio::time::sleep(Duration::from_millis(400)).await;
}

fn is_heard(m: &ToClient) -> bool {
    matches!(m, ToClient::Event(Event::Heard { .. }))
}

/// Review Focus 2, first half: silence before the first syllable never opens a turn. Ten silent
/// microphone frames, then `Tab` — in `auto` mode the detector is the only thing that may start a
/// turn, so nothing is transcribed and no provider is called.
#[tokio::test]
async fn auto_mode_never_transcribes_silence_before_speech() {
    let chat = FakeChat::new(vec![]);
    let (tx, mut rx, transcribe) = auto_session(chat.clone(), FakeSpeech::new(), "no debería oírse", 100);
    next_matching(&mut rx, is_done, 500).await; // the greeting
    past_the_guard().await;

    send(&tx, Command::ListenStart);
    for _ in 0..10 {
        send_mic(&tx, silent_frame());
    }
    send(&tx, Command::ListenStop);

    let seen = drain(&mut rx, 300).await;
    assert!(!seen.iter().any(is_heard), "silence must not reach transcription: {seen:#?}");
    assert!(
        transcribe.pcms.lock().unwrap().is_empty(),
        "no audio the detector did not hear may be transcribed"
    );
    assert_eq!(chat.calls(), 0, "and nothing may ask the chat provider");
}

/// Review Focus 2, second half, end to end: a voiced run followed by enough silence ends the
/// utterance by itself, and only the audio the detector confirmed reaches the transcriber. `Tab`
/// afterwards must not transcribe the same audio twice.
#[tokio::test]
async fn auto_mode_transcribes_only_the_audio_the_detector_heard() {
    let chat = FakeChat::new(vec![Ok(vec![S1])]);
    let (tx, mut rx, transcribe) = auto_session(chat, FakeSpeech::new(), "¿y el ticket?", SILENCE_MS);
    next_matching(&mut rx, is_done, 500).await; // the greeting
    past_the_guard().await;

    send(&tx, Command::ListenStart);

    // The utterance's waveform is the detector's choice, not the test's: a pure tone is not speech
    // and `earshot` is a learned network, so a candidate it never calls voice could not end a turn
    // at all. `voiced_wave` asks it, and panics with every candidate's scores if none of them is
    // voice — that outcome is a finding about the detector, not something to paper over.
    let wave = voiced_wave();

    // Room tone before the first syllable: a fifth of a second of it, and none of it is kept.
    // Twenty frames is a multiple of four, so the detector's 256-sample windows and the client's
    // 320-sample frames line up exactly when the voice starts.
    for _ in 0..ROOM_TONE_FRAMES {
        send_mic(&tx, silent_frame());
    }
    // Just over a second of voice: far past the three windows of onset confirmation.
    let spoken = SPOKEN_FRAMES;
    for frame in 0..spoken {
        send_mic(&tx, candidate_frame(wave, frame));
    }
    // The hangover, in frames, exactly what `VadState` says it needs — no magic number here.
    let hangover = hangover_frames();
    for _ in 0..hangover {
        send_mic(&tx, silent_frame());
    }
    // Trailing silence after the utterance ended: it belongs to no turn and must be dropped.
    for _ in 0..5 {
        send_mic(&tx, silent_frame());
    }

    let heard = next_matching(&mut rx, is_heard, 1000).await;
    assert_eq!(heard, ToClient::Event(Event::Heard { text: "¿y el ticket?".into() }));

    let pcms = transcribe.pcms.lock().unwrap();
    assert_eq!(pcms.len(), 1, "one utterance, one transcription: {pcms:#?}");
    assert_eq!(pcms[0].sample_rate, 16000);
    let samples = &pcms[0].samples;
    let frame = silent_frame().len();

    // Everything the utterance keeps is whole frames of what the microphone sent.
    assert_eq!(samples.len() % frame, 0, "the utterance is whole mic frames: {pcms:#?}");
    // Review Focus 2, first half: the twenty leading silent frames are not in it. Every sample
    // before the tail is voice, so not one sample of the room tone before the first syllable
    // survived.
    let tail = hangover * frame;
    let voiced = samples.len().saturating_sub(tail);
    // Review Focus 2, first half: the room tone before the first syllable is not in the utterance.
    // The detector confirms an onset three windows in and the session keeps the utterance from the
    // frame that confirmation arrived on, so the utterance begins with voice and never with the
    // room tone the user spoke through. It is asserted this way, and not as "every sample is loud",
    // because a real utterance contains unvoiced frames — plosives, pauses — that belong to it and
    // must be kept.
    assert!(voiced >= frame, "the utterance holds at least the confirming frame: {pcms:#?}");
    let first = samples[..frame].iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
    assert!(first > 0.3, "the utterance starts with voice, not with room tone: {first}");
    // And the tail is exactly the silence budget the detector was configured with: not one frame
    // less, because the frame that reports the end is part of the utterance, and not one more,
    // because the room tone after it belongs to no turn.
    assert!(
        samples[voiced..].iter().all(|s| s.abs() < 0.01),
        "the hangover that ended the utterance is the only silence in it: {pcms:#?}"
    );
    // The onset is confirmed a few frames into the run, and those frames are not kept either: the
    // utterance is shorter than the voice that was sent, and not much shorter.
    // `MAX_ONSET_FRAMES` is the outer edge of what a learned detector may need to see the onset,
    // which is also why `voiced_wave` refuses a candidate that crosses it.
    assert!(
        voiced <= spoken * frame,
        "the frames before the onset was confirmed must not be transcribed: \
         utterance {} frames, pre-tail {} frames, voiced run sent {} frames, hangover {} frames",
        samples.len() / frame,
        voiced / frame,
        spoken,
        hangover
    );
    assert!(
        voiced >= (spoken - MAX_ONSET_FRAMES) * frame,
        "the whole voiced run except the confirmation is the utterance: \
         utterance {} frames, pre-tail {} frames, voiced run sent {} frames, hangover {} frames",
        samples.len() / frame,
        voiced / frame,
        spoken,
        hangover
    );
    drop(pcms);

    // `Tab` still stops the microphone by hand, and there is nothing left for it to add.
    send(&tx, Command::ListenStop);
    let after = drain(&mut rx, 300).await;
    assert!(!after.iter().any(is_heard), "the same audio must not be transcribed twice: {after:#?}");
    assert_eq!(transcribe.pcms.lock().unwrap().len(), 1);
}

/// The assistant's own voice never re-opens the microphone. A typed question is answered while the
/// stream is open; the detector is paused for the whole turn and for the 300 ms guard after it, so
/// what the microphone picks up during the reply is never kept. The quiet after the guard is what
/// proves it: a microphone armed again over an utterance the reply opened would report the end of
/// speech there and hand the transcriber the assistant's own voice.
#[tokio::test]
async fn the_assistants_own_voice_never_reopens_the_microphone() {
    let chat = FakeChat::new(vec![Ok(vec![S1])]);
    let (tx, mut rx, transcribe) = auto_session(chat, FakeSpeech::new(), "no debería oírse", 100);
    next_matching(&mut rx, is_done, 500).await; // the greeting
    past_the_guard().await;

    // The reply is being spoken; the microphone picks up the candidate the detector called voice —
    // the speaker's own audio, as the microphone would hear it — and it must be ignored. The
    // waveforms this used to send were ones the detector never called voice, so the assertion below
    // could not have failed whatever the session did.
    let wave = voiced_wave();

    send(&tx, Command::ListenStart);
    send(&tx, Command::Ask { text: "¿qué tal, compadre?".into() });
    for frame in 0..40 {
        send_mic(&tx, candidate_frame(wave, frame));
    }
    next_matching(&mut rx, is_done, 2000).await; // the reply ends
    let during = drain(&mut rx, 100).await;
    assert!(
        !during.iter().any(is_heard),
        "the assistant's own voice must not become an utterance: {during:#?}"
    );

    // The guard has elapsed and the detector is listening again. A fifth of a second of quiet —
    // past the 100 ms budget this session was built with — ends an utterance if one is open, and
    // `Tab` closes the microphone for good either way.
    past_the_guard().await;
    for _ in 0..8 {
        send_mic(&tx, silent_frame());
    }
    send(&tx, Command::ListenStop);
    let after = drain(&mut rx, 300).await;
    assert!(
        !after.iter().any(is_heard),
        "what the microphone picked up while the reply played must not be transcribed: {after:#?}"
    );
    assert!(transcribe.pcms.lock().unwrap().is_empty());
}

/// A turn that starts inside the previous turn's 300 ms guard must keep the detector paused for the
/// whole of itself. `run` reads the guard's deadline once per loop iteration, and `start_turn` used
/// to leave it alone: the stale deadline then fired in the middle of the new turn, resumed the
/// detector over the assistant's own voice, and the frames below were confirmed as an onset, waited
/// out the silence budget and were transcribed as the user's — the one thing Review Focus 2 forbids,
/// and every such turn could seed the next.
#[tokio::test]
async fn a_turn_started_inside_the_guard_keeps_the_detector_paused() {
    // A reply that lasts far longer than the guard the greeting leaves behind — five chunks at
    // 200 ms — so the stale deadline is certain to fall inside it. The second scripted outcome is
    // for the turn a leaked onset would start, so the failure is the assertion below and not the
    // chat fake running out of script.
    let chat = FakeChat::slow(
        vec![Ok(vec![S1, " ", S2, " ", S3]), Ok(vec![S2])],
        Duration::from_millis(200),
    );
    let (tx, mut rx, transcribe) =
        auto_session(chat, FakeSpeech::new(), "no debería oírse", SILENCE_MS);
    next_matching(&mut rx, is_done, 500).await; // the greeting ends: its 300 ms guard is armed

    // The detector's choice, asked before the clock below matters: `voiced_wave` runs the real
    // detector over the candidates and is quick, but it is synchronous, so it goes first.
    let wave = voiced_wave();

    // Inside that guard: late enough that the session has settled the greeting's turn and armed the
    // very deadline this test is about, early enough to be inside the 300 ms it lasts.
    tokio::time::sleep(Duration::from_millis(100)).await;

    send(&tx, Command::ListenStart);
    send(&tx, Command::Ask { text: "¿qué tal, compadre?".into() });

    // Past the greeting's deadline, with the reply still being spoken: what the microphone picks up
    // here is the assistant's own output, as it would be on a machine with open speakers. A whole
    // utterance's worth of it, plus the silence budget that would end it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    for frame in 0..SPOKEN_FRAMES {
        send_mic(&tx, candidate_frame(wave, frame));
    }
    for _ in 0..hangover_frames() + 2 {
        send_mic(&tx, silent_frame());
    }

    let seen = drain(&mut rx, 2500).await;
    assert!(
        !seen.iter().any(is_heard),
        "the reply's own voice must not open an utterance while the reply is speaking: {seen:#?}"
    );
    assert!(transcribe.pcms.lock().unwrap().is_empty(), "and nothing may be transcribed");
    assert!(seen.iter().any(is_done), "the reply itself must have finished: {seen:#?}");
}

/// `Reset` must not leave the detector paused for the rest of the session. `resume` is reachable only
/// from `guard_elapsed`, so a reset that dropped the deadline without resuming made `auto` mode deaf:
/// `Tab` opened the microphone, frames arrived, `push` reported nothing and nothing was ever
/// transcribed until some later turn ended. `F4` just after a reply — inside the guard — is the common
/// way to clear a conversation, which is exactly when it bit. The shipped `reset` resumes the detector
/// itself, so the microphone is live again as soon as the guard would have allowed it.
///
/// The tail is the session's own budget plus a margin, because a learned detector keeps scoring the
/// first windows of a silence as voice: the budget alone leaves `silence_run` short of what
/// `SpeechEnded` needs and the utterance never closes. The sibling auto test sends the same margin.
#[tokio::test]
async fn reset_inside_the_guard_leaves_auto_mode_listening() {
    let chat = FakeChat::new(vec![Ok(vec![S1])]);
    let (tx, mut rx, transcribe) =
        auto_session(chat, FakeSpeech::new(), "¿y el ticket?", SILENCE_MS);
    // `F4` as the session starts: the greeting's guard is up, the detector is paused, and this is the
    // moment that used to leave `auto` mode deaf for good. Nothing is awaited before or after it to
    // decide the timing, so the test cannot lose a race with the session's scheduling.
    send(&tx, Command::Reset);
    tokio::time::sleep(Duration::from_millis(700)).await;

    // `Tab`: the microphone opens, and the detector must be listening again.
    send(&tx, Command::ListenStart);

    let wave = voiced_wave();
    for _ in 0..ROOM_TONE_FRAMES {
        send_mic(&tx, silent_frame());
    }
    for frame in 0..SPOKEN_FRAMES {
        send_mic(&tx, candidate_frame(wave, frame));
    }
    for _ in 0..hangover_frames() + 5 {
        send_mic(&tx, silent_frame());
    }

    let heard = next_matching(&mut rx, is_heard, 2000).await;
    assert_eq!(heard, ToClient::Event(Event::Heard { text: "¿y el ticket?".into() }));
    assert_eq!(transcribe.pcms.lock().unwrap().len(), 1, "the utterance must still be transcribed");
}
