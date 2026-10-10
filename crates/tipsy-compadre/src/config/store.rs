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
        Ok(config) => Ok(LoadOutcome { config: config.with_builtin_personas(), warning: None }),
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
    use crate::config::{ProviderKind, Persona};
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
    fn partial_file_gets_defaults_and_keeps_user_personas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"
active_persona = "mine"

[listening]
mode = "auto"

[[personas]]
id = "mine"
name = "Mi Compadre"
language = "es-ES"
"#).unwrap();
        let c = load_or_create(&path).unwrap().config;
        assert_eq!(c.listening.mode, ListeningMode::Auto);
        assert_eq!(c.listening.silence_ms, 1000);
        assert_eq!(c.providers.transcribe.key, "openrouter");
        assert_eq!(c.active().name, "Mi Compadre");
        assert!(c.personas.iter().any(|p| p.id == "compae"), "built-in persona must be added");
    }

    #[test]
    fn user_override_of_builtin_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[[personas]]\nid = \"compae\"\nname = \"Mi versión\"\n").unwrap();
        let c = load_or_create(&path).unwrap().config;
        let compaes: Vec<&Persona> = c.personas.iter().filter(|p| p.id == "compae").collect();
        assert_eq!(compaes.len(), 1);
        assert_eq!(compaes[0].name, "Mi versión");
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
