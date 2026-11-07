//! `~/.config/slice/config.toml`. Every key is optional:
//!
//! ```toml
//! worktree_root = "~/work/slices"      # default: <repo>/../<repo>.slices/
//!
//! [status.in_progress]                 # set by open
//! clickup = "development"
//! monday = "In Progress"
//!
//! [status.review]                      # set by close
//! clickup = "dev complete"
//! monday = "Waiting for review"
//!
//! [status.done]                        # set by close --done
//! clickup = "complete"
//! monday = "Done"
//!
//! [harvest]                           # default: HARVEST_DEFAULT_PROJECT
//! project = "Acme Product"             # and HARVEST_DEFAULT_TASK
//! task = "New Development"
//!
//! [agents]                            # for slice run
//! spool = "~/bin/spool-linux-arm64"    # required by slice run
//! env_file = "~/.config/slice/agent.env"   # the default
//! ```

use std::path::PathBuf;

use serde::Deserialize;

use crate::ports::{Settings, Statuses};
use crate::run::AgentSettings;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub worktree_root: Option<String>,
    pub status: StatusConfig,
    pub harvest: HarvestConfig,
    pub agents: AgentsConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatusConfig {
    pub in_progress: StatusNames,
    pub review: StatusNames,
    pub done: StatusNames,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatusNames {
    pub clickup: Option<String>,
    pub monday: Option<String>,
}

impl StatusNames {
    fn or(&self, clickup: &str, monday: &str) -> Statuses {
        Statuses {
            clickup: self.clickup.clone().unwrap_or_else(|| clickup.to_string()),
            monday: self.monday.clone().unwrap_or_else(|| monday.to_string()),
        }
    }
}

/// Where slice timers are logged. Empty means Harvest's own defaults,
/// HARVEST_DEFAULT_PROJECT and HARVEST_DEFAULT_TASK, which `hv start` uses too.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HarvestConfig {
    pub project: String,
    pub task: String,
}

/// What slice run mounts and passes into an agent's container.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentsConfig {
    pub spool: Option<String>,
    pub env_file: Option<String>,
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
            in_progress: self.status.in_progress.or("development", "In Progress"),
            review: self.status.review.or("dev complete", "Waiting for review"),
            done: self.status.done.or("complete", "Done"),
            worktree_root: self.worktree_root.as_deref().map(expand_home),
        }
    }

    /// The agent settings for slice run, or an error naming what's missing:
    /// no Spool path, or one that doesn't exist (SLICE-8-3, SLICE-8-4).
    pub fn agent_settings(&self) -> Result<AgentSettings, String> {
        let spool = match self.agents.spool.as_deref() {
            Some(s) if !s.trim().is_empty() => expand_home(s),
            _ => return Err("no Spool binary: set [agents] spool in the slice config".to_string()),
        };
        if !spool.exists() {
            return Err(format!(
                "the Spool binary {} does not exist: check [agents] spool in the slice config",
                spool.display()
            ));
        }
        let env_file = match self.agents.env_file.as_deref() {
            Some(f) => expand_home(f),
            None => home().join(".config/slice/agent.env"),
        };
        Ok(AgentSettings { spool, env_file })
    }
}
