//! The live-session registry: who is connected, and how a change reaches all of them (spec §5.2).
//!
//! The `Hub` is the only thing that outlives a connection. A session's `from_server` end belongs to
//! whoever bridges that connection — the frame loop, a socket — but the senders a session is reached
//! through are the registry's, because a change made by one client has to reach the others.
//!
//! What the hub owns, and what it deliberately does not: it owns the configuration, where that
//! configuration is persisted, the credential store, and **the one `ProviderSlot` the whole server
//! runs on** — `ProviderSlot` is `Arc<RwLock<Providers>>`, so `reconfigure_all` swaps the connectors
//! in place for every session at once. It does **not** own a `Brain`: a `Brain` is built from the
//! active profile's prompt and voice, which is per session, and a change reaches each of them as
//! `Command::UpdateConfig`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tipsy_protocol::{ClientConfigView, Command, ConfigView, Event, ListeningMode, ToClient, ToServer};
use tokio::sync::mpsc::UnboundedSender;

use crate::brain::BrainConfig;
use crate::config::form::{config_of, view_of};
use crate::config::{Config, store};
use crate::keys::KeyResolver;
use crate::providers::{ProviderSlot, build_providers};
use crate::session::{SessionEnds, SessionPolicy, spawn_with_events};

/// One live session: the two ends the hub needs. `events` is how a broadcast reaches the client — the
/// same name `SessionEnds` gives that sender — and `to_server` is how a change the *session* has to
/// apply, a new configuration and the brain derived from it, is handed to the task that owns it.
struct Live {
    to_server: UnboundedSender<ToServer>,
    events: UnboundedSender<ToClient>,
}

/// The live sessions, the configuration they are all running, and the two things that make a change
/// stick: where it is written and how a key is stored.
///
/// Cheap to clone — every field is a handle — which is what lets a session hold one while the
/// connection that started it holds another.
#[derive(Clone)]
pub struct Hub {
    /// Keyed by the session id Task 1.2's counter hands out. A `BTreeMap`, so `session_ids` and any
    /// log line are in a stable order: a broadcast's iteration order must not be what makes a test
    /// flaky.
    live: Arc<Mutex<BTreeMap<u64, Live>>>,
    /// The configuration every session is running. Shared rather than copied per session: a save
    /// changes it once, and every session that re-derives anything does so from this one value.
    config: Arc<Mutex<Config>>,
    /// Where a change is persisted. The listener's `--bind` override never reaches this file
    /// (spec §5.1), which is why the path and not the loaded file is what a save writes.
    config_path: PathBuf,
    /// The server's credential store, for `SetKey`. Spec §9.4 rules on *when* it may be used, and
    /// Task 1.8 owns that gate. Behind an `Arc` because `KeyResolver` is not `Clone` (`keys.rs:79-82`
    /// holds two boxed trait objects) and this type has to be.
    keys: Arc<KeyResolver>,
    /// **The one provider stack the whole server runs on.** `ProviderSlot` is `Arc<RwLock<Providers>>`
    /// (`providers/mod.rs:167`) and `ProviderSlot::replace` (`:175`) exists precisely so a
    /// configuration change swaps the connectors in place for everyone at once: every session's
    /// `Brain` holds a clone of this same slot, so a `replace` is seen by all of them with no
    /// per-session work. One slot for the server, not one per connection.
    providers: ProviderSlot,
    /// The turn timeouts and the retry backoff every session's `Brain` is built with. The hub owns it
    /// so a connection carries exactly one fact of its own — its policy — and because nothing in this
    /// plan configures it: `BrainConfig::default` (`brain.rs:28-37`) is 30 s / 30 s / 60 s / 800 ms and
    /// no tab of the configuration screen reaches it.
    brain_config: BrainConfig,
}

impl Hub {
    pub fn new(config: Config, config_path: PathBuf, keys: KeyResolver, providers: ProviderSlot) -> Hub {
        Hub {
            live: Arc::new(Mutex::new(BTreeMap::new())),
            config: Arc::new(Mutex::new(config)),
            config_path,
            keys: Arc::new(keys),
            providers,
            brain_config: BrainConfig::default(),
        }
    }

    /// The stack the server is running on. Read by the in-process client, which owns the `Player`
    /// and the `Test` probe that both need it, and by the tests.
    pub fn providers(&self) -> ProviderSlot {
        self.providers.clone()
    }

    /// The file a change is written to. Read by the tests that prove persistence and by Task 1.4's
    /// error messages, so it is not private.
    pub fn path(&self) -> &Path {
        &self.config_path
    }

    /// The configuration as it stands. A clone: the caller never holds the lock.
    pub fn config(&self) -> Config {
        self.config.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// The configuration as the wire carries it. `include_token` **is** §4.3's rule, and the caller is
    /// the side that knows who is asking: a loopback connection, or one that presented this exact
    /// token, passes `true`; everyone else passes `false` and gets the emptied field.
    pub fn config_view(&self, include_token: bool) -> ConfigView {
        let view = view_of(&self.config());
        if include_token { view } else { view.redacted() }
    }

    /// Registers a session's two ends. The id is the session's own, from Task 1.2's counter; attaching
    /// an id twice replaces the entry, which is one session per id and nothing else. The second sender
    /// is named `events` because it is the channel every producer of `ToClient` writes to — the same
    /// name `SessionEnds` gives it (`session.rs:129`).
    pub fn attach(&self, session_id: u64, to_server: UnboundedSender<ToServer>, events: UnboundedSender<ToClient>) {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id, Live { to_server, events });
    }

    /// Forgets a session. Called when its connection ends, whatever ended it (spec §10): the hub holds
    /// nothing that outlives the session it belongs to.
    pub fn detach(&self, session_id: u64) {
        self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&session_id);
    }

    /// Writes one message to every live session and reports how many took it. A closed channel is not
    /// an error: that session's connection is on its way out and `detach` is about to run.
    pub fn broadcast(&self, message: ToClient) -> usize {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .filter(|session| session.events.send(message.clone()).is_ok())
            .count()
    }

    /// The ids of the live sessions, ascending.
    pub fn session_ids(&self) -> Vec<u64> {
        self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).keys().copied().collect()
    }

    /// The server half of a change, in two parts, because a change has two parts (spec §5.2).
    ///
    /// **The connectors** are rebuilt from the configuration and swapped into the one shared
    /// `ProviderSlot`, so every session's `Brain` sees them on its next turn with no per-session work
    /// at all: they all hold a clone of this same slot. That is what `ProviderSlot::replace`
    /// (`providers/mod.rs:175`) is for.
    ///
    /// **The `Brain`** is per session and has to be rebuilt in each of them, because the active
    /// profile's prompt and voice are what a `Brain` is built from: `Command::UpdateConfig` is handed
    /// to every session so each one re-derives its own, in its own task. A per-session provider
    /// rebuild would be redundant work on top of the `replace`.
    ///
    /// Whether the *clients* are told `ConfigChanged`, `ProfileChanged`, a fresh `Welcome` or nothing
    /// at all is the caller's decision, which is why this sends no event.
    ///
    /// A configuration whose connectors cannot be built — a key name with no key behind it — leaves
    /// the running stack alone rather than installing nothing: a save must never make a working server
    /// unable to answer. The connectors that were there keep serving, `false` says the stack did not
    /// turn over, and the caller that cares (`client.rs:702-705`) is the one that refuses the save in
    /// the first place. The session half runs either way, because the configuration *is* saved by
    /// then and every `Brain` must at least read the profile it now names.
    pub fn reconfigure_all(&self) -> bool {
        let config = self.config();
        let replaced = match build_providers(&config.providers, &self.keys) {
            Ok(rebuilt) => {
                self.providers.replace(rebuilt);
                true
            }
            Err(_) => false,
        };
        let live = self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for session in live.values() {
            // No payload: `Command::UpdateConfig` is a *client's* message and carries only the fields
            // a client may move, so a session cannot re-derive itself from it. The hub is the one owner
            // of the configuration and each session reads it back with `Hub::config` — which also
            // means no copy of the token travels on an internal channel.
            let _ = session.to_server.send(ToServer::Reconfigure);
        }
        replaced
    }

    /// A fresh `Welcome` to every session. `SetListeningMode` is the one command whose whole effect is
    /// that announcement (`session.rs:661-664`), which is why the hub re-sends it with the mode it has
    /// rather than needing a command of its own (spec §5.2: the mode is not configuration, so it is
    /// not a `ConfigChanged`).
    pub fn resend_welcome(&self) {
        let mode = self.config().listening.mode;
        let live = self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for session in live.values() {
            let _ = session.to_server.send(ToServer::Command(Command::SetListeningMode { mode }));
        }
    }

    /// Starts a session whose events this hub owns, and registers it before returning. The registry
    /// gets the session's two senders before anyone can read from its receiver, so its first `Welcome`
    /// cannot race the `attach` and no broadcast can miss it.
    ///
    /// The session is handed a clone of this same hub — the three commands of Task 1.1 are answered
    /// *through* it — a clone of **the server's one `ProviderSlot`**, so the stack a session's `Brain`
    /// runs on is the stack `reconfigure_all` replaces, and the hub's own `BrainConfig`. The policy is
    /// the only per-connection fact, because the configuration, the stack and the timeouts are all the
    /// hub's.
    pub fn spawn(&self, policy: SessionPolicy) -> SessionEnds {
        let ends =
            spawn_with_events(self.config(), self.providers.clone(), self.brain_config.clone(), policy, self.clone());
        self.attach(ends.session_id, ends.to_server.clone(), ends.events.clone());
        ends
    }

    /// Persist, remember, and re-derive every session from the value. `true` when the file was
    /// written. The order is the point, and it is the order `client.rs:693-720`'s save already uses:
    /// the file goes first and nothing running changes until it has, because a configuration this
    /// process cannot restart from is damage rather than a saved edit. A write that fails leaves the
    /// file, the sessions, their brains and the provider stack exactly as they were.
    ///
    /// What `reconfigure_all` found is not folded into the answer: the providers live in the shared
    /// slot and the callers that can *build* them are the ones that already refuse an unbuildable save
    /// (`client.rs:702-705`). A caller that wants to know whether the stack turned over calls
    /// `reconfigure_all` itself.
    pub fn apply_config(&self, config: Config) -> bool {
        if store::save(&self.config_path, &config).is_err() {
            return false;
        }
        *self.config.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
        let _ = self.reconfigure_all();
        true
    }

    /// `UpdateConfig` as the wire carries it: the fields a client may move, merged into the server's
    /// own configuration. **The merge is where the security rule lives** — each provider's `base_url`
    /// and `key_name` and the server's `token` are not in the client's shape at all, so they come from
    /// the server's value here (`ClientConfigView::merged_into`). The refusal is `config_of`'s, by the
    /// configuration screen's own row names, and a refused view writes nothing.
    pub fn update_config_from_view(&self, view: ClientConfigView) -> usize {
        let merged = view.merged_into(view_of(&self.config()));
        match config_of(merged) {
            Ok(config) => self.update_config(config),
            Err(_) => 0,
        }
    }

    /// The configuration a save produced: persist it, re-derive every session, and tell every client
    /// to ask again. Returns what `broadcast` reported, so a caller can log it and a test can assert
    /// on it.
    pub fn update_config(&self, config: Config) -> usize {
        if !self.apply_config(config) {
            return 0;
        }
        self.broadcast(ToClient::Event(Event::ConfigChanged))
    }

    /// `SetProfile`: persist the active profile, re-derive every session, and tell every client which
    /// profile is now speaking. The clients are told with `ProfileChanged` and not with a
    /// `ConfigChanged`, because a profile is not the configuration (spec §5.2).
    pub fn set_profile(&self, id: &str) -> usize {
        let mut config = self.config();
        if !config.profiles.iter().any(|profile| profile.id == id) {
            // An id that is not in the configuration is a no-op rather than a brain built around
            // nothing — the rule `session.rs:643-645` already applies.
            return 0;
        }
        config.active_profile = id.to_string();
        if !self.apply_config(config) {
            return 0;
        }
        let profile = self.config().active().view();
        self.broadcast(ToClient::Event(Event::ProfileChanged { profile }))
    }

    /// `SetListeningMode`: persist the mode, re-derive every session, and give every client a fresh
    /// `Welcome` — the mode is not configuration, so it is not a `ConfigChanged` (spec §5.2). The
    /// count is the live sessions, which is the number `resend_welcome` just announced to.
    pub fn set_listening_mode(&self, mode: ListeningMode) -> usize {
        let mut config = self.config();
        config.listening.mode = mode;
        if !self.apply_config(config) {
            return 0;
        }
        self.resend_welcome();
        self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).len()
    }

    /// Stores a key in the **server's** credential store (spec §4.2) and tells every client that the
    /// configuration changed. Spec §9.4's loopback-or-TLS gate is Task 1.8's and is deliberately not
    /// here: this is the call site it will guard, and `SessionPolicy.loopback` is the fact it reads.
    pub fn set_key(&self, name: &str, value: &str) -> usize {
        if self.keys.store().set(name, value).is_err() {
            return 0;
        }
        self.broadcast(ToClient::Event(Event::ConfigChanged))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The three fakes at the end of this module answer nothing, so these are only ever used to write
    // their signatures down. `crate::audio::pcm::Pcm` is the type `SpeechProvider` returns.
    use crate::audio::pcm::Pcm;
    use crate::keys::MemoryStore;
    use crate::providers::{
        ChatProvider, ChatRequest, ProviderError, Providers, SpeechProvider, SpeechRequest, TextStream,
        TranscribeProvider,
    };
    use tokio::sync::mpsc;

    /// A hub with no sessions, and a configuration file in a tempdir that nothing has read.
    fn hub() -> Hub {
        let dir = tempfile::tempdir().unwrap();
        Hub::new(
            Config::default(),
            dir.path().join("config.toml"),
            KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None)),
            silent(),
        )
    }

    /// One session that is not running: both of its ends, and both of their receivers. `reconfigure_all`
    /// talks to the *session*, so a test that watched only the client end would see nothing.
    fn ends() -> (
        UnboundedSender<ToServer>,
        UnboundedSender<ToClient>,
        mpsc::UnboundedReceiver<ToClient>,
        mpsc::UnboundedReceiver<ToServer>,
    ) {
        let (to_server, from_connection) = mpsc::unbounded_channel();
        // The client end is called `events` here for the same reason `Hub::attach` names it that: it is
        // the channel every producer of `ToClient` writes to.
        let (events, from_server) = mpsc::unbounded_channel();
        (to_server, events, from_server, from_connection)
    }

    /// Three connectors that answer nothing, so a broadcast's own delivery is what is under test.
    /// `build_providers` never runs in these tests — the slot is handed to the hub already built —
    /// which is why a fake with no key behind it is enough.
    fn silent() -> ProviderSlot {
        ProviderSlot::new(Providers {
            chat: Arc::new(SilentChat),
            speech: Arc::new(SilentSpeech),
            transcribe: Arc::new(SilentTranscribe),
        })
    }

    #[test]
    fn a_detached_session_is_not_broadcast_to() {
        let hub = hub();
        let (first_to_server, first_to_client, mut first_rx, _) = ends();
        let (second_to_server, second_to_client, mut second_rx, _) = ends();
        hub.attach(1, first_to_server, first_to_client);
        hub.attach(2, second_to_server, second_to_client);

        assert_eq!(hub.broadcast(ToClient::Event(Event::ConfigChanged)), 2);
        assert_eq!(first_rx.try_recv().unwrap(), ToClient::Event(Event::ConfigChanged));
        assert_eq!(second_rx.try_recv().unwrap(), ToClient::Event(Event::ConfigChanged));

        hub.detach(1);

        assert_eq!(hub.broadcast(ToClient::Event(Event::ConfigChanged)), 1);
        assert!(first_rx.try_recv().is_err(), "a detached session must not be written to");
        assert_eq!(second_rx.try_recv().unwrap(), ToClient::Event(Event::ConfigChanged));
        assert_eq!(hub.session_ids(), vec![2]);
    }

    /// A session whose connection died without detaching: `broadcast` counts what it delivered, and a
    /// closed channel is not one of those.
    #[test]
    fn a_session_whose_channel_is_closed_is_not_counted() {
        let hub = hub();
        let (live_to_server, live_to_client, mut live_rx, _) = ends();
        // Both ends of the dead session's client channel: a live *receiver* is enough to keep a sender
        // succeeding, so the shape this test needs is a channel with nothing on the other side at all.
        let (dead_to_server, dead_to_client, dead_rx, _) = ends();
        hub.attach(1, live_to_server, live_to_client);
        hub.attach(2, dead_to_server, dead_to_client);
        // The dead session is the one whose client end nobody holds any more — the shape of a socket
        // that went away before `detach` ran. The receiver goes with it, which is what the plan's
        // version of this test left in place: a held `_dead_rx` keeps the channel open, `broadcast`
        // then delivers to both sessions, and the assertion below fails on a hub that is behaving.
        drop(dead_rx);

        assert_eq!(hub.broadcast(ToClient::Event(Event::ConfigChanged)), 1);
        assert_eq!(live_rx.try_recv().unwrap(), ToClient::Event(Event::ConfigChanged));
    }

    /// Attaching the same id twice replaces the ends: one session per id, which is what the id is for.
    #[test]
    fn attaching_an_id_twice_leaves_one_session() {
        let hub = hub();
        let (first_to_server, first_to_client, mut first_rx, _) = ends();
        let (second_to_server, second_to_client, mut second_rx, _) = ends();
        hub.attach(7, first_to_server, first_to_client);
        hub.attach(7, second_to_server, second_to_client);

        assert_eq!(hub.broadcast(ToClient::Event(Event::ConfigChanged)), 1);
        assert!(first_rx.try_recv().is_err(), "the replaced session is gone");
        assert_eq!(second_rx.try_recv().unwrap(), ToClient::Event(Event::ConfigChanged));
        assert_eq!(hub.session_ids(), vec![7]);
    }

    /// The ids come in order, so a broadcast's iteration order cannot be what makes a test flaky.
    #[test]
    fn session_ids_are_ascending() {
        let hub = hub();
        let (a_to_server, a_to_client, _, _) = ends();
        let (b_to_server, b_to_client, _, _) = ends();
        hub.attach(11, a_to_server, a_to_client);
        hub.attach(2, b_to_server, b_to_client);
        assert_eq!(hub.session_ids(), vec![2, 11]);
    }

    /// `apply_config` writes the file and remembers the value — and hands every session
    /// `ToServer::Reconfigure`, which is the message and not an event: it is the session's own brain
    /// that is re-derived, from the configuration the hub now holds, and the client-facing
    /// `ConfigChanged` is a separate decision (spec §5.2).
    #[test]
    fn applying_a_configuration_persists_it_and_commands_every_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let hub = Hub::new(
            Config::default(),
            path.clone(),
            KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None)),
            silent(),
        );
        // The sessions' own ends, so the test can read what the hub sent them.
        let (first_to_server, first_to_client, _, mut first_commands) = ends();
        let (second_to_server, second_to_client, _, mut second_commands) = ends();
        hub.attach(1, first_to_server, first_to_client);
        hub.attach(2, second_to_server, second_to_client);

        let mut edited = Config::default();
        edited.server.bind = "127.0.0.1:7171".into();
        assert!(hub.apply_config(edited.clone()), "the file must be written");

        // The fan-out carries no payload on purpose: a client's `UpdateConfig` may not carry a
        // provider's address, a key name or the token, so a session re-derives itself from the hub
        // rather than from the message — and the hub is the only thing that may hand out the token.
        for rx in [&mut first_commands, &mut second_commands] {
            match rx.try_recv().unwrap() {
                ToServer::Reconfigure => {}
                other => panic!("a configuration change is a `Reconfigure` to the session, got {other:?}"),
            }
        }
        assert_eq!(store::load_or_create(&path).unwrap().config.server.bind, "127.0.0.1:7171");
        assert_eq!(hub.config().server.bind, "127.0.0.1:7171");
    }

    /// A write that fails must not leave the hub believing a configuration nobody can restart from.
    #[test]
    fn a_configuration_that_cannot_be_written_is_not_remembered() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be: `store::save`'s write fails, and that is the point.
        let path = dir.path().join("config.toml");
        std::fs::create_dir(&path).unwrap();
        let hub = Hub::new(
            Config::default(),
            path,
            KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None)),
            silent(),
        );

        let mut edited = Config::default();
        edited.server.bind = "127.0.0.1:7171".into();
        assert!(!hub.apply_config(edited), "a write that failed is not a change");
        assert_eq!(hub.config().server.bind, "127.0.0.1:7070", "the old value stands");
    }

    /// §4.3's rule in its smallest form: the one `bool` is the whole of it.
    #[test]
    fn the_token_leaves_the_view_only_when_the_caller_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.server.token = "s3cret".into();
        let hub = Hub::new(
            config,
            dir.path().join("config.toml"),
            KeyResolver::new(Box::new(MemoryStore::default()), Box::new(|_| None)),
            silent(),
        );

        assert_eq!(hub.config_view(true).server.token, "s3cret");
        assert_eq!(hub.config_view(false).server.token, "");
        assert_eq!(hub.config_view(false).server.bind, "127.0.0.1:7070", "only the token goes");
    }

    /// The fake chat connector: it always fails, and no test here reaches the failure.
    struct SilentChat;

    #[async_trait::async_trait]
    impl ChatProvider for SilentChat {
        async fn stream(&self, _req: ChatRequest) -> Result<TextStream, ProviderError> {
            Err(ProviderError::BadResponse("no provider in this test".into()))
        }
    }

    /// The fake speech connector, the same way. `max_parallel` is the trait's own default.
    struct SilentSpeech;

    #[async_trait::async_trait]
    impl SpeechProvider for SilentSpeech {
        async fn synthesize(&self, _req: SpeechRequest) -> Result<Pcm, ProviderError> {
            Err(ProviderError::BadResponse("no provider in this test".into()))
        }
    }

    /// The fake transcriber, the same way.
    struct SilentTranscribe;

    #[async_trait::async_trait]
    impl TranscribeProvider for SilentTranscribe {
        async fn transcribe(&self, _pcm: Pcm, _language: &str) -> Result<String, ProviderError> {
            Err(ProviderError::BadResponse("no provider in this test".into()))
        }
    }
}
