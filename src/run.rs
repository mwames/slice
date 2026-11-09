//! `slice run test|impl`: one agent run in a container on an export of the
//! repository's staged state (HEAD plus whatever is staged), then the path
//! check and staging of its patch (SLICE-8, SLICE-9).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde_json::{Value, json};

/// The role an agent runs as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Test,
    Impl,
}

impl Role {
    /// `test` or `impl`; anything else is `None`.
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "test" => Some(Role::Test),
            "impl" => Some(Role::Impl),
            _ => None,
        }
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
        let raw: RawRepoConfig = toml::from_str(text).map_err(|e| e.to_string())?;
        Ok(RepoConfig {
            test_globs: raw.paths.test,
            source_globs: raw.paths.source,
            test_prompt: raw
                .agent
                .test_prompt
                .unwrap_or_else(|| ".slice/test-agent.md".to_string()),
            impl_prompt: raw
                .agent
                .impl_prompt
                .unwrap_or_else(|| ".slice/impl-agent.md".to_string()),
            test_model: raw.agent.test_model.unwrap_or_else(|| "opus".to_string()),
            impl_model: raw.agent.impl_model.unwrap_or_else(|| "sonnet".to_string()),
            timeout_secs: raw.agent.timeout_secs.unwrap_or(2700),
        })
    }

    /// Whether `role` may write `path`, a repository-relative path with
    /// forward slashes (SLICE-8-5).
    pub fn writable(&self, role: Role, path: &str) -> bool {
        let matches = |globs: &[String]| globs.iter().any(|g| glob_match(g, path));
        match role {
            Role::Test => matches(&self.test_globs),
            Role::Impl => matches(&self.source_globs) && !matches(&self.test_globs),
        }
    }

    fn globs(&self, role: Role) -> &[String] {
        match role {
            Role::Test => &self.test_globs,
            Role::Impl => &self.source_globs,
        }
    }

    fn prompt(&self, role: Role) -> &str {
        match role {
            Role::Test => &self.test_prompt,
            Role::Impl => &self.impl_prompt,
        }
    }

    fn model(&self, role: Role) -> &str {
        match role {
            Role::Test => &self.test_model,
            Role::Impl => &self.impl_model,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawRepoConfig {
    paths: RawPaths,
    agent: RawAgent,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawPaths {
    test: Vec<String>,
    source: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawAgent {
    test_prompt: Option<String>,
    impl_prompt: Option<String>,
    test_model: Option<String>,
    impl_model: Option<String>,
    timeout_secs: Option<u64>,
}

/// Whether `path` matches `glob`: `**` matches any number of whole path
/// segments, `*` any run of characters within one segment, and everything
/// else itself (SLICE-8-5).
pub fn glob_match(glob: &str, path: &str) -> bool {
    let globs: Vec<&str> = glob.split('/').collect();
    let parts: Vec<&str> = path.split('/').collect();
    segments_match(&globs, &parts)
}

fn segments_match(globs: &[&str], parts: &[&str]) -> bool {
    match globs.split_first() {
        None => parts.is_empty(),
        Some((&"**", rest)) => (0..=parts.len()).any(|skip| segments_match(rest, &parts[skip..])),
        Some((g, rest)) => match parts.split_first() {
            Some((p, tail)) => segment_match(g.as_bytes(), p.as_bytes()) && segments_match(rest, tail),
            None => false,
        },
    }
}

/// `*` matches any run of characters; everything else matches itself.
fn segment_match(glob: &[u8], s: &[u8]) -> bool {
    match glob.split_first() {
        None => s.is_empty(),
        Some((b'*', rest)) => (0..=s.len()).any(|skip| segment_match(rest, &s[skip..])),
        Some((c, rest)) => s.first() == Some(c) && segment_match(rest, &s[1..]),
    }
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
    /// HEAD's full hash when the run started (SLICE-9-1, SLICE-9-5).
    pub head: String,
    /// The paths staged when the run started, sorted (SLICE-9-1,
    /// SLICE-9-5).
    pub staged: Vec<String>,
    /// HEAD's short hash as `git rev-parse --short` prints it, for the
    /// message (SLICE-9-1).
    pub head_short: String,
}

impl RunResult {
    /// 0 when the patch was staged, 1 otherwise.
    pub fn exit_code(&self) -> i32 {
        if self.outcome == Outcome::Staged { 0 } else { 1 }
    }

    /// The text `slice run` prints (SLICE-9-1, SLICE-9-3).
    pub fn message(&self) -> String {
        let dir = self.run_dir.display();
        let patch = self.run_dir.join("changes.patch");
        let why = self.reason.as_deref().unwrap_or("no reason given");
        let mut out = String::new();
        match self.outcome {
            Outcome::Failed => {
                out.push_str(&format!("run failed: {why}\n"));
                out.push_str(&format!("  run directory {dir}\n"));
                out.push_str(&format!("  agent log {}\n", self.run_dir.join("agent.log").display()));
                return out;
            }
            Outcome::Staged => out.push_str("staged the patch; nothing is committed\n"),
            Outcome::Blocked => out.push_str("blocked: nothing staged\n"),
            Outcome::Rejected => {
                out.push_str("rejected: the patch touches paths outside the role's write set; nothing staged\n");
                for p in &self.rejected_paths {
                    out.push_str(&format!("  {p}\n"));
                }
            }
            Outcome::Moved => out.push_str(&format!("nothing staged: {why}\n")),
        }
        if let Some(r) = &self.report {
            let status = match r.status {
                ReportStatus::Green => "green",
                ReportStatus::Blocked => "blocked",
            };
            out.push_str(&format!("status: {status}\nsummary: {}\n", r.summary));
            push_list(&mut out, "questions", &r.questions);
            push_list(&mut out, "assumptions", &r.assumptions);
            push_list(&mut out, "files", &r.files_touched);
        }
        if let Some(c) = self.cost_usd {
            out.push_str(&format!("cost: ${c:.2}\n"));
        }
        if let Some(ms) = self.duration_ms {
            out.push_str(&format!("duration: {:.1} s\n", ms as f64 / 1000.0));
        }
        out.push_str(&format!("run directory: {dir}\n"));
        out.push_str(&format!("started from HEAD {}\n", self.head_short));
        push_list(&mut out, "staged when it started", &self.staged);
        if self.outcome != Outcome::Staged {
            out.push_str(&format!("patch kept at {}\n", patch.display()));
        }
        out
    }

    /// The `--json` output (SLICE-9-5).
    pub fn to_json(&self) -> Value {
        let outcome = match self.outcome {
            Outcome::Staged => "staged",
            Outcome::Rejected => "rejected",
            Outcome::Blocked => "blocked",
            Outcome::Moved => "moved",
            Outcome::Failed => "failed",
        };
        let report = self.report.as_ref().map(|r| {
            json!({
                "status": match r.status {
                    ReportStatus::Green => "green",
                    ReportStatus::Blocked => "blocked",
                },
                "files_touched": r.files_touched,
                "assumptions": r.assumptions,
                "questions": r.questions,
                "summary": r.summary,
            })
        });
        json!({
            "outcome": outcome,
            "run_dir": self.run_dir.display().to_string(),
            "report": report,
            "rejected_paths": self.rejected_paths,
            "cost_usd": self.cost_usd,
            "duration_ms": self.duration_ms,
            "turns": self.turns,
            "head": self.head,
            "staged": self.staged,
        })
    }
}

/// A repository with unstaged changes to tracked files (SLICE-8-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unstaged {
    pub repo: PathBuf,
    /// The tracked paths whose working-tree contents differ from the
    /// index, sorted.
    pub paths: Vec<String>,
}

/// Why `slice run` stopped before starting a container. Each exits 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// SLICE-8-2.
    NotARepository(PathBuf),
    /// SLICE-8-2: the working tree differs from the index.
    Dirty(Unstaged),
    /// SLICE-8-3 and SLICE-8-4: what's missing or wrong.
    Setup(String),
}

impl RunError {
    pub fn message(&self) -> String {
        match self {
            RunError::NotARepository(p) => format!("{} is not a git repository", p.display()),
            RunError::Dirty(u) => format!(
                "{} has unstaged changes to tracked files; stage them before slice run:\n  {}",
                u.repo.display(),
                u.paths.join("\n  ")
            ),
            RunError::Setup(what) => what.clone(),
        }
    }
}

fn push_list(out: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("{title}:\n"));
    for i in items {
        out.push_str(&format!("  - {i}\n"));
    }
}

/// Run git with an identity given on the command line and the given
/// environment; returns stdout, or stderr as the error.
fn git<I, S>(args: I, envs: &[(&str, &Path)]) -> Result<Vec<u8>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "user.name=slice",
            "-c",
            "user.email=slice@localhost",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.quotepath=false",
        ])
        .args(args);
    for (k, v) in envs {
        command.env(k, v);
    }
    let out = command.output().map_err(|e| format!("run git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(out.stdout)
}

fn git_text(repo: &Path, args: &[&str]) -> Result<String, String> {
    let mut full = vec!["-C", repo.to_str().ok_or("repository path is not UTF-8")?];
    full.extend(args);
    git(full, &[]).map(|o| String::from_utf8_lossy(&o).trim().to_string())
}

/// `'...'` for a shell, with embedded quotes escaped.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The `sh -c` script that runs claude on the role directory's files.
fn claude_script(model: &str) -> String {
    format!(
        "cd /work && exec claude -p \"$(cat /role/task.md)\" --output-format json \
         --model {} --append-system-prompt \"$(cat /role/prompt.md)\" \
         --json-schema \"$(cat /role/report.schema.json)\" \
         --permission-mode bypassPermissions --setting-sources '' \
         --strict-mcp-config --no-session-persistence",
        shell_quote(model)
    )
}

/// A new run directory under `runs_dir`, named for the role.
fn create_run_dir(runs_dir: &Path, role: Role) -> Result<PathBuf, String> {
    std::fs::create_dir_all(runs_dir).map_err(|e| format!("create {}: {e}", runs_dir.display()))?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = match role {
        Role::Test => "test",
        Role::Impl => "impl",
    };
    for n in 0.. {
        let dir = runs_dir.join(format!("{secs}-{name}-{n}"));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create {}: {e}", dir.display())),
        }
    }
    unreachable!()
}

/// The index's entries, as `git ls-files -s -z` prints them.
fn index_entries(repo: &Path) -> Result<Vec<u8>, String> {
    git(["-C", &repo.to_string_lossy(), "ls-files", "-s", "-z"], &[])
}

/// The regular files in the index.
fn index_files(entries: &[u8]) -> Result<Vec<String>, String> {
    let mut paths: Vec<String> = Vec::new();
    for entry in String::from_utf8_lossy(entries).split('\0').filter(|e| !e.is_empty()) {
        let (meta, path) = entry.split_once('\t').ok_or("unexpected ls-files output")?;
        // A symlink or submodule can't be mounted read-only safely: its host
        // path may lead out of the run directory.
        if !meta.starts_with("120000") && !meta.starts_with("160000") && !paths.iter().any(|p| p == path) {
            paths.push(path.to_string());
        }
    }
    Ok(paths)
}

/// The sorted paths of `git diff --name-only <args>`, renames as two paths.
fn diff_paths(repo: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let repo_arg = repo.to_string_lossy().to_string();
    let mut full = vec!["-C", repo_arg.as_str(), "diff", "--name-only", "-z", "--no-renames"];
    full.extend(args);
    let out = git(full, &[])?;
    let mut paths: Vec<String> = String::from_utf8_lossy(&out)
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    paths.sort();
    Ok(paths)
}

/// Export the index into `workspace`, and commit it as a baseline in `baseline`.
fn make_workspace(repo: &Path, run_dir: &Path, workspace: &Path, baseline: &Path) -> Result<(), String> {
    let index = run_dir.join("export.index");
    let envs = [("GIT_INDEX_FILE", index.as_path())];
    let repo_arg = repo.to_string_lossy().to_string();
    let real_index = git_text(repo, &["rev-parse", "--path-format=absolute", "--git-path", "index"])?;
    std::fs::copy(&real_index, &index).map_err(|e| format!("copy the index: {e}"))?;
    let prefix = format!("{}/", workspace.display());
    git(["-C", &repo_arg, "checkout-index", "-a", "--prefix", &prefix], &envs)?;
    std::fs::remove_file(&index).map_err(|e| e.to_string())?;

    git(["init", "-q", "--bare", &baseline.to_string_lossy()], &[])?;
    let dir = format!("--git-dir={}", baseline.display());
    git([&dir, "config", "core.bare", "false"], &[])?;
    let tree = format!("--work-tree={}", workspace.display());
    git([&dir, &tree, "add", "-A"], &[])?;
    git([&dir, &tree, "commit", "-q", "--allow-empty", "-m", "baseline"], &[])?;
    Ok(())
}

/// The workspace's changes against its baseline, new files included.
fn workspace_patch(workspace: &Path, baseline: &Path) -> Result<Vec<u8>, String> {
    let dir = format!("--git-dir={}", baseline.display());
    let tree = format!("--work-tree={}", workspace.display());
    git([&dir, &tree, "add", "-A"], &[])?;
    git([&dir, &tree, "diff", "--cached", "--binary", "--no-renames"], &[])
}

struct Output {
    report: Report,
    cost_usd: Option<f64>,
    duration_ms: Option<u64>,
    turns: Option<u64>,
}

/// The report, cost, duration and turns in claude's JSON output.
fn read_output(stdout: &str) -> Result<Output, String> {
    let value: Value = serde_json::from_str(stdout).map_err(|e| format!("the output is not JSON: {e}"))?;
    let structured = value
        .get("structured_output")
        .filter(|v| !v.is_null())
        .ok_or("the output has no structured_output")?;
    let report = Report::deserialize(structured).map_err(|e| format!("the report is not valid: {e}"))?;
    Ok(Output {
        report,
        cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
        duration_ms: value.get("duration_ms").and_then(Value::as_u64),
        turns: value.get("num_turns").and_then(Value::as_u64),
    })
}

/// Run one agent and stage its patch if it passes (SLICE-8, SLICE-9).
/// `settings` is slice's own agent config, or an error naming what's
/// missing from it.
pub fn run(
    containers: &dyn Containers,
    settings: Result<AgentSettings, String>,
    request: &RunRequest,
) -> Result<RunResult, RunError> {
    let role = request.role;
    let repo = git_text(&request.repo, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .map_err(|_| RunError::NotARepository(request.repo.clone()))?;
    let unstaged = diff_paths(&repo, &[]).map_err(|e| RunError::Setup(format!("git diff: {e}")))?;
    if !unstaged.is_empty() {
        return Err(RunError::Dirty(Unstaged { repo, paths: unstaged }));
    }

    let settings = settings.map_err(RunError::Setup)?;
    let head = git_text(&repo, &["rev-parse", "--verify", "HEAD"])
        .map_err(|e| RunError::Setup(format!("{} has no commit at HEAD: {e}", repo.display())))?;
    let head_short = git_text(&repo, &["rev-parse", "--short", "HEAD"]).map_err(RunError::Setup)?;
    let staged = diff_paths(&repo, &["--cached", "HEAD"]).map_err(RunError::Setup)?;
    let entries = index_entries(&repo).map_err(RunError::Setup)?;
    let toml = git_text(&repo, &["show", ":.slice.toml"])
        .map_err(|_| RunError::Setup(format!(".slice.toml is missing from the index in {}", repo.display())))?;
    let config = RepoConfig::parse(&toml).map_err(|e| RunError::Setup(format!(".slice.toml: {e}")))?;
    if config.globs(role).is_empty() {
        let (key, name) = match role {
            Role::Test => ("test", "test"),
            Role::Impl => ("source", "impl"),
        };
        return Err(RunError::Setup(format!(
            "no {key} globs: .slice.toml needs [paths] {key} for the {name} role"
        )));
    }
    let prompt_path = config.prompt(role);
    let prompt = git(
        [
            "-C".to_string(),
            repo.to_string_lossy().to_string(),
            "cat-file".to_string(),
            "blob".to_string(),
            format!(":{prompt_path}"),
        ],
        &[],
    )
    .map_err(|_| RunError::Setup(format!("the role prompt {prompt_path} is missing from the index")))?;
    let tracked = index_files(&entries).map_err(RunError::Setup)?;

    let setup = |what: &str, e: String| RunError::Setup(format!("{what}: {e}"));
    let runs_dir = std::path::absolute(&request.runs_dir).map_err(|e| setup("runs directory", e.to_string()))?;
    let run_dir = create_run_dir(&runs_dir, role).map_err(RunError::Setup)?;
    let workspace = run_dir.join("workspace");
    let role_dir = run_dir.join("role");
    let baseline = run_dir.join("baseline.git");
    make_workspace(&repo, &run_dir, &workspace, &baseline).map_err(|e| setup("export the index", e))?;
    std::fs::create_dir(&role_dir).map_err(|e| setup("role directory", e.to_string()))?;
    for (name, content) in [
        ("prompt.md", prompt.as_slice()),
        ("task.md", request.task.as_bytes()),
        ("report.schema.json", REPORT_SCHEMA.as_bytes()),
    ] {
        std::fs::write(role_dir.join(name), content).map_err(|e| setup("role directory", e.to_string()))?;
    }

    let mut mounts = vec![Mount {
        host: workspace.clone(),
        container: "/work".to_string(),
        read_only: false,
    }];
    for path in tracked.iter().filter(|p| !config.writable(role, p)) {
        mounts.push(Mount {
            host: workspace.join(path),
            container: format!("/work/{path}"),
            read_only: true,
        });
    }
    mounts.push(Mount {
        host: role_dir,
        container: "/role".to_string(),
        read_only: false,
    });
    mounts.push(Mount {
        host: settings.spool.clone(),
        container: "/usr/local/bin/spool".to_string(),
        read_only: true,
    });
    let spec = ContainerSpec {
        image: request.image.clone(),
        env_file: settings.env_file.clone(),
        timeout_secs: config.timeout_secs,
        mounts,
        command: vec!["sh".to_string(), "-c".to_string(), claude_script(config.model(role))],
    };
    let output = containers.run(&spec);

    let _ = std::fs::write(run_dir.join("result.json"), &output.stdout);
    let _ = std::fs::write(run_dir.join("agent.log"), &output.stderr);
    let mut result = RunResult {
        outcome: Outcome::Failed,
        run_dir: run_dir.clone(),
        report: None,
        rejected_paths: Vec::new(),
        cost_usd: None,
        duration_ms: None,
        turns: None,
        reason: None,
        head: head.clone(),
        staged,
        head_short,
    };
    let parsed = match &output.exit {
        ContainerExit::FailedToStart(why) => Err(format!("the container could not be started: {why}")),
        ContainerExit::TimedOut => Err(format!("the container timed out after {} s", config.timeout_secs)),
        ContainerExit::Exited(code) if *code != 0 => Err(format!("the container exited with status {code}")),
        ContainerExit::Exited(_) => read_output(&output.stdout),
    };
    let patch = workspace_patch(&workspace, &baseline);
    let output = match (parsed, &patch) {
        (Ok(output), Ok(_)) => output,
        (Err(why), _) => {
            result.reason = Some(why);
            return Ok(result);
        }
        (Ok(_), Err(why)) => {
            result.reason = Some(format!("could not read the workspace's changes: {why}"));
            return Ok(result);
        }
    };
    let patch = patch.unwrap_or_default();
    let patch_file = run_dir.join("changes.patch");
    if let Err(e) = std::fs::write(&patch_file, &patch) {
        result.reason = Some(format!("could not write changes.patch: {e}"));
        return Ok(result);
    }

    result.cost_usd = output.cost_usd;
    result.duration_ms = output.duration_ms;
    result.turns = output.turns;
    let green = output.report.status == ReportStatus::Green;
    result.report = Some(output.report);

    let rejected: Vec<String> = patch_paths(&String::from_utf8_lossy(&patch))
        .into_iter()
        .filter(|p| !config.writable(role, p))
        .collect();
    if !rejected.is_empty() {
        result.outcome = Outcome::Rejected;
        result.rejected_paths = rejected;
    } else if !green {
        result.outcome = Outcome::Blocked;
    } else {
        match stage(&repo, &head, &entries, &patch_file, patch.is_empty()) {
            Ok(()) => result.outcome = Outcome::Staged,
            Err(why) => {
                result.outcome = Outcome::Moved;
                result.reason = Some(why);
            }
        }
    }
    Ok(result)
}

/// Apply the patch to the repository's index and working tree, if the
/// repository is as the run found it.
fn stage(repo: &Path, head: &str, entries: &[u8], patch_file: &Path, empty: bool) -> Result<(), String> {
    let now = git_text(repo, &["rev-parse", "--verify", "HEAD"])?;
    if now != head {
        return Err(format!("HEAD moved from {head} to {now} while the agent ran"));
    }
    if index_entries(repo)? != entries {
        return Err("the index changed while the agent ran".to_string());
    }
    if !diff_paths(repo, &[])?.is_empty() {
        return Err("tracked files changed while the agent ran".to_string());
    }
    if empty {
        return Ok(());
    }
    git(
        [
            OsStr::new("-C"),
            repo.as_os_str(),
            OsStr::new("apply"),
            OsStr::new("--index"),
            patch_file.as_os_str(),
        ],
        &[],
    )
    .map(|_| ())
    .map_err(|e| format!("the patch does not apply: {e}"))
}

/// The repository-relative paths a unified diff adds, changes, deletes or
/// renames, both sides of a rename included (SLICE-9-2).
pub fn patch_paths(patch: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut add = |p: String| {
        if !paths.contains(&p) {
            paths.push(p);
        }
    };
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            for p in header_paths(rest) {
                add(p);
            }
        } else if let Some(rest) = line
            .strip_prefix("rename from ")
            .or_else(|| line.strip_prefix("rename to "))
            .or_else(|| line.strip_prefix("copy from "))
            .or_else(|| line.strip_prefix("copy to "))
        {
            add(unquote(rest).0);
        }
    }
    paths
}

/// The paths in a `diff --git a/<old> b/<new>` header: both when they can be
/// told apart, which a rename's own `rename from`/`rename to` lines cover
/// when they can't.
fn header_paths(rest: &str) -> Vec<String> {
    if rest.starts_with('"') {
        let (old, after) = unquote(rest);
        let new = unquote(after.trim_start()).0;
        return [old, new].iter().filter_map(|p| strip_side(p)).collect();
    }
    // Unquoted: `a/<p> b/<p>` when nothing is renamed, so the halves are equal.
    if rest.len() >= 5 && (rest.len() - 5) % 2 == 0 {
        let n = (rest.len() - 5) / 2;
        if rest.is_char_boundary(2 + n) && rest.is_char_boundary(2 + n + 3) {
            let (old, new) = (&rest[2..2 + n], &rest[2 + n + 3..]);
            if rest.starts_with("a/") && &rest[2 + n..2 + n + 3] == " b/" && old == new {
                return vec![old.to_string()];
            }
        }
    }
    // A rename or copy: its own `rename`/`copy` lines name the paths.
    Vec::new()
}

fn strip_side(p: &str) -> Option<String> {
    p.strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .map(str::to_string)
}

/// A path as git prints it: bare, or in double quotes with C escapes.
/// Returns the path and what follows it.
fn unquote(s: &str) -> (String, &str) {
    let Some(body) = s.strip_prefix('"') else {
        return (s.to_string(), "");
    };
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b'"' {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                c @ b'0'..=b'7' => {
                    let mut v = u32::from(c - b'0');
                    let mut digits = 1;
                    while digits < 3 && i + 1 < bytes.len() && (b'0'..=b'7').contains(&bytes[i + 1]) {
                        i += 1;
                        v = v * 8 + u32::from(bytes[i] - b'0');
                        digits += 1;
                    }
                    out.push(v as u8);
                }
                c => out.push(c),
            }
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    let after = body.get(i + 1..).unwrap_or("");
    (String::from_utf8_lossy(&out).to_string(), after)
}
