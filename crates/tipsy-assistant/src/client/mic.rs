//! The microphone: the device, the bounded queue behind it, and the login capture's edges.
//!
//! Three things live here and nothing else does. The **device** is opened and let go by `sync_mic`,
//! which is the only place `Mic::start` is called and the only place a stream that refuses to open is
//! turned into something the user can read — on the login screen's own line when that screen is up, and
//! in the HUD's notice otherwise. The **queue** is the channel the device thread writes into and the
//! frame loop drains, once a frame, in `drain_mic`. The **login capture's edges** are `login_capture`,
//! `sync_login_mic` and `close_login_mic`: the screen's state is the authority, and the server's
//! `LoginStart` / `LoginStop` are sent on the edges of that state and never on its value.
//!
//! The decisions themselves are not here. `tipsy_ui::policy` owns `mic_route`/`MicRoute` and
//! `mic_wanted`, because the browser client compiles against `tipsy-ui` and must reach the same ones;
//! this file is what applies them to a real device on a real thread.

use super::*;
// `mic_route`/`MicRoute` arrive through the parent's own import, which the glob above carries in;
// `mic_wanted` is not the parent's to name any more, so the child imports it itself.
use tipsy_ui::policy::mic_wanted;

impl Client {
    /// Forwards every captured frame and shows its loudness in the oscilloscope.
    ///
    /// Every frame goes to exactly one place, and [`mic_route`] is where that is decided: the login's
    /// own capture first — it is the only thing on screen, its answer is spoken rather than typed, and
    /// a live call is not what is being answered — then a live call, then the session's listening
    /// path. With no stream open there is nothing to route at all, which is the whole of the fix: a
    /// queue that emptied before `Mic::start` succeeded used to send audio the spec promises never
    /// leaves the machine (spec §9, §10).
    pub(super) fn drain_mic(&mut self, to_server: &UnboundedSender<ToServer>) {
        let route = mic_route(self.mic.is_some(), self.login_capture(), self.call.is_some());
        while let Ok(samples) = self.mic_frames.try_recv() {
            match (route, &self.call) {
                // A call owns the microphone while it is live: the session's own
                // `ListenStart`/`ListenStop` bookkeeping must not see frames it never asked for.
                (MicRoute::Call, Some(call)) => call.push_mic(samples.clone()),
                (MicRoute::Session, _) => {
                    let _ = to_server.send(ToServer::Audio(Frame::Mic(samples.clone())));
                }
                // No capture is running — or the call this frame was routed to is gone, which is the
                // same thing one frame later: the frame is dropped rather than kept, exactly as it is
                // dropped when the HUD's own microphone is shut.
                (MicRoute::Drop | MicRoute::Call, _) => {}
            }
            self.state.push_mic_level(rms(&i16_to_f32(&samples)) * self.mic_gain);
        }
    }

    /// True while a login capture should be running: the screen is up, it has a voice switch, and its
    /// state says the microphone is open. That state is the whole of the authority — it is what the
    /// third refusal and a failed answer lower — so this is a read and never a decision, and it says
    /// nothing about the user's own switch or about the device.
    pub(super) fn login_capture(&self) -> bool {
        self.state.login.as_ref().is_some_and(|login| login.voice && login.open)
    }

    /// Reconciles the microphone with [`Client::login_capture`]: the server's own `LoginStart` /
    /// `LoginStop` are sent **on the edges** of that state, never on its value, because the two are
    /// not the same thing any more than a key press is.
    ///
    /// The reducer cannot raise an action from `apply` — a refusal that stops listening is expressed
    /// as `LoginState::open == false` and nothing else — so the client reads that state once a frame
    /// and tells the server what it is doing. It touches nothing of the user's: `state.mic_open` is
    /// the HUD's own switch, and this screen has no business writing it (spec §6.2, review finding 3).
    /// It runs before `sync_mic`, so the device a `LoginStart` asks for is opened before any frame is
    /// routed (`drain_mic`).
    pub(super) fn sync_login_mic(&mut self, to_server: &UnboundedSender<ToServer>) {
        // `login_mic` is the request in flight, not the capture: a `LoginStart` that has gone out has
        // to be answered by a `LoginStop` even if the capture has already given up on it.
        if !self.login_mic && self.login_capture() {
            if to_server.send(ToServer::Command(Command::LoginStart)).is_ok() {
                self.login_mic = true;
            }
        } else if self.login_mic && !self.login_capture() {
            self.close_login_mic(to_server);
        }
    }

    /// Ends the login capture: the server is told to stop, and the device is let go by the next
    /// `sync_mic` — which is now a step of its own, because the capture is no longer written into the
    /// user's microphone switch. Calling it with the capture already down is harmless, which is what
    /// lets `UiAction::LoginDone` and the reconciliation both end it.
    pub(super) fn close_login_mic(&mut self, to_server: &UnboundedSender<ToServer>) {
        self.login_mic = false;
        let _ = to_server.send(ToServer::Command(Command::LoginStop));
    }

    /// Opens or closes the microphone so it matches what wants it: the user's own switch, a live call
    /// that streams until it ends, or the login capture, which is the screen's own state.
    ///
    /// Returns whether a stream is open afterwards, which is what `drain_mic` routes on. A device that
    /// refuses to open also ends the login capture and says why on the screen's own line (spec §9):
    /// leaving it claiming a microphone it does not have is the other half of the same defect.
    pub(super) fn sync_mic(&mut self) -> bool {
        let wanted = mic_wanted(self.state.mic_open, self.call.is_some(), self.login_capture());
        match (wanted, self.mic.is_some()) {
            (true, false) => {
                let tx = self.mic_tx.clone();
                match Mic::start(move |samples| {
                    // Bounded and lossy: a busy client drops frames rather than stalling the device.
                    let _ = tx.try_send(samples);
                }) {
                    Ok(mic) => self.mic = Some(mic),
                    Err(e) => {
                        // Nothing to capture from; leave the HUD saying so.
                        self.state.set_mic_open(false);
                        let no_mic = self.state.strings().login_no_mic;
                        if self.state.login.is_some() {
                            // The screen's own line, because the HUD's notice is behind the veil and
                            // nobody can read it there (spec §9). `open = false` stops the capture and
                            // the client's own gate stops asking for a stream, so the request is retired
                            // here rather than with a `LoginStop`: the server's stop sets a flag, clears
                            // a buffer and cancels a token — it sends no event and touches no screen
                            // state — so a stop would be a round trip that changes nothing a user can
                            // see (the scoped re-review caught this comment claiming otherwise).
                            if let Some(login) = self.state.login.as_mut() {
                                login.line = LoginLine::Failed(no_mic.to_string());
                                login.open = false;
                            }
                            self.login_mic = false;
                        } else {
                            self.state.notify(e);
                        }
                    }
                }
            }
            (false, true) => self.mic = None,
            _ => {}
        }
        self.mic.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::LoginView;
    use tipsy_ui::LOGIN_AUTO_REARM;

    use crate::client::tests::{mock_client, sync_login, welcome};

    /// Audio frames queued for the client, as the microphone thread queues them.
    fn queue_frames(client: &Client, frames: &[&[i16]]) {
        for frame in frames {
            client.mic_tx.try_send(frame.to_vec()).expect("the queue is bounded but empty here");
        }
    }

    /// Review finding 1: the login's frames are routed by the state, but **the device is what gates
    /// them**. With no stream open there is nothing to drain, so a frame already sitting in the queue
    /// — what a screen that opens mid-capture meets — must not reach the server (spec §9's "no stream,
    /// no `LoginStart`" and §10's "nothing leaves until it is speech").
    ///
    /// This is the test the reviewer could not find: no test on this machine can open a device, and
    /// the whole of the failure path is reachable from that one fact.
    #[test]
    fn a_login_capture_with_no_stream_forwards_no_audio_at_all() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
        assert!(client.login_capture(), "the screen wants a capture");
        assert!(client.mic.is_none(), "and the device cannot be opened by a test");
        let wanted = mic_wanted(client.state.mic_open, false, client.login_capture());
        assert!(wanted, "the stream *is* wanted: this test is about the frame before it exists");
        queue_frames(&client, &[&[1, 2, 3], &[4, 5, 6]]);

        let (to_server, mut sent) = tokio::sync::mpsc::unbounded_channel();
        client.drain_mic(&to_server);

        assert!(sent.try_recv().is_err(), "a capture with no stream must forward no audio");
        assert!(client.mic_frames.try_recv().is_err(), "and the frames are not kept either");
    }

    /// Review finding 3: the login screen must not touch the switch the user set for themselves. A
    /// user who had `Tab`-toggled the microphone on before the door finds it exactly as they left it
    /// afterwards — the spec's own words are "unless the listening mode wants it open", and the switch
    /// is where that is written down.
    #[test]
    fn the_login_screen_leaves_the_huds_microphone_switch_alone() {
        for before in [false, true] {
            let mut client = mock_client();
            let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
            client.state.mic_open = before;
            client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
            assert_eq!(sync_login(&mut client), vec![Command::LoginStart], "before = {before}");
            assert_eq!(client.state.mic_open, before, "the screen does not touch the user's switch");

            // The door: the screen goes, the capture goes, and what the microphone returns to is the
            // switch's own value — the "unless the listening mode wants it open" the spec asks for.
            let (events, _results) = tokio::sync::mpsc::unbounded_channel();
            client.on_action(UiAction::LoginDone, &to_server, &events);
            client.sync_login_mic(&to_server);

            assert!(client.state.login.is_none(), "the screen is down");
            assert_eq!(client.state.mic_open, before, "and the user's switch is where they left it");
            assert_eq!(
                mic_wanted(client.state.mic_open, client.call.is_some(), client.login_capture()),
                before,
                "the device follows that switch, not the screen (before = {before})"
            );
            assert_eq!(commands.try_recv().unwrap(), ToServer::Command(Command::LoginStop));
            assert!(commands.try_recv().is_err(), "and nothing else was sent");
        }
    }

    /// The switch is the whole difference: with `voice = false` no stream is asked for at all, so
    /// nothing is recorded and no audio leaves the machine (spec §5, §10).
    #[test]
    fn the_voice_switch_off_opens_no_stream_and_sends_nothing() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: false }))));
        assert!(sync_login(&mut client).is_empty(), "there is no capture to start");
        assert!(!client.state.mic_open, "and no microphone to light the badge");
        assert!(client.mic.is_none(), "and no device open");
        // `Tab` has nothing to toggle, either: the screen's own state refuses it for the same reason.
        assert!(client.state.on_key(KeyInput::Tab).is_empty());
    }

    /// The capture is a **state the client reconciles** and not an action the reducer sends: a
    /// refusal that stops listening is expressed as `LoginState::open == false`, and this is where
    /// that becomes a `LoginStop` and a closed stream (spec §6.2).
    #[test]
    fn the_capture_opens_and_closes_with_the_login_state() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart]);
        // The three refusals in a row shut the capture from the screen's own state, and the client
        // follows it on the next reconciliation.
        for _ in 0..LOGIN_AUTO_REARM {
            client.on_message(&to_server, ToClient::Event(Event::LoginVerdict { ok: false, name: None }));
        }
        let login = client.state.login.as_ref().unwrap();
        assert!(!login.open, "the third refusal stops the automatic re-arm");
        assert_eq!(sync_login(&mut client), vec![Command::LoginStop], "and the server is told to stop");
        assert!(!client.state.mic_open, "and the badge goes out");
        assert!(client.mic.is_none(), "and the device is let go");
        // `Tab` is consent: it re-arms and the client follows it back the other way.
        assert_eq!(client.state.on_key(KeyInput::Tab), vec![UiAction::Send(Command::LoginStart)]);
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart]);
    }

    /// `UiAction::LoginDone` is the door: the screen goes, the capture stops, and the microphone
    /// returns to what the listening mode wanted **rather than staying open for the login** — and
    /// "what the listening mode wanted" is the user's own switch, which this fixture has on.
    #[test]
    fn the_door_closes_the_screen_and_the_capture() {
        let mut client = mock_client();
        client.state.mic_open = true;
        client.state.show_login(true);
        client.state.login.as_mut().unwrap().open = true;
        // The capture as the frame loop would have it: reconciled once, with no second start.
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart]);
        assert_eq!(sync_login(&mut client), Vec::<Command>::new(), "and never twice");

        let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::LoginDone, &to_server, &events);
        client.sync_login_mic(&to_server);

        assert!(client.state.login.is_none(), "the screen is down");
        assert!(
            client.state.mic_open,
            "the microphone returns to the switch the user set, not to off: review finding 3"
        );
        assert!(client.mic.is_none(), "no device in this fixture");
        assert_eq!(
            commands.try_recv().unwrap(),
            ToServer::Command(Command::LoginStop),
            "the server must know the capture is over"
        );
    }

    /// A pass ends the capture and tells the server so (spec §6.2). The state closes `open` on the
    /// verdict, and this reconciliation is what turns that falling edge into the `LoginStop`: without
    /// it the session's own capture stays active (`session.rs:346-349` routes frames to the login
    /// while it is), so a word said after "Pasa, Huésped." was still judged and could shut the door.
    #[test]
    fn a_pass_sends_login_stop_and_ends_the_capture() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(welcome(Some(LoginView { voice: true }))));
        assert!(client.login_capture(), "the capture opens with the screen");
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart], "and the server is told");

        client.on_message(&to_server, ToClient::Event(Event::LoginVerdict { ok: true, name: Some("Huésped".into()) }));

        assert!(!client.login_capture(), "the pass ends the capture");
        assert_eq!(sync_login(&mut client), vec![Command::LoginStop], "and the server hears about it");
        assert!(client.state.login.is_some(), "the screen stays until Enter, as the spec says");
    }

    /// A failed answer closes the capture — the microphone is off rather than repeating a failure —
    /// and the client follows the state, exactly as it does for the third refusal. What it does not
    /// touch is the user's own switch (review finding 3): the fixture has it on, and it stays on.
    #[test]
    fn a_failed_answer_turns_the_capture_off() {
        let mut client = mock_client();
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.state.mic_open = true;
        client.state.show_login(true);
        client.state.login.as_mut().unwrap().open = true;
        assert_eq!(sync_login(&mut client), vec![Command::LoginStart]);

        client.on_message(
            &to_server,
            ToClient::Event(Event::Error {
                code: tipsy_protocol::ErrorCode::NoKey,
                message: "no key named `openrouter`".into(),
                recoverable: true,
            }),
        );
        assert!(!client.state.login.as_ref().unwrap().open);
        assert_eq!(sync_login(&mut client), vec![Command::LoginStop]);
        assert!(client.state.mic_open, "the switch the user set is not the screen's to move");
        assert!(!client.login_capture(), "but the login's own capture is over");
        assert!(
            mic_wanted(client.state.mic_open, false, client.login_capture()),
            "and the microphone stays open for the reason the user opened it"
        );
    }
}
