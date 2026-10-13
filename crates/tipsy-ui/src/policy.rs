//! The decisions a tipsy-assistant client makes, with no device, socket or clock in reach.
//!
//! Every function here was a private free function of `tipsy-assistant`'s `client.rs`. It lives in
//! `tipsy-ui` because that crate is the one both clients compile against — the native terminal client
//! and Plan 3's browser client — and the one that must keep compiling to `wasm32-unknown-unknown`.
//! That second fact is also the membership rule: nothing here may reach for a device, a socket, the
//! file system, the operating system's clock or `crossterm`. See `client.rs`'s `key_input` (a crossterm
//! `KeyEvent`) and `clock_view` (this process's clock) for the two functions the rule keeps out.

use tipsy_protocol::{Capability, Command, Event};

use crate::Hit;

/// Whether a command starts the next turn, which is what makes the audio kept for `Replay` stale.
///
/// The session cancels the running turn and starts another one for `Ask`, `Say` and `ListenStop`
/// (`session.rs:283-287`); `Reset` starts the conversation over (`session.rs:272-281`).
pub fn starts_turn(command: &Command) -> bool {
    match command {
        // The session ignores an empty question and a bare `!` (`session.rs:233-238`, `:242-249`),
        // so those begin nothing and must not silence a reply in progress.
        Command::Ask { text } | Command::Say { text } => !text.trim().is_empty(),
        Command::ListenStop | Command::Reset => true,
        // A Test is the one command the native client answers itself, in `test_provider`: it makes a
        // real call to a provider, but it starts no turn, speaks nothing of its own and writes
        // nothing to the log. This arm is therefore load-bearing for the screen as well —
        // `test_interrupts_playback` reads the decision from it — and may not be flipped to `true`
        // without also deciding that a one-word probe should cut a sentence in half.
        Command::Hello { .. }
        | Command::TestProvider { .. }
        | Command::ListenStart
        | Command::Cancel
        | Command::SetProfile { .. }
        | Command::SetListeningMode { .. } => false,
        // The login screen's three commands belong to the login capture, never to the conversation:
        // they start no turn, and the client routes them to the screen rather than through this
        // decision.
        Command::LoginStart | Command::LoginStop | Command::LoginAnswer { .. } => false,
        // Reading, writing and keying the configuration start no turn, speak nothing and write
        // nothing to the log.
        Command::GetConfig | Command::UpdateConfig { .. } | Command::SetKey { .. } => false,
    }
}

/// Whether the queued audio has to go because the turn it belongs to is over.
///
/// Everything that starts a turn leaves nothing of the previous one to hear; `Cancel`
/// (`session.rs:266-270`) is the one command that ends a turn without starting another.
pub fn interrupts_playback(command: &Command) -> bool {
    starts_turn(command) || matches!(command, Command::Cancel)
}

/// Whether the screen's Test quiets the audio the device is still speaking.
///
/// It does not, and the answer comes from the same rule [`starts_turn`] applies to commands: the
/// probe is a diagnostic — it says nothing out loud, plays nothing (the speech probe counts the
/// samples and drops them) and adds nothing to the log — so a reply in progress keeps playing. This
/// function exists so the screen's path, which never passes through `UiAction::Send`, takes that
/// decision deliberately instead of inheriting it from an arm that only ever existed to make a match
/// exhaustive.
pub fn test_interrupts_playback(capability: Capability) -> bool {
    starts_turn(&Command::TestProvider { capability })
}

/// Whether the audio the device has queued must be dropped right now.
///
/// The two inputs are the whole rule: the user is interrupting (`interrupting`) and there is
/// something left to drop (`playing`). Nothing else flushes. A status change in particular is never
/// a trigger: a reply that finishes normally crosses `Speaking` → `Idle` (`brain.rs:110-115`,
/// `:153-158`) while the device is still speaking it out, so flushing there would cut every reply
/// off mid-sentence — and, because rodio's `clear` pauses the sink, leave the rest of the session
/// silent.
pub fn flush_on_interrupt(interrupting: bool, playing: bool) -> bool {
    interrupting && playing
}

/// Whether a command means the user is taking the floor while a call is live.
///
/// In push mode these verbs are the microphone key and the cancel; in a call the microphone is
/// already open for the whole call, so both directions of that key mean the same thing — "stop
/// talking, I am talking" — and they are answered locally with `response.cancel` rather than sent to
/// a session that is not driving the conversation. The server's own VAD is what reports a barge-in
/// and what knows the number.
pub fn takes_the_floor(command: &Command) -> bool {
    matches!(
        command,
        Command::Ask { .. }
            | Command::Say { .. }
            | Command::ListenStart
            | Command::ListenStop
            | Command::Cancel
    )
}

/// Whether the microphone stream should be open: the user asked for it — with the microphone key, or
/// because the login capture said so — or a call is live and streams it for as long as it lasts.
///
/// The login capture is its own reason and never a reason to write the switch: with the door shut
/// there is exactly one microphone that matters, and it is the screen's, not the HUD's (login spec
/// §6.2, review finding 3).
pub fn mic_wanted(mic_open: bool, live: bool, login: bool) -> bool {
    mic_open || live || login
}

/// Where one captured frame goes, or nowhere at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicRoute {
    /// To the session as a `Frame::Mic`: the listening path, and the login's own detector on the
    /// server.
    Session,
    /// To the live call, which owns the microphone while it lasts.
    Call,
    /// Nowhere: no capture is running, so there is nothing to send and nothing to keep.
    Drop,
}

/// The one routing decision the capture loop makes, as a function of what is open.
///
/// `device` is `Mic::is_some()` and it comes first on purpose: a frame belongs to a *stream*, and
/// "the screen wants a capture" is not the same fact (login spec §9: a denied microphone means no
/// stream, no `LoginStart`). The login beats a live call because it is the only thing on screen and
/// its answer is not the call's.
pub fn mic_route(device: bool, login: bool, live: bool) -> MicRoute {
    match (device, login, live) {
        (false, _, _) => MicRoute::Drop,
        (true, true, _) => MicRoute::Session,
        (true, false, true) => MicRoute::Call,
        (true, false, false) => MicRoute::Session,
    }
}

/// The sample rate an event announces, when it announces one.
///
/// **New in this plan, and the only new behaviour in it.** `on_message` assigns the announced rate to
/// the client's rate *before* it hands the audio to the device, and Plan 3's browser client has to do
/// the same thing in the same order. This function is that rule as a value, so neither client can get
/// the order wrong alone.
pub fn speech_rate(event: &Event) -> Option<u32> {
    match event {
        Event::SpeechChunk { sample_rate, .. } => Some(*sample_rate),
        _ => None,
    }
}

/// The hit whose clickable title contains the click, if any.
///
/// The rectangles are half-open: the cell at `area.right()` and the one at `area.bottom()` are
/// outside. The native client's mouse path and Plan 3's browser path both call this.
pub fn hit_at(hits: &[Hit], column: u16, row: u16) -> Option<Hit> {
    hits.iter().copied().find(|hit| {
        column >= hit.area.x && column < hit.area.right() && row >= hit.area.y && row < hit.area.bottom()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use tipsy_protocol::{ClientKind, ListeningMode};

    #[test]
    fn a_speech_chunk_is_the_only_event_that_announces_a_rate() {
        assert_eq!(speech_rate(&Event::SpeechChunk { seq: 0, sample_rate: 24_000 }), Some(24_000));
        assert_eq!(speech_rate(&Event::Done { first_audio_ms: Some(1), total_ms: 2 }), None);
        assert_eq!(speech_rate(&Event::Status { state: tipsy_protocol::AssistantState::Idle, detail: None }), None);
    }

    #[test]
    fn only_a_shown_hit_contains_a_click_and_its_edges_are_half_open() {
        use ratatui::layout::Rect;
        let hits = [Hit { area: Rect::new(2, 3, 4, 2), key: crate::KeyInput::F(9), action: None }];
        assert!(hit_at(&hits, 2, 3).is_some(), "the top-left cell is inside");
        assert!(hit_at(&hits, 5, 4).is_some(), "the bottom-right cell is inside");
        assert!(hit_at(&hits, 6, 4).is_none(), "the cell at right() is outside");
        assert!(hit_at(&hits, 5, 5).is_none(), "the cell at bottom() is outside");
        assert!(hit_at(&hits, 1, 3).is_none());
    }

    #[test]
    fn the_capture_needs_a_device_before_anything_else_can_route_it() {
        assert_eq!(mic_route(false, true, true), MicRoute::Drop);
        assert_eq!(mic_route(true, true, true), MicRoute::Session, "the login beats the call");
        assert_eq!(mic_route(true, false, true), MicRoute::Call);
        assert_eq!(mic_route(true, false, false), MicRoute::Session);
    }

    #[test]
    fn a_probe_never_interrupts_and_a_cancel_always_does() {
        assert!(!test_interrupts_playback(Capability::Chat));
        assert!(interrupts_playback(&Command::Cancel));
        assert!(!interrupts_playback(&Command::ListenStart));
        assert!(interrupts_playback(&Command::ListenStop));
    }

    /// The routing decision itself, as a table: the login capture goes to the session even while a
    /// call is live, a call keeps its own frames otherwise, and **nothing goes anywhere without a
    /// device**. `Mic` wraps a `cpal::Stream`, so a test cannot build one; this is the half of
    /// `drain_mic` a test can pin, and it is the half the defect was in.
    #[test]
    fn the_login_capture_is_routed_to_the_session_and_needs_a_device() {
        use MicRoute::{Call, Drop, Session};
        // (device open, login capture, a call is live) → where the frame goes
        for (open, login, live, want) in [
            (false, true, false, Drop),
            (false, true, true, Drop),
            (false, false, false, Drop),
            (false, false, true, Drop),
            (true, true, true, Session),
            (true, true, false, Session),
            (true, false, true, Call),
            (true, false, false, Session),
        ] {
            assert_eq!(mic_route(open, login, live), want, "device={open} login={login} live={live}");
        }
    }

    #[test]
    fn only_an_interrupt_with_audio_left_to_drop_flushes() {
        // Barge-in, or the cancel the user asked for, while the reply is being spoken: the rest of
        // the queued audio goes.
        assert!(flush_on_interrupt(true, true));
        // An interrupt with nothing queued has nothing to drop — a cancel that arrives while the
        // session is still thinking, with no sentence synthesised yet.
        assert!(!flush_on_interrupt(true, false));
        // The status change that ends a normal reply. `Done` and `Idle` arrive while the device is
        // still speaking, so this row is what keeps every completed reply whole.
        assert!(!flush_on_interrupt(false, true));
        assert!(!flush_on_interrupt(false, false));
    }

    #[test]
    fn the_commands_that_end_a_turn_are_the_ones_that_interrupt_it() {
        // Barge-in: the session cancels the running turn and starts another one in its place.
        for command in [
            Command::Ask { text: "¿qué pasa?".into() },
            Command::Say { text: "hola compadre".into() },
            Command::ListenStop,
            Command::Reset,
        ] {
            assert!(interrupts_playback(&command), "{command:?} ends the running turn");
        }
        // `Cancel` ends a turn without starting one, and is the other half of the rule.
        assert!(interrupts_playback(&Command::Cancel));
        // Nothing else quiets the device: opening the microphone leaves the reply audible until the
        // utterance that follows it, and the session ignores the empty question a bare `!` sends.
        for command in [
            Command::ListenStart,
            Command::Ask { text: String::new() },
            Command::Say { text: "   ".into() },
            Command::SetProfile { id: "compae".into() },
        ] {
            assert!(!interrupts_playback(&command), "{command:?} leaves the reply alone");
        }
    }

    /// The three reasons a stream may be open, and the two ways it is closed. The login capture is its
    /// own reason (review finding 3): it needs no help from the user's switch, and it leaves it alone.
    #[test]
    fn the_stream_opens_for_the_switch_the_call_or_the_login_capture() {
        assert!(mic_wanted(true, false, false), "the microphone key still opens it");
        assert!(mic_wanted(false, true, false), "a call streams the microphone for as long as it lasts");
        assert!(mic_wanted(false, false, true), "and the login capture opens it with the screen");
        assert!(mic_wanted(true, true, true));
        assert!(!mic_wanted(false, false, false), "with none of the three there is nothing to open");
        // The switch is not consulted for the login's sake and never written by the screen: the value
        // it held is the value it keeps, whichever of the three opened the stream.
        for switch in [false, true] {
            assert_eq!(mic_wanted(switch, false, false), switch, "the switch alone is the switch");
        }
    }

    #[test]
    fn the_verbs_that_take_the_floor_are_the_ones_a_call_answers_locally() {
        for command in [
            Command::Ask { text: "¿qué pasa?".into() },
            Command::Say { text: "hola compadre".into() },
            Command::ListenStart,
            Command::ListenStop,
            Command::Cancel,
        ] {
            assert!(takes_the_floor(&command), "{command:?} means the user is taking the floor");
        }
        for command in [
            Command::Reset,
            Command::SetProfile { id: "meigo".into() },
            Command::SetListeningMode { mode: ListeningMode::Push },
            Command::Hello { token: None, client: ClientKind::Tui, protocol_version: 1 },
        ] {
            assert!(!takes_the_floor(&command), "{command:?} is not an interruption");
        }
    }

    /// The interruption decision Task 7 had to take, named and proved rather than inherited: a Test
    /// makes a real call, but it is a diagnostic — it speaks nothing of its own, writes nothing to
    /// the log and starts no turn — so it never quiets the reply being spoken. The answer comes from
    /// `starts_turn`, the one helper that decides whether a command interrupts, so flipping that arm
    /// flips this too.
    #[test]
    fn a_test_probe_is_not_a_turn_and_leaves_the_playing_audio_alone() {
        for capability in [Capability::Chat, Capability::Speech, Capability::Transcribe] {
            assert!(
                !test_interrupts_playback(capability),
                "{capability} is a diagnostic, not a turn"
            );
            assert!(!interrupts_playback(&Command::TestProvider { capability }));
        }
        // The whole rule at once: audio is queued and the user asked for a Test, and the audio stays
        // where it is. Had this been true, a one-word probe would cut a sentence in half.
        assert!(!flush_on_interrupt(test_interrupts_playback(Capability::Speech), true));
    }
}
