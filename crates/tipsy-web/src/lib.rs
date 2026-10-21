//! The browser client's entry point.
//!
//! `start()` is what `wasm-bindgen --target web`'s generated `init()` calls. The render loop is the
//! native client's frame loop with the socket's queue in place of `from_server`
//! (`crates/tipsy-assistant/src/client.rs:272-349` is the loop being mirrored): tick, drain, draw.
//! `draw_web` wraps `requestAnimationFrame` itself, so the loop needs **no async runtime** — only the
//! socket's callbacks do, and those use `spawn_local` (ratzilla-and-wasm-audio.md §6.4 point 5).
//!
//! **What this file deliberately does not have.** The translation itself — Ratzilla's key and mouse
//! events into `tipsy_ui`'s vocabulary, the click test and the clock's numbering — is [`hud`]'s, where
//! a host test can reach it; the two callbacks registered below are one line each and hold no decision
//! of their own. One `UiAction` is inert here and named rather than implied: `ToggleLive`, which has no
//! browser implementation in this phase. The login door is [`login_door`]'s — the screen is
//! `tipsy_ui`'s and is not reimplemented anywhere — and this file's whole part in it is three call
//! sites: `route` offers each event to `Door::note_event` after `state.apply`, the frame loop asks
//! `Door::sync` for the capture's edges and drains the microphone behind `Door::sends_frames`, and
//! `dispatch`'s `UiAction::LoginDone` arm takes the screen down.
//!
//! **The barge-in is [`playback::Playback::on_command`]'s and its call site is [`dispatch`].** The rule
//! itself is not defined here and is not repeated here — `on_command` calls `tipsy_ui::policy`'s own
//! functions, and the browser's only contribution is the clock it passes.
//!
//! **The microphone is Task 3.3's and it is wired here**, because `#[wasm_bindgen(start)]` is the only
//! entry point `init()` calls and this is where the `UiState`, the queue and the render loop live: the
//! gate is built from this function, the capture it opens is kept in one [`mic::CaptureSlot`] for the
//! whole run, and every frame drains that capture inside the one borrow of the state the frame owns.
//!
//! **The reply is Task 3.4's, and it plays through the run's own `AudioContext`.** [`playback`] is the
//! timeline — pure, host-tested — and [`web_audio`] is the presenter that schedules one
//! `AudioBufferSourceNode` per sentence on it. There is exactly one `AudioContext` per run — a second
//! one would be a second clock — and **the user's press creates it whether or not the microphone
//! capture succeeded** ([`mic::AudioSlot`], the phase review's finding 4): a page whose `getUserMedia`
//! was refused, or whose device is busy, still hears the greeting and every reply, and the microphone
//! is one consumer of the context rather than the thing that brings it into being. That is also why a
//! `Frame::Speech` arriving before the press is **dropped** rather than kept — the events around it
//! still reach the HUD, and the audio has nowhere to go.
//!
//! **The gate, and the failure it has to survive, are [`mic::press_to_start`]'s; this file reads the
//! fact.** The overlay is opaque and full-viewport, so **every** path out of the gate takes it off the
//! page (the phase review's Blocker 2) and the reason is recorded in the state: the login screen's own
//! line when one is up, the HUD's notice when none is. A screen that opens **after** the refusal — the
//! probe answers at load, before `Welcome` arrives — is put right by the frame loop from
//! [`mic::MicFailure`], so it never claims a `● rec` for a device that will not open and never sends a
//! `LoginStart` for one.

pub mod hud;
pub mod login_door;
pub mod mic;
pub mod playback;
pub mod socket;
pub mod web_audio;
pub mod wire;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use tipsy_protocol::{AssistantState, ClientConfigView, Command, Event, Frame, ToClient, ToServer};
use tipsy_ui::{ClockView, ConfigEffect, ConfigForm, Hit, NoticeKind, UiAction, UiState, policy};
use wasm_bindgen::prelude::*;

// `draw_web` and the three event hooks are `WebRenderer`'s, and the trait has to be in scope for
// `Terminal<WebGl2Backend>` to have them (`ratzilla-0.3.1/src/render.rs:63-98`); the `WebEventHandler`
// trait behind them does not need to be named, which is why it is not imported.
use ratzilla::WebRenderer;

/// The wasm entry point (the front matter's shared contract pins this name).
///
/// The backend is the one Task 3.1's verdict named; the three backends are interchangeable behind
/// `WebRenderer`, so this is the one line a later switch changes.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let floating = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .is_some_and(|search| socket::floating_from_query(&search));
    let backend = if floating {
        ratzilla::WebGl2Backend::new_with_options(
            ratzilla::backend::webgl2::WebGl2BackendOptions::new().disable_auto_css_resize(),
        )
    } else {
        ratzilla::WebGl2Backend::new()
    }
    .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let mut terminal =
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
    // **The run's own audio, filled by the same press and independently of the capture** (finding 4):
    // the reply is scheduled on this context, so a refused microphone is a page that plays but cannot
    // listen rather than a page that can do neither.
    let audio: mic::AudioSlot = Rc::new(RefCell::new(None));
    // Why the microphone will not open, once the page knows: the probe runs here, before `Welcome` and
    // so before any login screen exists, and the screen that arrives afterwards has to say its piece
    // rather than claim a capture it can never have (Blocker 2).
    let mic_failure: mic::MicFailure = Rc::new(RefCell::new(None));
    // The "press to start" screen (v1 spec `:193-194`). It builds its own elements, so the page Task
    // 3.2 created needs no change for it, and a microphone that cannot work is a **pressable** button
    // with the reason beside it — the press is the gesture the audio context needs and the way past an
    // otherwise opaque overlay — rather than a button over a page nobody can reach. It gets the
    // `Outbox` too, because the press opens the HUD's microphone and the server has to be told
    // (`Command::ListenStart`: a switch the session does not know about is one whose frames it drops).
    mic::press_to_start(
        Rc::clone(&state),
        Rc::clone(&capture),
        Rc::clone(&audio),
        Rc::clone(&mic_failure),
        outbox.clone(),
    );

    // The reply's timeline: one per run, fed by the frame loop below and presented by `web_audio`.
    let shared = Rc::new(RefCell::new(Shared {
        state,
        hits: Vec::new(),
        queued: Vec::new(),
        clock: hud::clock_from(0, 0, 0, 0, 0, 0),
        title: String::new(),
        playback: playback::Playback::new(),
        outbox,
        capture,
        audio,
        mic_failure,
        // `[listening] mic_gain` (finding 7): the identity until this client has been told the
        // configuration, which a browser learns from `Event::Config` and from nowhere else.
        mic_gain: mic::MicGain::default(),
        queue,
        // **One `Door` per `start()`, and it carries the request in flight and no run-scoped bit at
        // all**: the once-per-run rule is `UiState::login_seen`, asked of the state by `show_login`
        // (`crates/tipsy-ui/src/state.rs:212-218`, `:293-314`). A second one would be a second
        // `started` flag, so the page could send two `LoginStart`s for one capture or miss the
        // `LoginStop` that belongs to the one it opened — the defect `login_door.rs` exists to prevent.
        door: login_door::Door::new(),
        quit: false,
    }));

    // **The keyboard half of the backend, measured in 0.3.1's own source rather than taken from the
    // research note.** The note and this plan's Step 4 both say `WebGl2Backend` "does not support key
    // events and will silently succeed without registering any handlers". `WebGl2Backend::on_key_event`
    // in 0.3.1 does the opposite: it sets `tabindex="0"` on its own canvas and attaches a `keydown`
    // listener to it (`backend/webgl2.rs:920-942`), so the plan's separate `set_attribute("tabindex",
    // "0")` is redundant and is not written here. What it does *not* do is **focus** the element, and
    // the spike's own measurement is that keys arrive "given `tabindex="0"` **and focus**" — its
    // counters stayed at zero until it called `focus()`. So the two calls to [`focus_canvas`] below
    // are the load-bearing half: one here for the page's initial state, and one from the mouse
    // callback because the gate's "press to start" button holds the focus until it is removed.
    //
    // Registered BEFORE `draw_web`: it consumes the `Terminal` by value, so a callback added
    // afterwards has nothing to attach to (`ratzilla-0.3.1/src/render.rs:67`).
    terminal
        .on_key_event({
            let shared = Rc::clone(&shared);
            move |event: ratzilla::event::KeyEvent| {
                // The whole translation is `hud`'s and is host-tested; this is the adapter's call site and
                // nothing else. A key the HUD has no opinion about does not reach the queue at all.
                if let Some(key) = hud::key_input(hud::raw_key(&event)) {
                    shared.borrow_mut().queued.push(hud::Input::Key(key));
                }
            }
        })
        // The plan's Step 4 writes `?` here. It does not compile: `on_key_event` answers
        // `Result<(), ratzilla::error::Error>`, and ratzilla's error type has no `From` into `JsValue` —
        // so the conversion is the same one the two backend lines above use.
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    terminal
        .on_mouse_event({
            let shared = Rc::clone(&shared);
            move |event: ratzilla::event::MouseEvent| {
                if let Some(input) = hud::mouse_input(&event) {
                    // A pointer that reaches the canvas focuses it, and that is what gives the keyboard
                    // back after the gate's button took the focus and was then removed from the page.
                    focus_canvas();
                    shared.borrow_mut().queued.push(input);
                }
            }
        })
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    focus_canvas();

    let mut last = js_sys::Date::now();
    terminal.draw_web(move |frame| {
        let mut guard = shared.borrow_mut();
        let shared: &mut Shared = &mut guard;
        // The user quit: the socket is closed, nothing reconnects, and the frame that is on screen
        // stays there. Stopping `requestAnimationFrame` is not something `draw_web` offers, so the
        // loop keeps running and draws nothing — which is the same picture for a fortieth of the cost.
        if shared.quit {
            return;
        }
        let now = js_sys::Date::now();
        let dt = (now - last).max(0.0) as u64;
        last = now;

        // `[listening] mic_gain`, as the configuration this client has been given spells it (finding
        // 7). A browser has no `config.toml`: the value arrives with `Event::Config` — the answer to
        // the `GetConfig` the configuration screen sends when it opens — and it is kept here so that
        // closing the screen does not unlearn it. `configured_gain` refuses a placeholder form and an
        // unsaved edit, which is what makes reading it every frame honest.
        if let Some(gain) = mic::configured_gain(&shared.state.borrow()) {
            shared.mic_gain = gain;
        }

        // The frame's whole borrow of the state, and it answers with the actions the input raised: the
        // door's reconciliation has to follow `hud::drain` **inside the same frame** — a `Tab` that just
        // closed the capture is a `LoginStop` now, not on the next one — and `drain_capture` has to be
        // inside the one `borrow_mut` this frame owns, because a second one is the panic Task 3.5's own
        // comment warns about. The actions are dispatched after the draw, as they always were.
        let actions = {
            let mut state = shared.state.borrow_mut();
            state.tick(dt);
            shared.clock = clock_view();

            // The frame loop drains everything that arrived since the last frame, exactly as the native
            // loop drains `from_server` (`client.rs:322-324`). The one clock the reply is scheduled on
            // is the press's `AudioContext`, and it is `None` until that press.
            let audio_now = audio_now(&shared.audio);
            let (welcomed, scheduled) = {
                let mut queue = shared.queue.borrow_mut();
                route(&mut queue, &mut shared.playback, &mut shared.door, &mut state, audio_now)
            };
            // The language first: `apply` is what sets it from `Welcome`, and the "connection
            // restored" notice is the one thing this side says, so it is said in the profile's
            // language and not in the default one.
            if welcomed && shared.outbox.welcomed() {
                let text = state.strings().web_online;
                state.notify(text);
            }
            if let Some(audio_now) = audio_now {
                present(&mut shared.playback, &shared.audio, audio_now, &scheduled);
            }
            state.local_output = audio_now.is_some_and(|now| shared.playback.is_playing(now));
            // Everything the handlers queued since the last frame, in the order it arrived, tested
            // against the hits of the frame the user was actually looking at.
            let mut actions = hud::drain(&mut state, &shared.hits, &mut shared.queued);
            // **A microphone that refused before the screen was up.** The probe answers at load and a
            // press can fail while no screen exists, so the screen that opens afterwards is put right
            // here — before `sync`, so the same frame sends no `LoginStart` for a device that will not
            // open, and before the draw below, so no `● rec` is ever painted for one.
            let mic_failed = shared.mic_failure.borrow().is_some();
            if mic_failed {
                mic::carry_no_microphone(&mut state);
                // The reducer's own `Tab`, and the empty-box re-arm, still raise `LoginStart` in the
                // one frame before the carry above has run. A client whose microphone refused must
                // not announce a capture it cannot have: the server would drop the frames anyway, and
                // this keeps the wire honest rather than merely harmless.
                actions.retain(|action| !matches!(action, UiAction::Send(Command::LoginStart)));
            }
            // The capture's edges: a screen that just opened with `voice` sends `LoginStart`; `Tab`,
            // the third refusal, a failed device and a **pass** all close the capture and send
            // `LoginStop`; and the command goes down the same socket path every other HUD command does
            // (a `UiAction::Send` reaches the same `Outbox` through `dispatch` below). **One call,
            // because `sync` consumes the edge**: calling it twice would send nothing the second time.
            let capture_command = match shared.door.sync(&state) {
                login_door::MicStep::Start => Some(Command::LoginStart),
                login_door::MicStep::Stop => Some(Command::LoginStop),
                login_door::MicStep::None => None,
            };
            if let Some(command) = capture_command {
                send(shared, command);
            }
            // The microphone, once per frame and inside the one borrow this frame owns: the port's
            // handler only appends what the worklet posted, and this is the only place a captured
            // frame may reach the socket — behind [`login_door::Door::sends_frames`] and nothing else.
            // No capture means no frames, which is what the gate's un-pressed state is.
            drain_microphone(&shared.door, &mut state, &shared.capture, &shared.outbox, &shared.mic_gain);
            actions
        };

        shared.title = {
            let state = shared.state.borrow();
            state.profile.as_ref().map(|profile| profile.name.clone()).unwrap_or_default()
        };
        let hits = {
            let state = shared.state.borrow();
            if floating && frame.area().width <= 600 {
                tipsy_ui::draw_compact(frame, &state)
            } else {
                tipsy_ui::draw(frame, &state, &shared.clock, &shared.title)
            }
        };
        shared.hits = hits;
        for action in actions {
            dispatch(action, shared);
        }
    });
    Ok(())
}

/// Everything the render loop and the two handlers share.
///
/// **`state` is the `Rc<RefCell<UiState>>` Task 3.2's socket and Task 3.3's gate already hold, not a
/// copy.** The plan's sketch gave this struct an owned `UiState`, which would have been a second one:
/// the socket's close handler writes the `web_offline` notice into *its* state and the gate writes
/// `mic_open` into its own, so a struct with a third would render a HUD nobody was updating.
///
/// The same correction applies to the two fields the plan gave it that cannot be built: there is no
/// `to_server` (that is Task 3.2's [`socket::Outbox`], which is `Clone` and follows a reconnect) and
/// no `web_sys::WebSocket` (the outbox owns the cell; [`socket::Outbox::close`] is the way in) — and
/// there is no `AudioContext` either, because the one context per run belongs to the capture
/// ([`mic::Capture::context`]) and a second one would be a second clock.
struct Shared {
    state: Rc<RefCell<UiState>>,
    /// The hits of the frame that is on screen, which is what a click is tested against.
    hits: Vec<Hit>,
    /// Input since the last frame, in arrival order.
    queued: Vec<hud::Input>,
    /// The clock, refreshed once per frame from the browser's own time.
    clock: ClockView,
    /// The profile's title, as the frame that drew the header used it.
    title: String,
    /// Task 3.4's timeline, which `Replay` and the barge-in both read.
    playback: playback::Playback,
    /// Task 3.2's sender for everything the HUD asks the server for.
    outbox: socket::Outbox,
    /// Task 3.3's capture slot, which holds the microphone's graph while a capture runs.
    capture: mic::CaptureSlot,
    /// **The run's one `AudioContext`**, filled by the same press and independent of the capture
    /// (finding 4): playback's clock, and the reason a refused microphone still hears the reply.
    audio: mic::AudioSlot,
    /// Why the microphone will not open, once the page knows (Blocker 2): read by the frame loop so a
    /// screen that opens after the refusal is typing-only and says so.
    mic_failure: mic::MicFailure,
    /// `[listening] mic_gain` as the last filled, clean configuration carried it, and the identity
    /// until this client has been given one (finding 7).
    mic_gain: mic::MicGain,
    /// The messages the socket has queued, drained once per frame.
    queue: Rc<RefCell<VecDeque<ToClient>>>,
    /// Task 3.7's login door: the screen's first-show, the capture's edges and the one predicate a
    /// captured frame has to pass. One per `start()`, held here because the frame loop, `route` and
    /// `dispatch` all need it and a second one would be a second `started` flag.
    door: login_door::Door,
    /// The user pressed `Esc`: the socket is closed and nothing more is drawn.
    quit: bool,
}

/// Puts the keyboard focus on the backend's canvas.
///
/// `WebGl2Backend::on_key_event` makes the canvas focusable and attaches its listener to it
/// (`backend/webgl2.rs:920-942`), but a listener on an element that does not hold the focus never
/// fires: the spike recorded `keys=0` while `on_key_event` returned `Ok(())`, and its counters only
/// moved once it had focused the element. The canvas is not in the page's own markup at all — the
/// backend appends it to `<body>` (`backend/utils.rs:255-265`, `:276-294`) — so it is found by tag
/// name and nothing else. A page that refuses the focus (a background tab) simply has no keyboard
/// until it is activated again, which is why this is not an error.
fn focus_canvas() {
    let Some(canvas) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector("canvas").ok().flatten())
    else {
        return;
    };
    if let Ok(element) = canvas.dyn_into::<web_sys::HtmlElement>() {
        // `HtmlElement::focus` is web-sys's associated-function shape and answers `Result<(), JsValue>`
        // (0.3.106's `gen_HtmlElement.rs`): a page that refuses the focus — a background tab — is not
        // an error this client has anything to do about.
        let _ = web_sys::HtmlElement::focus(&element);
    }
}

/// The one clock the reply is scheduled on: the `AudioContext` the user's press created, and `None`
/// until then. `current_time` is MDN's, in seconds, and it stops advancing while the context is
/// suspended.
///
/// **It is the audio slot that is read, not the capture** (finding 4): the context exists as soon as
/// the press does, so a page whose `getUserMedia` was refused is a page that plays the greeting and
/// every reply. Reading the capture here is what used to make playback impossible without a
/// successful capture — `route` drops a `Frame::Speech` when this is `None`.
fn audio_now(audio: &mic::AudioSlot) -> Option<f64> {
    audio.borrow().as_ref().map(|context| context.current_time())
}

/// Schedules the chunks the timeline placed and drops the nodes whose audio has ended.
///
/// The frame callback cannot propagate an error, and a silent client is the worst outcome this file
/// has: a failure goes where the browser console can be read.
fn present(
    playback: &mut playback::Playback,
    audio: &mic::AudioSlot,
    audio_now: f64,
    scheduled: &[playback::Scheduled],
) {
    if let Some(ctx) = audio.borrow().as_ref() {
        for chunk in scheduled {
            if let Err(error) = web_audio::play(ctx, chunk) {
                web_sys::console::error_1(&error);
            }
        }
    }
    // Nodes whose audio has ended are forgotten so the map cannot grow for the length of a reply. The
    // prior art does this from each node's `onended`; `web_audio.rs` says why this does it from the
    // core instead.
    for id in playback.finished(audio_now) {
        web_audio::forget(id);
    }
}

/// The microphone, once per frame: the port's handler only appends what the worklet posted, and this
/// is the only place a captured frame may reach the socket. **The guard is the door's**, and it is
/// inside [`mic::drain_capture`] rather than here, so the one send site and the one predicate cannot
/// drift apart; the gain is `[listening] mic_gain`'s, applied to the level the `VOZ` panel shows.
fn drain_microphone(
    door: &login_door::Door,
    state: &mut UiState,
    capture: &mic::CaptureSlot,
    outbox: &socket::Outbox,
    gain: &mic::MicGain,
) {
    if let Some(capture) = capture.borrow().as_ref() {
        mic::drain_capture(capture, door, state, outbox, gain);
    }
}

/// Sends one command, and treats a socket that is not open as the native client treats a closed
/// channel: the message is dropped (`client.rs:465`).
///
/// It is not silent, though, because the browser console is the only place a reader can see it — and
/// `NotOpen` is the ordinary case on a page that has not finished connecting yet, so only the other
/// refusals are worth a line.
fn send(shared: &Shared, command: Command) {
    // A let-chain rather than two nested `if`s: `clippy::collapsible_if` fires on the nesting, and the
    // gate for this phase is exactly the seven findings that predate it.
    if let Err(error) = shared.outbox.send(ToServer::Command(command))
        && !matches!(error, wire::WireError::NotOpen)
    {
        web_sys::console::warn_1(&JsValue::from_str(&format!("tipsy-web: {error}")));
    }
}

/// The action loop's other half: the native client's `on_action` (`client.rs:428-509`) with a socket
/// instead of a file.
///
/// Every arm is an `UiAction` this build has, and the two this build does not implement say so on
/// screen rather than only in a comment: `ToggleLive` has no browser implementation in this phase —
/// there is no realtime call on a web client, and no command to send — and the Server tab's
/// Regenerate row has no command in this protocol either, because `store::new_token` lives in the
/// server process. `LoginDone` is Task 3.7's, and the screen goes down here.
fn dispatch(action: UiAction, shared: &mut Shared) {
    let mut state = shared.state.borrow_mut();
    match action {
        UiAction::Send(command) => {
            // **The barge-in, and it is `Playback::on_command`'s, not this arm's.** That method applies
            // `tipsy_ui::policy`'s three decisions — `flush_on_interrupt`, `interrupts_playback` and
            // `starts_turn` — in the native client's order (`client.rs:458-464`), which is the plan's
            // five lines as one call. Re-deriving the `&&` here is exactly what Task 3.4 removed.
            let now = audio_now(&shared.audio).unwrap_or(0.0);
            web_audio::silence(&shared.playback.on_command(&command, now));
            send(shared, command);
        }
        // There is no terminal to restore in a browser: the socket closes, nothing reconnects, and the
        // frame on screen stays (`socket.rs`'s `Outbox::close`).
        UiAction::Quit => {
            shared.outbox.close();
            shared.quit = true;
        }
        // `client.rs:468-474`: what `Replay` replays is the reply the client kept, whatever entry the
        // HUD selected — the native client ignores the index too. Task 3.4 owns the audio and the
        // timeline, and a `Replay` that is audible does not touch the sequence position.
        UiAction::Replay(_) => {
            let Some(now) = audio_now(&shared.audio) else {
                // No press yet, so no context: there is nothing to play through and nothing kept.
                return;
            };
            if let Some(scheduled) = shared.playback.replay(now) {
                present(&mut shared.playback, &shared.audio, now, std::slice::from_ref(&scheduled));
            }
        }
        // F9 and the gear. There is no `config.toml` in a browser, so the screen is read over the wire
        // — Phase 1's `GetConfig` (spec §4.2, §6.2: "Reading: `GetConfig` on open"). The screen is
        // already up, on the placeholder form `toggle_config_form` put there (`state.rs:271-281`); the
        // answer is `Event::Config`, which `route` below turns into `show_config_form`
        // (`state.rs:287-291`), and *that* is what marks the form filled and so saveable.
        UiAction::OpenConfig => send(shared, Command::GetConfig),
        // Every other screen action goes through the form's own reducer, exactly as it does natively
        // (`client.rs:485-503`), and the effects it returns are what the browser has to do about it.
        UiAction::Config(action) => {
            let effects = match state.config_form.as_mut() {
                Some(form) => form.apply(action),
                None => Vec::new(),
            };
            for effect in effects {
                match effect {
                    // **The save, over the wire, as a shape rather than a check.** `Command::
                    // UpdateConfig` carries `Box<ClientConfigView>` and not a `ConfigView` (spec §4.3):
                    // a client may edit this configuration and may not move a provider's `base_url`, a
                    // provider's `key_name` or the server's `token`, so the conversion used here is the
                    // one that cannot spell them. The plan's Step 4 writes `ConfigView::from(form)` for
                    // this arm, which does not compile against this tree's protocol.
                    ConfigEffect::Save => {
                        if let Some(form) = state.config_form.as_ref() {
                            let config = ClientConfigView::from(form);
                            send(shared, Command::UpdateConfig { config: Box::new(config) });
                        }
                    }
                    // The probe runs against the server's providers (spec §5.2) and its answer is
                    // `Event::TestResult`, which `route` puts on the screen's notice line. It says
                    // nothing out loud, so it must not quiet a reply being spoken — and rather than
                    // assume that, the decision is `hud::test_barge_in`'s, which is
                    // `policy::test_interrupts_playback` joined with `policy::flush_on_interrupt` the
                    // way the native client joins them (`client.rs:496-500`).
                    ConfigEffect::Test(capability) => {
                        let now = audio_now(&shared.audio).unwrap_or(0.0);
                        web_audio::silence(&hud::test_barge_in(&mut shared.playback, capability, now));
                        send(shared, Command::TestProvider { capability });
                    }
                    // No command in the protocol this plan fixes mints a token: §4.2 lists `GetConfig`,
                    // `UpdateConfig` and `SetKey`, and `store::new_token` (`config/store.rs:28-48`)
                    // lives in the server process. The browser therefore cannot regenerate one, and
                    // this arm must NOT invent a command or mint a token client-side. **The row now
                    // says so** (the phase review's nit): the text goes on the screen's own notice
                    // line, which is the only line visible while the screen has the keyboard — the
                    // HUD's notice is behind it.
                    ConfigEffect::RegenerateToken => {
                        let text = state.strings().cfg_regenerate_native.to_string();
                        if let Some(form) = state.config_form.as_mut() {
                            form.notice = Some((NoticeKind::Warning, text));
                        }
                    }
                    ConfigEffect::None => {}
                }
            }
        }
        // F10 has no browser meaning in this phase: `tipsy-ui` maps the key to `ToggleLive` (Plan 2C's
        // indicator), and a web client has no realtime socket to open, no `Command` that would open
        // one and nothing for an indicator to report. The arm is inert rather than unreachable — the
        // key is the HUD's, and swallowing it is the honest answer — but **it now says so** (the phase
        // review's nit): an inert key that explains itself is not the same as one that does nothing.
        UiAction::ToggleLive => {
            let text = state.strings().live_web_unavailable;
            state.notify(text);
        }
        // The door opened and the user pressed `Enter`: the screen goes down. The capture went down
        // before it — a pass closes `LoginState::open` in the reducer
        // (`crates/tipsy-ui/src/state.rs:607`), so the frame loop's `Door::sync` already sent the
        // `LoginStop`, and a `Door` that tracked the capture itself would be the second copy this
        // branch removed. Nothing here remembers the pass: `login_seen` was set when `show_login`
        // showed the screen, which is what keeps a re-sent `Welcome` from putting the veil back.
        UiAction::LoginDone => shared.door.admitted(&mut state),
    }
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
/// **The door is offered every event, after the reducer has had it** (Task 3.7): `Door::note_event`
/// acts on `Welcome.login` alone and declines the rest, and the two login events that carry data —
/// `LoginHeard` and `LoginVerdict` — are `apply`'s, not the door's. There is no `LoginLevel` event in
/// this build, and the login's events have **no** equivalent of the gap Task 3.5 found in the
/// configuration pair: `state.rs` handles all three of them.
///
/// The order inside one message is the native client's `on_message`: the announced rate first, then the
/// audio it belongs to, and the turn boundaries where its own bookkeeping sits.
///
/// **The configuration screen's two answers are this function's, and the plan believed they were
/// `UiState::apply`'s.** They are not, and the tree says so twice: `state.rs:633` is
/// `Event::Config { .. } => {}` and `state.rs:552` is `Event::TestResult { .. } => {}`, both
/// deliberately empty because `tipsy-ui` owns no file, no connector and no token. The native client is
/// what renders them (`client.rs:369-391`), and this is the browser's half of the same job — without
/// it the screen Task 3.5's own `UiAction::OpenConfig` opens can never be filled and so can never be
/// saved (`ConfigForm`'s `filled` flag is what `Save` checks, `config_form.rs:712-721`).
fn route(
    queue: &mut VecDeque<ToClient>,
    playback: &mut playback::Playback,
    door: &mut login_door::Door,
    state: &mut UiState,
    audio_now: Option<f64>,
) -> (bool, Vec<playback::Scheduled>) {
    let mut welcomed = false;
    let mut play = Vec::new();
    while let Some(message) = queue.pop_front() {
        match message {
            ToClient::Event(event) => {
                // `policy::speech_rate` is the native client's own rule and the reason it lives in
                // `tipsy-ui` at all: the rate is set *before* the audio it belongs to, and this is the
                // browser's half of that order.
                if let Some(rate) = policy::speech_rate(&event) {
                    playback.set_rate(rate);
                }
                // The three turn boundaries. `end_of_turn` touches no node: `Done` and `Idle` arrive
                // while the device is still speaking the last sentence, so the audio of the turn that
                // just ended keeps playing.
                if ends_turn(&event) {
                    playback.end_of_turn();
                }
                // The screen's two answers, exactly as the native client's `on_message` renders them.
                apply_screen_answer(state, &event);
                welcomed |= matches!(event, Event::Welcome { .. });
                // **The reducer first, the door second.** `state.apply` is what moves every phase,
                // refusal and verdict — its `LoginHeard` and `LoginVerdict` arms are its own
                // (`crates/tipsy-ui/src/state.rs:590-629`) — and the one thing it deliberately does
                // not do is show the screen: its `Welcome` arm destructures `login` away (`:475`), so
                // `Door::note_event` is the only thing that can put one up, and `show_login` is itself
                // the once-per-run rule (`:293-314`) rather than a flag out here.
                state.apply(&event);
                door.note_event(state, &event);
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

/// The two messages the configuration screen is waiting for, rendered into the form.
///
/// Both are the client's business and not `UiState::apply`'s, because `tipsy-ui` owns no file, no
/// connector and no credential store: `apply`'s `Event::Config` and `Event::TestResult` arms are empty
/// on purpose (`state.rs:552`, `:633`). The native client renders them in `on_message`
/// (`client.rs:369-391`), and the browser needs the same two lines or the screen it opens is a
/// placeholder nobody can fill or save.
///
/// `show_config_form` is what marks the form filled (`state.rs:287-291`), which is the flag
/// `ConfigEffect::Save` checks — so a `Test` result for a screen that is not up is quietly dropped
/// rather than opening a form nobody asked for.
fn apply_screen_answer(state: &mut UiState, event: &Event) {
    match event {
        Event::Config { config } => state.show_config_form(ConfigForm::from(config.as_ref().clone())),
        Event::TestResult { capability, ok, latency_ms, message } => {
            if let Some(form) = state.config_form.as_mut() {
                form.test_result(*capability, *ok, *latency_ms, message.clone());
            }
        }
        _ => {}
    }
}

/// The HUD's clock, from the page's own `Date`.
///
/// It reads six numbers off the browser and hands them to [`hud::clock_from`], which is the host-tested
/// half: the mapping that is not obvious is the weekday, because `Date.getDay()` numbers from Sunday
/// and the HUD numbers from Monday. `client.rs:566-576` is the native twin, and it is *not* shared: the
/// native one reads `chrono::Local`, this one reads `js_sys::Date`, and only the numbering is common.
///
/// The `Date` getters are the one part no host test can reach — `js_sys::Date::new_0()` panics outside
/// wasm — which is why the arithmetic is not in here.
fn clock_view() -> ClockView {
    let now = js_sys::Date::new_0();
    hud::clock_from(
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds(),
        now.get_day(),
        now.get_date(),
        now.get_month(),
    )
}

/// `Date::get_day()`: `0` is Sunday. `ClockView::weekday`: `0` is Monday, and `tipsy-ui`'s weekday
/// table starts at `lunes`/`Monday`. [`hud::clock_from`] is its only production caller.
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

    /// A door no event has reached, for the tests below that are about the timeline, the header and
    /// the configuration screen: Task 3.7's door is offered every event by `route`, and a `Welcome`
    /// without a login screen must leave it exactly where it was.
    fn closed_door() -> login_door::Door {
        login_door::Door::new()
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

        let (welcomed, play) = route(&mut queue, &mut playback, &mut closed_door(), &mut state, None);

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
        let (_, play) = route(&mut first, &mut playback, &mut closed_door(), &mut state, Some(0.0));
        assert_eq!(play.len(), 1);
        assert_eq!(play[0].rate, 16_000, "the buffer is created at the announced rate");
        assert_eq!(play[0].at, playback::CUSHION_S);

        let mut second = VecDeque::from(vec![
            ToClient::Event(Event::SpeechChunk { seq: 1, sample_rate: 16_000 }),
            // 8 000 samples = half a second at 16 kHz.
            ToClient::Audio(Frame::Speech { seq: 1, samples: vec![0i16; 8_000] }),
        ]);
        let (_, play) = route(&mut second, &mut playback, &mut closed_door(), &mut state, Some(0.0));
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

        let (_, play) = route(&mut queue, &mut playback, &mut closed_door(), &mut state, Some(5.0));

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
            let (_, scheduled) = route(&mut queue, &mut playback, &mut closed_door(), &mut state, Some(0.0));
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
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.target-sandbox/live-34/frames.txt");
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

    /// **The screen's two answers, which the plan believed `UiState::apply` already handled.** It does
    /// not — `state.rs:633` and `:552` are empty arms on purpose, because `tipsy-ui` owns no file and no
    /// connector — and without this routing the screen `UiAction::OpenConfig` opens is a placeholder
    /// that can never be filled and therefore never saved (`config_form.rs:712-721` refuses an unfilled
    /// form). The native client renders both in `on_message` (`client.rs:369-391`); this is that half
    /// on the browser's side of the socket.
    #[test]
    fn the_configuration_answer_fills_the_form_and_is_what_makes_it_saveable() {
        let mut state = UiState::new();
        state.apply(&welcome());
        // The gear's path, raised by the state itself: `toggle_config_form` puts the placeholder up and
        // asks the client for the real thing.
        assert_eq!(state.on_key(tipsy_ui::KeyInput::F(9)), vec![UiAction::OpenConfig]);
        assert!(
            !state.config_form.as_ref().expect("the placeholder is up").filled(),
            "and the placeholder cannot be saved: `Save` refuses it"
        );

        // The server's answer, in the shape the socket hands it over.
        let mut served = ConfigForm::default();
        served.active_profile = "meigo".into();
        let answer = Event::Config { config: Box::new(tipsy_protocol::ConfigView::from(&served)) };
        apply_screen_answer(&mut state, &answer);

        let form = state.config_form.as_ref().expect("the screen is still up");
        assert!(form.filled(), "the answer is what marks the form saveable");
        assert_eq!(form.active_profile, "meigo", "and it carries the server's own values");
        assert_eq!(
            state.config_form.as_mut().unwrap().apply(tipsy_ui::ConfigAction::Save),
            vec![ConfigEffect::Save],
            "so `Ctrl+S` now raises the effect `dispatch` sends as `UpdateConfig`"
        );
    }

    /// The probe's answer goes to the screen's notice line and nowhere else — including the case where
    /// there is no screen, which must stay that way rather than opening one.
    #[test]
    fn a_test_result_lands_on_the_screen_and_opens_nothing_when_there_is_none() {
        let mut state = UiState::new();
        state.apply(&welcome());
        let result = Event::TestResult {
            capability: tipsy_protocol::Capability::Chat,
            ok: true,
            latency_ms: 42,
            message: "ok".into(),
        };

        apply_screen_answer(&mut state, &result);
        assert!(state.config_form.is_none(), "a probe for a screen that is not up opens nothing");

        state.show_config_form(ConfigForm::default());
        apply_screen_answer(&mut state, &result);
        let (_, text) =
            state.config_form.as_ref().and_then(|form| form.notice.clone()).expect("the answer is on the notice line");
        assert!(text.contains("42"), "and the latency is what the row reports: {text}");
    }

    /// **The page, as a file, because there is no browser on this machine to load it.** Two of the
    /// document's facts are load-bearing for tasks that have already run:
    ///
    /// - the bundle's path, the join between this page and Task 3.6's `--out-dir web/dist`. Task 3.2
    ///   measured `/assets/dist/tipsy_web.js` answering 200 with the size of the file on disk once
    ///   `wasm-bindgen` had run, and 404 before it;
    /// - **no positioned overlay over the canvas**, the trap the spike measured twice
    ///   (`ratzilla-spike-verdict.md` §5.4 point 2) and which the plan's own Step 5 walks into: its
    ///   `#hud { position: absolute; inset: 0 }` is an empty div covering the viewport, and a
    ///   positioned sibling paints *and hit-tests* above the static canvas — every click the HUD's
    ///   `hit_at` waits for would land on the div instead.
    ///
    /// It asks about the **file** and not about a directory holding it (the Phase 2 lesson), and it
    /// does not skip: `web/client/index.html` is committed, so a checkout that lacks it has to fail
    /// here rather than pass quietly. The one coupling it accepts is the gate's id: that string is
    /// Task 3.3's `mic.rs:91`, and a rename there is meant to break this test until the page follows.
    #[test]
    fn the_page_loads_the_bundle_the_dist_step_writes_and_puts_nothing_over_the_canvas() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/client/index.html");
        let page = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        // A commented-out rule is not a rule, and this page's own comment *names* the trap this test
        // exists for — so the two assertions below read the document with its comments removed. The
        // rest read it verbatim, which is where the import and the script tag have to be.
        let live = without_comments(&page);

        assert!(
            page.contains(r#"import init from "/assets/dist/tipsy_web.js";"#),
            "the page must import the file `wasm-bindgen --target web --out-dir web/dist` writes"
        );
        assert!(page.contains(r#"<script type="module">"#), "a bare `import` needs a module script");
        assert!(!page.contains("fetch("), "the page fetches nothing at run time (spec §10)");
        assert!(
            live.contains("html.floating canvas { width: 100%; height: 100%; }")
                && live.contains("get(\"floating\") === \"1\""),
            "the floating canvas must follow its resized viewport"
        );

        assert!(
            !live.contains(r#"id="hud""#),
            "the plan's `#hud` div is a second element the backend does not use: its canvas is appended \
             to `<body>` and there is nothing for the div to hold"
        );
        assert!(
            !live.contains("position: absolute"),
            "no element may be positioned over the canvas: a positioned sibling hit-tests above it and \
             would swallow every click (spike §5.4 point 2)"
        );
        assert!(
            live.contains("#tipsy-gate") && live.contains("position: fixed"),
            "the gate Task 3.3's `gate_elements` appends to `<body>` is styled here, and it is the one \
             positioned element — and it is removed once the microphone is open"
        );
        assert!(page.contains("monospace"), "the stack ends in a generic family to fall back to");
    }

    /// **The header a real server announced, clicked — on the profile the server sent.**
    ///
    /// `.target-sandbox/live-35.ps1` (a real `serve`, a stub provider and a real `ClientWebSocket`)
    /// captures the `Welcome` text frame to `live-35/welcome.json`. This decodes it with the client's own
    /// `wire::event_from_text`, routes it through the same `route` the frame loop calls, draws a real
    /// frame and clicks the gear's own cell through `hud::drain` — the whole glue minus Ratzilla and the
    /// DOM. **The profile is the server's, so the badge's drawn width is the server's too**, which is
    /// the one thing the plan's font section was worried about and the one thing a hand-written fixture
    /// cannot pin: `render.rs` measures every column of that row because the title is the profile's own
    /// (`render.rs:211-214`).
    ///
    /// It asks about the **file**, not about a directory holding it — the Phase 2 lesson, where
    /// `.is_dir()` was true in a fresh clone and the skip became dead code. A checkout that has never run
    /// the live check skips this and the reason is printed (visible under `cargo test -- --nocapture`);
    /// both sides are recorded in the ledger.
    #[test]
    fn the_header_a_real_server_announced_is_clickable_on_this_client() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.target-sandbox/live-35/welcome.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            println!(
                "skipping `{}`: it is not there. Run `.target-sandbox/live-35.ps1` (a real `serve`, a \
                 stub provider and a real WebSocket) to produce it — the assertions below need a \
                 `Welcome` a real server sent, profile and all.",
                path.display()
            );
            return;
        };

        // The client's own decoder, on the bytes a real server sent.
        let event = wire::event_from_text(text.trim()).expect("the live `Welcome` decodes");
        let announced = match &event {
            Event::Welcome { profile, .. } => profile.name.clone(),
            other => panic!("the live check captured {other:?}, not a `Welcome`"),
        };

        let mut state = UiState::new();
        let mut playback = playback::Playback::new();
        let mut queue = VecDeque::from(vec![ToClient::Event(event)]);
        // The call the frame loop makes, with no audio context: the events reach the HUD either way.
        let (welcomed, scheduled) = route(&mut queue, &mut playback, &mut closed_door(), &mut state, None);
        assert!(welcomed, "`route` reports the `Welcome` the restored-connection notice is said on");
        assert!(scheduled.is_empty() && queue.is_empty(), "and nothing is scheduled before a press");

        let title = state.profile.as_ref().expect("the `Welcome` reached the HUD").name.clone();
        assert_eq!(title, announced, "the badge carries the server's own profile name");

        let clock = hud::clock_from(17, 41, 3, 0, 11, 9);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).expect("a test terminal");
        let mut hits = Vec::new();
        terminal.draw(|frame| hits = tipsy_ui::draw(frame, &state, &clock, &title)).expect("one frame");

        assert!(hits.len() >= 2, "the header reports its two controls first: {hits:?}");
        let (mode, gear) = (hits[0], hits[1]);
        assert_eq!(mode.key, tipsy_ui::KeyInput::F(9), "the mode word is the door key: {mode:?}");
        assert_eq!(gear.key, tipsy_ui::KeyInput::F(9), "and so is the gear: {gear:?}");
        assert_eq!(mode.area.right(), gear.area.x, "the mode ends where the gear begins");
        assert_eq!(
            gear.area.right(),
            120,
            "the gear is flush against the right edge whatever the profile's own name measures"
        );
        assert!(mode.area.width > 0 && gear.area.width > 0, "both are clickable: {mode:?} {gear:?}");

        // The whole click path, on the server's own profile.
        let mut queued = vec![hud::Input::Click { col: gear.area.x, row: 0 }];
        assert_eq!(hud::drain(&mut state, &hits, &mut queued), vec![UiAction::OpenConfig]);
        assert!(state.config_form.is_some(), "the door is open on the real profile's header");
    }

    /// One frame `.target-sandbox/live-37.ps1` captured, or a printed reason and `None`.
    ///
    /// **It asks about the file and not about a directory holding it** — the Phase 2 lesson, where
    /// `.is_dir()` was true in a fresh clone because a `README.md` was committed and the skip became
    /// dead code. Every path it hands back is a message a real server sent over a real socket.
    fn live_37(name: &str) -> Option<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.target-sandbox/live-37").join(name);
        match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(_) => {
                println!(
                    "skipping `{}`: it is not there. Run `.target-sandbox/live-37.ps1` (a real `serve` \
                     with `[login] enabled` and an allowed entry, a stub provider and a real WebSocket) \
                     to produce it — the assertions below need the login frames a real server sent.",
                    path.display()
                );
                None
            }
        }
    }

    /// **The login door a real server asked for, opened and closed on this client.**
    ///
    /// `.target-sandbox/live-37.ps1` captures the `Welcome` a real `serve` sends with `[login] enabled`
    /// and an allowed entry (`live-37/welcome.json`) and the `LoginVerdict` it answers a typed
    /// `LoginAnswer` with (`live-37/verdict.json`). Both are decoded with the client's own
    /// `wire::event_from_text` and routed through the same `route` the frame loop calls, with a real
    /// [`login_door::Door`] — so the screen, the capture's two edges and the frame gate are exercised on
    /// an announcement **this client did not compose**. It is the whole host-side lifecycle of the door
    /// minus Ratzilla and the DOM, and it is what a hand-written fixture cannot pin: that the server's
    /// own `voice` flag is what opens the capture, and that its own verdict is what closes it.
    #[test]
    fn the_login_door_a_real_server_asked_for_opens_and_closes_on_this_client() {
        let (Some(announced), Some(ruling)) = (live_37("welcome.json"), live_37("verdict.json")) else {
            return;
        };

        let welcome = wire::event_from_text(announced.trim()).expect("the live `Welcome` decodes");
        let view = match &welcome {
            Event::Welcome { login: Some(view), .. } => view.clone(),
            other => panic!("the live check captured {other:?}, not a `Welcome` with a login screen"),
        };
        assert!(view.voice, "the live check's `[login] voice = true` is what the server announces");

        let mut state = UiState::new();
        let mut door = login_door::Door::new();
        let mut playback = playback::Playback::new();
        let mut queue = VecDeque::from(vec![ToClient::Event(welcome)]);
        let (welcomed, scheduled) = route(&mut queue, &mut playback, &mut door, &mut state, None);
        assert!(welcomed, "`route` reports the `Welcome` the restored-connection notice is said on");
        assert!(scheduled.is_empty() && queue.is_empty(), "and nothing is scheduled before a press");

        // The screen, from the server's own announcement.
        let screen = state.login.as_ref().expect("the server asked for a login screen");
        assert!(screen.voice, "the switch is the server's, not this client's opinion of it");
        assert!(screen.open, "and with the voice on the capture opens with the screen");
        assert_eq!(screen.phase, tipsy_ui::LoginPhase::Listening, "which is the phase the keys read");
        assert!(door.sends_frames(&state), "so a frame may leave the page");
        assert_eq!(door.sync(&state), login_door::MicStep::Start, "and the server is told, once");
        assert_eq!(door.sync(&state), login_door::MicStep::None, "and never twice");

        // The ruling a real server sent for a typed answer: the door opens, the capture closes.
        let verdict = wire::event_from_text(ruling.trim()).expect("the live `LoginVerdict` decodes");
        assert!(
            matches!(verdict, Event::LoginVerdict { ok: true, .. }),
            "the live answer carried the configured keywords: {verdict:?}"
        );
        let mut queue = VecDeque::from(vec![ToClient::Event(verdict)]);
        route(&mut queue, &mut playback, &mut door, &mut state, None);

        let screen = state.login.as_ref().expect("the screen stays until `Enter`, spec §6.2");
        assert_eq!(screen.phase, tipsy_ui::LoginPhase::Passed, "the verdict is what opens the door");
        assert!(!screen.open, "and the reducer closed the capture the moment it did");
        assert!(!door.sends_frames(&state), "so no frame may leave the page any more");
        assert!(!door.capture_open(&state), "and the `● rec` badge is gone with it");
        assert_eq!(door.sync(&state), login_door::MicStep::Stop, "which is the `LoginStop` §6.2 asks for");

        // `Enter`, then the screen goes — and the run remembers, so a re-sent `Welcome` cannot put the
        // veil back over a HUD the user is already inside.
        assert_eq!(
            state.on_key(tipsy_ui::KeyInput::Enter),
            vec![UiAction::LoginDone],
            "the screen the server asked for is entered with the key the state table names"
        );
        door.admitted(&mut state);
        assert!(state.login.is_none() && state.login_seen, "the door is behind us and stays behind us");
        assert!(!door.note_event(&mut state, &welcome_like()), "and a later announcement opens nothing");
    }

    /// A `Welcome` with a login screen, for the one assertion above that needs a second announcement.
    fn welcome_like() -> Event {
        Event::Welcome {
            session_id: 2,
            profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
            ui_language: UiLanguage::Es,
            listening_mode: ListeningMode::Push,
            web_bundle: true,
            login: Some(tipsy_protocol::LoginView { voice: true }),
        }
    }

    /// A [`Shared`] for the host: every field but the socket is buildable in a test, and the socket is
    /// a handle with nothing behind it ([`socket::Outbox::for_tests`]). It exists for the one
    /// `dispatch` arm that is otherwise unreachable without a browser.
    fn shared_for_tests() -> Shared {
        Shared {
            state: Rc::new(RefCell::new(UiState::new())),
            hits: Vec::new(),
            queued: Vec::new(),
            clock: hud::clock_from(0, 0, 0, 0, 0, 0),
            title: String::new(),
            playback: playback::Playback::new(),
            outbox: socket::Outbox::for_tests(),
            capture: Rc::new(RefCell::new(None)),
            audio: Rc::new(RefCell::new(None)),
            mic_failure: Rc::new(RefCell::new(None)),
            mic_gain: mic::MicGain::default(),
            queue: Rc::new(RefCell::new(VecDeque::new())),
            door: login_door::Door::new(),
            quit: false,
        }
    }

    /// **`Enter` on a passed screen is `UiAction::LoginDone`, and this drives the arm that answers it.**
    ///
    /// The live-fixture test above drives `Door::admitted` directly; this goes through [`dispatch`],
    /// which is what the frame loop actually calls — and that arm is the one line in this file nothing
    /// else could see. It is measured rather than asserted: with the arm's body emptied, this test is
    /// the only one in the crate that goes red (the ledger's M7, which was void before this test
    /// existed and is the reason `Outbox::for_tests` does).
    #[test]
    fn enter_on_a_passed_screen_takes_the_screen_down_through_dispatch() {
        let mut shared = shared_for_tests();
        let announced = welcome_like();
        let view = match &announced {
            Event::Welcome { login: Some(view), .. } => view.clone(),
            other => panic!("the fixture is a `Welcome` with a login screen: {other:?}"),
        };
        // The screen, exactly as `route` puts it up: the reducer first, the door second.
        shared.state.borrow_mut().apply(&announced);
        {
            let mut state = shared.state.borrow_mut();
            assert!(shared.door.open_screen(&mut state, Some(&view)));
        }

        let pass = Event::LoginVerdict { ok: true, name: Some("Huésped".into()) };
        shared.state.borrow_mut().apply(&pass);
        assert_eq!(
            shared.state.borrow_mut().on_key(tipsy_ui::KeyInput::Enter),
            vec![UiAction::LoginDone],
            "the key the screen's own table maps once the door is open"
        );

        dispatch(UiAction::LoginDone, &mut shared);
        assert!(shared.state.borrow().login.is_none(), "the arm takes the screen down");
        assert!(shared.state.borrow().login_seen, "and the run still remembers it was asked");
    }

    /// **The two keys this build does not implement now say so where the user is looking** (the phase
    /// review's nit). `F10`'s `ToggleLive` and the Server tab's `Ctrl+R` were inert with a comment in
    /// the code and nothing on screen. [`dispatch`] is the only caller either arm has, so both are
    /// reachable from a host test — which is also why this is the test that fails if either goes back
    /// to being silent.
    #[test]
    fn the_two_keys_this_build_does_not_implement_say_so() {
        let es = tipsy_ui::strings(tipsy_protocol::UiLanguage::Es);
        let mut shared = shared_for_tests();

        // The token row: the configuration screen's own notice line, because the HUD's line is behind
        // the full-screen form.
        shared.state.borrow_mut().show_config_form(ConfigForm::default());
        dispatch(UiAction::Config(tipsy_ui::ConfigAction::RegenerateToken), &mut shared);
        let notice = shared
            .state
            .borrow()
            .config_form
            .as_ref()
            .and_then(|form| form.notice.clone())
            .expect("the row explains itself on the screen's notice line");
        assert_eq!(notice.1, es.cfg_regenerate_native);
        assert_eq!(notice.0, NoticeKind::Warning, "it is not an error: it is a row that is not here");

        // And `F10` on the HUD: the notice line the HUD draws, and no call started.
        dispatch(UiAction::ToggleLive, &mut shared);
        assert_eq!(shared.state.borrow().notice.as_ref().map(|(text, _)| text.as_str()), Some(es.live_web_unavailable));
        assert!(!shared.state.borrow().live, "and it starts nothing, which is the point");
    }

    /// **The reply's clock is the context the press made, not the capture's** (the phase review's
    /// finding 4). `audio_now` and `present` are called from inside `draw_web`'s callback, which needs a
    /// real browser, so no host test can run them — and the decision they carry is *which slot the
    /// `AudioContext` lives in*, which is exactly the decision that made playback impossible without a
    /// successful capture. This reads the committed source the way `mic.rs` pins its one send site, and
    /// it is falsifiable: pointing either call back at the capture fails it.
    #[test]
    fn the_reply_is_scheduled_on_the_context_the_press_made_and_not_on_the_capture() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("lib.rs");
        let source = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let production = &source[..source.find("#[cfg(test)]").expect("this module follows")];
        for call in ["let audio_now = audio_now(&shared.audio);", "present(&mut shared.playback, &shared.audio"] {
            assert!(production.contains(call), "the clock is the run's audio slot: `{call}` is missing");
        }
        assert!(
            !production.contains("audio_now(&shared.capture)")
                && !production.contains("present(&mut shared.playback, &shared.capture"),
            "and it is never the capture's context: a refused microphone must still play the greeting \
             and every reply (finding 4)"
        );
    }

    /// An HTML document with its HTML comments and its CSS block comments removed.
    ///
    /// The page's own comments *name* the trap the two assertions above exist for — that is what a
    /// comment is for — and a commented-out rule is not a rule. `-->` inside an HTML comment is not a
    /// thing and there is no nesting; a `*/` inside a CSS comment is not a thing either, and the only
    /// `/*` in this document is the one opening each block comment.
    fn without_comments(html: &str) -> String {
        let stripped = strip(html, "<!--", "-->");
        strip(&stripped, "/*", "*/")
    }

    fn strip(text: &str, open: &str, close: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find(open) {
            out.push_str(&rest[..start]);
            match rest[start..].find(close) {
                Some(end) => rest = &rest[start + end + close.len()..],
                None => return out,
            }
        }
        out.push_str(rest);
        out
    }
}
