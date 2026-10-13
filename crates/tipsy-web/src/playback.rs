//! Browser playback of the server's speech, as a timeline the tests can read.
//!
//! The wire is one binary frame per spoken **sentence**, announced by the `SpeechChunk` that precedes
//! it (`crates/tipsy-protocol/src/lib.rs:151`, `:178`; `brain.rs:454-455` sends the pair). `seq` counts
//! those sentences inside one turn and restarts at 0 for the next one (`brain.rs:392-393`, `:198`), so
//! this module owns a timeline rather than a queue: every chunk is placed after the previous one, which
//! is what rodio's `append` does on the native side (`audio/device.rs:53-54`).
//!
//! **A pure core and a thin presenter.** This file is the core: the anchor, the cursor arithmetic, the
//! gap decision, the clamp that keeps every chunk out of the past, the envelope and the barge-in reset,
//! driven by
//! `(seq, samples, now)` and nothing else. The `AudioBufferSourceNode`s are `web_audio.rs`'s and only
//! present the PCM this file schedules. That split is the shape the prior-art review names, quoting the
//! working browser audio it read (`codewhale-audio-prior-art.md` §3, from
//! `pet/src/core/pet-audio.ts:10-12`):
//!
//! > Core emits score events; WebAudio / AVAudioEngine only present their PCM. Calling at any display
//! > cadence produces the same score when all world ticks are supplied. Calling twice for a world tick
//! > cannot retrigger a voice.
//!
//! Both of those sentences are requirements here, and both have a test: a chunk sequence scores the
//! same however often the presenter asks (`the_same_chunks_score_the_same_at_any_cadence`) and a frame
//! the socket delivers twice is scheduled once (`a_frame_delivered_twice_cannot_schedule_twice`).
//!
//! **The four scheduling details the prior-art review pins, and what this wire does with each**
//! (`codewhale-audio-prior-art.md` §3, which quotes `pet/src/ui/pet.ts`):
//!
//! 1. *"Skip a buffer that has entirely passed (`:43`) instead of scheduling into the past."* **Taken,
//!    as a property rather than a branch**: nothing this file returns is ever in the past — every chunk
//!    is placed at `max(cursor, now + CUSHION_S)` — so there is no past chunk and nothing to skip. The
//!    `:43` guard exists because the pet's anchor is fixed at sound-on and a late frame lands behind it;
//!    this timeline is anchored per turn and clamped, which is the same guarantee with no dead branch.
//!    `a_late_chunk_is_placed_now_plus_the_cushion_and_is_never_truncated` pins it.
//! 2. *"A partly-past buffer starts at `currentTime` with a non-zero offset into itself (`:45`)."*
//!    **Not applied, deliberately, and this is the one place this wire parts company with the idiom.**
//!    The pet's voices are notes at absolute positions in a score, with other voices to stay in time
//!    with, so entering a note mid-way is right. A reply here is a single stream of whole sentences
//!    synthesized one after another, and `cursor < now` has exactly one cause: the sentence took longer
//!    to arrive than the previous one takes to play. Starting it at an offset would silently eat its
//!    first words, and skipping it would eat the sentence — a slow speech provider must never cost the
//!    user words. What the offset protects against (hearing something twice) is already impossible:
//!    a chunk is scheduled at most once (`:last_seq`), a slot on the timeline is claimed once
//!    (`:cursor`), and the two things that do replay audio are deliberate (`Replay`) or a new turn
//!    (`end_of_turn`, whose `seq` restarts at 0 by construction). The test above
//!    (`a_late_chunk_is_placed_now_plus_the_cushion_and_is_never_truncated`) is the one that asserts the
//!    whole sentence is still there, and the admission is written down here rather than left for a
//!    reviewer to find: if a future plan gives this client a fixed anchor and several synchronised
//!    streams, this is the line to revisit.
//! 3. *"A single `Set` of live nodes with `onended` cleanup (`:26`, `:44`) and one `silence()` that stops
//!    them all (`:33`)."* **Taken**, with `onended`'s cleanup moved out of the browser — see
//!    `web_audio.rs`, and [`Playback::finished`] for what replaces it and why.
//! 4. *"An explicit lookahead anchor (`:71`) rather than 'now'."* **Taken verbatim**: [`CUSHION_S`] is
//!    the prior art's own 80 ms, and a turn's first chunk is placed that far ahead of the context's
//!    clock.
//!
//! Nothing here touches a browser type: this file compiles and its tests run on this host.

// The decisions this file does not own. `client.rs` was where they used to be copied from, and it is
// not where they live any more: `tipsy_ui::policy` is the one home, shared with the native client.
// `Command` stays: the test module's barge-in test names the sixteen-command cohort with it, and this
// task's `Consumes` line lists it. Only the two *definitions* left this file.
use tipsy_protocol::Command;
use tipsy_ui::policy::{flush_on_interrupt, interrupts_playback, starts_turn};

/// How far in front of `now` a turn's first chunk is placed.
///
/// Scheduling at "now" on every frame is what makes a jitter buffer stutter, and this is the one number
/// the prior art sets explicitly: `anchor = audio.currentTime - seconds + .08`
/// (`pet/src/ui/pet.ts:71`, quoted in `codewhale-audio-prior-art.md:97-98`). 80 ms is comfortably more
/// than a frame's own jitter and far too little to be heard as a delay before the first word.
pub const CUSHION_S: f64 = 0.08;

/// One chunk the presenter must hand to a fresh `AudioBufferSourceNode`.
#[derive(Debug, Clone, PartialEq)]
pub struct Scheduled {
    /// Identifies the node for the presenter's live-node set and for `barge_in`.
    pub id: u64,
    /// Where it starts, in `AudioContext.currentTime` seconds. Never in the past.
    pub at: f64,
    /// `Some(seq)` for a chunk off the wire; `None` for a `Replay`.
    pub seq: Option<u32>,
    /// The rate in force when it was scheduled, which is the rate the buffer must be created at.
    pub rate: u32,
    pub samples: Vec<i16>,
}

/// A chunk on the timeline that has not finished yet: what `barge_in` has to stop.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Live {
    id: u64,
    ends_at: f64,
}

pub struct Playback {
    /// The rate announced by the last `SpeechChunk`; `24_000` until one arrives, the native default
    /// (`client.rs:101`).
    rate: u32,
    /// The next start time on this turn's timeline. `None` until a turn's first chunk anchors it, and
    /// `None` again after every turn boundary and every barge-in.
    cursor: Option<f64>,
    /// The `seq` of the last chunk accepted this turn: the duplicate guard.
    last_seq: Option<u32>,
    /// Sentences the server never delivered this turn (a forward sequence gap).
    gaps: u32,
    next_id: u64,
    live: Vec<Live>,
    /// Everything received this turn, which is what `Replay` plays and what the envelope is built from
    /// (`client.rs:257`, `:271`).
    reply: Vec<i16>,
}

impl Default for Playback {
    /// `clippy::new_without_default` is part of the workspace's `-D warnings` gate: a public `new`
    /// with no `Default` is a finding, and `Playback::new()` has a fixed default rate, so a derive is
    /// not available.
    fn default() -> Self {
        Self::new()
    }
}

impl Playback {
    pub fn new() -> Self {
        Self {
            // `client.rs:101`: the native client starts at 24 kHz and takes the real rate from the
            // first `SpeechChunk` (`:199-201`).
            rate: 24_000,
            cursor: None,
            last_seq: None,
            gaps: 0,
            next_id: 1,
            live: Vec::new(),
            reply: Vec::new(),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn set_rate(&mut self, rate: u32) {
        if rate > 0 {
            self.rate = rate;
        }
    }

    /// One whole sentence off the wire, or `None` when it must not be played.
    ///
    /// `now` is `AudioContext.currentTime`. Every chunk is placed at `max(cursor, now + CUSHION_S)`: a
    /// start time in the past would make `start(when)` play at once (MDN `AudioBufferSourceNode.start()`:
    /// "If `when` is less than `AudioContext.currentTime`, or if it's 0, the sound begins to play at
    /// once"), which is how a stalled reply collapses into one simultaneous burst — that is the prior
    /// art's `:43` guard, as a property of this function instead of a branch in it (module doc, item 1).
    pub fn chunk(&mut self, seq: u32, samples: Vec<i16>, now: f64) -> Option<Scheduled> {
        // An empty chunk is not audio: the native client drops it (`client.rs:254-256`).
        if samples.is_empty() {
            return None;
        }
        if let Some(last) = self.last_seq {
            if seq <= last {
                // A repeat inside the turn — the retrigger guard, and the reason its own test exists.
                // The transport is TCP-ordered, so a *backwards* jump is a server bug rather than a
                // reordering (research note §5.3 marks that guarantee UNVERIFIED); either way this is not
                // audio the user may hear twice or out of order.
                return None;
            }
            // A forward jump: the sentences the server never delivered. Their length is not knowable
            // from their sequence numbers, so nothing is invented for them — the note's
            // `(seq - last_played - 1) * 0.020` sketch belongs to a 20 ms chunk shape this wire does
            // not use (`lib.rs:178`: one frame is one sentence). The count is the whole record.
            self.gaps += seq - last - 1;
        }
        let duration = samples.len() as f64 / f64::from(self.rate);
        let at = self.cursor.map_or(now + CUSHION_S, |cursor| cursor.max(now + CUSHION_S));
        self.cursor = Some(at + duration);
        self.last_seq = Some(seq);
        self.reply.extend_from_slice(&samples);
        Some(self.schedule(Some(seq), at, duration, samples))
    }

    fn schedule(&mut self, seq: Option<u32>, at: f64, duration: f64, samples: Vec<i16>) -> Scheduled {
        let id = self.next_id;
        self.next_id += 1;
        self.live.push(Live { id, ends_at: at + duration });
        Scheduled { id, at, seq, rate: self.rate, samples }
    }

    /// A turn is over: the next `SpeechChunk` carries `seq` 0 again (`brain.rs:198`), so the cursor and
    /// the numbering start from nothing. The audio already scheduled is **not** touched — `Done` and
    /// `Idle` arrive while the device is still speaking the last sentence (`client.rs:217-219`).
    pub fn end_of_turn(&mut self) {
        self.cursor = None;
        self.last_seq = None;
        self.gaps = 0;
    }

    /// Barge-in: everything that is still to be heard goes, and the timeline starts over. Returns the
    /// ids of the nodes the caller must `stop()` and `disconnect()` — the prior art's `silence()`
    /// (`pet/src/ui/pet.ts:33`), with the ids this core owns instead of a set the presenter owns. Only
    /// the chunks whose audio has not ended by `now` come back: stopping a finished node is meaningless,
    /// and the ids of the finished ones are `finished`'s to report.
    pub fn barge_in(&mut self, now: f64) -> Vec<u64> {
        self.cursor = None;
        self.last_seq = None;
        self.gaps = 0;
        self.live.drain(..).filter(|live| live.ends_at > now).map(|live| live.id).collect()
    }

    /// **The whole barge-in rule for one command, which is the browser's and the native client's at
    /// once.** The plan's Step 5 writes these five lines inline wherever an action is dispatched
    /// (`client.rs:361-367`, `:578-580`, `:658-660`), and Task 3.5's `dispatch` is this method's one
    /// production call site:
    ///
    /// ```text
    /// let now = shared.ctx.current_time();
    /// web_audio::silence(&shared.playback.on_command(&command, now));
    /// ```
    ///
    /// It is a method here rather than five lines in `lib.rs` for the reason this task exists: the
    /// decisions are `tipsy_ui::policy`'s (`flush_on_interrupt`, `interrupts_playback`, `starts_turn`),
    /// the timeline is this file's, and the join between them is the one thing that must not be
    /// re-derived per shell. `flush_on_interrupt` is imported and called rather than open-coded as
    /// `interrupts_playback(..) && ..` — same value, and neither client can drift on that `&&`.
    ///
    /// The returned ids are the nodes the caller must silence; an empty `Vec` means nothing was
    /// playing and nothing was touched. `forget_reply` is called for a command that starts a turn
    /// whether or not anything was playing: what `Replay` would queue is the previous turn's audio, and
    /// that is stale the moment the next turn begins.
    pub fn on_command(&mut self, command: &Command, now: f64) -> Vec<u64> {
        let stop = if flush_on_interrupt(interrupts_playback(command), self.is_playing(now)) {
            // The browser's half of "the client sends `Cancel` and drops its audio queue" (v1 spec
            // §5.3), and the prior art's one `silence()`: stop everything that is playing, in one call.
            self.barge_in(now)
        } else {
            // Nothing to drop. **`barge_in` is not called**, so an interrupt that arrives between
            // sentences leaves the cursor, the sequence position and the gap count exactly where they
            // were — the timeline of the turn that is still arriving is not reset by a keystroke that
            // had no audio to stop.
            Vec::new()
        };
        if starts_turn(command) {
            self.forget_reply();
        }
        stop
    }

    /// The ids whose audio has ended by `now`: the presenter drops those node references.
    ///
    /// The prior art does this from the node's own `onended` (`pet/src/ui/pet.ts:44`). From Rust that
    /// means a `Closure` per node, and its lifetime is the hazard `wasm-bindgen`'s own documentation
    /// warns about: `Closure::forget` leaks it for the life of the page, and storing it in the entry the
    /// callback removes drops the closure while it is running. The end time was known when the chunk was
    /// scheduled, so the core reports it instead — the same bookkeeping, no leak, and `ended` stays what
    /// MDN says it is (it fires after an explicit `stop()` too, so it could not prove that a sentence was
    /// heard; research note §5.1, §5.4).
    pub fn finished(&mut self, now: f64) -> Vec<u64> {
        let (done, live): (Vec<Live>, Vec<Live>) = self.live.drain(..).partition(|live| live.ends_at <= now);
        self.live = live;
        done.into_iter().map(|live| live.id).collect()
    }

    /// `Replay`: the reply kept for the selected entry, played again from `now`. The sequence position
    /// is untouched and the reply is not extended — the native client's `Replay` only queues the audio
    /// it kept (`client.rs:371-377`). A replay the user just asked for is heard now, whatever the
    /// timeline's own position is, which is why a stale cursor is not used here.
    pub fn replay(&mut self, now: f64) -> Option<Scheduled> {
        if self.reply.is_empty() {
            return None;
        }
        let samples = self.reply.clone();
        let duration = samples.len() as f64 / f64::from(self.rate);
        let at = self.cursor.filter(|cursor| *cursor > now).unwrap_or(now + CUSHION_S);
        self.cursor = Some(at + duration);
        Some(self.schedule(None, at, duration, samples))
    }

    /// The next turn starts: what `Replay` would replay is the previous turn's audio (`client.rs:364-367`).
    pub fn forget_reply(&mut self) -> bool {
        let had = !self.reply.is_empty();
        self.reply.clear();
        had
    }

    /// The envelope of everything heard this turn: one value per 50 ms, peak-normalised.
    ///
    /// The arithmetic is `crates/tipsy-assistant/src/audio/pcm.rs:58-66` (`hop = rate / 20`, `rms` per
    /// hop, divided by the peak) with `i16_to_f32` from `:24-26`; `tipsy-assistant` is the native binary
    /// and is not a dependency of this crate, so the two implementations are the same twelve lines
    /// rather than one function — and the test above reproduces `pcm.rs:157-166`'s own fixture so a
    /// drift between them shows up as a failing test here. This is the *same observation* the native
    /// client feeds its oscilloscope with (`client.rs:271`), which is what keeps the two HUDs looking
    /// alike: building it per chunk instead of over the accumulated reply would normalise each sentence
    /// to its own peak and draw a different picture.
    pub fn envelope(&self) -> Vec<f32> {
        let floats: Vec<f32> = self.reply.iter().map(|s| *s as f32 / 32768.0).collect();
        let hop = (self.rate / 20).max(1) as usize;
        let env: Vec<f32> = floats
            .chunks(hop)
            .map(|window| {
                if window.is_empty() {
                    0.0
                } else {
                    (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt()
                }
            })
            .collect();
        let peak = env.iter().cloned().fold(0.0, f32::max);
        if peak <= 0.0 {
            return env;
        }
        env.into_iter().map(|value| value / peak).collect()
    }

    pub fn reply(&self) -> &[i16] {
        &self.reply
    }

    pub fn gaps(&self) -> u32 {
        self.gaps
    }

    pub fn is_playing(&self, now: f64) -> bool {
        self.live.iter().any(|live| live.ends_at > now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::{AssistantState, Capability, ClientKind, Event, ListeningMode};

    /// 24 000 samples at 24 kHz is one second: the arithmetic every test below relies on.
    fn samples(ms: u64, rate: u32) -> Vec<i16> {
        vec![0i16; (rate as u64 * ms / 1000) as usize]
    }

    fn sine(freq: f32, rate: u32, ms: u32) -> Vec<i16> {
        let n = (rate as u64 * ms as u64 / 1000) as usize;
        (0..n)
            .map(|i| {
                let s = (i as f32 * freq * std::f32::consts::TAU / rate as f32).sin() * 0.5;
                (s * 32767.0) as i16
            })
            .collect()
    }

    #[test]
    fn the_first_chunk_is_anchored_a_cushion_ahead_and_the_rest_follow_it_gaplessly() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let first = p.chunk(0, samples(1000, 24_000), 10.0).expect("the first chunk plays");
        assert_eq!(first.at, 10.0 + CUSHION_S, "the anchor is `now` plus the cushion, never `now` itself");
        assert_eq!(first.seq, Some(0));
        let second = p.chunk(1, samples(500, 24_000), 10.1).expect("the second chunk plays");
        assert_eq!(second.at, 11.0 + CUSHION_S, "the second starts exactly where the first ends: no gap, no overlap");
        let third = p.chunk(2, samples(500, 24_000), 10.2).expect("the third chunk plays");
        assert_eq!(third.at, 11.5 + CUSHION_S);
    }

    /// The prior art's `:43` guard, as a property of this timeline rather than a branch in it: no chunk
    /// is ever placed in the past, however late it arrives, and a late one is not truncated either.
    #[test]
    fn a_late_chunk_is_placed_now_plus_the_cushion_and_is_never_truncated() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        p.chunk(0, samples(500, 24_000), 10.0).expect("plays"); // 10.08 .. 10.58
        // The frame loop stalled and this chunk arrives long after its slot would have been: a sentence
        // whose synthesis took longer than the previous one takes to play. Its audio is whole and its
        // first words matter, so it is placed ahead of `now` and played from its start.
        let late = p.chunk(1, samples(1000, 24_000), 12.0).expect("plays");
        assert_eq!(late.at, 12.0 + CUSHION_S, "in the future, never in the past: `start(when)` with a past `when` plays at once");
        assert_eq!(late.samples.len(), 24_000, "and not one sample of it is dropped (the offset the prior art uses is rejected above)");
        let next = p.chunk(2, samples(500, 24_000), 12.1).expect("plays");
        assert_eq!(next.at, 13.0 + CUSHION_S, "the timeline continues from the new anchor, gaplessly");
        assert_eq!(p.gaps(), 0, "a late chunk is not a gap: nothing was missing from the wire");
    }

    /// The prior art's second invariant, in this wire's terms: a frame the socket delivers twice (a
    /// re-sent batch, a caller that drains its queue twice) must not schedule a second buffer. A
    /// replayed buffer is audible corruption, and this is the test that pins it.
    #[test]
    fn a_frame_delivered_twice_cannot_schedule_twice() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let first = p.chunk(0, samples(1000, 24_000), 10.0).expect("plays");
        assert!(p.chunk(0, samples(1000, 24_000), 10.05).is_none(), "the same sentence twice is scheduled once");
        assert_eq!(p.reply().len(), 24_000, "and it was not appended a second time either");
        assert_eq!(p.gaps(), 0, "a repeat is not a gap");
        let next = p.chunk(1, samples(500, 24_000), 10.06).expect("the turn continues");
        assert_eq!(next.at, 11.0 + CUSHION_S, "the repeat left the timeline untouched");
        assert_ne!(next.id, first.id);
    }

    /// The prior art's first invariant: "Calling at any display cadence produces the same score when all
    /// world ticks are supplied" (`pet-audio.ts:10-12`). The score here is `at`, and the only things that
    /// may change it are the order of the chunks and their arrival times — never how often the presenter
    /// asks. A *stall* is a different case and is the test above.
    #[test]
    fn the_same_chunks_score_the_same_at_any_cadence() {
        let run = |nows: [f64; 4]| {
            let mut p = Playback::new();
            p.set_rate(24_000);
            let mut out = Vec::new();
            for (i, now) in nows.into_iter().enumerate() {
                out.push(p.chunk(i as u32, samples(1000, 24_000), now).expect("plays").at);
            }
            out
        };
        let fast = run([0.0, 0.010, 0.020, 0.030]);
        let slow = run([0.0, 0.050, 0.060, 0.070]);
        assert_eq!(fast, slow, "one frame loop drains at 30 Hz and another at 10 Hz; the score is identical");
        assert_eq!(fast, [0.08, 1.08, 2.08, 3.08]);
    }

    #[test]
    fn a_forward_gap_keeps_the_timeline_gapless_and_counts_the_missing_sentences() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        p.chunk(0, samples(1000, 24_000), 10.0).expect("plays");
        let after_gap = p.chunk(3, samples(1000, 24_000), 10.1).expect("the next sentence that did arrive plays");
        assert_eq!(after_gap.at, 11.0 + CUSHION_S, "no silence is invented for the two sentences that never arrived");
        assert_eq!(p.gaps(), 2, "the missing sentences are counted, and that is all a gap can do here");
    }

    #[test]
    fn the_rate_is_the_one_the_speech_chunk_announced() {
        let mut p = Playback::new();
        assert_eq!(p.rate(), 24_000, "the native default (`client.rs:101`)");
        // 16 000 samples last one second at 16 kHz and two thirds of a second at 24 kHz: playing the
        // reply at the microphone's own 16 kHz (device.rs:18) is what makes it audibly slow.
        p.set_rate(16_000);
        assert_eq!(p.chunk(0, samples(1000, 16_000), 0.0).expect("plays").at, CUSHION_S);
        let one_second = p.chunk(1, samples(1000, 16_000), 0.0).expect("plays");
        assert_eq!(one_second.at, 1.0 + CUSHION_S);
        assert_eq!(one_second.rate, 16_000, "the chunk carries the rate its buffer must be created at");
        // The same 16 000 samples, announced at 24 kHz, are two thirds of a second: the cursor moves by
        // the announced rate and not by the microphone's.
        p.set_rate(24_000);
        let two_thirds = p.chunk(2, samples(1000, 16_000), 1.0).expect("plays");
        assert!((two_thirds.at - 2.08).abs() < 1e-9, "{}", two_thirds.at);
        assert_eq!(two_thirds.rate, 24_000, "and a later rate change is what the *next* chunk gets");
        let after = p.chunk(3, samples(1000, 24_000), 2.0).expect("plays");
        assert!((after.at - (2.08 + 2.0 / 3.0)).abs() < 1e-9, "16 000 samples at 24 kHz is 0.667 s: {}", after.at);
    }

    #[test]
    fn a_turn_boundary_forgets_the_timeline_and_leaves_the_audio_playing() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let first = p.chunk(0, samples(1000, 24_000), 10.0).expect("plays");
        p.chunk(1, samples(1000, 24_000), 10.0).expect("plays");
        assert_eq!(p.gaps(), 0);
        p.end_of_turn();
        // `Done` and `Idle` arrive while the device is still speaking (`client.rs:217-219`): the
        // scheduled chunks are still live, and only the next turn's numbering is new.
        assert!(p.is_playing(10.5), "the audio of the turn that just ended is still live");
        let next_turn = p.chunk(0, samples(500, 24_000), 20.0).expect("seq restarts at 0, and it plays");
        assert_eq!(next_turn.at, 20.0 + CUSHION_S, "a fresh turn is anchored at `now`, not at the old cursor");
        assert_eq!(p.gaps(), 0, "and the gap count is per turn");
        assert_eq!(p.finished(11.5), vec![first.id], "the first turn's finished chunk is still reported");
    }

    #[test]
    fn barge_in_hands_back_every_live_node_and_starts_a_new_epoch() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let a = p.chunk(0, samples(1000, 24_000), 10.0).expect("plays");
        let b = p.chunk(1, samples(1000, 24_000), 10.0).expect("plays");
        assert!(p.is_playing(10.0));
        assert_eq!(p.barge_in(10.0), vec![a.id, b.id], "every node that is playing is the caller's to stop");
        assert!(!p.is_playing(10.0), "and the timeline holds nothing");
        let after = p.chunk(0, samples(500, 24_000), 10.2).expect("plays");
        assert_eq!(after.at, 10.2 + CUSHION_S, "the next turn's first chunk is anchored at `now`");
    }

    /// A chunk that has finished is not handed back by a barge-in: stopping it is meaningless, and
    /// handing its id back would let the presenter try to stop a node it has already dropped.
    #[test]
    fn barge_in_hands_back_only_what_has_not_finished() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let past = p.chunk(0, samples(500, 24_000), 10.0).expect("plays"); // 10.08 .. 10.58
        let live = p.chunk(1, samples(1000, 24_000), 12.0).expect("plays"); // 12.08 .. 13.08
        assert_eq!(p.barge_in(12.0), vec![live.id], "the finished chunk is not the caller's to stop");
        assert!(!p.is_playing(12.0));
        let _ = past;
    }

    #[test]
    fn finished_reports_the_nodes_whose_audio_has_ended() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        let a = p.chunk(0, samples(1000, 24_000), 10.0).expect("plays"); // 10.08 .. 11.08
        let b = p.chunk(1, samples(500, 24_000), 10.0).expect("plays"); // 11.08 .. 11.58
        assert!(p.finished(10.5).is_empty(), "nothing has finished yet");
        assert_eq!(p.finished(11.2), vec![a.id]);
        assert_eq!(p.finished(11.6), vec![b.id]);
        assert!(p.finished(12.0).is_empty(), "and a finished node is never reported twice");
    }

    #[test]
    fn replay_plays_the_kept_reply_without_disturbing_the_sequence() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        assert!(p.replay(10.0).is_none(), "nothing has been heard, so there is nothing to replay");
        p.chunk(0, samples(1000, 24_000), 10.0).expect("plays"); // 10.08 .. 11.08
        p.chunk(1, samples(1000, 24_000), 10.0).expect("plays"); // 11.08 .. 12.08
        // The cue is behind the cursor, so it queues after what is already scheduled: rodio's `append`.
        let again = p.replay(11.5).expect("the reply is kept until the next turn starts");
        assert!((again.at - 12.08).abs() < 1e-9, "{}", again.at);
        assert_eq!(again.seq, None, "a replay is not a sentence off the wire");
        assert_eq!(p.reply().len(), 48_000);
        // The sequence position is untouched: the turn that is still arriving continues where it was.
        let next = p.chunk(2, samples(500, 24_000), 11.6).expect("plays");
        assert!((next.at - (12.08 + 48_000.0 / 24_000.0)).abs() < 1e-9, "the cursor moved by the replay, exactly as rodio's queue does: {}", next.at);
        assert_eq!(p.gaps(), 0);
        // A replay asked for after the timeline went stale is still heard *now*: the user asked for it.
        let stale = p.replay(40.0).expect("the reply is still kept");
        assert_eq!(stale.at, 40.0 + CUSHION_S);
        assert!(p.forget_reply());
        assert!(!p.forget_reply(), "and forgetting an empty reply reports nothing to forget");
        assert!(p.replay(41.0).is_none());
    }

    #[test]
    fn the_envelope_is_the_native_one_value_per_50_ms_normalised_to_the_peak() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        // The same fixture as `crates/tipsy-assistant/src/audio/pcm.rs:157-166`: 50 ms of silence and
        // 100 ms of tone, which is three hops at 24 kHz.
        p.chunk(0, vec![0i16; 24_000 / 20], 0.0).expect("plays");
        p.chunk(1, sine(220.0, 24_000, 100), 0.0).expect("plays");
        let env = p.envelope();
        assert_eq!(env.len(), 3, "one value per 50 ms (`state.rs:14`, `render.rs:32`)");
        assert_eq!(env[0], 0.0);
        assert!((env.iter().cloned().fold(0.0, f32::max) - 1.0).abs() < 1e-6, "normalised to the peak, as `pcm.rs:58-66` does");
        // Silence is silence, and not a row of ones.
        let mut quiet = Playback::new();
        quiet.set_rate(24_000);
        quiet.chunk(0, vec![0i16; 2400], 0.0).expect("plays");
        assert!(quiet.envelope().iter().all(|v| *v == 0.0));
    }

    /// The payload `Command::UpdateConfig` needs, so the cohort below can name all **sixteen**
    /// commands. The plan's own cohort named thirteen of them: Task 1.1 of this same plan added
    /// `GetConfig`/`UpdateConfig`/`SetKey` (and their three `starts_turn` arms) after the plan's test
    /// was written, and those three are exactly the commands a writer of the client's policy would
    /// otherwise have to re-decide by hand.
    fn update_config() -> Command {
        use tipsy_protocol::{
            ClientConfigView, ClientServerView, ListeningView, LoginSettingsView, RealtimeView,
        };
        Command::UpdateConfig {
            config: Box::new(ClientConfigView {
                active_profile: "compae".into(),
                providers: Vec::new(),
                listening: ListeningView {
                    mode: ListeningMode::Push,
                    silence_ms: "900".into(),
                    mic_gain: "1.0".into(),
                    mic_device: None,
                },
                realtime: RealtimeView {
                    enabled: false,
                    model: "gpt-realtime".into(),
                    key_name: "STUB".into(),
                    voice: "marin".into(),
                    max_minutes: 5,
                },
                login: LoginSettingsView { enabled: false, voice: false, allowed: Vec::new() },
                profiles: Vec::new(),
                server: ClientServerView { bind: "127.0.0.1:7070".into(), lan: false },
            }),
        }
    }

    /// **The cohort, and the expected answer written out rather than computed.** One row per command,
    /// with the two decisions `tipsy_ui::policy` makes about it: whether it starts a turn, and whether
    /// it interrupts playback. The two `bool`s are literals, so a change to any arm of `starts_turn`
    /// fails here instead of agreeing with itself — and `the_cohort_names_every_command_this_build_has`
    /// is what keeps the rows honest about covering every variant.
    ///
    /// Eighteen rows for sixteen variants: `Ask` and `Say` appear twice each, because the *empty*
    /// question and the bare `!` are the two payloads whose decision differs from their variant's
    /// (`session.rs:233-238` ignores both, so neither may silence a reply in progress).
    fn command_cohort() -> Vec<(Command, bool, bool)> {
        vec![
            // (command, starts a turn, interrupts playback)
            (Command::Hello { token: None, client: ClientKind::Web, protocol_version: 1 }, false, false),
            (Command::Ask { text: "¿qué pasa?".into() }, true, true),
            (Command::Ask { text: "   ".into() }, false, false),
            (Command::Say { text: "hola compadre".into() }, true, true),
            (Command::Say { text: String::new() }, false, false),
            (Command::ListenStart, false, false),
            (Command::ListenStop, true, true),
            (Command::Cancel, false, true),
            (Command::Reset, true, true),
            (Command::SetProfile { id: "compae".into() }, false, false),
            (Command::SetListeningMode { mode: ListeningMode::Push }, false, false),
            (Command::TestProvider { capability: Capability::Speech }, false, false),
            (Command::LoginStart, false, false),
            (Command::LoginStop, false, false),
            (Command::LoginAnswer { text: "compadre".into() }, false, false),
            (Command::GetConfig, false, false),
            (update_config(), false, false),
            (Command::SetKey { name: "STUB".into(), value: "no-es-un-secreto-real".into() }, false, false),
        ]
    }

    /// The `type` tag of every command this build has, in the protocol's own spelling
    /// (`tipsy-protocol/src/lib.rs:348-392`, `rename_all = "snake_case"`). Sixteen, not the thirteen the
    /// plan's front matter claims. Sorted, because the assertion compares sorted sets.
    const COMMAND_TAGS: [&str; 16] = [
        "ask",
        "cancel",
        "get_config",
        "hello",
        "listen_start",
        "listen_stop",
        "login_answer",
        "login_start",
        "login_stop",
        "reset",
        "say",
        "set_key",
        "set_listening_mode",
        "set_profile",
        "test_provider",
        "update_config",
    ];

    /// The cohort above is a hand-written list, and a hand-written list is the shape that rots: this
    /// reads each row's own serde tag and asserts the set is exactly the protocol's. A seventeenth
    /// command added to `tipsy-protocol` fails here until a row with its decisions is written — the
    /// same device `wire.rs` uses for `Event`.
    #[test]
    fn the_cohort_names_every_command_this_build_has() {
        let mut tags: Vec<String> = command_cohort()
            .iter()
            .map(|(command, _, _)| {
                let value = serde_json::to_value(command).expect("a command serialises");
                value.get("type").and_then(serde_json::Value::as_str).expect("every command is tagged").to_string()
            })
            .collect();
        tags.sort();
        tags.dedup();
        assert_eq!(tags, COMMAND_TAGS, "the cohort does not cover exactly the protocol's commands");
    }

    #[test]
    fn the_barge_in_rule_is_the_native_one() {
        for (command, starts, interrupts) in command_cohort() {
            assert_eq!(starts_turn(&command), starts, "starts_turn({command:?})");
            assert_eq!(interrupts_playback(&command), interrupts, "interrupts_playback({command:?})");
            // The whole rule is one expression, and this is it: `Cancel` ends a turn without starting
            // one (`session.rs:266-270`), and nothing else interrupts that does not start a turn.
            assert_eq!(interrupts, starts || command == Command::Cancel, "{command:?}");
        }
    }

    /// **The test the split exists for.** The browser's barge-in and the native client's are one rule
    /// because this calls `tipsy_ui::policy`'s functions instead of reproducing their answers: for every
    /// command, what the timeline does is what
    /// `flush_on_interrupt(interrupts_playback(command), is_playing(now))` and `starts_turn(command)`
    /// say. No copy of either decision appears in this file, and mutating the shared function is what
    /// makes these assertions fail — measured, and recorded in the ledger's induced failures.
    #[test]
    fn the_timeline_takes_its_barge_in_from_the_shared_rule() {
        for (command, starts, interrupts) in command_cohort() {
            let mut p = Playback::new();
            p.set_rate(24_000);
            let playing = p.chunk(0, samples(1000, 24_000), 10.0).expect("plays");
            assert!(p.is_playing(10.0), "{command:?}: the fixture has to be playing for this to mean anything");

            let stop = p.on_command(&command, 10.0);

            let want = if flush_on_interrupt(interrupts, true) { vec![playing.id] } else { Vec::new() };
            assert_eq!(stop, want, "{command:?} silences exactly what the shared guard says");
            assert_eq!(p.is_playing(10.0), want.is_empty(), "{command:?}: a silenced node is gone, and a kept one still plays");
            assert_eq!(p.reply().is_empty(), starts, "{command:?}: the kept reply is stale exactly when a turn starts");
            assert_eq!(p.replay(11.0).is_none(), starts, "{command:?}: and a stale reply is not replayable");
        }
    }

    /// The `playing` half of `flush_on_interrupt`, which the plan calls "not load-bearing in a
    /// browser" — it is load-bearing here, and this is what it protects. A command that interrupts
    /// while the last sentence has already been spoken out must leave the turn's own bookkeeping alone:
    /// **`barge_in` is not called**, so the sequence position the late sentences of the running turn are
    /// still measured against survives, and no gap is invented.
    #[test]
    fn an_interrupt_with_nothing_left_to_drop_leaves_the_timeline_where_it_was() {
        let mut p = Playback::new();
        p.set_rate(24_000);
        p.chunk(0, samples(1000, 24_000), 10.0).expect("plays"); // 10.08 .. 11.08
        assert!(!p.is_playing(12.0), "the sentence has been spoken out and the next one has not arrived");

        let stop = p.on_command(&Command::Ask { text: "¿y ahora?".into() }, 12.0);
        assert!(stop.is_empty(), "nothing was playing, so there is nothing to stop");
        assert!(p.reply().is_empty(), "and the kept reply is still stale: this command starts a turn");
        assert!(p.replay(12.5).is_none());

        // `barge_in` would have reset the sequence position; it was not called, so the turn's own
        // numbering is intact — the duplicate is still a duplicate and no gap was counted.
        assert!(p.chunk(0, samples(500, 24_000), 12.0).is_none(), "the sequence position survived");
        assert_eq!(p.gaps(), 0);
        let next = p.chunk(1, samples(500, 24_000), 12.0).expect("the running turn continues");
        assert_eq!(next.at, 12.0 + CUSHION_S, "and it continues from `now`, because the cursor was 11.08");
    }

    #[test]
    fn an_event_that_ends_a_turn_is_the_one_that_resets_it() {
        // The three boundaries the caller watches, named here so the wiring in Step 5 cannot drift:
        // `Heard` starts a turn (a call's own sequence counter restarts there, `realtime/mod.rs:611-613`),
        // `Done` ends one that completed (`brain.rs:115-118`, `:166-169`) and `Idle` ends one that was
        // cancelled and gets no `Done` (`brain.rs:110`, `:162-163`; `state.rs:315-317`).
        let boundaries = [
            Event::Heard { text: "¿y el ticket?".into() },
            Event::Done { first_audio_ms: Some(300), total_ms: 1200 },
            Event::Status { state: AssistantState::Idle, detail: None },
        ];
        assert_eq!(boundaries.len(), 3);
        assert!(matches!(boundaries[2], Event::Status { state: AssistantState::Idle, .. }));
    }
}
