//! The HUD's host-independent half: the clock the client injects, the keys it translates and the
//! clicks it dispatches.
//!
//! `tipsy_ui::draw` is called verbatim on both clients (`render.rs:72`), and the translation from a
//! terminal-shaped event to `KeyInput`/`UiAction` is what has to exist twice — once over crossterm's
//! types (`crates/tipsy-compadre/src/client.rs:539-563`) and once over Ratzilla's. This module is the
//! second one's decisions.
//!
//! **`key_input` and the clock's numbering are per shell on purpose, and that is not duplication to
//! remove.** The client-modularity refactor moved the client's decision policy into `tipsy_ui::policy`
//! and left exactly these two behind, because `tipsy-ui` may never gain `crossterm` (`key_input` takes a
//! crossterm `KeyEvent` on the native side) and because there are two clocks: the native `clock_view`
//! reads `chrono::Local` (`client.rs:566-576`) and `lib.rs`'s reads `js_sys::Date`. [`clock_from`] is
//! this shell's only clock *arithmetic* — six numbers in, a `ClockView` out — which is what makes the
//! one off-by-one (`Date.getDay()` counts from Sunday, the HUD from Monday) host-testable. `hit_at` was
//! the third candidate and it did move, which is why the line below imports it. A later reader who
//! "finishes the job" by folding these two into `tipsy-ui` breaks the wasm build and merges two
//! different clocks.
//!
//! **The Ratzilla-facing half is here too, and that is this task's one deviation from the plan, made
//! for the reason this whole task is shaped the way it is.** The plan put the adapter from Ratzilla's
//! `KeyEvent`/`MouseEvent` into `lib.rs` as "a dumb adapter" nothing could test, because it believed
//! the Ratzilla types were `#[cfg]`-gated to wasm and could never reach a test. Task 3.4 measured that
//! the `#[cfg]` is not executable here at all — the crate's wasm half compiles for the host — and
//! `ratzilla::event::KeyEvent` is a plain struct of plain fields. So the adapter is a pure function
//! over plain values with a host test under it ([`raw_key`], [`mouse_input`]) and the two closures in
//! `lib.rs` are one line each. There is no browser on this machine; this is the only side of that
//! translation a test can reach.

use tipsy_protocol::Capability;
use tipsy_ui::{ClockView, Hit, KeyInput, UiAction, UiState};

// The click test is the client's, not this shell's: it used to be copied from `client.rs` and it lives
// in `tipsy_ui::policy` now, so both clients run the same half-open containment and neither can drift
// from the other. A plain `use`, not a `pub use`: nothing outside this module calls it — `drain`
// below and the tests in this file are the whole of its audience.
use tipsy_ui::policy::hit_at;

use crate::playback::Playback;

/// The subset of a key event the HUD has an opinion about, filled from Ratzilla's `KeyEvent` — the
/// same crossterm-*shaped* vocabulary, a separate type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKey {
    Char { c: char, ctrl: bool },
    Enter,
    Backspace,
    Tab,
    Esc,
    Up,
    Down,
    Left,
    Right,
    F(u8),
    /// Anything the HUD does not use: `Home`, `PageDown`, a media key, a bare modifier.
    Other,
}

/// One thing a handler received, waiting for the frame that acts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Key(KeyInput),
    /// The cell the pointer went down on, in the grid's own coordinates.
    Click { col: u16, row: u16 },
}

/// Ratzilla's key event in this crate's vocabulary, one arm per variant of its `KeyCode` (0.3.1's
/// `event.rs:52-85`), so a key Ratzilla can send is either a key the HUD has or `RawKey::Other`.
///
/// The `ctrl` flag is carried rather than acted on: `KeyInput` has no modifier, and turning a chord
/// into the control character it stands for is [`key_input`]'s decision. The five keys Ratzilla names
/// that the HUD does not use — `Delete`, `Home`, `End`, `PageUp`, `PageDown` — and its `Unidentified`
/// are `Other`, which is the native client's `_ => None` arm (`client.rs:561`).
pub fn raw_key(event: &ratzilla::event::KeyEvent) -> RawKey {
    use ratzilla::event::KeyCode;
    match event.code {
        KeyCode::Char(c) => RawKey::Char { c, ctrl: event.ctrl },
        KeyCode::Enter => RawKey::Enter,
        KeyCode::Backspace => RawKey::Backspace,
        KeyCode::Tab => RawKey::Tab,
        KeyCode::Esc => RawKey::Esc,
        KeyCode::Up => RawKey::Up,
        KeyCode::Down => RawKey::Down,
        KeyCode::Left => RawKey::Left,
        KeyCode::Right => RawKey::Right,
        KeyCode::F(n) => RawKey::F(n),
        KeyCode::Delete | KeyCode::Home | KeyCode::End | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Unidentified => {
            RawKey::Other
        }
    }
}

/// Ratzilla's mouse event, when it is the one event a click is: **a button going down, and only
/// that.**
///
/// Ratzilla's `WebGl2Backend` registers `mousedown`, `mouseup` and `click` on the canvas
/// (`backend/event_callback.rs:123-131`), so one press of the pointer arrives twice — first as
/// `ButtonDown` and then as `SingleClick` — and the native client matches only
/// `MouseEventKind::Down(_)` (`client.rs:304`). Accepting the second kind as well would raise the
/// same click twice, and a click on the header's gear toggles: the configuration screen would open
/// and close again before the next frame was drawn.
///
/// Every button counts, exactly as it does natively (`Down(_)` is `Down(Left)` for a left click and
/// `Down(Right)` for a right one): the button is not part of the HUD's vocabulary, and a page that
/// decided otherwise would answer a right click on the gear differently from the terminal.
pub fn mouse_input(event: &ratzilla::event::MouseEvent) -> Option<Input> {
    matches!(event.kind, ratzilla::event::MouseEventKind::ButtonDown(_))
        .then_some(Input::Click { col: event.col, row: event.row })
}

/// `client.rs:539-563` in this crate's vocabulary. The only two decisions a browser adds are the ones
/// the native function already makes: a control chord becomes the control character no text row wants,
/// and everything the HUD does not name becomes `None`.
pub fn key_input(raw: RawKey) -> Option<KeyInput> {
    match raw {
        RawKey::Enter => Some(KeyInput::Enter),
        RawKey::Backspace => Some(KeyInput::Backspace),
        RawKey::Tab => Some(KeyInput::Tab),
        RawKey::Esc => Some(KeyInput::Esc),
        RawKey::Up => Some(KeyInput::Up),
        RawKey::Down => Some(KeyInput::Down),
        RawKey::Left => Some(KeyInput::Left),
        RawKey::Right => Some(KeyInput::Right),
        RawKey::F(n) => Some(KeyInput::F(n)),
        RawKey::Char { c, ctrl: true } => {
            let lower = c.to_ascii_lowercase();
            lower.is_ascii_lowercase().then_some(KeyInput::Char((lower as u8 & 0x1f) as char))
        }
        RawKey::Char { c, ctrl: false } => Some(KeyInput::Char(c)),
        RawKey::Other => None,
    }
}

// `hit_at` is not written here any more: it is `tipsy_ui::policy`'s, imported in the `use` list above.
// Its body and its half-open edges are `crates/tipsy-ui/src/policy.rs:149-153`, and the click test
// below is the same case the native client's own
// `policy::tests::only_a_shown_hit_contains_a_click_and_its_edges_are_half_open` pins.

/// Applies everything the handlers queued since the last frame, in the order it arrived.
///
/// The clicks are tested against the `hits` the *previous* frame drew — which is the frame the user
/// was looking at, and the native client does exactly this: it keeps the `Vec<Hit>` `draw_frame`
/// returned and matches the next frame's mouse events against it (`client.rs:285`, `:304-308`).
pub fn drain(state: &mut UiState, hits: &[Hit], queued: &mut Vec<Input>) -> Vec<UiAction> {
    let mut actions = Vec::new();
    for input in queued.drain(..) {
        match input {
            Input::Key(key) => actions.extend(state.on_key(key)),
            Input::Click { col, row } => {
                if let Some(hit) = hit_at(hits, col, row) {
                    // The click names its own target where the control has one: `on_hit` dispatches
                    // `hit.action` and falls back to `hit.key` (`state.rs:325-357`), which is the whole
                    // of what keeps a click on "Profiles" from arriving as F9.
                    actions.extend(state.on_hit(&hit));
                }
                // A click on nothing is nothing. The native client ignores it too (`client.rs:304-308`
                // only pushes a hit that `hit_at` found), and inventing a focus change or a "miss"
                // notice here would make a stray click change the screen.
            }
        }
    }
    actions
}

/// The browser's local time, in the HUD's numbering. `client.rs:566-576` is the native twin and the
/// same numbers come out of it; the mapping that is not obvious is the weekday, because
/// `Date.getDay()` numbers from Sunday and the HUD numbers from Monday — `crate::weekday_from_sunday`
/// is that one mapping, and it has the crate's own test under it.
pub fn clock_from(hours: u32, minutes: u32, seconds: u32, js_day: u32, day: u32, month: u32) -> ClockView {
    ClockView {
        time: format!("{hours:02}:{minutes:02}:{seconds:02}"),
        weekday: crate::weekday_from_sunday(js_day),
        day: day as u8,
        month: month as u8,
    }
}

/// What the screen's `Test` button does to the reply being spoken: the ids the caller must silence.
///
/// The screen's path never passes through `Playback::on_command`, because a `Test` is not a command
/// the HUD sends — it is a `ConfigEffect` — so this is the one action whose barge-in rule is not
/// already inside the timeline. It is written here, as the rule and not as an omission:
/// `policy::test_interrupts_playback` is the imported answer to "does a probe take the floor"
/// (today: no, it is a diagnostic that says nothing out loud), and `policy::flush_on_interrupt` is
/// the same `&&` `on_command` uses, so a reply a probe interrupted nothing of stays exactly where it
/// was. **The test below fails the moment `test_interrupts_playback` moves** — which is the proof
/// that this is the shared rule and not a second copy of it.
pub fn test_barge_in(playback: &mut Playback, capability: Capability, now: f64) -> Vec<u64> {
    let quiet = tipsy_ui::policy::flush_on_interrupt(
        tipsy_ui::policy::test_interrupts_playback(capability),
        playback.is_playing(now),
    );
    if quiet {
        playback.barge_in(now)
    } else {
        // `barge_in` is deliberately not called: it would reset the cursor, the sequence position
        // and the gap count of a turn that is still arriving, for a probe that stopped nothing.
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::Terminal;
    use tipsy_protocol::ProfileView;

    fn welcome() -> tipsy_protocol::Event {
        tipsy_protocol::Event::Welcome {
            session_id: 1,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: tipsy_protocol::UiLanguage::Es,
            listening_mode: tipsy_protocol::ListeningMode::Push,
            web_bundle: true,
            login: None,
        }
    }

    #[test]
    fn the_key_vocabulary_is_the_native_clients() {
        assert_eq!(key_input(RawKey::Enter), Some(KeyInput::Enter));
        assert_eq!(key_input(RawKey::Backspace), Some(KeyInput::Backspace));
        assert_eq!(key_input(RawKey::Tab), Some(KeyInput::Tab));
        assert_eq!(key_input(RawKey::Esc), Some(KeyInput::Esc));
        assert_eq!(key_input(RawKey::Up), Some(KeyInput::Up));
        assert_eq!(key_input(RawKey::Down), Some(KeyInput::Down));
        assert_eq!(key_input(RawKey::Left), Some(KeyInput::Left));
        assert_eq!(key_input(RawKey::Right), Some(KeyInput::Right));
        assert_eq!(key_input(RawKey::F(9)), Some(KeyInput::F(9)));
        assert_eq!(key_input(RawKey::Char { c: 'ñ', ctrl: false }), Some(KeyInput::Char('ñ')));
        assert_eq!(key_input(RawKey::Other), None, "a key the HUD has no opinion about is not an action");
    }

    #[test]
    fn a_control_chord_arrives_as_the_control_character_it_stands_for() {
        // The native twin is `client.rs:556-559`: `KeyInput` has no modifier, so Ctrl+S and Ctrl+R are
        // the control characters the screen's save and regenerate rows are bound to
        // (`ConfigForm::SAVE_KEY`, `ConfigForm::REGENERATE_KEY`).
        assert_eq!(key_input(RawKey::Char { c: 's', ctrl: true }), Some(KeyInput::Char('\u{13}')));
        assert_eq!(key_input(RawKey::Char { c: 'S', ctrl: true }), Some(KeyInput::Char('\u{13}')), "the case folds first");
        assert_eq!(key_input(RawKey::Char { c: 'r', ctrl: true }), Some(KeyInput::Char('\u{12}')));
        // A chord that names no ASCII letter is not a key this HUD has.
        assert_eq!(key_input(RawKey::Char { c: '1', ctrl: true }), None);
        assert_eq!(key_input(RawKey::Char { c: 'ñ', ctrl: true }), None);
    }

    #[test]
    fn a_click_lands_on_the_rectangle_that_contains_it_and_on_nothing_else() {
        let hits = vec![
            Hit { area: Rect::new(10, 2, 6, 1), key: KeyInput::F(7), action: None },
            Hit { area: Rect::new(20, 2, 4, 1), key: KeyInput::F(8), action: None },
        ];
        assert_eq!(hit_at(&hits, 10, 2).map(|h| h.key), Some(KeyInput::F(7)), "the first column is inside");
        assert_eq!(hit_at(&hits, 15, 2).map(|h| h.key), Some(KeyInput::F(7)), "and so is the last: right() is exclusive");
        assert!(hit_at(&hits, 16, 2).is_none(), "one column past the end is not a hit");
        assert!(hit_at(&hits, 9, 2).is_none());
        assert!(hit_at(&hits, 10, 3).is_none(), "one row below is not a hit either");
        assert_eq!(hit_at(&hits, 21, 2).map(|h| h.key), Some(KeyInput::F(8)), "overlapping rectangles report the first");
    }

    #[test]
    fn the_actions_come_out_in_the_order_the_input_arrived() {
        let mut state = UiState::new();
        state.apply(&welcome());
        let hits = vec![Hit { area: Rect::new(20, 2, 4, 1), key: KeyInput::F(8), action: None }];
        let mut queued = vec![
            Input::Key(KeyInput::Char('h')),
            Input::Click { col: 20, row: 2 },
            Input::Key(KeyInput::Char('i')),
        ];
        let actions = drain(&mut state, &hits, &mut queued);
        assert!(actions.is_empty(), "none of these three asks the server for anything: {actions:?}");
        assert_eq!(state.input, "hi", "the two keys reached the question box in order, around the click");
        assert_eq!(state.tab, tipsy_ui::Tab::Network, "and the click switched the tab");
        assert!(queued.is_empty(), "the queue is drained, not copied");
    }

    #[test]
    fn a_click_that_lands_on_nothing_is_not_an_action() {
        let mut state = UiState::new();
        state.apply(&welcome());
        let hits = vec![Hit { area: Rect::new(20, 2, 4, 1), key: KeyInput::F(8), action: None }];
        let mut queued = vec![Input::Click { col: 0, row: 0 }];
        assert!(drain(&mut state, &hits, &mut queued).is_empty());
        assert_eq!(state.tab, tipsy_ui::Tab::Chat, "and nothing on screen changed");
        assert!(state.input.is_empty());
    }

    /// **The plan's own version of this test asserted `is_empty()`, and that could not pass**: opening
    /// the screen *is* an action, `UiState::toggle_config_form` returns `vec![UiAction::OpenConfig]`
    /// for the opening half (`state.rs:271-281`), and that action is the whole reason the browser has
    /// to answer it with `Command::GetConfig` (Task 3.5's `dispatch`). The test is rewritten to say
    /// what the click really raises, and to pin it against the key path rather than in a literal.
    #[test]
    fn a_click_on_a_control_raises_the_action_its_key_raises() {
        let hits = vec![Hit { area: Rect::new(78, 0, 2, 1), key: KeyInput::F(9), action: None }];
        let mut queued = vec![Input::Click { col: 78, row: 0 }];
        let mut clicked = UiState::new();
        clicked.apply(&welcome());
        let mut typed = UiState::new();
        typed.apply(&welcome());

        let from_click = drain(&mut clicked, &hits, &mut queued);
        let from_key = typed.on_key(KeyInput::F(9));

        assert_eq!(from_click, vec![UiAction::OpenConfig], "the gear's click raises the door's action");
        assert_eq!(from_click, from_key, "and it is the same action `F9` raises, not a second one");
        assert!(clicked.config_form.is_some() && typed.config_form.is_some(), "the screen is open either way");
        // The form the screen opens on is the placeholder: it is not filled until the server's
        // `Event::Config` answers, and an unfilled form refuses `Ctrl+S` (`config_form.rs:712-721`).
        assert!(!clicked.config_form.as_ref().unwrap().filled());
    }

    #[test]
    fn the_clock_is_the_browsers_local_time_in_the_huds_numbering() {
        // A Sunday: JavaScript numbers the week from Sunday (`Date.getDay()` 0..6) and the HUD from
        // Monday (`client.rs:566-576`, `now.weekday().num_days_from_monday()`), and JavaScript's month
        // is already 0-based like `now.month0()`.
        let sunday = clock_from(9, 5, 7, 0, 4, 9);
        assert_eq!(sunday.time, "09:05:07");
        assert_eq!(sunday.weekday, 6, "Sunday is the seventh day of the HUD's week");
        assert_eq!(sunday.day, 4);
        assert_eq!(sunday.month, 9);
        assert_eq!(clock_from(23, 59, 59, 1, 31, 0).weekday, 0, "Monday is 0");
        assert_eq!(clock_from(0, 0, 0, 6, 1, 11).weekday, 5, "Saturday is 5");
    }

    /// **Every `KeyCode` Ratzilla 0.3.1 can produce** (`ratzilla-0.3.1/src/event.rs:52-85`), one row
    /// each, so the adapter cannot lose a key the HUD has or invent one it does not. The three
    /// unassigned keys are the ones the native client's `_ => None` arm swallows.
    #[test]
    fn every_key_ratzilla_can_send_becomes_the_raw_key_the_hud_speaks() {
        use ratzilla::event::{KeyCode, KeyEvent};
        let key = |code| KeyEvent { code, ctrl: false, alt: false, shift: false };
        let ctrl = |code| KeyEvent { code, ctrl: true, alt: false, shift: false };

        for (code, want) in [
            (KeyCode::Char('ñ'), RawKey::Char { c: 'ñ', ctrl: false }),
            (KeyCode::F(9), RawKey::F(9)),
            (KeyCode::Backspace, RawKey::Backspace),
            (KeyCode::Enter, RawKey::Enter),
            (KeyCode::Left, RawKey::Left),
            (KeyCode::Right, RawKey::Right),
            (KeyCode::Up, RawKey::Up),
            (KeyCode::Down, RawKey::Down),
            (KeyCode::Tab, RawKey::Tab),
            (KeyCode::Esc, RawKey::Esc),
            (KeyCode::Delete, RawKey::Other),
            (KeyCode::Home, RawKey::Other),
            (KeyCode::End, RawKey::Other),
            (KeyCode::PageUp, RawKey::Other),
            (KeyCode::PageDown, RawKey::Other),
            (KeyCode::Unidentified, RawKey::Other),
        ] {
            assert_eq!(raw_key(&key(code.clone())), want, "{code:?} is {want:?}");
        }
        assert_eq!(
            raw_key(&ctrl(KeyCode::Char('s'))),
            RawKey::Char { c: 's', ctrl: true },
            "the modifier is carried, and the folding to a control character is `key_input`'s"
        );
        assert_eq!(key_input(raw_key(&ctrl(KeyCode::Char('s')))), Some(KeyInput::Char('\u{13}')));
    }

    /// **One press is one click.** Ratzilla's `WebGl2Backend` registers `mousedown`, `mouseup` and
    /// `click` (`backend/event_callback.rs:123-131`), so a single press arrives twice — as
    /// `ButtonDown` and then as `SingleClick`. The native client matches `MouseEventKind::Down(_)`
    /// only (`client.rs:304`), and accepting the second would toggle the door twice for one press:
    /// the configuration screen would open and close before the next frame drew.
    #[test]
    fn only_a_button_press_is_a_click_and_a_second_kind_for_the_same_press_is_not() {
        use ratzilla::event::{MouseButton, MouseEvent, MouseEventKind};
        let mouse = |kind| MouseEvent { kind, col: 7, row: 3, ctrl: false, alt: false, shift: false };

        assert_eq!(
            mouse_input(&mouse(MouseEventKind::ButtonDown(MouseButton::Left))),
            Some(Input::Click { col: 7, row: 3 })
        );
        for kind in [
            MouseEventKind::ButtonUp(MouseButton::Left),
            MouseEventKind::SingleClick(MouseButton::Left),
            MouseEventKind::DoubleClick(MouseButton::Left),
            MouseEventKind::Moved,
            MouseEventKind::Entered,
            MouseEventKind::Exited,
            MouseEventKind::Unidentified,
        ] {
            assert_eq!(mouse_input(&mouse(kind)), None, "{kind:?} is not a new click");
        }
    }

    /// **The header is drawn, not guessed.** `tipsy_ui::draw` is called exactly as the browser calls
    /// it, on a real `Terminal`, and its own `Hit`s are what a click is tested against — so this pins
    /// the header's geometry from the other side, which is what Task 2.4 measured on the terminal page
    /// (two header hits, both `F(9)`, the gear flush right, and the badge drawn but not reported).
    ///
    /// The whole frame reports five: the header's two come first (`render.rs:131-133`), and the three
    /// after them are the log panel's `CHAT`/`RED` tabs and its clear button (`render.rs:349-351`).
    #[test]
    fn the_header_reports_its_two_controls_and_the_gear_sits_in_the_last_columns() {
        let mut state = UiState::new();
        state.apply(&welcome());
        let clock = clock_from(17, 41, 3, 0, 11, 9);
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("a test terminal");
        let mut hits = Vec::new();
        terminal
            .draw(|frame| hits = tipsy_ui::draw(frame, &state, &clock, "El Compae"))
            .expect("one frame");

        assert!(hits.len() >= 2, "the header reports its two controls first: {hits:?}");
        let (mode, gear) = (hits[0], hits[1]);
        assert_eq!(mode.key, KeyInput::F(9), "the mode word is the door key: {mode:?}");
        assert_eq!(gear.key, KeyInput::F(9), "and so is the gear: {gear:?}");
        assert!(
            hits[2..].iter().all(|hit| hit.key != KeyInput::F(9)),
            "the door is the header's key and no pane behind it answers `F9`: {hits:?}"
        );
        assert!(
            mode.area.y == 0 && mode.area.height == 1 && gear.area.y == 0 && gear.area.height == 1,
            "both are on the header's own row and one row tall: {mode:?} {gear:?}"
        );
        assert!(mode.area.x < gear.area.x, "the mode word is drawn before the gear");
        assert_eq!(mode.area.right(), gear.area.x, "the mode's rectangle ends where the gear's begins");
        assert_eq!(gear.area.right(), 120, "and the gear is flush against the right edge of the frame");
        assert!(mode.area.width > 0 && gear.area.width > 0, "both are clickable: {mode:?} {gear:?}");

        // The claim the hit test makes, on the frame that was just drawn: the top-left cell of the
        // gear is the gear, and the cell one row below it is nothing at all.
        assert_eq!(hit_at(&hits, gear.area.x, 0).map(|hit| hit.area), Some(gear.area));
        assert!(hit_at(&hits, gear.area.x, 1).is_none(), "the header's rectangles are one row tall");
        // And the plan's own instruction — "click the header's profile title" — is wrong, which Task
        // 2.4 measured first: the badge is *drawn* and reports no rectangle at all. Columns 0..13 on
        // row 0 hold ` ◉ EL COMPAE ` and none of them is a hit.
        for column in [0u16, 5, 12] {
            assert!(hit_at(&hits, column, 0).is_none(), "column {column} is the badge, which is not a control");
        }
    }

    /// The same frame, clicked: the gear's own cell opens the screen and one row below it does not —
    /// the second half is the `DomBackend` off-by-one class the spike measured (`#179`/`#188`), asked
    /// of this code rather than of the backend.
    #[test]
    fn a_click_on_the_gear_opens_the_screen_and_the_cell_below_it_does_not() {
        let mut state = UiState::new();
        state.apply(&welcome());
        let clock = clock_from(17, 41, 3, 0, 11, 9);
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("a test terminal");
        let mut hits = Vec::new();
        terminal
            .draw(|frame| hits = tipsy_ui::draw(frame, &state, &clock, "El Compae"))
            .expect("one frame");
        let gear = hits[1].area;

        let mut queued = vec![Input::Click { col: gear.x, row: 0 }];
        assert_eq!(drain(&mut state, &hits, &mut queued), vec![UiAction::OpenConfig], "the gear's own cell");
        assert!(state.config_form.is_some(), "and the screen is up");

        let mut state = UiState::new();
        state.apply(&welcome());
        let mut queued = vec![Input::Click { col: gear.x, row: 1 }];
        assert!(drain(&mut state, &hits, &mut queued).is_empty(), "the cell below the header is not the gear");
        assert!(state.config_form.is_none(), "and nothing on screen changed");
    }

    /// **The screen's `Test` is a diagnostic and never quiets a reply**, and the browser asks that
    /// rule instead of assuming it. The proof that the rule is imported and not a coincidence is that
    /// the same test fails when `tipsy_ui::policy::test_interrupts_playback` is flipped — exactly the
    /// mutation that moves the native client's tests too.
    #[test]
    fn a_probe_never_quiets_a_reply_that_is_playing() {
        let mut playback = Playback::new();
        playback.set_rate(24_000);
        // One sentence of 0.1 s, anchored 0.08 s ahead of the clock: it is playing at 0.0.
        playback.chunk(0, vec![1_000i16; 2_400], 0.0).expect("the first sentence is placed");
        assert!(playback.is_playing(0.0), "the reply is audible at the clock this asks about");

        for capability in [Capability::Chat, Capability::Speech, Capability::Transcribe] {
            assert!(
                test_barge_in(&mut playback, capability, 0.0).is_empty(),
                "{capability} is a probe, not a turn: it silences nothing"
            );
        }

        // And the timeline is where it was: the next sentence of the same turn is still gapless
        // behind the first. Had `barge_in` run, the cursor would have been reset and this would land
        // at `CUSHION_S` instead.
        let next = playback.chunk(1, vec![2_000i16; 2_400], 0.0).expect("the second sentence is placed");
        assert!(
            (next.at - (crate::playback::CUSHION_S + 0.1)).abs() < 1e-9,
            "the turn's own cursor survived the probe: {} ",
            next.at
        );
    }
}
