use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub sync: SyncConfig,
}

#[derive(Serialize, Deserialize, Default)]
pub struct SyncConfig {
    pub username: Option<String>,
    pub password: Option<String>,
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ankr")
        .join("config.toml")
}

pub fn load() -> Config {
    let path = config_path();
    let Ok(text) = std::fs::read_to_string(&path) else { return Config::default() };
    toml::from_str(&text).unwrap_or_default()
}

pub fn save(config: &Config) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, toml::to_string_pretty(config)?)?;
    Ok(())
}
