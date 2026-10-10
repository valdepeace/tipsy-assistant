//! API keys: environment variables first, then the OS credential store.

use std::collections::HashMap;
use std::sync::Mutex;

pub const SERVICE: &str = "tipsy-compadre";

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("credential store error: {0}")]
    Store(String),
}

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>, KeyError>;
    fn set(&self, name: &str, value: &str) -> Result<(), KeyError>;
    fn delete(&self, name: &str) -> Result<(), KeyError>;
}

/// Windows Credential Manager, macOS Keychain or Secret Service on Linux.
pub struct OsKeyring;

impl SecretStore for OsKeyring {
    fn get(&self, name: &str) -> Result<Option<String>, KeyError> {
        let entry = keyring::Entry::new(SERVICE, name).map_err(|e| KeyError::Store(e.to_string()))?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeyError::Store(e.to_string())),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<(), KeyError> {
        let entry = keyring::Entry::new(SERVICE, name).map_err(|e| KeyError::Store(e.to_string()))?;
        entry.set_password(value).map_err(|e| KeyError::Store(e.to_string()))
    }

    fn delete(&self, name: &str) -> Result<(), KeyError> {
        let entry = keyring::Entry::new(SERVICE, name).map_err(|e| KeyError::Store(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KeyError::Store(e.to_string())),
        }
    }
}

#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<String>, KeyError> {
        Ok(self.0.lock().unwrap().get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), KeyError> {
        self.0.lock().unwrap().insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), KeyError> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

/// `openrouter` → `TIPSY_KEY_OPENROUTER`; `my-key.2` → `TIPSY_KEY_MY_KEY_2`.
pub fn env_var_name(name: &str) -> String {
    let suffix: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect();
    format!("TIPSY_KEY_{suffix}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Env,
    Store,
}

pub struct KeyResolver {
    store: Box<dyn SecretStore>,
    env: Box<dyn Fn(&str) -> Option<String> + Send + Sync>,
}

impl KeyResolver {
    pub fn new(store: Box<dyn SecretStore>, env: Box<dyn Fn(&str) -> Option<String> + Send + Sync>) -> Self {
        Self { store, env }
    }

    pub fn system() -> Self {
        Self::new(Box::new(OsKeyring), Box::new(|var| std::env::var(var).ok()))
    }

    pub fn store(&self) -> &dyn SecretStore {
        self.store.as_ref()
    }

    pub fn resolve(&self, name: &str) -> Result<Option<(String, KeySource)>, KeyError> {
        if let Some(v) = (self.env)(&env_var_name(name)).filter(|v| !v.trim().is_empty()) {
            return Ok(Some((v.trim().to_string(), KeySource::Env)));
        }
        Ok(self.store.get(name)?.map(|v| (v, KeySource::Store)))
    }
}

/// `sk-or-v1-abcdef123456` → `sk-or-…3456`. Short keys are fully hidden.
pub fn redact(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 10 {
        return "••••".to_string();
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver(env: &'static [(&'static str, &'static str)]) -> KeyResolver {
        KeyResolver::new(
            Box::new(MemoryStore::default()),
            Box::new(move |var| env.iter().find(|(k, _)| *k == var).map(|(_, v)| v.to_string())),
        )
    }

    #[test]
    fn env_var_names() {
        assert_eq!(env_var_name("openrouter"), "TIPSY_KEY_OPENROUTER");
        assert_eq!(env_var_name("my-key.2"), "TIPSY_KEY_MY_KEY_2");
    }

    #[test]
    fn env_wins_over_store() {
        let r = resolver(&[("TIPSY_KEY_OPENROUTER", " from-env ")]);
        r.store().set("openrouter", "from-store").unwrap();
        assert_eq!(r.resolve("openrouter").unwrap(), Some(("from-env".to_string(), KeySource::Env)));
    }

    #[test]
    fn falls_back_to_store_and_ignores_blank_env() {
        let r = resolver(&[("TIPSY_KEY_OPENROUTER", "   ")]);
        assert_eq!(r.resolve("openrouter").unwrap(), None);
        r.store().set("openrouter", "from-store").unwrap();
        assert_eq!(r.resolve("openrouter").unwrap(), Some(("from-store".to_string(), KeySource::Store)));
        r.store().delete("openrouter").unwrap();
        assert_eq!(r.resolve("openrouter").unwrap(), None);
    }

    #[test]
    fn redaction() {
        assert_eq!(redact("sk-or-v1-abcdef123456"), "sk-or-…3456");
        assert_eq!(redact("short"), "••••");
        // Multi-byte characters must not panic: 12 chars → first 6 + "…" + last 4.
        assert_eq!(redact("ñáéíóúñáéíóú"), "ñáéíóú…éíóú");
    }
}
