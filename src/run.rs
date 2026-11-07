//! `slice run test|impl`: one agent run in a container on an export of the
//! repository at HEAD, then the path check and staging of its patch
//! (SLICE-8, SLICE-9).

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

/// The role an agent runs as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Test,
    Impl,
}

impl Role {
    /// `test` or `impl`; anything else is `None`.
    pub fn parse(s: &str) -> Option<Role> {
        todo!("SLICE-8-1")
    }
}

/// A repository's `.slice.toml`, with defaults filled in (SLICE-8-4).
#[derive(Clone, Debug, PartialEq)]
pub struct RepoConfig {
    pub test_globs: Vec<String>,
    pub source_globs: Vec<String>,
    pub test_prompt: String,
    pub impl_prompt: String,
    pub test_model: String,
    pub impl_model: String,
    pub timeout_secs: u64,
}

impl RepoConfig {
    /// Parse `.slice.toml`. An unknown key is an error naming it.
    pub fn parse(text: &str) -> Result<RepoConfig, String> {
        todo!("SLICE-8-4")
    }

    /// Whether `role` may write `path`, a repository-relative path with
    /// forward slashes (SLICE-8-5).
    pub fn writable(&self, role: Role, path: &str) -> bool {
        todo!("SLICE-8-5")
    }
}

/// Whether `path` matches `glob`: `**` matches any number of whole path
/// segments, `*` any run of characters within one segment, and everything
/// else itself (SLICE-8-5).
pub fn glob_match(glob: &str, path: &str) -> bool {
    todo!("SLICE-8-5")
}

/// What slice's own config says about agents (SLICE-8-4).
#[derive(Clone, Debug, PartialEq)]
pub struct AgentSettings {
    /// The Linux Spool binary mounted into the container.
    pub spool: PathBuf,
    /// The token's env file, passed to the container by path.
    pub env_file: PathBuf,
}

/// The report schema given to the agent, written to the role directory as
/// `report.schema.json` (SLICE-8-6).
pub const REPORT_SCHEMA: &str = r#"{
  "type": "object",
  "required": ["status", "files_touched", "assumptions", "questions", "summary"],
  "additionalProperties": false,
  "properties": {
    "status": { "type": "string", "enum": ["green", "blocked"] },
    "files_touched": { "type": "array", "items": { "type": "string" } },
    "assumptions": { "type": "array", "items": { "type": "string" } },
    "questions": { "type": "array", "items": { "type": "string" } },
    "summary": { "type": "string", "maxLength": 500 }
  }
}
"#;

/// A path mounted into the container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub host: PathBuf,
    pub container: String,
    pub read_only: bool,
}

/// Everything needed to start the container (SLICE-8-7).
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerSpec {
    pub image: String,
    pub env_file: PathBuf,
    pub timeout_secs: u64,
    pub mounts: Vec<Mount>,
    /// The command run inside the container: `sh -c <script>` running
    /// claude with its flags.
    pub command: Vec<String>,
}

/// How the container ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerExit {
    Exited(i32),
    TimedOut,
    FailedToStart(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerOutput {
    pub exit: ContainerExit,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a container to completion. The real one is Podman.
pub trait Containers {
    fn run(&self, spec: &ContainerSpec) -> ContainerOutput;
}

/// What `slice run` was asked to do.
#[derive(Clone, Debug, PartialEq)]
pub struct RunRequest {
    pub role: Role,
    /// The repository's top level.
    pub repo: PathBuf,
    /// The task's text, read from the --task file.
    pub task: String,
    pub image: String,
    /// Where run directories are created: `$XDG_STATE_HOME/slice/runs`.
    pub runs_dir: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportStatus {
    Green,
    Blocked,
}

/// The agent's structured report (SLICE-8-8).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Report {
    #[serde(deserialize_with = "status_from_str")]
    pub status: ReportStatus,
    pub files_touched: Vec<String>,
    pub assumptions: Vec<String>,
    pub questions: Vec<String>,
    pub summary: String,
}

fn status_from_str<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ReportStatus, D::Error> {
    match String::deserialize(d)?.as_str() {
        "green" => Ok(ReportStatus::Green),
        "blocked" => Ok(ReportStatus::Blocked),
        other => Err(serde::de::Error::custom(format!("unknown status {other}"))),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Staged,
    Rejected,
    Blocked,
    Moved,
    Failed,
}

/// How a run that started a container ended (SLICE-8-8, SLICE-9).
#[derive(Clone, Debug, PartialEq)]
pub struct RunResult {
    pub outcome: Outcome,
    pub run_dir: PathBuf,
    pub report: Option<Report>,
    pub rejected_paths: Vec<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub turns: Option<u64>,
    /// Why the run failed or the patch didn't apply.
    pub reason: Option<String>,
}

impl RunResult {
    /// 0 when the patch was staged, 1 otherwise.
    pub fn exit_code(&self) -> i32 {
        todo!("SLICE-9")
    }

    /// The text `slice run` prints (SLICE-9-1, SLICE-9-3).
    pub fn message(&self) -> String {
        todo!("SLICE-9")
    }

    /// The `--json` output (SLICE-9-5).
    pub fn to_json(&self) -> Value {
        todo!("SLICE-9-5")
    }
}

/// Why `slice run` stopped before starting a container. Each exits 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// SLICE-8-2.
    NotARepository(PathBuf),
    /// SLICE-8-2.
    Dirty(PathBuf),
    /// SLICE-8-3 and SLICE-8-4: what's missing or wrong.
    Setup(String),
}

impl RunError {
    pub fn message(&self) -> String {
        todo!("SLICE-8-2")
    }
}

/// Run one agent and stage its patch if it passes (SLICE-8, SLICE-9).
/// `settings` is slice's own agent config, or an error naming what's
/// missing from it.
pub fn run(
    containers: &dyn Containers,
    settings: Result<AgentSettings, String>,
    request: &RunRequest,
) -> Result<RunResult, RunError> {
    todo!("SLICE-8")
}

/// The repository-relative paths a unified diff adds, changes, deletes or
/// renames, both sides of a rename included (SLICE-9-2).
pub fn patch_paths(patch: &str) -> Vec<String> {
    todo!("SLICE-9-2")
}
