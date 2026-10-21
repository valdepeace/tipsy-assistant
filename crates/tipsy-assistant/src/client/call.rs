//! The live realtime call's lifecycle: the socket's handle, and the one exit every way it dies
//! comes through.
//!
//! Four things live here and nothing else does. `toggle_call` is the key's own decision — `self.call`
//! is the client's handle on the socket, so it, and not the HUD's indicator, says which way `F10`
//! goes; `start_call` opens the socket or explains why it will not, with the key resolved *here* so a
//! name nobody holds is a sentence on the key press and not an asynchronous failure; `end_call` lets
//! the handle go and silences the device, because the audio still queued belongs to a conversation
//! that is gone; and `call_ended` is the rule that decides when that happens — a call's ending is a
//! live-flag event and nothing else.
//!
//! `sync_call` is the frame loop's own edge. A barge-in is the one thing the call's socket task
//! cannot do for itself: stopping the device means touching the `Player`, which lives on the
//! terminal's thread. So the task raises a flag, the frame loop reads it once a frame and flushes,
//! and the count starts again from nothing — the chunks it just dropped were never heard.
//!
//! What is *not* here: `on_message` stays in the parent and calls [`call_ended`] — it is the frame
//! loop's one entry point for every event, not the call's. The verbs that take the floor are not
//! here either. `tipsy_ui::policy::takes_the_floor` decides them, because the browser client
//! compiles against `tipsy-ui` and must reach the same one; `on_action` applies it, and a live call
//! is only one of the two things that rule reaches.

use super::*;

/// Whether a live-state event ends the call here.
///
/// The socket task decides when a call is over — it says so with `Live { active: false }` — and the
/// client's only job is to let the handle go and quiet the device. A `Live { active: true }` is a
/// call starting, which `start_call` already holds the handle for.
pub(super) fn call_ended(event: &Event) -> bool {
    matches!(event, Event::Live { active: false })
}

impl Client {
    /// Lets go of the call and silences the device: the audio still queued belongs to a call that is
    /// over, which is the same rule the user's own barge-in follows (`flush_on_interrupt`).
    pub(super) fn end_call(&mut self) {
        self.call = None;
        if flush_on_interrupt(true, self.playing()) {
            self.flush_playback();
        }
    }

    /// Starts a live call, or ends the one that is running.
    ///
    /// `self.call` is the client's own handle on the socket, so it — and not the HUD's indicator,
    /// which only the call task may raise — is what decides which way the key goes.
    pub(super) fn toggle_call(&mut self) {
        if let Some(call) = self.call.take() {
            call.stop();
            return;
        }
        if let Err(why) = self.start_call() {
            // The mode is never silently on (Review Focus 3): a refusal is a sentence the user
            // reads, never a key press that quietly did nothing.
            self.state.notify(why);
        }
    }

    /// Opens the socket, or explains why it will not.
    ///
    /// The key is resolved the way every provider's is (`KeyResolver`: the environment first, then
    /// the credential store), and resolving it *here* is what makes a name nobody holds an answer to
    /// the key press instead of an asynchronous failure: no socket is touched until it has resolved.
    /// Only the key *name* reaches `CallSetup` — the value is never logged, never put in an event and
    /// never in an error message, and the handshake resolves the name again when it opens.
    pub(super) fn start_call(&mut self) -> Result<(), String> {
        let realtime = self.config.realtime.clone();
        if !realtime.enabled {
            return Err(self.state.strings().live_off.to_string());
        }
        // A `--connect` client holds no resolver: the keys are the server's, and a call is opened
        // here, in this process. Refused as a sentence rather than a panic, because the mode is never
        // silently on (Review Focus 3).
        let Some(keys) = &self.keys else {
            return Err("a remote client holds no keys; live calls run in the client that owns them".to_string());
        };
        if keys.resolve(&realtime.key).map_err(|e| e.to_string())?.is_none() {
            return Err(format!("no key named `{}`: set {} or store it", realtime.key, env_var_name(&realtime.key)));
        }
        let events = self.events.clone().ok_or("the client's frame loop is not running")?;
        // Whatever the session was still saying belongs to the pipeline the call replaces.
        self.flush_playback();
        self.call = Some(Call::open(
            CallSetup::from_config(&self.config, Arc::new(keys.clone()), realtime.key),
            Heard::new(),
            events,
        ));
        Ok(())
    }

    /// Acts on what the call's socket task decided since the last frame.
    ///
    /// A barge-in is the one thing the socket task cannot do itself: stopping the device means
    /// touching the `Player`, which lives on this thread. The task marks it, the frame loop flushes,
    /// and the count starts again from nothing — the chunks it just dropped were never heard.
    pub(super) fn sync_call(&mut self) {
        // The bookkeeping is an `Arc` shared with the socket task, so the handle on the client is
        // only read to reach it: nothing here borrows the client while the device is flushed.
        let Some(heard) = self.call.as_ref().map(|call| Arc::clone(call.heard())) else {
            return;
        };
        if heard.take_interrupted() {
            // The count first: whatever the device had queued when the flag was raised was not
            // heard, and a stale reading left behind after the flush would overstate it.
            heard.clear();
            self.flush_playback();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::AssistantState;

    use crate::client::tests::resolver;
    use crate::keys::MemoryStore;

    #[test]
    fn the_client_lets_go_of_a_call_when_the_socket_says_it_is_over() {
        // A `Call` needs a socket, so what this pins is the rule: a call's ending is a live-flag
        // event and nothing else. A status change must never be mistaken for it — that is what
        // stops a finished reply from being cut off mid-sentence (`flush_on_interrupt`).
        assert!(call_ended(&Event::Live { active: false }));
        assert!(!call_ended(&Event::Live { active: true }));
        assert!(!call_ended(&Event::Status { state: AssistantState::Idle, detail: None }));
        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(Event::Live { active: false }));
        assert!(client.call.is_none(), "there was no call, and none is invented");
    }

    #[test]
    fn a_call_never_starts_while_the_mode_is_disabled() {
        let mut config = Config::default();
        config.realtime.enabled = false;
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);
        let why = client.start_call().expect_err("a disabled mode starts nothing");
        assert!(why.contains("enabled"), "the refusal names the setting: {why}");
        assert!(client.call.is_none(), "Review Focus 3: the mode is never silently on");
    }

    /// The one test that presses the key: `Tab` with a live call is the user taking the floor, and it
    /// has to reach the socket as `response.cancel` without ending the call. Every link is unit-tested
    /// on its own — `UiState::on_key(Tab)` → `ListenStart`, `takes_the_floor`, `Call::interrupt`, the
    /// socket task's control arm — and this is the one that drives them all the way to a real socket
    /// and back (the final review's item 7).
    #[tokio::test]
    async fn tab_with_a_live_call_cancels_the_response_and_leaves_the_call_running() {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        // A loopback socket that completes a realtime handshake, records what the call sends, and
        // answers a cancel with one audio delta — the proof that the same call is still reading.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let url = format!("ws://{}/v1/realtime?model=gpt-realtime-2.1", listener.local_addr().unwrap());
        let (frames, mut sent) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("the call connects");
            let mut socket = tokio_tungstenite::accept_async(stream).await.expect("the handshake");
            socket
                .send(Message::text(r#"{"type":"session.created","session":{"id":"sess_fixture"}}"#))
                .await
                .expect("the client is listening");
            while let Some(Ok(message)) = socket.next().await {
                let Message::Text(text) = message else { continue };
                let value: serde_json::Value = serde_json::from_str(text.as_str()).expect("JSON");
                let cancel = value["type"] == "response.cancel";
                let _ = frames.send(text.as_str().to_string());
                if cancel {
                    socket
                        .send(Message::text(
                            r#"{"type":"response.output_audio.delta","response_id":"resp_1","item_id":"item_a","content_index":0,"delta":"AAECAw=="}"#,
                        ))
                        .await
                        .expect("the call is listening");
                }
            }
        });

        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);
        // The call the key will interrupt: the loopback socket above, with an empty key *name* so the
        // handshake carries no `Authorization` header and no fixture here is a secret.
        let (events, mut from_call) = tokio::sync::mpsc::unbounded_channel();
        let options = crate::realtime::socket::CallOptions::at(&url);
        let setup = CallSetup {
            url,
            options,
            config: crate::config::RealtimeConfig { key: String::new(), ..Default::default() },
            resolver: Arc::new(KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None))),
            model: "gpt-realtime-2.1".into(),
            instructions: "eres un compae".into(),
            voice: "marin".into(),
            silence_ms: 1000,
            language: "es-ES".into(),
            limit: Duration::from_secs(600),
        };
        client.call = Some(Call::open(setup, Heard::new(), events.clone()));
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();

        // The session is configured first, so the key press meets a live call and not a handshake.
        let update = tokio::time::timeout(Duration::from_secs(5), sent.recv())
            .await
            .expect("the call never configured its session")
            .expect("the fake server is running");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&update).unwrap()["type"], "session.update");

        // The key press, through the same two functions the frame loop uses.
        for action in client.state.on_key(KeyInput::Tab) {
            client.on_action(action, &to_server, &events);
        }

        let cancel = tokio::time::timeout(Duration::from_secs(5), sent.recv())
            .await
            .expect("pressing Tab with a live call never sent `response.cancel`")
            .expect("the fake server is running");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&cancel).unwrap()["type"], "response.cancel");

        // And the call lives on: the same socket answers with the model's audio, which the client
        // reads exactly as it does in its frame loop.
        let mut spoke = false;
        while !spoke {
            let message = tokio::time::timeout(Duration::from_secs(5), from_call.recv())
                .await
                .expect("the call stopped reporting after the cancel")
                .expect("the call's channel");
            assert!(
                !matches!(message, ToClient::Event(Event::Live { active: false })),
                "a cancel is not an ending: {message:?}"
            );
            spoke = matches!(message, ToClient::Audio(Frame::Speech { .. }));
            client.on_message(&to_server, message);
        }
        assert!(client.call.is_some(), "the call is still the client's to end");
    }
}
