//! The configuration screen's actions: opening it, probing a provider, saving, and the token row.
//!
//! Eight things live here and nothing else does. `open_screen` reads `config.toml` **now** — a file
//! hand-edited while the app runs is what the screen shows, never the copy the startup read — and
//! `open_screen_remote` asks the server for its configuration instead, because a `--connect` client
//! holds no file of its own. `on_config_effect` is the dispatcher the screen's three effects arrive
//! through. `save` is the order Review Focus 1 is about: the connectors are built first, then the
//! file is written through the one writer, and only then does anything running change; `save_remote`
//! sends the form and nothing else, because over a socket the file, the keys and the connectors are
//! the server's. `test_provider` answers the screen's Test button — a row whose key nobody holds is
//! answered at once with no request at all, and the probe itself runs on a task of its own, so the
//! HUD never waits for a provider. `regenerate_token` fills the Server tab's row and writes nothing:
//! the file changes on save, only on save. `config_notice` is where every one of those reasons goes —
//! the screen's own line while it is up, the HUD's otherwise.
//!
//! What is *not* here. The two arms of `on_action` that reach these methods — `UiAction::OpenConfig`
//! and `UiAction::Config` — stay in the parent, because moving them would rewrite a body this move
//! may not touch; `on_message` is what renders an `Event::Config` and an `Event::TestResult` into the
//! form, and it stays there for the same reason. `key_input` and `clock_view` stay in the parent too:
//! they read a terminal's key events and this process's clock.

use super::*;

impl Client {
    /// Opens the configuration screen on a copy read from the file *now* (Review Focus 3): a
    /// `config.toml` edited while the app runs is what the screen shows. Opening changes nothing
    /// that is running — `Esc` discards the edits, and only a save applies them.
    ///
    /// The screen is put up filled only when the file was read: a read that fails leaves the
    /// placeholder form on screen, unmarked (finding G), and that form refuses `Ctrl+S` and `F4`
    /// rather than writing blank base URLs and models over the user's `config.toml`. The reason goes
    /// where the user reads it — the screen's own notice line, through [`Client::config_notice`] —
    /// and not into the header's status span, which the header's own layout is free to cut to zero
    /// columns while the screen is up.
    ///
    /// A remote client has no file to read for this: the screen that matters is the server's, so it
    /// asks for it instead.
    pub(super) fn open_screen(&mut self, to_server: &UnboundedSender<ToServer>) {
        if self.attachment.is_remote() {
            return self.open_screen_remote(to_server);
        }
        match store::load_or_create(&self.config_path) {
            Ok(loaded) => {
                let mut form = form_of(&loaded.config);
                // A file that had to be backed up and reset is worth saying in the notice line: the
                // user's own file is gone, and the form no longer holds what they wrote.
                if let Some(warning) = loaded.warning {
                    form.notice = Some((NoticeKind::Warning, warning));
                }
                self.state.show_config_form(form);
            }
            Err(e) => self.config_notice(NoticeKind::Error, format!("{}: {e}", self.config_path.display())),
        }
    }

    /// Performs one effect the screen asked for.
    ///
    /// `events` is the client's own event sender: a Test reports its answer through it, so the
    /// result is rendered like anything else the session says.
    pub(super) fn on_config_effect(
        &mut self,
        effect: ConfigEffect,
        to_server: &UnboundedSender<ToServer>,
        events: &UnboundedSender<ToClient>,
    ) {
        match effect {
            ConfigEffect::None => {}
            ConfigEffect::Save => self.save(to_server),
            // The Test needs the sender now: over a socket the probe runs on the server.
            ConfigEffect::Test(capability) => self.test_provider(capability, to_server, events),
            // The token row is **native-only**, and now it says so instead of editing a value the
            // wire will not carry: a client's `UpdateConfig` has no `server.token` field at all
            // (spec §13.6 — the row stays inert over the wire, by design), so minting one here would
            // show the user a fresh password that the server never learns and that they cannot use.
            ConfigEffect::RegenerateToken => {
                if self.attachment.is_remote() {
                    self.config_notice(
                        NoticeKind::Warning,
                        "the token belongs to the server; regenerate it on the machine running `serve`",
                    );
                    return;
                }
                self.regenerate_token();
            }
        }
    }

    /// The `--connect` open: the server is asked for its configuration, and until the answer arrives
    /// the screen shows this client's own copy with a notice that says so. Opening on nothing at all
    /// would make `F9` look broken for a round trip.
    pub(super) fn open_screen_remote(&mut self, to_server: &UnboundedSender<ToServer>) {
        let mut form = form_of(&self.config);
        form.notice = Some((
            NoticeKind::Warning,
            format!("asking {} for its configuration", self.attachment.describe()),
        ));
        self.state.show_config_form(form);
        let _ = to_server.send(ToServer::Command(Command::GetConfig));
    }

    /// Runs the screen's Test against the row on screen and reports the answer into the client's own
    /// event channel, so it is rendered like anything else the session says.
    ///
    /// The connector is built *here*, before the call: a row that names a key nobody holds is then
    /// answered at once, with no latency and without a single request — which is what makes "no key"
    /// a different answer from "the provider refused". The probe itself runs on a task of its own,
    /// so the HUD never waits for a provider.
    pub(super) fn test_provider(
        &mut self,
        capability: Capability,
        to_server: &UnboundedSender<ToServer>,
        events: &UnboundedSender<ToClient>,
    ) {
        if self.attachment.is_remote() {
            // The providers and the keys are the server's, so the probe is too (spec §6.2). The
            // answer comes back as `Event::TestResult` through the socket and lands in the same
            // notice line as any other result.
            let _ = to_server.send(ToServer::Command(Command::TestProvider { capability }));
            return;
        }
        // The two fields are `Some` together for the whole of a bare run, and the guard above has
        // already sent every remote Test to the server, so neither of these can fail here.
        let Some(keys) = &self.keys else { return };
        // The row the user is looking at, converted the way a save converts it: a Test can never
        // quietly use a connector other than the one on screen.
        let converted = self
            .state
            .config_form
            .as_ref()
            .and_then(|form| form.providers.iter().find(|row| row.capability == capability))
            .map(provider_config);
        let cfg = match converted {
            // No screen, or no row for this capability: nothing to test and nothing to say.
            None => return,
            Some(Err(message)) => return self.config_notice(NoticeKind::Error, message),
            Some(Ok(cfg)) => cfg,
        };
        let connector = match build_one(&cfg, capability, keys) {
            Ok(connector) => connector,
            Err(e) => {
                // No request is made, so there is no latency to report; the message is the builder's
                // own, which names the key and the environment variable that would provide it.
                let _ = events.send(ToClient::Event(Event::TestResult {
                    capability,
                    ok: false,
                    latency_ms: 0,
                    message: e.to_string(),
                }));
                return;
            }
        };
        // The profile's language, read now: the transcription probe sends it, and it is the same tag
        // every other transcription carries.
        let language = self.config.active().language.clone();
        let events = events.clone();
        tokio::spawn(async move {
            let probe = connector.probe(&language).await;
            let _ = events.send(ToClient::Event(Event::TestResult {
                capability,
                ok: probe.ok,
                latency_ms: probe.latency_ms,
                message: probe.message,
            }));
        });
    }

    /// Writes the screen's configuration and applies it to the running session.
    ///
    /// The order is the whole of Review Focus 1: the connectors are built first, then the file is
    /// written through the one writer, and only then does anything running change. Nothing is
    /// written until every row parses and every key resolves, because a `config.toml` the app cannot
    /// start from is damage rather than a saved edit; and a refused write leaves the file, the
    /// running stack and the session exactly as they were.
    ///
    /// Over a socket none of that is this process's to do: the file, the keys and the connectors are
    /// the server's, so the form goes over the wire and the answer comes back as a fresh
    /// `Event::Config` (spec §6.2).
    pub(super) fn save(&mut self, to_server: &UnboundedSender<ToServer>) {
        if self.attachment.is_remote() {
            return self.save_remote(to_server);
        }
        // 1. The form in the file's shape.
        let parsed = self.state.config_form.as_ref().map(config_from);
        let config = match parsed {
            None => return,
            Some(Err(message)) => return self.config_notice(NoticeKind::Error, message),
            Some(Ok(config)) => config,
        };
        // 2. The connectors that configuration names.
        let (Some(keys), Some(providers)) = (&self.keys, &self.providers) else {
            // Unreachable while `Attachment::InProcess` owns the two `Some`s; the guard is what
            // keeps a remote client from ever reaching a builder.
            return;
        };
        let rebuilt = match build_providers(&config.providers, keys) {
            Ok(providers) => providers,
            Err(e) => return self.config_notice(NoticeKind::Error, e.to_string()),
        };
        // 3. The file itself.
        if let Err(e) = store::save(&self.config_path, &config) {
            return self.config_notice(NoticeKind::Error, format!("{}: {e}", self.config_path.display()));
        }
        // 4. What is running: the stack in place, then the fields the channel carries.
        self.mic_gain = config.listening.mic_gain;
        providers.replace(rebuilt);
        let _ = to_server.send(ToServer::Command(Command::SetListeningMode { mode: config.listening.mode }));
        let _ = to_server.send(ToServer::Command(Command::SetProfile { id: config.active_profile.clone() }));
        self.config = config;
        if let Some(form) = self.state.config_form.as_mut() {
            form.dirty = false;
            form.notice = None;
        }
    }

    /// The `--connect` save: the server owns the file, the keys and the connectors, so this sends the
    /// form and nothing else (spec §6.2). No local `build_providers` — this client holds no key — and
    /// no local `store::save`: a second copy of the server's configuration on another machine is a
    /// second source of truth, and the server answers `ConfigChanged` to everyone when it saves.
    ///
    /// The server may refuse the view, and a refused view is **silent** — the hub writes nothing and
    /// says nothing (spec §4.2) — so this client's own screen keeps the refusal it had and the notice
    /// below is a statement about what was *sent*, never a claim that it was saved.
    pub(super) fn save_remote(&mut self, to_server: &UnboundedSender<ToServer>) {
        let Some(form) = self.state.config_form.as_ref() else { return };
        // Two shapes, one conversion out: the screen's form is not the wire's view (§4.3), and the
        // view the wire carries is the shape a **client** may send — no provider `base_url`, no
        // provider `key_name`, no `server.token`. Those three rows are still editable on this screen,
        // and a remote save simply does not carry them (`ClientConfigView`), which is the rule rather
        // than a check the server has to make. The server's own answer to `ConfigChanged` puts the
        // server's values back on the screen, so an edit that did not cross is visible rather than
        // mysterious.
        let config = ClientConfigView::from(form);
        let _ = to_server.send(ToServer::Command(Command::UpdateConfig { config: Box::new(config) }));
        if let Some(form) = self.state.config_form.as_mut() {
            form.dirty = false;
            form.notice = Some((
                NoticeKind::Warning,
                format!("sent to {}; this client's own config.toml is unchanged", self.attachment.describe()),
            ));
        }
    }

    /// Fills the Server tab's token row with a fresh token from the OS generator. This is an edit
    /// like any other: the file changes when the user saves, never here.
    pub(super) fn regenerate_token(&mut self) {
        match store::new_token() {
            Ok(token) => {
                if let Some(form) = self.state.config_form.as_mut() {
                    form.server.token = token;
                    form.dirty = true;
                }
            }
            Err(e) => self.config_notice(NoticeKind::Error, e.to_string()),
        }
    }

    /// Puts a configuration message in the screen's notice line. The text is the low-level truth —
    /// the path and its OS error, the row that would not parse, the provider's own words — because
    /// the form carries no language of its own (the plan's `ConfigForm` has no `lang` field) and
    /// must not invent copy it cannot translate. Falls back to the HUD's notice when no screen is
    /// open.
    pub(super) fn config_notice(&mut self, kind: NoticeKind, text: impl Into<String>) {
        let text = text.into();
        if let Some(form) = self.state.config_form.as_mut() {
            form.notice = Some((kind, text));
            return;
        }
        self.state.notify(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tipsy_protocol::ListeningMode;
    use tipsy_ui::ConfigAction;

    use crate::client::tests::{mock_config, resolver};

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The screen with the edits a user would have made in it: the form holds them, the file does
    /// not, and `dirty` says so.
    fn open_with_edits(client: &mut Client, edited: &Config) {
        client.state.show_config_form(form_of(edited));
        client.state.config_form.as_mut().unwrap().dirty = true;
    }

    /// The screen's Test button, pressed the way the client receives it: the focused row of the
    /// form decides which capability is probed, and the probe makes a real call against the row.
    fn press_test(client: &mut Client, to_server: &UnboundedSender<ToServer>, events: &UnboundedSender<ToClient>) {
        client.on_action(UiAction::Config(ConfigAction::Test), to_server, events);
    }

    /// Plan 3 Task 1.7's audited rule: the `--connect` client must not start a server, must not build a
    /// provider and must not read the credential store (spec §7.2, §6.2).
    ///
    /// The instrument is `providers::builds_on_this_thread`, a counter bumped at the top of
    /// `build_providers` and `build_one`; it is a thread-local because this binary's other tests build
    /// providers on purpose, on other threads, at the same time. The first half of the test drives the
    /// in-process path so a counter that never moves cannot make the audit pass by accident — and it is
    /// an async test because that half's Test runs its probe on a task of its own.
    #[tokio::test]
    async fn the_remote_path_builds_no_provider_and_asks_the_server_instead() {
        let config = mock_config("http://127.0.0.1:9");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // The file this machine owns, written before anything runs: a remote save must leave it byte
        // for byte as it is, because the server's configuration is not this machine's to keep.
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        // The instrument first: the in-process Test does resolve a key and build a connector.
        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut local = Client::new(config.clone(), path.clone(), keys, slot);
        local.state.show_config_form(form_of(&config));
        local.state.config_form.as_mut().unwrap().field = 0;
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        let before_builds = crate::providers::builds_on_this_thread();
        local.on_action(UiAction::Config(ConfigAction::Test), &to_server, &events);
        assert!(
            crate::providers::builds_on_this_thread() > before_builds,
            "the instrument must see the in-process Test build its connector, or this test proves nothing"
        );

        // And now the remote path: opening the screen, testing and saving, with no resolver and no stack.
        let mut remote = Client::remote(config, path.clone(), "ws://127.0.0.1:7070/ws");
        let mut form = form_of(&mock_config("http://127.0.0.1:9"));
        // `form_of` marks the form filled — the file was read — and Save is refused on a form that is
        // not, exactly as the screen's own reducer refuses it.
        assert!(form.filled(), "the screen's save is only reachable from a filled form");
        form.field = 0;
        remote.state.show_config_form(form);
        let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, mut results) = tokio::sync::mpsc::unbounded_channel();

        let before_builds = crate::providers::builds_on_this_thread();
        remote.open_screen(&to_server);
        press_test(&mut remote, &to_server, &events);
        remote.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);
        assert_eq!(
            crate::providers::builds_on_this_thread(),
            before_builds,
            "the remote path built a provider stack, which means it read a key it must not read"
        );
        assert!(results.try_recv().is_err(), "a remote Test must not answer locally");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "a remote save must not write this machine's config.toml: the server owns the file"
        );

        // The three effects went to the server as the three commands, in order (spec §6.2).
        let mut sent = Vec::new();
        while let Ok(ToServer::Command(command)) = commands.try_recv() {
            sent.push(command);
        }
        assert!(
            matches!(
                sent.as_slice(),
                [
                    Command::GetConfig,
                    Command::TestProvider { capability: Capability::Chat },
                    Command::UpdateConfig { .. },
                ]
            ),
            "{sent:?}"
        );
    }

    /// Review Focus 3, second half: the screen opens on the file as it is *now*. A `config.toml`
    /// hand-edited while the app runs is what the second open shows, never the copy the first one
    /// read, and opening changes nothing that is running.
    #[test]
    fn opening_the_screen_again_reads_the_file_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config.clone(), path.clone(), keys, slot);
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();

        client.open_screen(&to_server);

        let first = client.state.config_form.as_ref().unwrap();
        assert_eq!(first.listening.silence_ms, "1000");
        assert!(first.server.token.is_empty());

        // The user edits the file in another window while the app keeps running.
        let mut edited = config.clone();
        edited.listening.silence_ms = 4321;
        edited.server.token = "pasted-by-hand".into();
        store::save(&path, &edited).unwrap();

        client.open_screen(&to_server);
        let second = client.state.config_form.as_ref().unwrap();
        assert_eq!(second.listening.silence_ms, "4321", "the screen must read the file again");
        assert_eq!(second.server.token, "pasted-by-hand");
        assert_eq!(client.config, config, "opening the screen changes nothing that is running");
    }

    /// The scoped re-review's finding 2: a file that cannot be read leaves the placeholder screen up
    /// and empty, and the reason is on the screen's own notice line. The header's status span, the
    /// only other place it could show, is the one span the header's layout may cut to zero columns.
    #[test]
    fn a_file_that_cannot_be_read_says_so_on_the_screen_itself() {
        let dir = tempfile::tempdir().unwrap();
        // A parent that is a file rather than a directory: nothing can be created under it, so the
        // load fails the way it does when the user's configuration directory is not writable.
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, "x").unwrap();
        let path = blocker.join("config.toml");

        let config = mock_config("http://127.0.0.1:1");
        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, path.clone(), keys, slot);

        // The real door: `F9` puts the placeholder form up — that is the state's own toggle — and
        // asks the client for the file, exactly as the key loop and a click on the gear do.
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        for action in client.state.on_key(KeyInput::F(9)) {
            client.on_action(action, &to_server, &events);
        }

        let form = client.state.config_form.as_ref().expect("the screen stays up when the file cannot be read");
        let (kind, text) = form.notice.clone().expect("the screen itself must say why it is empty");
        assert_eq!(kind, NoticeKind::Error);
        assert!(text.contains("config.toml"), "the notice names the file it could not read: {text}");
        assert!(text.contains("not-a-dir"), "and the path that failed: {text}");
        assert!(
            client.state.notice.is_none(),
            "the HUD's line is not where a screen notice belongs: {path:?} is already named on screen"
        );
    }

    /// Review Focus 1: a write that fails changes nothing at all. The file is byte for byte what it
    /// was, the running configuration and its provider stack are the ones the app started with, and
    /// the screen says why — while the edits stay in the form, unsaved.
    #[test]
    fn a_refused_save_leaves_the_file_and_the_running_configuration_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let stack_before = slot.current();
        let mut client = Client::new(config.clone(), path.clone(), keys, slot.clone());

        let mut edited = config.clone();
        edited.listening.silence_ms = 4321;
        edited.server.token = "pasted-by-hand".into();
        open_with_edits(&mut client, &edited);

        // A config path the process may not write: read-only is the cheapest way to make the write
        // itself fail — after every row parsed and every connector built, which is where the
        // interesting part of the order is.
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        assert!(std::fs::metadata(&path).unwrap().permissions().readonly(), "the file must be read-only for this test to prove anything");

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);

        assert_eq!(std::fs::read(&path).unwrap(), before, "a refused save must not touch the file");
        assert_eq!(client.config, config, "the running configuration is the one it was");
        assert!(
            std::sync::Arc::ptr_eq(&stack_before.chat, &slot.current().chat),
            "the running provider stack must not have been swapped"
        );
        let form = client.state.config_form.as_ref().unwrap();
        let (kind, text) = form.notice.clone().expect("a refused save must say so");
        assert_eq!(kind, NoticeKind::Error);
        assert!(text.contains("config.toml"), "the notice must name the file: {text}");
        assert!(form.dirty, "the edits are still unsaved: {form:?}");
        // And the file still holds the value a next launch must read.
        assert!(!std::fs::read_to_string(&path).unwrap().contains("4321"));

        // Let the temp directory go away. Clearing the bit back is the one place `false` is the
        // point, so clippy's `permissions_set_readonly_false` is silenced rather than obeyed.
        #[allow(clippy::permissions_set_readonly_false)]
        {
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(false);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
    }

    /// The other half of Review Focus 1, and Review Focus 5's "no key": a configuration whose
    /// connectors cannot be built is never written at all. Writing it would leave a `config.toml`
    /// the app cannot start from — a fresh launch fails with exit code 2 — so the key is checked
    /// before the file is touched.
    #[test]
    fn a_save_whose_providers_cannot_be_built_never_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let stack_before = slot.current();
        let mut client = Client::new(config.clone(), path.clone(), keys, slot.clone());

        // The user typed a key name nobody holds. Everything else about this save is valid, and the
        // write itself would have succeeded.
        let mut edited = config.clone();
        edited.providers.chat.key = "not-in-the-store".into();
        open_with_edits(&mut client, &edited);

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);

        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "a configuration the app cannot start from must never be written"
        );
        assert_eq!(client.config, config);
        assert!(std::sync::Arc::ptr_eq(&stack_before.chat, &slot.current().chat));
        let (kind, text) = client.state.config_form.as_ref().unwrap().notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Error);
        assert!(
            text.contains("not-in-the-store") && text.contains("TIPSY_KEY_NOT_IN_THE_STORE"),
            "the notice must name the key and how to provide it: {text}"
        );
    }

    /// A save that works: the file holds the edits, the running configuration is the one that was
    /// written, the connectors are rebuilt *in place*, and the session is told the fields the
    /// channel carries — in that order.
    #[test]
    fn a_saved_configuration_is_written_and_swapped_into_the_running_stack() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let stack_before = slot.current();
        let mut client = Client::new(config.clone(), path.clone(), keys, slot.clone());

        let mut edited = config.clone();
        edited.listening.silence_ms = 4321;
        edited.listening.mode = ListeningMode::Auto;
        edited.active_profile = "senyor".into();
        open_with_edits(&mut client, &edited);

        let (to_server, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::Save), &to_server, &events);

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("silence_ms = 4321"), "{text}");
        assert!(text.contains("mode = \"auto\""), "{text}");
        assert!(text.contains("active_profile = \"senyor\""), "{text}");
        assert_eq!(store::load_or_create(&path).unwrap().config, edited, "the file is the whole form");
        assert_eq!(client.config, edited, "the running configuration is what was written");
        assert!(!std::sync::Arc::ptr_eq(&stack_before.chat, &slot.current().chat), "rebuilt in place");
        assert!(!client.state.config_form.as_ref().unwrap().dirty, "saved means nothing unsaved");
        assert!(client.state.config_form.as_ref().unwrap().notice.is_none());

        // The session learns the mode and the profile over the channel it already uses.
        assert_eq!(
            commands.try_recv().unwrap(),
            ToServer::Command(Command::SetListeningMode { mode: ListeningMode::Auto })
        );
        assert_eq!(
            commands.try_recv().unwrap(),
            ToServer::Command(Command::SetProfile { id: "senyor".into() })
        );
    }

    /// Regenerating fills the row the screen shows and writes nothing: the file changes on save,
    /// only on save, and only through `store::save`.
    #[test]
    fn regenerating_the_token_fills_the_row_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = mock_config("http://127.0.0.1:1");
        store::save(&path, &config).unwrap();
        let before = std::fs::read(&path).unwrap();

        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config, path.clone(), keys, slot);
        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        client.open_screen(&to_server);
        assert!(client.state.config_form.as_ref().unwrap().server.token.is_empty());

        let (events, _results) = tokio::sync::mpsc::unbounded_channel();
        client.on_action(UiAction::Config(ConfigAction::RegenerateToken), &to_server, &events);

        let form = client.state.config_form.as_ref().unwrap();
        assert_eq!(form.server.token.len(), 32, "16 bytes as lowercase hex: {:?}", form.server.token);
        assert!(
            form.server.token.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "the token's alphabet is 0-9a-f: {:?}",
            form.server.token
        );
        assert!(form.dirty, "a fresh token is an unsaved edit like any other");
        assert_eq!(std::fs::read(&path).unwrap(), before, "nothing is written until the user saves");
    }

    /// Review Focus 5: the result of a Test comes back through the client's own event channel and
    /// lands in the screen's notice line with the latency the call really took.
    #[tokio::test]
    async fn the_test_button_reports_a_real_answer_with_its_latency() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(30))
                    .set_body_raw(
                        "data: {\"choices\":[{\"delta\":{\"content\":\"pong\"}}]}\n\ndata: [DONE]\n\n",
                        "text/event-stream",
                    ),
            )
            .mount(&server)
            .await;

        let config = mock_config(&server.uri());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let keys = resolver();
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config.clone(), path, keys, slot);
        client.state.show_config_form(form_of(&config));
        // Task 3's Providers tab lists the three capabilities in order, so its first row is chat's.
        // The assertion on `capability` below is what catches a different layout.
        client.state.config_form.as_mut().unwrap().field = 0;
        // The reply `F2` replays, and the audio the device may still have queued: a probe is a
        // diagnostic, so pressing Test must leave both exactly where they are.
        client.reply = vec![7, 8, 9];

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, mut results) = tokio::sync::mpsc::unbounded_channel();
        press_test(&mut client, &to_server, &events);
        assert_eq!(client.reply, vec![7, 8, 9], "a Test is not a turn: the last reply stays replayable");

        // The probe runs on a task of its own — the HUD never waits for a provider.
        let message = tokio::time::timeout(std::time::Duration::from_secs(5), results.recv())
            .await
            .expect("the probe must answer within five seconds")
            .expect("the client must still hold the channel");
        let ToClient::Event(Event::TestResult { capability, ok, latency_ms, message: text }) = &message else {
            panic!("a Test must answer with TestResult, got {message:?}");
        };
        // Copied out of the borrowed event: the message itself is moved into `on_message` below.
        let (capability, ok, latency_ms, text) = (*capability, *ok, *latency_ms, text.clone());
        assert_eq!(capability, Capability::Chat);
        assert!(ok, "the provider answered: {text}");
        assert!(latency_ms >= 20, "the latency must be the measured one, got {latency_ms}");

        client.on_message(&to_server, message);
        let (kind, notice) = client.state.config_form.as_ref().unwrap().notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Ok);
        assert!(notice.starts_with("chat: "), "{notice}");
        assert!(notice.contains(&format!("{latency_ms} ms")), "{notice}");
    }

    /// "No key" is not "the provider refused": the row names a key nobody holds, so the answer comes
    /// back with no latency at all and no request is ever made.
    #[tokio::test]
    async fn a_test_for_a_row_with_no_key_answers_without_calling_anything() {
        let server = MockServer::start().await; // nothing may reach it
        let config = mock_config(&server.uri());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let keys = resolver(); // holds `openrouter` only
        let slot = ProviderSlot::new(build_providers(&config.providers, &keys).unwrap());
        let mut client = Client::new(config.clone(), path, keys, slot);

        let mut edited = config.clone();
        edited.providers.chat.key = "not-in-the-store".into();
        client.state.show_config_form(form_of(&edited));
        client.state.config_form.as_mut().unwrap().field = 0;

        let (to_server, _commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, mut results) = tokio::sync::mpsc::unbounded_channel();
        press_test(&mut client, &to_server, &events);

        let message = results.try_recv().expect("a row with no key is answered at once, with no call");
        let ToClient::Event(Event::TestResult { capability, ok, latency_ms, message: text }) = &message else {
            panic!("a Test must answer with TestResult, got {message:?}");
        };
        let (capability, ok, latency_ms, text) = (*capability, *ok, *latency_ms, text.clone());
        assert_eq!(capability, Capability::Chat);
        assert!(!ok);
        assert_eq!(latency_ms, 0, "no call was made, so there is no latency to report: {text}");
        assert!(
            text.contains("not-in-the-store") && text.contains("TIPSY_KEY_NOT_IN_THE_STORE"),
            "the answer names the key and how to provide it: {text}"
        );
        assert!(server.received_requests().await.unwrap().is_empty(), "no key must not become a request");

        client.on_message(&to_server, message);
        let (kind, notice) = client.state.config_form.as_ref().unwrap().notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Error);
        assert!(notice.contains("not-in-the-store"), "{notice}");
    }
}
