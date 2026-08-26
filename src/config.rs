use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BroomConfig {
    pub root_path: Option<PathBuf>,
    pub keep_days: Option<u64>,
    pub keep_size_mb: Option<u64>,
    pub experimental_fine: Option<bool>,
    pub fine_only: Option<bool>,
    pub coarse_only: Option<bool>,
    pub trash: Option<bool>,
    pub history: Option<bool>,
    pub hidden: Option<bool>,
    pub ignore: Option<Vec<String>>,
    pub skip: Option<Vec<String>>,
}

impl BroomConfig {
    /// Load config from a specific path, or look in `./broom.toml`, then `~/.config/cargo-broom/config.toml`.
    pub fn load(explicit_path: Option<&Path>) -> Result<Self> {
        if let Some(path) = explicit_path {
            if path.exists() {
                let content = std::fs::read_to_string(path)
                    .with_context(|| format!("Failed to read config file at {}", path.display()))?;
                let config: BroomConfig = toml::from_str(&content).with_context(|| {
                    format!("Failed to parse config file at {}", path.display())
                })?;
                return Ok(config);
            } else {
                anyhow::bail!("Config file specified does not exist: {}", path.display());
            }
        }

        // Try local broom.toml
        let local_path = Path::new("broom.toml");
        if local_path.exists() {
            let content = std::fs::read_to_string(local_path).with_context(|| {
                format!("Failed to read config file at {}", local_path.display())
            })?;
            return toml::from_str(&content).with_context(|| {
                format!("Failed to parse config file at {}", local_path.display())
            });
        }

        // Try ~/.config/cargo-broom/config.toml
        if let Some(home) = dirs_home() {
            let global_path = home.join(".config").join("cargo-broom").join("config.toml");
            if global_path.exists() {
                let content = std::fs::read_to_string(&global_path).with_context(|| {
                    format!("Failed to read config file at {}", global_path.display())
                })?;
                return toml::from_str(&content).with_context(|| {
                    format!("Failed to parse config file at {}", global_path.display())
                });
            }
        }

        Ok(BroomConfig::default())
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
