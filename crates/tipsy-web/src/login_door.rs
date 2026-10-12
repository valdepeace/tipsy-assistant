//! The login door, from the browser's side: the first-show call into the shared state, the capture's
//! reconciliation and the one predicate that decides whether a frame may leave the page.
//!
//! The screen itself is `tipsy-ui`'s and is *not* reimplemented here: `UiState::login`,
//! `LoginState`'s phases, the key table that raises `LoginStart` / `LoginStop` / `LoginAnswer` and
//! `render_login.rs`'s veil all arrive from `master` and are driven by `UiState::apply` and
//! `UiState::on_key`. What this module owns is the engine around them, which is per client:
//!
//! 1. **`Welcome.login` is the only authority, once per run — and `UiState` is where that rule
//!    lives.** `UiState::apply`'s `Welcome` arm deliberately ignores the `login` field
//!    (`crates/tipsy-ui/src/state.rs:470-509`: the pattern names five fields and `..` covers
//!    `session_id` and `login`), so a mere `Welcome` never puts a screen up; only `show_login` does —
//!    and `show_login` itself refuses once the run has been asked (`:293-314`, `login_seen` at
//!    `:212-218`). The server re-sends `Welcome` on `SetProfile` and `SetListeningMode`
//!    (`Hub::resend_welcome`), and the native client's arm is nothing but that return value. This
//!    module carries **no second copy of the bit**: a reconnect, a re-sent announcement and a native
//!    client all ask the same state, so a browser-side flag would be a second home for a rule that
//!    already has one; and `state.login.is_none()`, the guard this replaced, cannot answer it at all,
//!    because `LoginDone` puts `login` back to `None` (`UiState::login_key`'s passed branch,
//!    `:394`), which is exactly what it holds before the first screen is ever shown. That is what
//!    commit `b9f2402` removed, and this module does not bring it back.
//! 2. **The microphone follows `LoginState::open`, and nothing else.** The native client reads
//!    `login.voice && login.open` once a frame and sends `LoginStart` / `LoginStop` **on the edges**
//!    (`crates/tipsy-compadre/src/client/mic.rs`). The browser's `Tab`, its third refusal and a pass
//!    move exactly the same field (`crates/tipsy-ui/src/state.rs:414-424`, `:615-617`, `:607`), so the
//!    same edge detector applies.
//! 3. **A frame leaves the page only while that capture is open.** This is the login plan's review
//!    finding 1, on the other client: the native rule is "the capture first, then the device, then the
//!    frames" and "a frame belongs to a *stream*" (`tipsy_ui::policy::mic_route`). Here the device is
//!    Task 3.3's gate and the stream is this predicate. It reads the state's `voice && open` and keeps
//!    no copy of either: a pass closes the capture in the reducer (`state.rs:607`), so the gate shuts
//!    with the door and the reconciliation sends the `LoginStop` spec §6.2 asks for. The browser does
//!    not decide that a capture is over; the state says so, and this module is only the reader.
//!
//! **The window this closes, measured on the tree it was written against.** Until this module had a
//! caller, `mic::drain_capture`'s whole guard was `state.mic_open`, and Task 3.3's "press to start"
//! click sets that flag the moment the device opens (`mic.rs`'s `press_to_start`) — *before* `Welcome`
//! arrives and whatever `Welcome.login.voice` turns out to be. So a page pressed its gate and uploaded
//! room tone to `/ws` from that press until something turned the flag off, including for the whole of a
//! `voice = false` screen where spec §9 says no audio leaves the machine at all. It never reached a
//! provider (the session drops frames with no `LoginStart`), but the wire carried them. The predicate
//! below is what stopped it, and `the_press_that_opened_the_hud_microphone_does_not_open_the_door`
//! pins the case.
//!
//! **One consequence, named rather than implied.** `sends_frames` is false whenever no screen is up,
//! so on `/` no microphone frame leaves the page after the door either: the HUD's own push-to-talk is
//! the native client's in this tier. That is the plan's Step 3 note 2, and **it is the plan's decision
//! and not the spec's** — the plan cites spec §8.2 for it, and §8.2 is its table of the three jobs a
//! browser does (capture, playback, transport); it says nothing about push-to-talk, and nothing else in
//! the web-tier spec does either. A `Tab` on the HUD still raises `ListenStart` and still moves
//! `state.mic_open` — those are `UiState`'s and untouched — and no frame follows them. It is the price
//! of the guard being the door's and nothing else.
//!
//! Everything in this file is pure and runs on the host target (Task 3.2 Step 1): no `web_sys` value
//! is constructed, no socket is touched, and no test needs a browser.

use tipsy_protocol::{Event, LoginView};
use tipsy_ui::{LoginPhase, UiState};

/// The browser's read of one screen. `UiState` stays the state machine, and it holds the answer to
/// every question this type asks: what is left here is the request in flight, and nothing else.
#[derive(Debug, Default)]
pub struct Door {
    /// Whether this run has told the server a capture is open. This is the native client's
    /// `login_mic`: **the request in flight, not the capture**, so a `LoginStart` that went out is
    /// always answered by a `LoginStop` — and a screen that opens without a voice switch never sends a
    /// `LoginStart` at all.
    started: bool,
}

/// What the client still owes the server about the capture, at one frame's edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicStep {
    /// Send `Command::LoginStart`.
    Start,
    /// Send `Command::LoginStop`.
    Stop,
    /// Nothing changed.
    None,
}

impl Door {
    pub fn new() -> Self {
        Self::default()
    }

    /// The whole of `Welcome`'s login half: the screen goes up on the **first** `Welcome` that
    /// carries a `LoginView`, and on none after it. `UiState::show_login` is both the call and the
    /// rule — it returns whether *this* call was the one that showed the screen — so there is no
    /// guard to write here and no bit to keep (`crates/tipsy-ui/src/state.rs:293-314`, and the native
    /// arm that does the same thing is `crates/tipsy-compadre/src/client.rs`'s `Welcome` branch).
    ///
    /// Returns whether a screen went up, which is `show_login`'s own answer and is not re-derived.
    pub fn open_screen(&mut self, state: &mut UiState, login: Option<&LoginView>) -> bool {
        let Some(view) = login else {
            // `Welcome.login == None`: there is no screen, now or later (`Welcome` is the only
            // authority, spec §3).
            return false;
        };
        // `show_login` is the whole of the once-per-run rule: it returns `false` for a second view
        // while the screen is up — the half-typed answer, the phase and the refusal streak stay put —
        // and `false` again after the door was passed, which is the case `login.is_none()` cannot see
        // (`state.rs:293-314`).
        if state.show_login(view.voice)
            && let Some(screen) = state.login.as_mut()
        {
            // The voice switch and the phase are the *client's* first-show setup, which is why
            // `show_login` returns whether it was the call that showed the screen rather than leaving
            // the caller to ask a second time. With the voice on, the capture opens **with** the
            // screen: the reconciliation below sees it on the very first frame and sends the
            // `LoginStart` then. The phase follows it, because the screen's own key table reads the
            // phase to decide what `Tab` and `Enter` mean (`state.rs:414`, `:401-407`).
            screen.open = view.voice;
            screen.phase = if view.voice { LoginPhase::Listening } else { LoginPhase::Waiting };
            return true;
        }
        false
    }

    /// `UiAction::LoginDone`: the door was passed. This is the **only** place the shared state may
    /// lose its screen, and it is enough on its own — the run-scoped bit was set when `show_login`
    /// first showed the screen (`crates/tipsy-ui/src/state.rs:303`), so there is nothing to remember
    /// here. The capture is already down by now: a pass closed it in the reducer (`:607`), which is
    /// why the frame loop's reconciliation sends the `LoginStop` spec §6.2 requires.
    pub fn admitted(&mut self, state: &mut UiState) {
        state.login = None;
    }

    /// **The predicate that keeps audio inside the machine.** True exactly while the screen is up, its
    /// voice switch is on and its capture is open — the native client's `login.voice && login.open` —
    /// and false at every other moment, including a screen that is up but shut (`Tab` off, a pass, the
    /// third refusal, a failed device), the whole of the time before `Welcome` arrives, and the whole
    /// of the time after the door.
    pub fn sends_frames(&self, state: &UiState) -> bool {
        state.login.as_ref().is_some_and(|screen| screen.voice && screen.open)
    }

    /// The screen's capture state, whether or not it may send: the `● rec` badge and `Tab` read the
    /// same field (`crates/tipsy-ui/src/render_login.rs:212`, spec §6).
    pub fn capture_open(&self, state: &UiState) -> bool {
        state.login.as_ref().is_some_and(|screen| screen.voice && screen.open)
    }

    /// Reconciles `Command::LoginStart` / `Command::LoginStop` with the screen's capture state, **on
    /// the edges**: the native client's own reconciliation, which exists because `UiState::apply`
    /// cannot raise an action — a refusal that stops listening is expressed as `open == false` and
    /// nothing else — so the client reads the state once a frame and tells the server what it is
    /// doing.
    ///
    /// The caller maps the answer onto the **same** path every other HUD command takes
    /// (`ToServer::Command` into Task 3.2's `Outbox`), which is why this returns a protocol-shaped
    /// step rather than carrying a socket.
    pub fn sync(&mut self, state: &UiState) -> MicStep {
        let wanted = self.capture_open(state);
        match (self.started, wanted) {
            (false, true) => {
                self.started = true;
                MicStep::Start
            }
            (true, false) => {
                self.started = false;
                MicStep::Stop
            }
            _ => MicStep::None,
        }
    }

    /// One message from the socket, as far as the door is concerned. Only `Welcome` reaches it; a
    /// `LoginHeard` or a `LoginVerdict` belongs to `UiState::apply`
    /// (`crates/tipsy-ui/src/state.rs:590-629`) and an `Error` to its own arm (`:636-655`), so this
    /// never second-guesses a phase. The caller applies the event **first** and offers it here second,
    /// which is the order the frame loop uses.
    pub fn note_event(&mut self, state: &mut UiState, event: &Event) -> bool {
        match event {
            Event::Welcome { login, .. } => self.open_screen(state, login.as_ref()),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use tipsy_protocol::{
        ClientKind, Command, ListeningMode, ProfileView, UiLanguage, PROTOCOL_VERSION,
    };
    use tipsy_ui::KeyInput;

    /// The `Welcome` the browser acts on.
    ///
    /// **The plan's `login_view` helper does not compile and this is the correction.** It builds
    /// `LoginView { enabled, voice, allowed }` — three fields, with the allowlist among them — but
    /// `LoginView` is `Welcome`'s login-*screen* payload and carries exactly one field, `voice`
    /// (`crates/tipsy-protocol/src/lib.rs:116-120`). The three-field type exists and is a **different
    /// one**: `LoginSettingsView { enabled, voice, allowed }` (`:179-183`), the `[login]` section
    /// `ConfigView` carries. `allowed` is never read here, and that is the point: the door is `Some`
    /// or `None` and the allowlist never reaches a client that does not fetch the configuration
    /// (spec §4.3, the L2 decision).
    fn login_view(voice: bool) -> LoginView {
        LoginView { voice }
    }

    fn welcome(login: Option<LoginView>) -> Event {
        Event::Welcome {
            session_id: 7,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: UiLanguage::Es,
            listening_mode: ListeningMode::Push,
            web_bundle: true,
            login,
        }
    }

    /// The `Hello` Task 3.2's socket sends first, as the JSON the `Outbox` puts on the wire. The
    /// login's three commands go down that same path, so the test asserts on the text and not on a
    /// private side-channel.
    fn hello_json() -> String {
        serde_json::to_string(&Command::Hello {
            token: None,
            client: ClientKind::Web,
            protocol_version: PROTOCOL_VERSION,
        })
        .expect("a command serialises")
    }

    /// A 320-sample frame, the shape Task 3.3's `FrameAssembler` emits.
    fn mic_frame() -> Vec<i16> {
        vec![1_000i16; 320]
    }

    /// The verdict that opens the door. A pass closes the capture in the reducer
    /// (`crates/tipsy-ui/src/state.rs:607`), so this is the event that shuts the browser's frame gate
    /// without `Door` remembering anything.
    fn pass() -> Event {
        Event::LoginVerdict { ok: true, name: Some("Andrés".into()) }
    }

    #[test]
    fn a_welcome_with_a_login_view_puts_the_screen_up_and_starts_the_capture() {
        let mut state = UiState::new();
        let mut door = Door::new();

        assert!(door.open_screen(&mut state, Some(&login_view(true))), "the first Welcome opens the screen");
        let screen = state.login.as_ref().expect("the screen is up");
        assert!(screen.voice, "the voice switch came from the server");
        assert!(screen.open, "and the capture opens with the screen, without a key");
        assert_eq!(screen.phase, LoginPhase::Listening, "which is what the phase says");
        assert!(door.sends_frames(&state), "so the frames may leave");

        assert_eq!(door.sync(&state), MicStep::Start, "and the client tells the server once");
        assert_eq!(door.sync(&state), MicStep::None, "and never twice");
    }

    /// **What the browser still owns: asking the state, once, and never asking on its own.** The rule
    /// itself is tested where it lives — `state::tests::the_screen_is_shown_once_per_run`
    /// (`crates/tipsy-ui/src/state.rs:1439-1450`) is the reference, and this does not restate it; it
    /// proves only that `tipsy-web`'s wiring is one `show_login` behind `Welcome` and no bit of its
    /// own. A re-sent `Welcome` — the one `SetProfile` and `SetListeningMode` make the session send —
    /// must leave the half-typed answer, the phase and the refusal streak exactly where the user left
    /// them, and an announcement after the door opened must never put the veil back.
    #[test]
    fn a_second_welcome_never_re_enters_the_login_screen() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(true)));

        // While the screen is up: the same view again moves nothing, and the type-into-'a' is the
        // work that would be lost if this re-entered.
        state.on_key(KeyInput::Char('a'));
        assert!(
            !door.note_event(&mut state, &welcome(Some(login_view(true)))),
            "a second Welcome does not re-enter the screen"
        );
        let screen = state.login.as_ref().expect("the screen is still up");
        assert_eq!(screen.input, "a", "and the half-typed answer is still there");
        assert_eq!(screen.phase, LoginPhase::Listening);
        assert_eq!(screen.refusals, 0);

        // The door opens and the user presses `Enter`: `UiAction::LoginDone`, and the screen goes.
        state.apply(&pass());
        assert_eq!(state.on_key(KeyInput::Enter), vec![tipsy_ui::UiAction::LoginDone]);
        door.admitted(&mut state);
        assert!(state.login.is_none(), "the screen is down");
        assert!(state.login_seen, "and the run remembers that it was asked");

        assert!(
            !door.note_event(&mut state, &welcome(Some(login_view(true)))),
            "the announcement after the door never puts the screen back up"
        );
        assert!(state.login.is_none(), "the HUD stays the HUD");
    }

    /// **A pass closes the capture, and the gate follows the state rather than a flag of its own.** A
    /// pass ends `LoginState::open` in the reducer (`crates/tipsy-ui/src/state.rs:607`), and that is
    /// what turns spec §6.2's `LoginStop` — and the end of `Frame::Mic` — into a consequence of the
    /// state instead of a second copy of "is the capture open" kept beside it.
    /// `state::tests::a_pass_closes_the_capture_it_found_open` (`state.rs:1457-1467`) is the reference
    /// for the state half; this is the browser's half: the predicate and the edge the frame loop reads.
    #[test]
    fn a_pass_takes_the_frames_away_because_the_state_says_so() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(true)));
        assert_eq!(door.sync(&state), MicStep::Start, "the capture opens with the screen");

        state.apply(&pass());

        assert_eq!(state.login.as_ref().unwrap().phase, LoginPhase::Passed);
        assert!(
            !state.login.as_ref().unwrap().open,
            "the state closed the capture the moment the door opened"
        );
        assert!(!door.sends_frames(&state), "so no frame may leave the page any more");
        assert!(!door.capture_open(&state), "and there is no capture to draw");
        assert_eq!(door.sync(&state), MicStep::Stop, "which is the LoginStop spec §6.2 asks for");
        assert_eq!(door.sync(&state), MicStep::None, "once, on the edge");

        // The screen stays up until `Enter`: the door is open, the capture is not.
        assert_eq!(state.on_key(KeyInput::Enter), vec![tipsy_ui::UiAction::LoginDone]);
        door.admitted(&mut state);
        assert!(state.login.is_none());
        assert_eq!(door.sync(&state), MicStep::None, "the `LoginStop` went out at the pass");
    }

    /// Spec §9: a denied or unavailable microphone means no `LoginStart` and no stream, and the screen
    /// stays typing-only. The `voice = false` case is the same path without the failure: the switch is
    /// an explicit choice, and typing always works (spec §6.2).
    #[test]
    fn no_capture_no_login_start_and_no_frame_leaves() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(false)));

        let screen = state.login.as_ref().unwrap();
        assert!(!screen.voice);
        assert!(!screen.open, "no stream is opened at all");
        assert_eq!(screen.phase, LoginPhase::Waiting, "so the phase waits for a key");
        assert!(!door.sends_frames(&state), "and no frame may leave the page");
        assert_eq!(door.sync(&state), MicStep::None, "a typing-only client sends no `LoginStart`");
    }

    /// **The window this task closes, pinned.** The "press to start" gate sets `UiState::mic_open` the
    /// moment the device opens — before `Welcome` arrives — and it stays set while the login screen is
    /// up. Every state below has that flag on and **not one of them may send a frame**: there is no
    /// screen yet, the screen is typing-only, and the HUD is in front after the door. This is the test
    /// that would have been red against Task 3.3's `state.mic_open` guard, and it is the whole reason
    /// the guard is the door's.
    #[test]
    fn the_press_that_opened_the_hud_microphone_does_not_open_the_door() {
        let mut state = UiState::new();
        let mut door = Door::new();
        // The press: the capture is open and the HUD's own switch says so.
        state.mic_open = true;

        assert!(!door.sends_frames(&state), "the press alone is not a door: no `Welcome` has arrived");
        assert_eq!(door.sync(&state), MicStep::None, "and nothing is said about a capture nobody has");

        door.open_screen(&mut state, Some(&login_view(false)));
        assert!(state.mic_open, "the HUD's switch is still on");
        assert!(!door.sends_frames(&state), "but a typing-only screen opens no stream at all");

        // The door opens and the screen goes: the flag the press set is still on, and it is not the
        // door's answer.
        state.login = None;
        assert!(!door.sends_frames(&state), "after the door the frame path is the HUD's, not the door's");
    }

    /// **The capture's edges, including the failed device.** `Tab` and the third refusal move
    /// `LoginState::open` (`crates/tipsy-ui/src/state.rs:414-424`, `:615-617`), and a microphone that
    /// will not open clears it too (Task 3.3's gate). Every one of those is a `LoginStop` here, and
    /// none of them is a frame. The pass's own edge is the test above this one.
    #[test]
    fn the_capture_opens_and_closes_with_the_state_and_only_an_open_capture_sends() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(true)));
        assert_eq!(door.sync(&state), MicStep::Start);

        // `Tab`: capture on → off → on, the transitions of §6.2.
        assert_eq!(state.on_key(KeyInput::Tab), vec!(tipsy_ui::UiAction::Send(Command::LoginStop)));
        assert!(!door.sends_frames(&state), "a shut capture sends nothing");
        assert_eq!(door.sync(&state), MicStep::Stop);
        assert_eq!(state.on_key(KeyInput::Tab), vec!(tipsy_ui::UiAction::Send(Command::LoginStart)));
        assert_eq!(door.sync(&state), MicStep::Start);

        // The device that will not open: the client's own error path lowers the capture, and the
        // frame's reconciliation must follow it without a second `LoginStart`.
        state.login.as_mut().unwrap().open = false;
        assert!(!door.sends_frames(&state), "a failed device is not a capture");
        assert_eq!(door.sync(&state), MicStep::Stop, "and the server is told the capture is over");
        assert_eq!(door.sync(&state), MicStep::None);
    }

    /// The door's own wiring against the shared reducer: a typed answer leaves as
    /// `LoginAnswer { text }` — the text trimmed, the box cleared, the phase `Judging` — which is the
    /// **same** `UiAction::Send` every other HUD command uses (spec §6.2). No second path exists.
    #[test]
    fn the_typed_answer_is_the_state_machines_own_action() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(true)));

        for c in " andres  ".chars() {
            state.on_key(KeyInput::Char(c));
        }
        assert_eq!(
            state.on_key(KeyInput::Enter),
            vec!(tipsy_ui::UiAction::Send(Command::LoginAnswer { text: "andres".into() }))
        );
        let screen = state.login.as_ref().unwrap();
        assert_eq!(screen.input, "", "the box is cleared");
        assert_eq!(screen.phase, LoginPhase::Judging, "and the answer is with the server");
        // The capture stays open while a verdict is in flight: the next thing said is the next try
        // with no key at all, which is §6.2's auto re-arm.
        assert!(door.sends_frames(&state));
    }

    /// Nothing but `Welcome` reaches the door at all: the verdicts and the utterances are the shared
    /// reducer's, and this must not "help" with them.
    ///
    /// **The plan's version of this test does not pass and this is the correction.** It calls
    /// `door.note_event` alone and then asserts the screen's line and phase moved — but `note_event`
    /// declines every event that is not a `Welcome`, so nothing would have moved the state. The
    /// client's order is the reducer's turn first and the door's second, which is exactly what is
    /// asserted here: `apply` is what changes the screen, and the door's answer is `false`.
    #[test]
    fn a_verdict_is_the_shared_states_business_not_the_doors() {
        let mut state = UiState::new();
        let mut door = Door::new();
        door.open_screen(&mut state, Some(&login_view(true)));

        let heard = Event::LoginHeard { text: "soy andres".into() };
        state.apply(&heard);
        assert!(!door.note_event(&mut state, &heard), "the door has no opinion about an utterance");
        assert_eq!(
            state.login.as_ref().unwrap().line,
            tipsy_ui::LoginLine::Heard("soy andres".into())
        );

        let refused = Event::LoginVerdict { ok: false, name: None };
        state.apply(&refused);
        assert!(!door.note_event(&mut state, &refused), "nor about a verdict");
        assert_eq!(state.login.as_ref().unwrap().phase, LoginPhase::Refused);
        assert!(door.sends_frames(&state), "attempt 1 leaves the capture open, as the state says");
    }

    /// **The predicate is one call away from the socket, and a frame it refuses is dropped.**
    /// `mic::drain_frames` is Task 3.3's pure half and takes the guard as a boolean; `mic::drain_capture`
    /// is the one place a captured frame may reach `Outbox::send`, and the boolean it passes is
    /// `Door::sends_frames` and nothing else. `drain_capture` itself cannot run on the host — it takes
    /// a real `AudioContext`, a real `MediaStream` and a `Closure` — which is why Task 3.3 split the
    /// decision out, and why this is the join a host can prove.
    #[test]
    fn the_frame_path_drops_what_the_shut_door_refuses() {
        // The door is shut: a screen that does not want a microphone, with frames already queued.
        // A fresh pair, because `show_login` is once per run — a second `open_screen` on the same
        // state would be refused and would prove nothing about this predicate.
        let (mut state, mut door) = (UiState::new(), Door::new());
        door.open_screen(&mut state, Some(&login_view(false)));
        let mut queue = VecDeque::from(vec![mic_frame(), mic_frame()]);
        let refused = crate::mic::drain_frames(&mut queue, door.sends_frames(&state));
        assert!(refused.send.is_empty(), "not one frame may leave the page");
        assert!(queue.is_empty(), "and the capture's queue is emptied, not kept: the drop is the point");
        assert_eq!(refused.levels.len(), 2, "the oscilloscope is fed either way, as the native one is");

        // The same frames behind an open capture leave, whole and in the order they arrived.
        let (mut state, mut door) = (UiState::new(), Door::new());
        door.open_screen(&mut state, Some(&login_view(true)));
        let mut queue = VecDeque::from(vec![mic_frame(), mic_frame()]);
        let sent = crate::mic::drain_frames(&mut queue, door.sends_frames(&state));
        assert_eq!(sent.send.len(), 2, "an open capture sends what it collected");
        assert_eq!(sent.send[0], mic_frame(), "and sends the frame itself, not a copy of the state");
        assert!(queue.is_empty());
    }

    /// **The predicate is at the one send site, and this is the only test that can say so.**
    ///
    /// `mic::drain_capture` is the single place a captured frame may reach `Outbox::send`, and it cannot
    /// run on the host: it takes a real `AudioContext`, a real `MediaStream` and a `Closure`, which is
    /// exactly why Task 3.3 split the *decision* out into the pure `drain_frames`. What is left in it is
    /// one line, and a mutation of that line breaks no other test in this crate — measured, not
    /// assumed: putting the pre-Task-3.7 guard `state.mic_open` back into it leaves every other test
    /// green, because no host test can build the `Capture` the call needs. So the call site is pinned
    /// the way this phase pins the page's `import init from …` and `mic.rs` pins `WORKLET_URL` against
    /// the file on disk: by reading the committed source and asking about the text. It is blunt, and it
    /// is the only gate there is on the line this whole task is about.
    #[test]
    fn the_one_send_site_guards_on_this_predicate_and_not_on_the_huds_switch() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/mic.rs");
        let source =
            std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let start = source.find("pub fn drain_capture(").expect("`drain_capture` is the one send site");
        // Its body: from its signature to the first `}` in column 0, which is where it ends.
        let rest = &source[start..];
        let end = rest.find("\n}").expect("the function's closing brace");
        let body = &rest[..end];
        assert!(
            body.contains("door.sends_frames(state)"),
            "the frame gate must be the door's own predicate, read from the state:\n{body}"
        );
        assert!(
            !body.contains("state.mic_open"),
            "and it must not be the HUD's microphone switch, which the press-to-start click sets \
             before `Welcome` arrives — that is the window this task closes:\n{body}"
        );
        assert_eq!(
            source.matches("outbox.send(").count(),
            1,
            "there is exactly one place a captured frame may reach the socket, and it is this one"
        );
    }

    /// The frame the predicate has to keep inside: Task 3.2's `Hello` and the login's own three
    /// commands are the same `ToServer::Command` path, and a frame is only ever
    /// `ToServer::Audio(Frame::Mic(..))` behind the predicate (Task 3.3's `drain_capture` takes the
    /// predicate and never calls `Outbox::send` without it).
    #[test]
    fn the_commands_that_leave_are_the_protocols_own_and_the_hello_is_first() {
        assert_eq!(
            hello_json(),
            r#"{"type":"hello","token":null,"client":"web","protocol_version":1}"#,
            "Task 3.2's socket sends this before anything else"
        );

        // The three login commands, as the wire spells them (login spec §4).
        assert_eq!(
            serde_json::to_string(&Command::LoginStart).unwrap(),
            r#"{"type":"login_start"}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::LoginStop).unwrap(),
            r#"{"type":"login_stop"}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::LoginAnswer { text: "andres".into() }).unwrap(),
            r#"{"type":"login_answer","text":"andres"}"#
        );

        // …and a 320-sample microphone frame is the protocol's own `0x01` binary frame, which is what
        // the predicate guards (`crates/tipsy-protocol/src/lib.rs:480-487`).
        let bytes = tipsy_protocol::Frame::Mic(mic_frame()).encode();
        assert_eq!(bytes[0], 0x01, "the frame kind byte");
        assert_eq!(bytes.len(), 1 + 640, "one kind byte and 320 little-endian samples");
    }
}
