//! Playback: the device, the flush that silences it, and the reply kept for `Replay`.
//!
//! Two things live here and nothing else does. The **device** is asked what it is still playing
//! (`playing`) and is silenced (`flush_playback`) — the only two things the client ever does to it.
//! The **reply kept for `Replay`** is the client's copy of the audio it has handed over (`reply`),
//! grown by `on_speech` and dropped by the parent when the next turn begins, because what `F2`
//! replays is the last reply.
//!
//! The rule that decides *when* to flush is not here: `tipsy_ui::policy` owns `flush_on_interrupt`,
//! because the browser client compiles against `tipsy-ui` and must reach the same one. Nothing here
//! decides either — these are the two places the decision is applied to a real device, and both are
//! reached from the parent (`on_action`, `end_call`, `sync_call`, `start_call`, the frame loop).

use super::*;

impl Client {
    /// True while the device still has audio queued or being played.
    ///
    /// The device is the only honest source for this: the session reports the end of a turn before
    /// the device has finished speaking, so its status cannot say what is still audible.
    pub(super) fn playing(&self) -> bool {
        self.player.as_ref().is_some_and(Player::playing)
    }

    /// Silences the device: whatever rodio still has queued is audio the assistant no longer wants
    /// to say. The samples already kept for `Replay` are untouched.
    ///
    /// Takes `&self` on purpose: the only thing it touches is the device, so a caller holding the
    /// live call's bookkeeping can silence the device without giving that borrow up.
    pub(super) fn flush_playback(&self) {
        if let Some(player) = &self.player {
            player.stop();
        }
    }

    /// Plays one spoken sentence, and shows the envelope of the reply it belongs to.
    pub(super) fn on_speech(&mut self, samples: Vec<i16>) {
        if samples.is_empty() {
            return;
        }
        self.reply.extend_from_slice(&samples);
        let bytes = samples.len() * 2;
        if let Some(player) = &self.player {
            player.play(samples, self.rate);
        }
        // While a call is live the device is asked what it has actually played — the queue it still
        // holds is subtracted, because a response is generated faster than it is spoken and the
        // bytes merely handed over would overstate what the user heard (Review Focus 2). The device
        // is reachable only from this thread, so this is where the number is worked out.
        if let (Some(call), Some(player)) = (&self.call, &self.player) {
            let (remaining, pos) = player.progress();
            call.heard().push(bytes, remaining, pos.as_millis() as u64);
        }
        // The oscilloscope shows the reply as it is heard, so it is rebuilt as it grows.
        self.state.start_voice(envelope(&i16_to_f32(&self.reply), self.rate));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::AssistantState;

    use crate::client::tests::resolver;

    #[test]
    fn a_chunk_header_sets_the_rate_the_device_is_told() {
        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.on_message(&to_server, ToClient::Event(Event::SpeechChunk { seq: 0, sample_rate: 24000 }));
        client.on_message(&to_server, ToClient::Audio(Frame::Speech { seq: 0, samples: vec![5, 6] }));
        // There is no output device in a test — `Player::new` needs one — so the playback itself is
        // skipped; what is pinned is the rate it would be handed, which is the announced one and
        // never a guess about which mode is running.
        assert_eq!(client.rate, 24000);
        assert_eq!(client.reply, vec![5, 6], "the reply is kept for `F2` as it always was");
    }

    #[test]
    fn a_finished_reply_keeps_its_audio_until_a_new_turn_starts() {
        let (to_server, _from_client) = tokio::sync::mpsc::unbounded_channel();
        let config = Config::default();
        let keys = resolver();
        let providers = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, PathBuf::from("config.toml"), keys, providers);

        client.on_message(&to_server, ToClient::Event(Event::Status { state: AssistantState::Speaking, detail: None }));
        client.on_message(&to_server, ToClient::Event(Event::SpeechChunk { seq: 0, sample_rate: 24000 }));
        client.on_message(&to_server, ToClient::Audio(Frame::Speech { seq: 0, samples: vec![7, 8, 9] }));
        client.on_message(&to_server, ToClient::Event(Event::Reply { text: "Aquí andamos.".into() }));
        client.on_message(&to_server, ToClient::Event(Event::Done { first_audio_ms: Some(1), total_ms: 2 }));
        assert_eq!(client.reply, vec![7, 8, 9], "`F2` has the reply as soon as it is spoken");

        // The idle that ends every normal reply: the device is still speaking, so it must keep both
        // the queue and what `F2` replays.
        client.on_message(&to_server, ToClient::Event(Event::Status { state: AssistantState::Idle, detail: None }));
        assert_eq!(client.reply, vec![7, 8, 9], "a normal `Speaking -> Idle` keeps the last reply");

        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Send(Command::Ask { text: "¿y eso?".into() }), &to_server, &events);
        assert!(client.reply.is_empty(), "the next turn speaks a reply of its own");
    }
}
