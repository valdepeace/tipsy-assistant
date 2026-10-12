//! Locating, loading and saving `config.toml`.

use std::path::{Path, PathBuf};

use super::model::Config;

pub struct LoadOutcome {
    pub config: Config,
    /// Set when the file was unreadable and had to be replaced by defaults.
    pub warning: Option<String>,
}

/// The environment variable that moves the whole configuration directory somewhere else for one run:
/// `config.toml`, the log beside it and a generated TLS pair.
///
/// **It is what makes a live end-to-end check possible.** The default is the user's real
/// `%APPDATA%\tipsy-compadre`, so running `serve` or `--connect` by hand used to write the user's own
/// configuration, log and certificate, and three tasks of this phase stopped rather than do that.
pub const CONFIG_DIR_ENV: &str = "TIPSY_CONFIG_DIR";

/// Where `config.toml` lives: the directory [`CONFIG_DIR_ENV`] names, or the OS configuration
/// directory.
///
/// Everything this program keeps beside the configuration is derived from this one path — the log
/// through `logging::log_path`, and the generated certificate pair through `server::run`'s
/// `tls::Paths` — so one variable moves all three at once.
pub fn default_path() -> PathBuf {
    config_dir(std::env::var_os(CONFIG_DIR_ENV)).join("config.toml")
}

/// The directory half of [`default_path`], written as a function of the override so both halves can be
/// tested without touching the process environment: `std::env::set_var` is `unsafe` in edition 2024
/// and process-global in a test binary whose tests run in parallel.
fn config_dir(override_dir: Option<std::ffi::OsString>) -> PathBuf {
    match override_dir {
        // An empty variable is not an override: `TIPSY_CONFIG_DIR=` must not leave `config.toml` in
        // whatever directory the process happened to start in.
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("tipsy-compadre"),
    }
}

pub fn save(path: &Path, config: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(config).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// Bytes of OS entropy behind one token: 128 bits, i.e. 32 lowercase hex characters.
const TOKEN_BYTES: usize = 16;
const HEX: &[u8; 16] = b"0123456789abcdef";

/// A fresh `server.token`: [`TOKEN_BYTES`] bytes from the operating system's cryptographic
/// generator (`getrandom` 0.4.3, already in `Cargo.lock` through `rand` 0.10.3, so this adds no
/// crate to the workspace) rendered as 32 lowercase hex characters, alphabet `0-9a-f`.
///
/// Called only when someone asks: the configuration screen's **Regenerate** button
/// (`ConfigEffect::RegenerateToken`, Task 6) or a user pasting a value of their own. Nothing calls
/// it while loading, so an empty `token` stays empty until then (`ServerConfig::has_token`).
pub fn new_token() -> std::io::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    let mut token = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(token)
}

/// Loads the config, creating it with defaults if missing. A corrupt file is renamed to
/// `config.toml.bad-<unix seconds>` and replaced by defaults, never overwritten in place.
pub fn load_or_create(path: &Path) -> std::io::Result<LoadOutcome> {
    if !path.exists() {
        let config = Config::default();
        save(path, &config)?;
        return Ok(LoadOutcome { config, warning: None });
    }
    let text = std::fs::read_to_string(path)?;
    match toml::from_str::<Config>(&text) {
        Ok(config) => Ok(LoadOutcome { config: config.with_builtin_profiles(), warning: None }),
        Err(e) => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let backup = path.with_file_name(format!(
                "{}.bad-{stamp}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("config.toml")
            ));
            std::fs::rename(path, &backup)?;
            let config = Config::default();
            save(path, &config)?;
            Ok(LoadOutcome {
                config,
                warning: Some(format!("config.toml was invalid ({e}); saved as {} and reset", backup.display())),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LoginEntry, ProviderKind, Profile, ServerConfig};
    use tipsy_protocol::ListeningMode;

    /// The override the live end-to-end check needs: one variable moves the configuration, its log
    /// and the generated certificate together, and an empty value is not an override.
    ///
    /// `default_path` itself is not driven here — writing the process environment would be `unsafe`
    /// and racy under parallel tests — so what is pinned is the rule it is built from. The live run
    /// recorded in the ledger is what shows the variable reaches the real process.
    #[test]
    fn the_configuration_directory_can_be_moved_for_one_run() {
        let dir = tempfile::tempdir().unwrap();
        let chosen = dir.path().join("tipsy-live");
        assert_eq!(config_dir(Some(chosen.clone().into_os_string())), chosen);

        let fallback = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("tipsy-compadre");
        assert_eq!(config_dir(Some(std::ffi::OsString::new())), fallback, "an empty override is not one");
        assert_eq!(config_dir(None), fallback, "no override is the OS configuration directory");
        assert!(config_dir(None).ends_with("tipsy-compadre"));
    }

    #[test]
    fn creates_default_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.toml");
        let out = load_or_create(&path).unwrap();
        assert!(path.exists());
        assert!(out.warning.is_none());
        assert_eq!(out.config.active().id, "compae");
        assert_eq!(out.config.providers.chat.model, "google/gemini-2.5-flash");
        assert_eq!(out.config.server.bind, "127.0.0.1:7070");
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.listening.mode = ListeningMode::Auto;
        c.providers.speech.kind = ProviderKind::Elevenlabs;
        save(&path, &c).unwrap();
        assert_eq!(load_or_create(&path).unwrap().config, c);
    }

    #[test]
    fn partial_file_gets_defaults_and_keeps_user_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"
active_profile = "mine"

[listening]
mode = "auto"

[[profiles]]
id = "mine"
name = "Mi Compadre"
language = "es-ES"
"#).unwrap();
        let c = load_or_create(&path).unwrap().config;
        assert_eq!(c.listening.mode, ListeningMode::Auto);
        assert_eq!(c.listening.silence_ms, 1000);
        assert_eq!(c.providers.transcribe.key, "openrouter");
        assert_eq!(c.active().name, "Mi Compadre");
        assert!(c.profiles.iter().any(|p| p.id == "compae"), "built-in profile must be added");
    }

    /// The vocabulary the configuration uses changed; the files already on disk did not. A
    /// `config.toml` written with `active_persona` and `[[personas]]` must load to exactly the
    /// configuration the new spelling produces — same active profile, same nine profiles in the same
    /// order — and a save must write the new spelling, so the alias is the only place the old words
    /// survive.
    #[test]
    fn a_file_written_before_the_rename_loads_the_same_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let before = dir.path().join("before.toml");
        let after = dir.path().join("after.toml");
        std::fs::write(&before, r#"
active_persona = "mine"

[listening]
mode = "auto"

[[personas]]
id = "mine"
name = "Mi Compadre"
language = "es-ES"
"#).unwrap();
        std::fs::write(&after, r#"
active_profile = "mine"

[listening]
mode = "auto"

[[profiles]]
id = "mine"
name = "Mi Compadre"
language = "es-ES"
"#).unwrap();

        let from_before = load_or_create(&before).unwrap().config;
        let from_after = load_or_create(&after).unwrap().config;
        assert_eq!(from_before, from_after, "the old keys must load as the new ones do");
        // Not merely ignored: the old keys were read, so the user's own profile is the active one.
        assert_eq!(from_before.active().name, "Mi Compadre");
        assert_eq!(from_before.profiles.len(), 10, "the nine built-ins join the user's own profile");

        let saved = dir.path().join("saved.toml");
        save(&saved, &from_before).unwrap();
        let text = std::fs::read_to_string(&saved).unwrap();
        assert!(text.contains("active_profile = \"mine\""), "{text}");
        assert!(text.contains("[[profiles]]"), "{text}");
        assert!(!text.contains("active_persona"), "a save writes the new spelling only: {text}");
        assert!(!text.contains("[[personas]]"), "a save writes the new spelling only: {text}");
        assert_eq!(load_or_create(&saved).unwrap().config, from_before);
    }

    #[test]
    fn user_override_of_builtin_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[[profiles]]\nid = \"compae\"\nname = \"Mi versión\"\n").unwrap();
        let c = load_or_create(&path).unwrap().config;
        let compaes: Vec<&Profile> = c.profiles.iter().filter(|p| p.id == "compae").collect();
        assert_eq!(compaes.len(), 1);
        assert_eq!(compaes[0].name, "Mi versión");
    }

    #[test]
    fn overriding_one_builtin_keeps_the_other_eight() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[[profiles]]\nid = \"compae\"\nname = \"Mi Compae\"\n").unwrap();
        let config = load_or_create(&path).unwrap().config;
        assert_eq!(config.profiles.len(), 9, "the override must not remove the other built-ins");
        let compaes: Vec<&Profile> = config.profiles.iter().filter(|p| p.id == "compae").collect();
        assert_eq!(compaes.len(), 1);
        assert_eq!(compaes[0].name, "Mi Compae");
    }

    #[test]
    fn corrupt_file_is_backed_up_and_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is = = not toml [[[").unwrap();
        let out = load_or_create(&path).unwrap();
        assert!(out.warning.is_some());
        assert_eq!(out.config, Config::default());
        let backups: Vec<_> = std::fs::read_dir(dir.path()).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("config.toml.bad-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(backups[0].path()).unwrap(), "this is = = not toml [[[");
    }

    #[test]
    fn voice_key_and_voice_lookup() {
        let c = Config::default();
        assert_eq!(c.providers.speech.voice_key(), "gemini");
        assert_eq!(c.active().voice_for(&c.providers.speech), Some("Charon".to_string()));
        let mut eleven = c.providers.speech.clone();
        eleven.model = "elevenlabs/eleven-flash-v2.5".into();
        assert_eq!(c.active().voice_for(&eleven), Some("george".to_string()));
        eleven.model = "deepgram/aura-2".into();
        assert_eq!(c.active().voice_for(&eleven), None);
    }

    /// Review Focus 3, first half: a `config.toml` written by Plan 1 (no `token`, no
    /// `mic_device`) is a valid file, not a corrupt one: it loads, nothing else moves, and the two
    /// new fields come up at their defaults.
    #[test]
    fn plan_1_file_without_token_or_mic_device_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"
active_profile = "compae"

[server]
bind = "127.0.0.1:7070"
lan = false

[listening]
mode = "auto"
silence_ms = 1500
mic_gain = 9.5
"#).unwrap();

        let out = load_or_create(&path).unwrap();
        assert!(out.warning.is_none(), "an old file is not a corrupt file: {:?}", out.warning);
        assert!(out.config.server.token.is_empty(), "the token defaults in empty, never generated");
        assert!(!out.config.server.has_token(), "an empty token means no token yet");
        assert!(out.config.listening.mic_device.is_none(), "no device is configured by default");
        // Nothing else moved.
        assert_eq!(out.config.server.bind, "127.0.0.1:7070");
        assert!(!out.config.server.lan);
        assert_eq!(out.config.listening.mode, ListeningMode::Auto);
        assert_eq!(out.config.listening.silence_ms, 1500);
        assert_eq!(out.config.listening.mic_gain, 9.5);
        // Loading is not a migration: the user's file is left exactly as it was.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("token"), "loading must not rewrite the file, got:\n{text}");
    }

    /// Review Focus 3, second half: nothing is cached between loads, so a file edited while the app
    /// runs is seen the next time the screen opens one — including a token pasted in by hand.
    #[test]
    fn a_file_edited_after_the_first_load_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[server]\nbind = \"127.0.0.1:7070\"\n").unwrap();
        assert!(load_or_create(&path).unwrap().config.server.token.is_empty());

        std::fs::write(&path, "[server]\nbind = \"127.0.0.1:7070\"\ntoken = \"pasted-by-hand\"\n").unwrap();
        let again = load_or_create(&path).unwrap().config;
        assert_eq!(again.server.token, "pasted-by-hand");
        assert!(again.server.has_token());
    }

    /// The value the WebSocket server will require in Plan 3 is the one that comes back from the
    /// file, spelled the same way.
    #[test]
    fn token_survives_a_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.server.token = "0123456789abcdef0123456789abcdef".into();
        c.server.lan = true;
        c.listening.mic_device = Some("Rode NT-USB".into());
        save(&path, &c).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("token = \"0123456789abcdef0123456789abcdef\""),
            "the token must be written under [server], got:\n{text}"
        );
        assert!(
            text.contains("mic_device = \"Rode NT-USB\""),
            "the device must be written under [listening], got:\n{text}"
        );
        assert_eq!(load_or_create(&path).unwrap().config, c);
    }

    /// `mic_device` is an `Option` because toml skips a `None` field instead of refusing to write
    /// the file, and because "no device chosen" must not be spelled as an empty device name.
    #[test]
    fn absent_mic_device_is_none_and_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let c = Config::default();
        save(&path, &c).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("mic_device"), "a None device is skipped, got:\n{text}");
        assert!(text.contains("mic_gain = 12.0"), "the rest of [listening] is still written:\n{text}");

        let back = load_or_create(&path).unwrap().config;
        assert!(back.listening.mic_device.is_none());
        assert_eq!(back, c);
    }

    /// The documented shape of a generated token: 32 lowercase hex characters, and two calls do not
    /// collide. The shape is the contract the screen's row and Plan 3's handshake rely on.
    #[test]
    fn new_token_has_the_documented_shape_and_differs_between_calls() {
        let first = new_token().unwrap();
        let second = new_token().unwrap();
        assert_eq!(first.len(), 32, "16 bytes as lowercase hex, got {first:?}");
        assert!(
            first.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "the alphabet is 0-9a-f, got {first:?}"
        );
        assert_ne!(first, second, "two generations must not return the same token");
        let server = ServerConfig { token: first.clone(), ..ServerConfig::default() };
        assert!(server.has_token(), "a generated token counts as configured");
    }

    /// An empty token is a state, not an error: it is what a fresh install and every file written
    /// before this plan carry, and loading never fills it in — only the screen's Regenerate button
    /// (or the user's own paste) does.
    #[test]
    fn empty_token_is_a_state_and_is_never_filled_in_on_load() {
        assert!(!ServerConfig::default().has_token());
        assert!(!Config::default().server.has_token());
        let blank = ServerConfig { token: "   ".into(), ..ServerConfig::default() };
        assert!(!blank.has_token(), "a hand-edited blank token is not a token");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[server]\nbind = \"0.0.0.0:7070\"\nlan = true\n").unwrap();
        let loaded = load_or_create(&path).unwrap();
        assert!(loaded.warning.is_none(), "a missing token is not a corrupt file");
        assert!(loaded.config.server.token.is_empty(), "nothing is generated on load");
        assert!(!loaded.config.server.has_token());
    }

    /// Review Focus 3, the config half: a `config.toml` written before this plan has no `[realtime]`
    /// section at all, and it is a valid file, not a corrupt one. Every field of the new section
    /// defaults in and nothing else moves.
    #[test]
    fn a_config_written_before_this_plan_gets_the_realtime_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"
active_profile = "compae"

[providers.chat]
kind = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
model = "google/gemini-2.5-flash"
key = "openrouter"

[listening]
mode = "auto"
silence_ms = 1500
mic_gain = 9.5
"#).unwrap();

        let out = load_or_create(&path).unwrap();
        assert!(out.warning.is_none(), "an old file is not a corrupt file: {:?}", out.warning);
        let realtime = &out.config.realtime;
        assert!(!realtime.enabled, "the mode must never be silently on");
        assert_eq!(realtime.model, "gpt-realtime-2.1");
        assert_eq!(realtime.key, "openai", "a key *name*: the resolver looks the value up");
        assert_eq!(realtime.voice, "marin");
        assert_eq!(realtime.max_minutes, 10);
        // Turn detection is not a `[realtime]` field: it reuses the listening silence, which the old
        // file already carries and which must not be touched by this task.
        assert_eq!(out.config.listening.silence_ms, 1500);
        // Loading is not a migration: the user's file is left exactly as it was.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("[realtime]"), "loading must not rewrite the file, got:\n{text}");
    }

    /// A file from before this feature has no `[login]` section, and loading it must not invent one in
    /// the file: the screen never appears (no entries, so `applies` is false), and the file is untouched.
    #[test]
    fn a_file_without_the_login_section_loads_and_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[listening]\nmode = \"auto\"\n").unwrap();
        let c = load_or_create(&path).unwrap().config;
        assert!(c.login.enabled, "the switch defaults on; the entries are what make a screen");
        assert!(c.login.voice);
        assert!(c.login.allowed.is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("login"), "loading must not rewrite the file, got:\n{text}");
    }

    /// The section survives a save and a load with every field, and it is written as its own table.
    #[test]
    fn the_realtime_section_survives_a_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.realtime.enabled = true;
        config.realtime.model = "gpt-realtime-2.1-mini".into();
        config.realtime.key = "openai-realtime".into();
        config.realtime.voice = "cedar".into();
        config.realtime.max_minutes = 25;
        save(&path, &config).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        for line in [
            "[realtime]",
            "enabled = true",
            "model = \"gpt-realtime-2.1-mini\"",
            "key = \"openai-realtime\"",
            "voice = \"cedar\"",
            "max_minutes = 25",
        ] {
            assert!(text.contains(line), "the file must contain `{line}`, got:\n{text}");
        }
        assert!(!text.contains("silence_ms = 25"), "silence is not a realtime field:\n{text}");
        assert_eq!(load_or_create(&path).unwrap().config.realtime, config.realtime);
    }

    /// The section round-trips through the file, entries in order: file order is the ranking the matcher
    /// uses (spec R5), so a save that reordered them would change who is greeted first.
    #[test]
    fn login_round_trips_and_keeps_its_entry_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.login.enabled = true;
        c.login.voice = false;
        c.login.allowed = vec![
            LoginEntry { name: "Andrés".into(), keywords: vec!["andres".into(), "core".into()] },
            LoginEntry { name: "María".into(), keywords: vec!["maria".into(), "data".into()] },
        ];
        save(&path, &c).unwrap();
        let again = load_or_create(&path).unwrap().config;
        assert_eq!(again.login, c.login);
        assert_eq!(again.login.allowed[0].name, "Andrés", "the first entry is still first");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[login.allowed]]"), "the array is written as an array of tables:\n{text}");
    }
}
