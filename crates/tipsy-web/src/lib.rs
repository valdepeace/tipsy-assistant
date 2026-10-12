//! The browser client's entry point.
//!
//! `start()` is what `wasm-bindgen --target web`'s generated `init()` calls. The render loop is the
//! native client's frame loop with the socket's queue in place of `from_server`
//! (`crates/tipsy-compadre/src/client.rs:272-349` is the loop being mirrored): tick, drain, draw.
//! `draw_web` wraps `requestAnimationFrame` itself, so the loop needs **no async runtime** — only the
//! socket's callbacks do, and those use `spawn_local` (ratzilla-and-wasm-audio.md §6.4 point 5).
//!
//! **What this file deliberately does not have yet.** The key and mouse handlers and the page's
//! layout are Task 3.5's; the hits `tipsy_ui::draw` reports are therefore dropped here rather than
//! kept in a cell nothing reads, and Task 3.5 is what starts reading them. **The barge-in is
//! [`playback::Playback::on_command`]'s and its call site is Task 3.5's too**, for the same reason: the
//! only thing that produces a `UiAction` is the keyboard and the pointer. The rule itself is not defined
//! here and is not repeated here — `on_command` calls `tipsy_ui::policy`'s own functions.
//!
//! **The microphone is Task 3.3's and it is wired here**, because `#[wasm_bindgen(start)]` is the only
//! entry point `init()` calls and this is where the `UiState`, the queue and the render loop live: the
//! gate is built from this function, the capture it opens is kept in one [`mic::CaptureSlot`] for the
//! whole run, and every frame drains that capture inside the one borrow of the state the frame owns.
//!
//! **The reply is Task 3.4's, and it plays through the microphone's own context.** [`playback`] is the
//! timeline — pure, host-tested — and [`web_audio`] is the presenter that schedules one
//! `AudioBufferSourceNode` per sentence on it. There is exactly one `AudioContext` per run and it is
//! the capture's ([`mic::Capture::context`]): a second one would be a second clock. That is also why a
//! `Frame::Speech` arriving before the user's press is **dropped** rather than kept — the events around
//! it still reach the HUD, and the audio has nowhere to go.

pub mod mic;
pub mod playback;
pub mod socket;
pub mod web_audio;
pub mod wire;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use tipsy_protocol::{AssistantState, Event, Frame, ToClient};
use tipsy_ui::policy;
use wasm_bindgen::prelude::*;

// `draw_web` and the two event hooks are `WebRenderer`'s, and the trait has to be in scope for
// `Terminal<WebGl2Backend>` to have them (ratzilla 0.3.1's `render.rs`). Task 3.5 adds
// `WebEventHandler`'s two hooks beside it.
use ratzilla::WebRenderer;

/// The wasm entry point (the front matter's shared contract pins this name).
///
/// The backend is the one Task 3.1's verdict named; the three backends are interchangeable behind
/// `WebRenderer`, so this is the one line a later switch changes.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let backend = ratzilla::WebGl2Backend::new().map_err(|error| JsValue::from_str(&error.to_string()))?;
    let terminal =
        ratzilla::ratatui::Terminal::new(backend).map_err(|error| JsValue::from_str(&error.to_string()))?;

    let state = Rc::new(RefCell::new(tipsy_ui::UiState::new()));
    let queue: Rc<RefCell<VecDeque<ToClient>>> = Rc::new(RefCell::new(VecDeque::new()));

    // The page's own address, which is also the authority `/ws` compares the `Origin` against:
    // `location.host` verbatim, never a rewritten loopback spelling (the ledger's H1 ruling).
    let url = socket::url_from_location()?;
    let token = socket::token_from_location();
    let outbox = socket::spawn_connection(url, token, Rc::clone(&queue), Rc::clone(&state));

    // One per run: the gate fills it on the user's press, this loop drains it on every frame, and it
    // holds the `AudioContext` Task 3.4 plays the reply through. `None` means no capture is running.
    let capture: mic::CaptureSlot = Rc::new(RefCell::new(None));
    // The "press to start" screen (v1 spec `:193-194`). It builds its own elements, so the page Task
    // 3.2 created needs no change for it, and a microphone that cannot work is a *disabled* button
    // with a reason rather than a button that fails on its first press.
    mic::press_to_start(Rc::clone(&state), Rc::clone(&capture));

    let mut last = js_sys::Date::now();
    // The reply's timeline: one per run, fed by the frame loop below and presented by `web_audio`.
    // Task 3.5 moves it into the `Shared` its `dispatch` holds; nothing here reads it any other way.
    let mut playback = playback::Playback::new();
    terminal.draw_web({
        let outbox = outbox.clone();
        move |frame| {
            let now = js_sys::Date::now();
            let dt = (now - last).max(0.0) as u64;
            last = now;
            let title = {
                let mut state = state.borrow_mut();
                state.tick(dt);
                // The frame loop drains everything that arrived since the last frame, exactly as the
                // native loop drains `from_server` (`client.rs:322-324`).
                let mut queue = queue.borrow_mut();
                // The one clock the reply is scheduled on: the capture's `AudioContext`, and `None`
                // until the user's press. `current_time` is MDN's, in seconds, and it stops advancing
                // while the context is suspended.
                let audio_now = capture.borrow().as_ref().map(|capture| capture.context().current_time());
                let (welcomed, scheduled) = route(&mut queue, &mut playback, &mut state, audio_now);
                // The language first: `apply` is what sets it from `Welcome`, and the "connection
                // restored" notice is the one thing this side says, so it is said in the profile's
                // language and not in the default one.
                if welcomed && outbox.welcomed() {
                    let text = state.strings().web_online;
                    state.notify(text);
                }
                if let Some(audio_now) = audio_now {
                    if let Some(capture) = capture.borrow().as_ref() {
                        let ctx = capture.context();
                        for chunk in &scheduled {
                            // The frame callback cannot propagate an error, and a silent client is the
                            // worst outcome this file has: put it where the browser console can be read.
                            if let Err(error) = web_audio::play(ctx, chunk) {
                                web_sys::console::error_1(&error);
                            }
                        }
                    }
                    // Nodes whose audio has ended are forgotten so the map cannot grow for the length
                    // of a reply. The prior art does this from each node's `onended`; `web_audio.rs`
                    // says why this does it from the core instead.
                    for id in playback.finished(audio_now) {
                        web_audio::forget(id);
                    }
                }
                // The microphone, once per frame and inside the one borrow this frame owns: the port's
                // handler only appends what the worklet posted, and this is the only place a captured
                // frame may reach the socket. `capture().as_ref()` is `None` until the gate's click
                // opened one, which is what "no capture, no frames" means here.
                if let Some(capture) = capture.borrow().as_ref() {
                    mic::drain_capture(capture, &mut state, &outbox);
                }
                state.profile.as_ref().map(|profile| profile.name.clone()).unwrap_or_default()
            };
            let clock = clock_view();
            let state = state.borrow();
            // Task 3.5 keeps these: a click is tested against the hits of the frame on screen.
            let _hits = tipsy_ui::draw(frame, &state, &clock, &title);
        }
    });
    Ok(())
}

/// The socket's messages, routed into the HUD and the timeline.
///
/// The answer is `(welcomed, play)`: whether a `Welcome` arrived — the caller turns that into the
/// "connection restored" notice, in the language `apply` has just set — and the chunks the presenter
/// has to schedule, in the order the timeline placed them.
///
/// **`audio_now` is the whole of "there is a context to play through".** It is the capture's
/// `AudioContext.currentTime` in seconds, and `None` is the user not having pressed yet. The **events
/// reach the HUD either way** — `Welcome`, the login door's verdicts and every status line arrive
/// before the microphone is open, and a client that dropped them until the press would show nothing —
/// while a `Frame::Speech` arriving with `None` is **dropped rather than kept**: there is nothing to
/// schedule it on, a second `AudioContext` built here would be a second clock, and holding the samples
/// until the press would start the reply mid-turn against a `seq` numbering that has moved on. That is
/// Task 3.3's ledger note 1, as code.
///
/// The order inside one message is the native client's `on_message`: the announced rate first, then the
/// audio it belongs to, and the turn boundaries where its own bookkeeping sits.
fn route(
    queue: &mut VecDeque<ToClient>,
    playback: &mut playback::Playback,
    state: &mut tipsy_ui::UiState,
    audio_now: Option<f64>,
) -> (bool, Vec<playback::Scheduled>) {
    let mut welcomed = false;
    let mut play = Vec::new();
    while let Some(message) = queue.pop_front() {
        match message {
            ToClient::Event(event) => {
                // `policy::speech_rate` is the native client's own rule and the reason it lives in
                // `tipsy-ui` at all: the rate is set *before* the audio it belongs to, and this is the
                // browser's half of that order (`client.rs:363`).
                if let Some(rate) = policy::speech_rate(&event) {
                    playback.set_rate(rate);
                }
                // The three turn boundaries. `end_of_turn` touches no node: `Done` and `Idle` arrive
                // while the device is still speaking the last sentence, so the audio of the turn that
                // just ended keeps playing.
                if ends_turn(&event) {
                    playback.end_of_turn();
                }
                welcomed |= matches!(event, Event::Welcome { .. });
                state.apply(&event);
            }
            ToClient::Audio(Frame::Speech { seq, samples }) => {
                let Some(audio_now) = audio_now else {
                    continue;
                };
                if let Some(chunk) = playback.chunk(seq, samples, audio_now) {
                    play.push(chunk);
                    // `client.rs:270-271`: the oscilloscope gets the envelope of the whole reply,
                    // rebuilt as it grows, and `start_voice` restarts the playhead at the first hop.
                    // Only a chunk the timeline accepted rebuilds it — a duplicate is not new audio,
                    // and neither is an empty frame.
                    state.start_voice(playback.envelope());
                }
            }
            // A microphone frame coming back from the server is not a thing (`lib.rs:223` ignores it
            // too), and the server never sends one.
            ToClient::Audio(Frame::Mic(_)) => {}
        }
    }
    (welcomed, play)
}

/// The three boundaries that end a turn, as a function so the wiring cannot drift from the tests:
/// `Heard` starts a turn (a call's own sequence counter restarts there, `realtime/mod.rs:611-613`),
/// `Done` ends one that completed (`brain.rs:115-118`, `:166-169`) and `Idle` ends one that was
/// cancelled and gets no `Done` (`brain.rs:110`, `:162-163`; `state.rs:315-317`).
///
/// A status change that is not `Idle` is deliberately not a boundary: `Speaking` and `Thinking` happen
/// in the middle of a turn.
fn ends_turn(event: &Event) -> bool {
    matches!(event, Event::Heard { .. } | Event::Done { .. })
        || matches!(event, Event::Status { state: AssistantState::Idle, .. })
}

/// The HUD's clock, from the page's own `Date`. `ClockView.weekday` numbers the week from Monday
/// (`render.rs`'s `ClockView`, built natively at `client.rs:566-576`) and `js_sys::Date::get_day()`
/// numbers it from Sunday, which is the one off-by-one in this file — so it is a function of its own
/// with a test under it, and not an expression inside a `Date` call no host can make.
fn clock_view() -> tipsy_ui::ClockView {
    let now = js_sys::Date::new_0();
    tipsy_ui::ClockView {
        time: format!("{:02}:{:02}:{:02}", now.get_hours(), now.get_minutes(), now.get_seconds()),
        weekday: weekday_from_sunday(now.get_day()),
        day: now.get_date() as u8,
        month: now.get_month() as u8,
    }
}

/// `Date::get_day()`: `0` is Sunday. `ClockView::weekday`: `0` is Monday, and `tipsy-ui`'s weekday
/// table starts at `lunes`/`Monday`.
fn weekday_from_sunday(day: u32) -> u8 {
    ((day + 6) % 7) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    use tipsy_protocol::{ListeningMode, ProfileView, UiLanguage};

    /// The whole of the mapping, both ends: Sunday is the last row of the table and Monday the first,
    /// and a number outside a week cannot arrive because `get_day` returns `0..=6`.
    #[test]
    fn the_browsers_sunday_numbering_becomes_the_huds_monday_one() {
        // JS: Sun 0, Mon 1, Tue 2, Wed 3, Thu 4, Fri 5, Sat 6.
        // HUD: Mon 0, Tue 1, Wed 2, Thu 3, Fri 4, Sat 5, Sun 6.
        assert_eq!(weekday_from_sunday(0), 6, "Sunday is the last entry of `weekdays`");
        assert_eq!(weekday_from_sunday(1), 0, "Monday is the first");
        assert_eq!(weekday_from_sunday(2), 1);
        assert_eq!(weekday_from_sunday(3), 2);
        assert_eq!(weekday_from_sunday(4), 3);
        assert_eq!(weekday_from_sunday(5), 4);
        assert_eq!(weekday_from_sunday(6), 5, "Saturday is the sixth");
    }

    fn welcome() -> Event {
        Event::Welcome {
            session_id: 1,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: UiLanguage::Es,
            listening_mode: ListeningMode::Push,
            web_bundle: true,
            login: None,
        }
    }

    /// The decision the whole file exists for, on the side that has no browser: **the events reach the
    /// HUD before the microphone is open and the audio does not.**
    ///
    /// `audio_now: None` is the state every page starts in — Task 3.3's gate has not been pressed, so
    /// there is no `AudioContext`. The `Welcome` still has to land (the HUD has a profile name and a
    /// language before the user does anything) and the announced rate is still recorded, because it is a
    /// fact about the reply and not about the device; the sentence itself is dropped rather than kept,
    /// and nothing is scheduled.
    #[test]
    fn the_events_reach_the_hud_before_the_microphone_is_open_and_the_audio_does_not() {
        let mut queue = VecDeque::from(vec![
            ToClient::Event(welcome()),
            ToClient::Event(Event::SpeechChunk { seq: 0, sample_rate: 24_000 }),
            ToClient::Audio(Frame::Speech { seq: 0, samples: vec![1i16; 24_000] }),
            // A microphone frame coming back from the server is not a thing, and it must not panic,
            // stop the drain or be mistaken for speech.
            ToClient::Audio(Frame::Mic(vec![0i16; 320])),
        ]);
        let mut playback = playback::Playback::new();
        let mut state = tipsy_ui::UiState::new();

        let (welcomed, play) = route(&mut queue, &mut playback, &mut state, None);

        assert!(welcomed, "the caller has to be told, in the language `apply` just set");
        assert!(state.profile.is_some(), "the `Welcome` reached the HUD with no context in sight");
        assert_eq!(playback.rate(), 24_000, "the announced rate is recorded either way");
        assert!(play.is_empty(), "there is nothing to schedule on");
        assert!(playback.reply().is_empty(), "and the sentence was dropped, not kept for the press");
        assert!(queue.is_empty(), "the queue is drained whatever happens to the audio");
    }

    /// The rate has to be applied **before** the chunk it belongs to, which is the one rule
    /// `policy::speech_rate` exists to hold still. The observable consequence is the cursor: 16 000
    /// samples are a whole second at 16 kHz and two thirds of one at the default 24 kHz.
    #[test]
    fn the_announced_rate_is_set_before_the_chunk_it_owns() {
        let mut playback = playback::Playback::new();
        let mut state = tipsy_ui::UiState::new();

        let mut first = VecDeque::from(vec![
            ToClient::Event(Event::SpeechChunk { seq: 0, sample_rate: 16_000 }),
            ToClient::Audio(Frame::Speech { seq: 0, samples: vec![0i16; 16_000] }),
        ]);
        let (_, play) = route(&mut first, &mut playback, &mut state, Some(0.0));
        assert_eq!(play.len(), 1);
        assert_eq!(play[0].rate, 16_000, "the buffer is created at the announced rate");
        assert_eq!(play[0].at, playback::CUSHION_S);

        let mut second = VecDeque::from(vec![
            ToClient::Event(Event::SpeechChunk { seq: 1, sample_rate: 16_000 }),
            // 8 000 samples = half a second at 16 kHz.
            ToClient::Audio(Frame::Speech { seq: 1, samples: vec![0i16; 8_000] }),
        ]);
        let (_, play) = route(&mut second, &mut playback, &mut state, Some(0.0));
        assert_eq!(
            play[0].at,
            1.0 + playback::CUSHION_S,
            "the cursor advanced a whole second: 24 kHz arithmetic would have moved it by 0.667 s"
        );
    }

    /// The oscilloscope is fed from the reply this client plays, not from an `AnalyserNode`
    /// (`ratzilla-and-wasm-audio.md` §5.5) — the same observation `client.rs:270-271` makes.
    #[test]
    fn the_oscilloscope_follows_the_reply_the_frame_loop_scheduled() {
        let mut queue = VecDeque::from(vec![ToClient::Audio(Frame::Speech { seq: 0, samples: vec![1_000i16; 4_800] })]);
        let mut playback = playback::Playback::new();
        let mut state = tipsy_ui::UiState::new();

        let (_, play) = route(&mut queue, &mut playback, &mut state, Some(5.0));

        assert_eq!(play.len(), 1);
        assert!(state.voice_level() > 0.0, "`start_voice` was handed an envelope with a level in it");
    }

    /// The boundary set, both ways: the three events that end a turn, and the four neighbours that must
    /// not. The behavioural half follows the table below, because a rule that is only named can still be
    /// wired to the wrong events.
    #[test]
    fn only_the_three_turn_boundaries_end_a_turn() {
        for event in [
            Event::Heard { text: "¿y eso?".into() },
            Event::Done { first_audio_ms: Some(1), total_ms: 2 },
            Event::Status { state: AssistantState::Idle, detail: None },
        ] {
            assert!(ends_turn(&event), "{event:?} ends a turn");
        }
        for event in [
            Event::Status { state: AssistantState::Thinking, detail: None },
            Event::Status { state: AssistantState::Speaking, detail: None },
            Event::Reply { text: "Aquí andamos.".into() },
            Event::ReplyDelta { text: "Aquí ".into() },
        ] {
            assert!(!ends_turn(&event), "{event:?} happens in the middle of a turn");
        }
    }

    /// A turn boundary is what lets the next turn's `seq` restart at 0, and the observable consequence
    /// is the gap count rather than whether a repeat is refused: `end_of_turn` forgets the sequence
    /// position, so the *first* `seq` after a boundary is a new turn's own and a jump to `seq` 5 starts
    /// from 0 again. Without a boundary the same jump is four sentences the server never delivered.
    #[test]
    fn a_turn_boundary_is_what_lets_the_next_turns_seq_restart_at_zero() {
        let play = |boundary: Event| {
            let mut queue: VecDeque<ToClient> = VecDeque::new();
            queue.push_back(ToClient::Audio(Frame::Speech { seq: 0, samples: vec![0i16; 2_400] }));
            // The event under test lands *between* the two sentences, which is where the server puts it.
            queue.push_back(ToClient::Event(boundary));
            queue.push_back(ToClient::Audio(Frame::Speech { seq: 5, samples: vec![0i16; 2_400] }));
            let mut playback = playback::Playback::new();
            let mut state = tipsy_ui::UiState::new();
            let (_, scheduled) = route(&mut queue, &mut playback, &mut state, Some(0.0));
            (scheduled.len(), playback.gaps())
        };
        assert_eq!(
            play(Event::Status { state: AssistantState::Idle, detail: None }),
            (2, 0),
            "`Idle` ended the turn, so `seq` 5 is the next turn's own sentence and nothing is missing"
        );
        assert_eq!(
            play(Event::Status { state: AssistantState::Speaking, detail: None }),
            (2, 4),
            "a mid-turn status is not a boundary, so sentences 1..=4 really are missing"
        );
    }

    /// **The frames a real server sent, on a real socket, through this client's own decode and its own
    /// timeline.** `.target-sandbox/live-34.ps1` — a real `serve` with a stub provider behind a real
    /// `ClientWebSocket` — captures one turn's binary frames to `.target-sandbox/live-34/frames.txt`.
    /// This reads that file, decodes every frame with `wire::frame_from_bytes` (the client's decoder, not
    /// a second one written for the check) and places them on `playback::Playback`, asserting the two
    /// things the timeline promises on audio nobody composed here: the first sentence is anchored a
    /// cushion ahead of the clock, and every sentence after it starts exactly where the one before ends.
    ///
    /// **It asks about the file, not about a directory holding it** — the Phase 2 lesson, where
    /// `.is_dir()` was true in a fresh clone because a `README.md` was committed and the skip became dead
    /// code. A checkout that has never run the live check skips this, and the reason is printed (visible
    /// under `cargo test -- --nocapture`). Both sides are recorded in the ledger.
    #[test]
    fn the_frames_a_real_server_sent_decode_and_schedule_on_this_client() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.target-sandbox/live-34/frames.txt");
        let Ok(captured) = std::fs::read_to_string(&path) else {
            println!(
                "skipping `{}`: it is not there. Run `.target-sandbox/live-34.ps1` (a real `serve`, a stub \
                 provider and a real WebSocket) to produce it — the assertions below need frames a real \
                 server sent.",
                path.display()
            );
            return;
        };

        let mut chunks = Vec::new();
        for line in captured.lines().filter(|line| line.starts_with("frame ")) {
            let field = |name: &str| {
                line.split_whitespace()
                    .find_map(|part| part.strip_prefix(name))
                    .unwrap_or_else(|| panic!("`{line}` has no `{name}`"))
            };
            let seq: u32 = field("seq=").parse().expect("seq is a number");
            let rate: u32 = field("rate=").parse().expect("rate is a number");
            let hex = field("hex=");
            assert_eq!(hex.len() % 2, 0, "`{line}` is not whole bytes");
            let bytes: Vec<u8> = (0..hex.len() / 2)
                .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex digits"))
                .collect();
            // The client's own decoder, on the bytes a real server sent.
            let frame = wire::frame_from_bytes(&bytes).expect("the live frame decodes");
            let Frame::Speech { seq: decoded_seq, samples } = frame else {
                panic!("a `0x02` frame decoded to {frame:?}");
            };
            assert_eq!(decoded_seq, seq, "the header's `seq` is the one the check recorded");
            assert_eq!(bytes.len(), 5 + samples.len() * 2, "`Frame::encode`'s own arithmetic");
            chunks.push((rate, seq, samples));
        }
        assert!(!chunks.is_empty(), "the live frames file is there but holds no frame lines");

        // One turn, so `seq` counts its sentences from 0.
        let seqs: Vec<u32> = chunks.iter().map(|(_, seq, _)| *seq).collect();
        assert_eq!(seqs, (0..chunks.len() as u32).collect::<Vec<_>>(), "one turn's own numbering");

        let mut playback = playback::Playback::new();
        let mut starts = Vec::new();
        let mut durations = Vec::new();
        let mut total = 0usize;
        for (rate, seq, samples) in chunks {
            durations.push(samples.len() as f64 / f64::from(rate));
            playback.set_rate(rate);
            let scheduled = playback.chunk(seq, samples, 0.0).expect("every live frame is audio this turn wants");
            assert_eq!(scheduled.rate, rate, "the buffer is created at the rate the server announced");
            starts.push(scheduled.at);
            total += scheduled.samples.len();
        }

        assert_eq!(starts[0], playback::CUSHION_S, "the first sentence is anchored a cushion ahead of the clock");
        let mut expected = playback::CUSHION_S;
        for (i, at) in starts.iter().enumerate() {
            assert!((at - expected).abs() < 1e-9, "sentence {i} starts at {at}, not {expected}: the cursor is gapless");
            expected += durations[i];
        }
        assert_eq!(playback.reply().len(), total, "the reply is every sample the server sent, once");
        assert_eq!(playback.gaps(), 0, "a live turn has no missing sentences");
        assert!(!playback.envelope().is_empty(), "and the oscilloscope has something to draw");
    }
}
