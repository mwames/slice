//! `~/.config/slice/config.toml`. Every key is optional:
//!
//! ```toml
//! branch_prefix = "slice/"
//! worktree_root = "~/work/slices"      # default: <repo>/../<repo>.slices/
//!
//! [status.in_progress]
//! clickup = "development"
//! monday = "In Progress"
//!
//! [harvest]
//! project = "Acme Product"
//! task = "New Development"
//! ```

use std::path::PathBuf;

use serde::Deserialize;

use crate::ports::Settings;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub branch_prefix: Option<String>,
    pub worktree_root: Option<String>,
    pub status: StatusConfig,
    pub harvest: HarvestConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatusConfig {
    pub in_progress: InProgress,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InProgress {
    pub clickup: Option<String>,
    pub monday: Option<String>,
}

/// Where slice timers are logged. The defaults match `hv start`.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HarvestConfig {
    pub project: String,
    pub task: String,
}

impl Default for HarvestConfig {
    fn default() -> Self {
        HarvestConfig {
            project: "Acme Product".to_string(),
            task: "New Development".to_string(),
        }
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config"));
        base.join("slice").join("config.toml")
    }

    /// Load the config file; a missing file means all defaults.
    pub fn load() -> Result<Config, String> {
        let path = Config::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => Config::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn settings(&self) -> Settings {
        Settings {
            clickup_in_progress: self
                .status
                .in_progress
                .clickup
                .clone()
                .unwrap_or_else(|| "development".to_string()),
            monday_in_progress: self
                .status
                .in_progress
                .monday
                .clone()
                .unwrap_or_else(|| "In Progress".to_string()),
            branch_prefix: self.branch_prefix.clone().unwrap_or_else(|| "slice/".to_string()),
            worktree_root: self.worktree_root.as_deref().map(expand_home),
        }
    }
}
