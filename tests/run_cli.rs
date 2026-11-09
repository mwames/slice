//! `slice run` from the command line: usage errors and the checks made before
//! a container starts (SLICE-8-1 to SLICE-8-4), that the podman on PATH is
//! what starts it (SLICE-8-7), and the --json fields naming what a run started
//! from (SLICE-9-5). The binary runs with HOME, XDG_CONFIG_HOME and
//! XDG_STATE_HOME in a fresh temporary directory, and a
//! `podman` on PATH that only records that it was called, so nothing can
//! reach real configuration, state or Podman.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::agent::*;

/// A home for one run of the binary: a clean repository at `app`, a task
/// file, a Spool binary, a slice config naming it, and a fake podman.
struct Home {
    root: TempDir,
    repo: PathBuf,
    task: PathBuf,
}

impl Home {
    fn new(name: &str) -> Home {
        let (root, repo) = repo(name);
        let settings = agent_settings(&root);
        write(
            &root,
            "config/slice/config.toml",
            &format!(
                "[agents]\nspool = \"{}\"\nenv_file = \"{}\"\n",
                settings.spool.display(),
                settings.env_file.display()
            ),
        );
        write(&root, "task.md", TASK);
        write(
            &root,
            "fakebin/podman",
            &format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\nexit 125\n",
                root.join("podman-called").display()
            ),
        );
        make_executable(&root.join("fakebin/podman"));
        Home {
            task: root.join("task.md"),
            repo,
            root,
        }
    }

    fn config(&self, text: &str) {
        write(&self.root, "config/slice/config.toml", text);
    }

    fn slice(&self, dir: &Path, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.root.join("fakebin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_slice"))
            .args(args)
            .env_clear()
            .env("PATH", path)
            .env("HOME", &*self.root)
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .current_dir(dir)
            .output()
            .unwrap()
    }

    /// No container started and no run directory created.
    fn assert_nothing_ran(&self, what: &str) {
        assert!(!self.root.join("podman-called").exists(), "{what}: podman was run");
        assert_eq!(
            run_dirs(&self.root.join("state/slice/runs")),
            Vec::<PathBuf>::new(),
            "{what}: no run directory"
        );
        assert!(
            !self.root.join(".local/state/slice/runs").exists(),
            "{what}: no run directory in the default place"
        );
    }
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Stderr without the usage text, which names every flag.
fn problem(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| {
            let l = l.trim_start();
            !l.starts_with("usage:") && !l.starts_with("slice ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `args` after `run` is a usage error naming one of `names`.
fn assert_usage_error(name: &str, args: &[&str], names: &[&str]) {
    let home = Home::new(name);
    let before = repo_state(&home.repo);
    let mut full = vec!["run"];
    full.extend(args);

    let out = home.slice(&home.repo, &full);

    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(out.status.code(), Some(2), "{full:?}: exit code; stderr: {stderr}");
    let problem = problem(&stderr);
    assert!(
        names.iter().any(|n| problem.contains(n)),
        "{full:?}: names one of {names:?}: {stderr}"
    );
    assert!(
        out.stdout.is_empty(),
        "{full:?}: stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    home.assert_nothing_ran(name);
    assert_eq!(repo_state(&home.repo), before, "{full:?}: repository unchanged");
}

// SLICE-8-1
#[test]
fn run_without_a_role_is_a_usage_error() {
    assert_usage_error("no-role", &[], &["role", "test", "impl"]);
    assert_usage_error(
        "no-role-flags",
        &["--task", "task.md", "--image", IMAGE],
        &["role", "test", "impl"],
    );
}

// SLICE-8-1
#[test]
fn run_with_another_role_is_a_usage_error() {
    for (i, role) in ["deploy", "tests", "source", "IMPL", "Test"].into_iter().enumerate() {
        let home = Home::new(&format!("bad-role-{i}"));
        let task = home.task.display().to_string();
        let out = home.slice(&home.repo, &["run", role, "--task", &task, "--image", IMAGE]);
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert_eq!(out.status.code(), Some(2), "{role}: {stderr}");
        assert!(problem(&stderr).contains(role), "{role}: named: {stderr}");
        home.assert_nothing_ran(role);
    }
}

// SLICE-8-1
#[test]
fn run_without_task_or_image_is_a_usage_error() {
    let home = Home::new("missing-flags");
    let task = home.task.display().to_string();
    for (args, flag) in [
        (vec!["run", "test", "--image", IMAGE], "--task"),
        (vec!["run", "impl", "--task", &task], "--image"),
        (vec!["run", "test", "--task", &task, "--image"], "--image"),
        (vec!["run", "test", "--image", IMAGE, "--task"], "--task"),
    ] {
        let out = home.slice(&home.repo, &args);
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert_eq!(out.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(problem(&stderr).contains(flag), "{args:?}: names {flag}: {stderr}");
        home.assert_nothing_ran(flag);
    }
}

// SLICE-8-1
#[test]
fn run_with_an_unreadable_task_is_a_usage_error() {
    let home = Home::new("no-task-file");
    let missing = home.root.join("no-such-task.md");
    let missing = missing.display().to_string();
    let out = home.slice(&home.repo, &["run", "test", "--task", &missing, "--image", IMAGE]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(problem(&stderr).contains(&missing), "names the file: {stderr}");
    home.assert_nothing_ran("missing task");

    let dir = home.root.display().to_string();
    let out = home.slice(&home.repo, &["run", "test", "--task", &dir, "--image", IMAGE]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(out.status.code(), Some(2), "a directory: {stderr}");
    assert!(problem(&stderr).contains(&dir), "names the directory: {stderr}");
    home.assert_nothing_ran("task is a directory");
}

// SLICE-8-1
#[test]
fn run_with_a_flag_it_does_not_take_is_a_usage_error() {
    for (i, extra) in [
        vec!["--base", "main"],
        vec!["--type", "feature"],
        vec!["--done"],
        vec!["--reason", "why"],
        vec!["--model", "opus"],
    ]
    .into_iter()
    .enumerate()
    {
        let home = Home::new(&format!("bad-flag-{i}"));
        let task = home.task.display().to_string();
        let mut args = vec!["run", "test", "--task", &task, "--image", IMAGE];
        args.extend(extra.iter().copied());
        let out = home.slice(&home.repo, &args);
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert_eq!(out.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(
            problem(&stderr).contains(extra[0]),
            "{args:?}: names {}: {stderr}",
            extra[0]
        );
        home.assert_nothing_ran(extra[0]);
    }
}

/// `slice run test` on `dir`, or with `--repo`, stops with exit 1 before a
/// container, naming each of `names`.
fn assert_stops(home: &Home, cwd: &Path, repo: Option<&Path>, names: &[&str]) {
    let task = home.task.display().to_string();
    let mut args = vec![
        "run".to_string(),
        "test".to_string(),
        "--task".to_string(),
        task,
        "--image".to_string(),
        IMAGE.to_string(),
    ];
    if let Some(repo) = repo {
        args.extend(["--repo".to_string(), repo.display().to_string()]);
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let out = home.slice(cwd, &args);

    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{args:?}: stderr: {stderr} stdout: {stdout}"
    );
    let said = format!("{stderr}{stdout}");
    assert!(!said.trim().is_empty(), "{args:?}: says why");
    for n in names {
        assert!(said.contains(n), "{args:?}: names {n:?}: {said}");
    }
    home.assert_nothing_ran(&format!("{args:?}"));
}

// SLICE-8-2
#[test]
fn dirty_repository_exits_1() {
    let home = Home::new("cli-dirty");
    write(&home.repo, "src/lib.rs", "// changed\n");
    assert_stops(&home, &home.repo, None, &["src/lib.rs"]);
    assert_stops(&home, &home.root, Some(&home.repo), &["src/lib.rs"]);
}

// SLICE-8-2
#[test]
fn staged_changes_do_not_stop_the_run() {
    // The fake podman exits 125, so the run fails, but only after it started.
    let home = Home::new("cli-staged");
    stage_work(&home.repo);
    let before = repo_state(&home.repo);
    let task = home.task.display().to_string();

    let out = home.slice(&home.repo, &["run", "test", "--task", &task, "--image", IMAGE]);

    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        home.root.join("podman-called").exists(),
        "the podman on PATH was run: {stderr}"
    );
    assert_eq!(repo_state(&home.repo), before, "nothing staged or changed");
}

// SLICE-9-5
#[test]
fn json_names_what_the_run_started_from() {
    let home = Home::new("cli-json-start");
    stage_work(&home.repo);
    let head = git(&home.repo, &["rev-parse", "HEAD"]);
    let task = home.task.display().to_string();

    let out = home.slice(
        &home.repo,
        &["run", "test", "--task", &task, "--image", IMAGE, "--json"],
    );

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).expect("one JSON object");
    assert_eq!(value["outcome"], "failed");
    assert_eq!(value["head"], serde_json::json!(head));
    assert_eq!(value["staged"], serde_json::json!(STAGED_WORK));
}

// SLICE-8-2
#[test]
fn repository_defaults_to_the_current_directory() {
    // The current directory is dirty; --repo names a clean repository that
    // has no .slice.toml. Each run names its own repository's problem.
    let home = Home::new("cli-cwd");
    let (_other_root, other) = repo_with("cli-cwd-other", None);
    write(&home.repo, "README.md", "# dirty\n");
    assert_stops(&home, &home.repo, Some(&other), &[".slice.toml"]);

    let clean = Home::new("cli-cwd-clean");
    std::fs::remove_file(clean.repo.join(".slice.toml")).unwrap();
    git(&clean.repo, &["commit", "-q", "-am", "no config"]);
    assert_stops(&clean, &clean.repo, None, &[".slice.toml"]);
}

// SLICE-8-2
#[test]
fn not_a_repository_exits_1() {
    let home = Home::new("cli-not-repo");
    let plain = home.root.join("plain");
    write(&plain, "file.txt", "x\n");
    assert_stops(&home, &plain, None, &[]);
    assert_stops(&home, &home.repo, Some(&plain), &[]);
}

// SLICE-8-3
#[test]
fn missing_setup_exits_1() {
    let home = Home::new("cli-no-spool");
    home.config("");
    assert_stops(&home, &home.repo, None, &["spool"]);

    let home = Home::new("cli-spool-missing");
    let missing = home.root.join("bin/gone");
    home.config(&format!("[agents]\nspool = \"{}\"\n", missing.display()));
    assert_stops(&home, &home.repo, None, &[&missing.display().to_string()]);

    let home = Home::new("cli-no-prompt");
    git(&home.repo, &["rm", "-q", ".slice/test-agent.md"]);
    git(&home.repo, &["commit", "-q", "-m", "no prompt"]);
    assert_stops(&home, &home.repo, None, &[".slice/test-agent.md"]);

    // The prompt's deletion staged, not committed.
    let home = Home::new("cli-prompt-deletion-staged");
    git(&home.repo, &["rm", "-q", ".slice/test-agent.md"]);
    assert_stops(&home, &home.repo, None, &[".slice/test-agent.md"]);

    // .slice.toml's deletion staged, not committed.
    let home = Home::new("cli-toml-deletion-staged");
    git(&home.repo, &["rm", "-q", ".slice.toml"]);
    assert_stops(&home, &home.repo, None, &[".slice.toml"]);

    let home = Home::new("cli-no-globs");
    write(&home.repo, ".slice.toml", "[paths]\nsource = [\"src/**\"]\n");
    git(&home.repo, &["commit", "-q", "-am", "no test globs"]);
    assert_stops(&home, &home.repo, None, &["test"]);
}

// SLICE-8-4
#[test]
fn unknown_config_keys_exit_1() {
    let home = Home::new("cli-unknown-agents-key");
    let settings = agent_settings(&home.root);
    home.config(&format!(
        "[agents]\nspool = \"{}\"\ntoken_value = \"x\"\n",
        settings.spool.display()
    ));
    assert_stops(&home, &home.repo, None, &["token_value"]);

    let home = Home::new("cli-unknown-repo-key");
    write(
        &home.repo,
        ".slice.toml",
        "[paths]\ntest = [\"tests/**\"]\nsource = [\"src/**\"]\nshared = [\"Cargo.toml\"]\n",
    );
    git(&home.repo, &["commit", "-q", "-am", "unknown key"]);
    assert_stops(&home, &home.repo, None, &["shared"]);
}

// SLICE-8-7
#[test]
fn run_starts_the_podman_on_path() {
    // The fake podman records its arguments and exits 125, so the run fails.
    let home = Home::new("cli-podman");
    let before = repo_state(&home.repo);
    let task = home.task.display().to_string();

    let out = home.slice(&home.repo, &["run", "test", "--task", &task, "--image", IMAGE]);

    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    let called = std::fs::read_to_string(home.root.join("podman-called")).expect("the podman on PATH was run");
    assert!(called.contains(IMAGE), "with the image: {called}");
    assert!(called.contains("opus"), "with the test role's model: {called}");
    assert_eq!(repo_state(&home.repo), before, "nothing staged or changed");
}
