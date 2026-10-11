//! Voice activity detection over the microphone frames the client streams.
//!
//! `earshot` answers one question per 256-sample window — "is this voice?" — and nothing else. The
//! onset confirmation, the hangover that turns `silence_ms` into `SpeechEnded`, the pause around the
//! assistant's own speech and the re-chunking of the client's 320-sample frames into the crate's
//! 256-sample windows all live here, so the crate's API appears in exactly one function:
//! [`Scorer::score`] on [`Earshot`].

/// Samples earshot scores per call: 256 = 16 ms at 16 kHz. Not a public constant of the crate in
/// 1.2.2, so it is ours; `predict_i16` rejects any other length.
const FRAME: usize = 256;

/// How many milliseconds one scored window covers at 16 kHz.
const FRAME_MS: u32 = 16;

/// Consecutive voiced frames before we call it speech: 3 x 16 ms = 48 ms. One frame is a chair
/// creak, a keyboard, a door; three in a row is a person starting a sentence.
const ONSET_FRAMES: u32 = 3;

/// What one pushed frame decided. `None` is every frame that changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent {
    /// A confirmed onset: the first frame of an utterance that is now open.
    SpeechStarted,
    /// The silence budget elapsed since the last voiced frame: the utterance is over.
    SpeechEnded,
    /// Nothing changed.
    None,
}

/// Scores one [`FRAME`]-sample window: `[0, 1]`, voice from 0.5 up.
///
/// A trait rather than a function pointer because the state machine's tests drive it with a known
/// script, and a plain `fn` cannot carry that script — nor hold earshot's lazily built network.
/// There are exactly two implementations: [`Earshot`], the real detector, and the test module's
/// scripted one.
pub trait Scorer {
    /// The score of one window of exactly [`FRAME`] samples.
    fn score(&mut self, frame: &[i16]) -> f32;
}

/// The real scorer: earshot's embedded network.
///
/// The detector is about 8 KiB and is built on the first window it is asked for, so constructing a
/// `VadState` — which every session does, in every mode — costs a `None` and no stack.
#[derive(Default)]
pub struct Earshot {
    detector: Option<Box<earshot::Detector>>,
}

impl Scorer for Earshot {
    /// earshot 1.2.2 exposes `predict_i16` and `predict_f32` and no generic `predict` (Task 8's
    /// Step 1 settles this against the fetched crate; the research note's `Frame`/`Score` pair
    /// exists only on `main`). This is the one place the crate is named, so a future version is a
    /// change here and nowhere else.
    fn score(&mut self, frame: &[i16]) -> f32 {
        // `default_boxed` keeps the detector's ~8 KiB off the stack; `alloc` is a default feature
        // of 1.2.2. `get_or_insert_with` is the lazy-init shape, so no `clippy::box_default`.
        let detector = self.detector.get_or_insert_with(earshot::Detector::default_boxed);
        detector.predict_i16(frame)
    }
}

/// Per-session detector state. Owned by the session task; not shared and not `Sync`.
///
/// The default scorer is the real detector, so a caller only ever writes `VadState::new(ms)`.
pub struct VadState<S = Earshot> {
    scorer: S,
    /// Consecutive voiced frames seen in the current run.
    speech_run: u32,
    /// Consecutive unvoiced frames seen since the last voiced one.
    silence_run: u32,
    /// The hangover, in frames: `silence_ms.div_ceil(16)`, at least as long as asked for.
    hangover_frames: u32,
    /// True while an utterance is open.
    in_speech: bool,
    /// True while the assistant speaks and while its 300 ms tail runs (the session owns the timer).
    paused: bool,
    /// Samples that do not yet fill a [`FRAME`] window. The client sends 320 at a time, so this is
    /// 64 after the first frame and refills unevenly; dropping it would lose 64 samples of speech
    /// per frame.
    carry: Vec<i16>,
}

impl VadState {
    /// A detector for a session configured with `silence_ms` of silence to end an utterance.
    pub fn new(silence_ms: u32) -> Self {
        Self::with_scorer(Earshot::default(), silence_ms)
    }
}

impl<S: Scorer> VadState<S> {
    /// The same detector over any scorer: the real one, or a script a test hands in. Private
    /// because `VadState::new` is the whole public surface; the tests reach it in-crate.
    fn with_scorer(scorer: S, silence_ms: u32) -> Self {
        Self {
            scorer,
            speech_run: 0,
            silence_run: 0,
            // The ceiling is what makes the budget *at least* `silence_ms`: 1000 ms becomes 63
            // frames = 1008 ms, and any budget shorter than one frame becomes one frame.
            hangover_frames: silence_ms.div_ceil(FRAME_MS),
            in_speech: false,
            paused: false,
            carry: Vec::with_capacity(FRAME),
        }
    }

    /// Stops deciding: the assistant is speaking, or its 300 ms tail has not elapsed. Frames are
    /// consumed and nothing is reported, and a half-heard utterance is abandoned rather than ended —
    /// a `SpeechEnded` here would close a turn on the assistant's own voice.
    pub fn pause(&mut self) {
        self.paused = true;
        self.in_speech = false;
        self.speech_run = 0;
        self.silence_run = 0;
    }

    /// Re-arms the detector and starts the onset search from nothing: the counters go back to zero
    /// and so does the carry, so a partial window left over from *before* the pause cannot be
    /// completed by the first samples of the next utterance. Those samples are the assistant's own
    /// tail, and a window that mixed the two would score neither.
    pub fn resume(&mut self) {
        self.paused = false;
        self.in_speech = false;
        self.speech_run = 0;
        self.silence_run = 0;
        // `clear`, not a fresh `Vec`: the capacity for one window is worth keeping.
        self.carry.clear();
    }

    /// True while an utterance is open: the samples pushed since `SpeechStarted` are its audio.
    pub fn in_speech(&self) -> bool {
        self.in_speech
    }

    /// The silence budget in frames: how many silent frames after the last voiced one report
    /// `SpeechEnded`. The session's tests count with this instead of repeating `div_ceil(16)`.
    ///
    /// A hand-edited `silence_ms = 0` is reported as the zero frames the file asked for, and still
    /// ends the utterance on the first silent frame: the frame that ends speech ends it.
    pub fn silence_frames(&self) -> u32 {
        self.hangover_frames
    }

    /// Scores as many whole [`FRAME`]-sample windows as `frame` completes and reports what they
    /// decided. Any frame length is accepted: the client sends 320 samples (`MIC_FRAME_SAMPLES`),
    /// earshot wants 256, and the remainder waits here for the next frame.
    ///
    /// A pause swallows frames — it returns before the crate is called, so the assistant's own
    /// voice cannot move the counters that decide when the user stopped talking — and does not
    /// buffer them: they are not part of any utterance.
    pub fn push(&mut self, frame: &[i16]) -> VadEvent {
        if self.paused {
            return VadEvent::None;
        }
        self.carry.extend_from_slice(frame);
        let mut event = VadEvent::None;
        let mut done = false;
        let mut offset = 0;
        while self.carry.len() - offset >= FRAME && !done {
            let score = self.scorer.score(&self.carry[offset..offset + FRAME]);
            offset += FRAME;
            match self.decide(score) {
                // `SpeechEnded` wins the frame: it is the one event that closes a turn, and a
                // window cannot both start and end one.
                VadEvent::None => {}
                decided => {
                    event = decided;
                    done = decided == VadEvent::SpeechEnded;
                }
            }
        }
        self.carry.drain(..offset);
        event
    }

    /// The state machine for one window's score: confirmation, then the hangover.
    fn decide(&mut self, score: f32) -> VadEvent {
        // 0.5 is the crate's own documented split ("Scores over 0.5 can generally be considered
        // voice"). The scripted tests pass 0.0 and 0.9, so only a score exactly on the boundary
        // could tell the two conventions apart.
        let voiced = score >= 0.5;
        if self.in_speech {
            if voiced {
                self.silence_run = 0;
                return VadEvent::None;
            }
            self.silence_run += 1;
            // `max(1)` is what makes a hand-edited `silence_ms = 0` a real budget. The detector
            // still reports the zero frames the file asked for (`silence_frames`), and the first
            // frame of silence after voice is the one that ends the utterance — "zero means the
            // frame that ends speech ends it", which is what
            // `the_silence_budget_is_reported_in_frames` claims. Without it `silence_run >= 0` is
            // unreachable for a zero budget and the utterance would never end by itself.
            if self.silence_run >= self.hangover_frames.max(1) {
                self.in_speech = false;
                self.silence_run = 0;
                self.speech_run = 0;
                return VadEvent::SpeechEnded;
            }
            return VadEvent::None;
        }
        if !voiced {
            // Decrement rather than zero: one dropped frame in an otherwise voiced run must not
            // restart the confirmation.
            self.speech_run = self.speech_run.saturating_sub(1);
            self.silence_run += 1;
            return VadEvent::None;
        }
        self.speech_run += 1;
        self.silence_run = 0;
        if self.speech_run >= ONSET_FRAMES {
            self.in_speech = true;
            self.speech_run = 0;
            return VadEvent::SpeechStarted;
        }
        VadEvent::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// The synthetic frame every state-machine test pushes: a square wave with the amplitude of
    /// speech, so a real detector would call it voiced. Nothing here reads audio from a device.
    fn loud() -> Vec<i16> {
        (0..FRAME).map(|i| if i % 2 == 0 { 12_000 } else { -12_000 }).collect()
    }

    /// The frame with the amplitude of a quiet room.
    fn silent() -> Vec<i16> {
        vec![0; FRAME]
    }

    /// What the scorer was asked to score, so the re-chunking is observable, plus the script of
    /// scores it hands out in order. Shared with the test through an `Rc<RefCell<..>>`.
    #[derive(Debug, Default)]
    struct Calls {
        scores: Vec<f32>,
        next: usize,
        sizes: Vec<usize>,
    }

    /// A scorer that plays `Calls::scores` in order, records every window length it was asked
    /// about, and answers 0.0 once the script runs out (a quiet room).
    struct Scripted(Rc<RefCell<Calls>>);

    impl Scorer for Scripted {
        fn score(&mut self, frame: &[i16]) -> f32 {
            let mut calls = self.0.borrow_mut();
            calls.sizes.push(frame.len());
            let score = calls.scores.get(calls.next).copied().unwrap_or(0.0);
            calls.next += 1;
            score
        }
    }

    fn state(calls: &Rc<RefCell<Calls>>, scores: &[f32], silence_ms: u32) -> VadState<Scripted> {
        calls.borrow_mut().scores = scores.to_vec();
        VadState::with_scorer(Scripted(Rc::clone(calls)), silence_ms)
    }

    /// `hangover_frames` is the silence budget in frames and `silence_frames()` reports it, so the
    /// session's tests can count what must reach the transcriber without duplicating the rounding.
    #[test]
    fn the_silence_budget_is_reported_in_frames() {
        assert_eq!(VadState::new(1000).silence_frames(), 63, "1000 ms of 16 ms frames");
        assert_eq!(VadState::new(32).silence_frames(), 2);
        assert_eq!(VadState::new(0).silence_frames(), 0, "zero means the frame that ends speech ends it");
    }

    /// Review Focus 2's first gate: a run of silent frames opens nothing and ends nothing, however
    /// long it lasts. Every full window is still scored exactly once, which is what makes the
    /// re-chunking observable.
    #[test]
    fn silence_alone_never_speaks() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut vad = state(&calls, &[0.0, 0.1, 0.4], 32);
        for _ in 0..10 {
            assert_eq!(vad.push(&silent()), VadEvent::None);
        }
        assert!(!vad.in_speech(), "silence must not open an utterance");
        assert_eq!(calls.borrow().next, 10, "ten full windows, ten scores");
        assert!(calls.borrow().sizes.iter().all(|n| *n == FRAME));
    }

    /// Onset confirmation: one loud frame is a chair creak. `SpeechStarted` arrives once, on the
    /// third consecutive voiced frame, and the frames before it are not part of any utterance.
    #[test]
    fn speech_starts_once_on_the_third_voiced_frame() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut vad = state(&calls, &[0.9, 0.9, 0.9, 0.9], 32);
        assert_eq!(vad.push(&loud()), VadEvent::None, "one frame is not speech yet");
        assert!(!vad.in_speech());
        assert_eq!(vad.push(&loud()), VadEvent::None, "two frames are not speech yet");
        assert_eq!(vad.push(&loud()), VadEvent::SpeechStarted, "the third confirms it");
        assert!(vad.in_speech());
        // The run is confirmed: further voiced frames are not another onset.
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(calls.borrow().sizes.len(), 4, "one score per full window");
    }

    /// The lone blip: an unconfirmed loud frame followed by silence never becomes an utterance and
    /// never ends one either — there was nothing to end.
    #[test]
    fn a_lone_loud_frame_never_becomes_an_utterance() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut vad = state(&calls, &[0.9, 0.0, 0.0], 32);
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&silent()), VadEvent::None);
        assert_eq!(vad.push(&silent()), VadEvent::None);
        assert!(!vad.in_speech());
    }

    /// The hangover: `silence_ms` of 16 ms frames (32 ms -> 2 frames) after the last voiced frame
    /// turns into exactly one `SpeechEnded`, and the frame that ends it was scored.
    #[test]
    fn speech_ends_once_after_the_silence_budget() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut vad = state(&calls, &[0.9, 0.9, 0.9, 0.9, 0.0, 0.0, 1.0], 32);
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::SpeechStarted);
        assert_eq!(vad.push(&loud()), VadEvent::None, "still inside the utterance");
        assert_eq!(vad.push(&silent()), VadEvent::None, "1 of 2 silent frames");
        assert_eq!(vad.push(&silent()), VadEvent::SpeechEnded, "2 of 2 ends it");
        assert!(!vad.in_speech());
        assert_eq!(vad.push(&loud()), VadEvent::None, "a new utterance starts over from nothing");
        assert_eq!(vad.silence_frames(), 2);
    }

    /// The pause the spec asks for: frames are consumed, nothing is reported, and a half-heard
    /// utterance is abandoned rather than ended — the assistant's own voice must not produce a
    /// `SpeechEnded` that would close a turn. A resume starts the onset search again, so it takes
    /// three voiced frames to speak once more.
    #[test]
    fn a_pause_swallows_frames_and_a_resume_starts_clean() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut vad = state(&calls, &[0.9, 0.9, 0.9, 0.9, 0.9, 0.9], 32);
        assert_eq!(vad.push(&loud()), VadEvent::None);
        vad.pause();
        for _ in 0..5 {
            assert_eq!(vad.push(&loud()), VadEvent::None, "a paused detector never speaks");
            assert_eq!(vad.push(&silent()), VadEvent::None, "and never ends an utterance");
        }
        assert!(!vad.in_speech(), "the half-heard utterance was thrown away, not ended");
        let scored_before = calls.borrow().next;
        assert_eq!(scored_before, 1, "only the unpaused frame was scored: {calls:#?}");
        vad.resume();
        assert_eq!(vad.push(&loud()), VadEvent::None, "the onset search restarts from nothing");
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::SpeechStarted, "three frames confirm it again");
    }

    /// Frames are not always 256 samples — the client's are 320 (`audio/device.rs:19`) — so the
    /// detector does its own re-chunking: earshot is handed exactly 256 samples at a time, and a
    /// frame that does not complete a window is carried, not dropped and not scored.
    #[test]
    fn a_frame_that_is_not_a_whole_window_is_carried() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut vad = state(&calls, &[0.0, 0.0, 0.0, 0.0], 32);
        assert_eq!(vad.push(&[1000; 255]), VadEvent::None);
        assert_eq!(calls.borrow().sizes.len(), 0, "255 samples are not a window");
        assert_eq!(vad.push(&[1000; 1]), VadEvent::None, "the 256th completes it");
        assert_eq!(calls.borrow().sizes.as_slice(), &[FRAME]);
        assert_eq!(vad.push(&[1000; 320]), VadEvent::None);
        assert_eq!(
            calls.borrow().sizes.as_slice(),
            &[FRAME, FRAME],
            "320 samples are one window plus a carry"
        );
        assert!(
            calls.borrow().sizes.iter().all(|n| *n == FRAME),
            "earshot only ever sees {FRAME} samples"
        );
    }

    /// The crate itself, on a window of the wrong length: this is what would panic inside
    /// `earshot::Detector::predict_i16` (a `debug_assert_eq!` on 256) if the re-chunking above
    /// were wrong. The whole state machine is by definition voice-then-silence, so the only thing
    /// asserted is that a run of it never panics and never ends more than once per silence budget.
    #[test]
    fn the_real_detector_survives_a_run_of_frames() {
        let mut vad = VadState::new(32);
        for _ in 0..4 {
            vad.push(&loud());
        }
        let mut endings = 0;
        for _ in 0..vad.silence_frames() + 2 {
            if vad.push(&silent()) == VadEvent::SpeechEnded {
                endings += 1;
            }
        }
        assert!(endings <= 1, "one budget of silence ends one utterance, got {endings}");
    }

    /// A hand-edited `silence_ms = 0` is a real budget: the detector reports the zero frames the
    /// file asked for and the first silent frame after voice is the one that ends the utterance —
    /// what `the_silence_budget_is_reported_in_frames` says zero means. The `> 0` guard that used to
    /// sit in front of the comparison made a zero budget unreachable, so the utterance never ended
    /// by itself, and a supported input (the file is hand-edited) left `auto` mode with no way to
    /// close a turn but `Tab`.
    #[test]
    fn a_zero_silence_budget_ends_the_utterance_on_the_first_silent_frame() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut vad = state(&calls, &[0.9, 0.9, 0.9, 0.0, 0.9, 0.9, 0.9], 0);
        assert_eq!(vad.silence_frames(), 0, "a zero budget is reported as zero frames");
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::SpeechStarted);
        assert_eq!(vad.push(&silent()), VadEvent::SpeechEnded, "the frame that ends speech ends it");
        assert!(!vad.in_speech());
        // And the detector starts over: the next utterance still needs three voiced frames.
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::SpeechStarted);
    }

    /// `resume` starts from nothing, the carry included. The 255 samples waiting for a window when
    /// the pause began are the assistant's own tail; completing that window with the first sample of
    /// the next utterance would score half of each, so the resume drops them instead.
    #[test]
    fn a_resume_drops_the_samples_carried_from_before_the_pause() {
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut vad = state(&calls, &[0.9, 0.9, 0.9, 0.9], 32);
        // 255 samples: one short of a window, so nothing has been scored.
        assert_eq!(vad.push(&[12_000; 255]), VadEvent::None);
        assert_eq!(calls.borrow().sizes.len(), 0, "255 samples are not a window");
        vad.pause();
        for _ in 0..5 {
            assert_eq!(vad.push(&loud()), VadEvent::None, "a paused detector swallows frames");
        }
        vad.resume();
        // One more sample is not a window either: the 255 from before the pause are gone.
        assert_eq!(vad.push(&[12_000; 1]), VadEvent::None);
        assert_eq!(
            calls.borrow().sizes.len(),
            0,
            "the carry from before the pause must not complete a window: {calls:#?}"
        );
        // Three whole windows of the new audio are the first thing scored after the resume.
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::None);
        assert_eq!(vad.push(&loud()), VadEvent::SpeechStarted);
        assert!(vad.in_speech());
        assert_eq!(calls.borrow().sizes.len(), 3, "one score per whole window after the resume");
        assert!(calls.borrow().sizes.iter().all(|n| *n == FRAME));
    }
}
