//! The browser client's entry point.
//!
//! `start()` is what `wasm-bindgen --target web`'s generated `init()` calls. The render loop is the
//! native client's frame loop with the socket's queue in place of `from_server`
//! (`crates/tipsy-compadre/src/client.rs:272-349` is the loop being mirrored): tick, drain, draw.
//! `draw_web` wraps `requestAnimationFrame` itself, so the loop needs **no async runtime** — only the
//! socket's callbacks do, and those use `spawn_local` (ratzilla-and-wasm-audio.md §6.4 point 5).
//!
//! **What this file deliberately does not have yet.** The key and mouse handlers and the page's
//! layout are Tasks 3.4 and 3.5's; the hits `tipsy_ui::draw` reports are therefore dropped here
//! rather than kept in a cell nothing reads, and Task 3.5 is what starts reading them.
//!
//! **The microphone is Task 3.3's and it is wired here**, because `#[wasm_bindgen(start)]` is the only
//! entry point `init()` calls and this is where the `UiState`, the queue and the render loop live: the
//! gate is built from this function, the capture it opens is kept in one [`mic::CaptureSlot`] for the
//! whole run, and every frame drains that capture inside the one borrow of the state the frame owns.

pub mod mic;
pub mod socket;
pub mod wire;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use tipsy_protocol::{Event, Frame, ToClient};
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
                while let Some(message) = queue.pop_front() {
                    match message {
                        ToClient::Event(event) => {
                            // The language first: `apply` is what sets it from `Welcome`, and the
                            // "connection restored" notice is the one thing this side says, so it is
                            // said in the profile's language and not in the default one.
                            state.apply(&event);
                            if matches!(event, Event::Welcome { .. }) && outbox.welcomed() {
                                let text = state.strings().web_online;
                                state.notify(text);
                            }
                        }
                        // Playback is Task 3.4's: a `Frame::Speech` is dropped here on purpose, which
                        // is why this task's client is silent. A `Frame::Mic` on this side is a frame
                        // this client never sent.
                        ToClient::Audio(Frame::Speech { .. } | Frame::Mic(_)) => {}
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
}
