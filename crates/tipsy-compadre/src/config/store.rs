//! Locating, loading and saving `config.toml`.

use std::path::{Path, PathBuf};

use super::model::Config;

pub struct LoadOutcome {
    pub config: Config,
    /// Set when the file was unreadable and had to be replaced by defaults.
    pub warning: Option<String>,
}

pub fn default_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tipsy-compadre")
        .join("config.toml")
}

pub fn save(path: &Path, config: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(config).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
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
    use crate::config::{ProviderKind, Profile};
    use tipsy_protocol::ListeningMode;

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
}
