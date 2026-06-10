use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Push-to-talk-tangent, t.ex. "F9" eller "ctrl+shift+KeyD"
    pub hotkey: String,
    /// KB-Whisper-modell: tiny | base | small | medium | large
    pub model: String,
    /// Språkkod för transkribering
    pub language: String,
    /// Lägg till ett blanksteg efter inskriven text
    pub append_space: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "F9".into(),
            model: "small".into(),
            language: "sv".into(),
            append_space: true,
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .expect("kunde inte hitta konfigurationskatalogen")
        .join("T-Whisper")
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_dir().join("config.toml");
        if path.exists() {
            Ok(toml::from_str(&std::fs::read_to_string(&path)?)?)
        } else {
            let cfg = Self::default();
            std::fs::create_dir_all(config_dir())?;
            std::fs::write(&path, toml::to_string(&cfg)?)?;
            Ok(cfg)
        }
    }
}
