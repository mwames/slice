//! For `slice run` (SLICE-8, SLICE-9): small git repositories in temporary
//! directories, and a fake `Containers` that records each `ContainerSpec`,
//! snapshots what the container would see, edits the workspace through its
//! `/work` mount the way an agent would, and returns the output it's given.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use slice::run::*;

/// A temporary directory, deleted with everything under it when it goes out
/// of scope, even when the test fails.
pub struct TempDir {
    pub path: PathBuf,
}

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh, empty temporary directory, unique within this process.
pub fn temp_dir(name: &str) -> TempDir {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("slice-run-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TempDir { path: dir }
}

/// Run git in `dir` with an identity given on the command line, and return
/// its trimmed stdout. Panics if git fails.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn write(dir: &Path, path: &str, content: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

pub const SLICE_TOML: &str = "[paths]\ntest = [\"tests/**\"]\nsource = [\"src/**\"]\n";
pub const TEST_PROMPT: &str = "# Role: test agent\nWrite the tests.\n";
pub const IMPL_PROMPT: &str = "# Role: implementation agent\nMake the tests pass.\n";
pub const TASK: &str = "Write the tests for SLICE-99.\n";
pub const IMAGE: &str = "localhost/slice-agent-rust:abc123";

/// The files committed by `repo`, other than `.slice.toml`.
pub fn base_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (".slice/test-agent.md", TEST_PROMPT),
        (".slice/impl-agent.md", IMPL_PROMPT),
        (".gitignore", "target/\n"),
        ("Cargo.toml", "[package]\nname = \"app\"\n"),
        ("README.md", "# app\n"),
        ("src/lib.rs", "pub fn f() -> u32 {\n    1\n}\n"),
        ("src/deep/mod.rs", "pub mod inner;\n"),
        ("tests/a.rs", "#[test]\nfn a() {}\n"),
        ("tests/common/mod.rs", "pub fn helper() {}\n"),
    ]
}

/// A git repository at `<root>/app` with `base_files` and the given
/// `.slice.toml` (none if `None`) committed on `main`.
pub fn repo_with(name: &str, slice_toml: Option<&str>) -> (TempDir, PathBuf) {
    let root = temp_dir(name);
    let repo = root.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    for (path, content) in base_files() {
        write(&repo, path, content);
    }
    if let Some(toml) = slice_toml {
        write(&repo, ".slice.toml", toml);
    }
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    (root, repo)
}

/// A repository with the standard `.slice.toml`.
pub fn repo(name: &str) -> (TempDir, PathBuf) {
    repo_with(name, Some(SLICE_TOML))
}

/// The paths tracked at HEAD.
pub fn tracked(repo: &Path) -> Vec<String> {
    let mut paths: Vec<String> = git(repo, &["ls-files"]).lines().map(str::to_string).collect();
    paths.sort();
    paths
}

/// A Spool binary and a token env file in `root`, as slice's config names
/// them.
pub fn agent_settings(root: &Path) -> AgentSettings {
    let spool = root.join("bin/spool-linux-arm64");
    write(root, "bin/spool-linux-arm64", "#!/bin/sh\n");
    let env_file = root.join("secrets/agent.env");
    write(
        root,
        "secrets/agent.env",
        &format!("CLAUDE_CODE_OAUTH_TOKEN={SECRET}\n"),
    );
    AgentSettings { spool, env_file }
}

/// The token in the env file from `agent_settings`.
pub const SECRET: &str = "sk-ant-oat01-never-leaves-the-env-file";

pub fn request(root: &Path, repo: &Path, role: Role) -> RunRequest {
    RunRequest {
        role,
        repo: repo.to_path_buf(),
        task: TASK.to_string(),
        image: IMAGE.to_string(),
        runs_dir: root.join("state/slice/runs"),
    }
}

/// The run directories under `runs_dir`, if it exists.
pub fn run_dirs(runs_dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(runs_dir) {
        Ok(entries) => entries.map(|e| e.unwrap().path()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Every file under `dir`, by path relative to `dir`, with its contents.
/// Skips `.git` at the top level when `skip_git` is set.
pub fn files(dir: &Path, skip_git: bool) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, skip_git: bool, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            if skip_git && rel == ".git" {
                continue;
            }
            if path.is_dir() {
                walk(base, &path, skip_git, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, skip_git, &mut out);
    out
}

/// Everything about a repository a run could change: HEAD, every ref, the
/// index, and every file in the working tree, untracked ones included.
#[derive(Debug, PartialEq, Eq)]
pub struct RepoState {
    pub head: String,
    pub refs: String,
    pub index: String,
    pub status: String,
    pub files: BTreeMap<String, Vec<u8>>,
}

pub fn repo_state(repo: &Path) -> RepoState {
    RepoState {
        head: git(repo, &["rev-parse", "HEAD"]),
        refs: git(repo, &["for-each-ref"]),
        index: git(repo, &["ls-files", "--stage"]),
        status: git(repo, &["status", "--porcelain", "--untracked-files=all"]),
        files: files(repo, true),
    }
}

/// The paths staged in the index against HEAD, as `git diff --cached
/// --name-status` prints them, one `<status>\t<path>` per line.
pub fn staged(repo: &Path) -> Vec<String> {
    let mut lines: Vec<String> = git(repo, &["diff", "--cached", "--name-status", "--no-renames"])
        .lines()
        .map(|l| l.replace('\t', " "))
        .collect();
    lines.sort();
    lines
}

/// A change an agent makes to its workspace.
#[derive(Clone, Debug)]
pub enum Edit {
    Write(&'static str, &'static str),
    Delete(&'static str),
    Rename(&'static str, &'static str),
}

/// What the container saw when it started.
#[derive(Clone, Debug, Default)]
pub struct Seen {
    /// Every file in the workspace mounted at `/work`.
    pub workspace: BTreeMap<String, Vec<u8>>,
    /// Every file in the directory mounted at `/role`.
    pub role: BTreeMap<String, Vec<u8>>,
}

/// A fake container runtime.
pub struct FakeContainers {
    pub edits: Vec<Edit>,
    pub output: ContainerOutput,
    /// Run while the container "runs", after the edits, for example to move
    /// the repository underneath it.
    pub during: Option<Box<dyn Fn()>>,
    pub specs: RefCell<Vec<ContainerSpec>>,
    pub seen: RefCell<Vec<Seen>>,
}

impl FakeContainers {
    pub fn new(edits: Vec<Edit>, output: ContainerOutput) -> FakeContainers {
        FakeContainers {
            edits,
            output,
            during: None,
            specs: RefCell::new(Vec::new()),
            seen: RefCell::new(Vec::new()),
        }
    }

    /// An agent that makes `edits` and reports `report`, with the cost,
    /// duration and turns of `claude_output`.
    pub fn reporting(edits: Vec<Edit>, report: Value) -> FakeContainers {
        FakeContainers::new(edits, exited(0, &claude_output(report)))
    }

    /// A runtime that must never be asked to start a container.
    pub fn unused() -> FakeContainers {
        FakeContainers::new(
            vec![],
            ContainerOutput {
                exit: ContainerExit::FailedToStart("the fake runtime was not expected to run".into()),
                stdout: String::new(),
                stderr: String::new(),
            },
        )
    }

    pub fn calls(&self) -> usize {
        self.specs.borrow().len()
    }

    /// The only spec this runtime was given.
    pub fn spec(&self) -> ContainerSpec {
        let specs = self.specs.borrow();
        assert_eq!(specs.len(), 1, "one container started");
        specs[0].clone()
    }

    pub fn seen(&self) -> Seen {
        self.seen.borrow()[0].clone()
    }
}

/// The host path mounted at `container`, if one is.
pub fn mount_at(spec: &ContainerSpec, container: &str) -> Option<PathBuf> {
    spec.mounts
        .iter()
        .find(|m| m.container == container)
        .map(|m| m.host.clone())
}

impl Containers for FakeContainers {
    fn run(&self, spec: &ContainerSpec) -> ContainerOutput {
        self.specs.borrow_mut().push(spec.clone());
        let workspace = mount_at(spec, "/work");
        let role = mount_at(spec, "/role");
        self.seen.borrow_mut().push(Seen {
            workspace: workspace.as_deref().map(|w| files(w, false)).unwrap_or_default(),
            role: role.as_deref().map(|r| files(r, false)).unwrap_or_default(),
        });
        if let Some(work) = workspace {
            for edit in &self.edits {
                match edit {
                    Edit::Write(path, content) => write(&work, path, content),
                    Edit::Delete(path) => std::fs::remove_file(work.join(path)).unwrap(),
                    Edit::Rename(from, to) => {
                        std::fs::create_dir_all(work.join(to).parent().unwrap()).unwrap();
                        std::fs::rename(work.join(from), work.join(to)).unwrap();
                    }
                }
            }
        }
        if let Some(during) = &self.during {
            during();
        }
        self.output.clone()
    }
}

pub fn exited(code: i32, stdout: &str) -> ContainerOutput {
    ContainerOutput {
        exit: ContainerExit::Exited(code),
        stdout: stdout.to_string(),
        stderr: "agent log line 1\nagent log line 2\n".to_string(),
    }
}

pub const COST: f64 = 0.13;
pub const DURATION_MS: u64 = 16791;
pub const TURNS: u64 = 11;

/// What `claude -p --output-format json --json-schema` prints, with `report`
/// as its structured output.
pub fn claude_output(report: Value) -> String {
    json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "duration_ms": DURATION_MS,
        "duration_api_ms": 15000,
        "num_turns": TURNS,
        "result": "Done.",
        "session_id": "0b7c1c8e-0000-4000-8000-000000000000",
        "total_cost_usd": COST,
        "usage": { "input_tokens": 10, "output_tokens": 20 },
        "structured_output": report,
    })
    .to_string()
}

pub fn report(status: &str, files_touched: &[&str]) -> Value {
    json!({
        "status": status,
        "files_touched": files_touched,
        "assumptions": ["SLICE-99-2 passes against the stub already"],
        "questions": if status == "blocked" { json!(["Does SLICE-99-3 mean bytes or characters?"]) } else { json!([]) },
        "summary": format!("Tests for SLICE-99, {status}."),
    })
}

pub fn green(files_touched: &[&str]) -> Value {
    report("green", files_touched)
}

pub fn blocked(files_touched: &[&str]) -> Value {
    report("blocked", files_touched)
}
