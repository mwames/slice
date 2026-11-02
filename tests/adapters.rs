//! The local adapters against real git and a real filesystem in temp dirs.
//! Harvest, ClickUp and monday are not exercised here: they need live
//! accounts.

use std::path::{Path, PathBuf};
use std::process::Command;

use slice::adapters::git::Git;
use slice::adapters::state::{FileState, manifest_yaml};
use slice::config::Config;
use slice::ports::*;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slice-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repo(name: &str) -> PathBuf {
    let dir = temp_dir(name).join("app");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(
        &dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "base",
        ],
    );
    dir
}

fn manifest() -> Manifest {
    Manifest {
        id: SliceId("ENG-1-1".into()),
        ticket: TicketRef("ENG-1".into()),
        ticket_system: TicketSystemKind::ClickUp,
        base_commit: "abc1234".into(),
        status: ManifestStatus::Open,
        acceptance_criteria: vec![],
        invariants: vec![],
        spec_hash: None,
        test_hash: None,
        budgets: vec![],
    }
}

#[test]
fn git_resolves_and_creates_and_removes_a_worktree() {
    let repo = repo("git");
    let head = git(&repo, &["rev-parse", "HEAD"]);

    assert_eq!(Git.resolve(&repo, "HEAD").unwrap(), head);
    let err = Git.resolve(&repo, "no-such-ref").unwrap_err();
    assert_eq!(err.source, Source::Git);

    let worktree = repo.parent().unwrap().join("app.slices/ENG-1-1");
    Git.create_worktree(&repo, "slice/ENG-1-1", &worktree, &head).unwrap();
    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&worktree, &["branch", "--show-current"]), "slice/ENG-1-1");
    let upstream = Command::new("git")
        .arg("-C")
        .arg(&worktree)
        .args(["rev-parse", "--abbrev-ref", "@{upstream}"])
        .output()
        .unwrap();
    assert!(!upstream.status.success(), "slice branch has no upstream");

    Git.remove_worktree(&repo, "slice/ENG-1-1", &worktree).unwrap();
    assert!(!worktree.exists());
    assert_eq!(git(&repo, &["branch", "--list", "slice/ENG-1-1"]), "");
}

/// Found in a live run: `git worktree add -b` creates the branch before it
/// notices the path exists, and a failed step is not undone by open, so the
/// adapter must not leave a branch behind.
#[test]
fn git_failed_worktree_leaves_no_branch() {
    let repo = repo("blocked");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let worktree = repo.parent().unwrap().join("app.slices/ENG-1-1");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::write(worktree.join("file"), "in the way").unwrap();

    let err = Git.create_worktree(&repo, "slice/ENG-1-1", &worktree, &head).unwrap_err();

    assert_eq!(err.source, Source::Git);
    assert_eq!(git(&repo, &["branch", "--list", "slice/ENG-1-1"]), "", "no branch left behind");
}

/// The cleanup above must never delete a branch that existed before.
#[test]
fn git_failed_worktree_keeps_existing_branch() {
    let repo = repo("existing");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["branch", "slice/ENG-1-1"]);
    let worktree = repo.parent().unwrap().join("app.slices/ENG-1-1");

    Git.create_worktree(&repo, "slice/ENG-1-1", &worktree, &head).unwrap_err();

    assert_eq!(git(&repo, &["branch", "--list", "slice/ENG-1-1"]), "slice/ENG-1-1");
}

#[test]
fn git_toplevel_from_subdirectory() {
    let repo = repo("toplevel");
    let sub = repo.join("src");
    std::fs::create_dir_all(&sub).unwrap();

    let top = Git::toplevel(&sub).unwrap();

    assert_eq!(top.canonicalize().unwrap(), repo.canonicalize().unwrap());
}

#[test]
fn state_round_trips_the_active_slice_and_counts_slices() {
    let dir = temp_dir("state");
    let state = FileState::new(dir.join("state"));
    let ticket = TicketRef("ENG-1".into());

    assert_eq!(state.active().unwrap(), None);
    assert_eq!(state.next_slice_id(&ticket).unwrap(), SliceId("ENG-1-1".into()));

    let slice = ActiveSlice {
        id: SliceId("ENG-1-1".into()),
        ticket: ticket.clone(),
        repo: PathBuf::from("/work/app"),
        branch: "slice/ENG-1-1".into(),
        worktree: PathBuf::from("/work/app.slices/ENG-1-1"),
        manifest: PathBuf::from("/work/app.slices/ENG-1-1/.slices/ENG-1-1.yaml"),
        timer: TimerId(42),
    };
    state.set_active(&slice).unwrap();
    assert_eq!(state.active().unwrap(), Some(slice));
    assert_eq!(state.next_slice_id(&ticket).unwrap(), SliceId("ENG-1-2".into()));

    state.clear_active().unwrap();
    assert_eq!(state.active().unwrap(), None);
    assert_eq!(
        state.next_slice_id(&ticket).unwrap(),
        SliceId("ENG-1-2".into()),
        "count survives clear"
    );
    state.clear_active().unwrap();
}

#[test]
fn state_reports_a_corrupt_file_as_local_state() {
    let dir = temp_dir("corrupt");
    std::fs::write(dir.join("active.json"), "not json").unwrap();

    let err = FileState::new(dir).active().unwrap_err();

    assert_eq!(err.source, Source::LocalState);
}

#[test]
fn manifest_written_as_yaml_and_removed() {
    let dir = temp_dir("manifest");
    let state = FileState::new(dir.join("state"));
    let path = dir.join("wt/.slices/ENG-1-1.yaml");

    state.write_manifest(&path, &manifest()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, manifest_yaml(&manifest()));
    assert!(text.contains("ticket: \"ENG-1\"\n"));
    assert!(text.contains("base_commit: \"abc1234\"\n"));
    assert!(text.contains("acceptance_criteria: []\n"));

    state.remove_manifest(&path).unwrap();
    assert!(!path.exists());
    state.remove_manifest(&path).unwrap();
}

#[test]
fn config_defaults_and_overrides() {
    let defaults = Config::parse("").unwrap().settings();
    assert_eq!(defaults.clickup_in_progress, "development");
    assert_eq!(defaults.monday_in_progress, "In Progress");
    assert_eq!(defaults.branch_prefix, "slice/");
    assert_eq!(defaults.worktree_root, None);

    let cfg = Config::parse(
        "branch_prefix = \"s/\"\nworktree_root = \"/tmp/slices\"\n[status.in_progress]\nclickup = \"doing\"\n[harvest]\nproject = \"P\"\ntask = \"T\"\n",
    )
    .unwrap();
    let s = cfg.settings();
    assert_eq!(s.clickup_in_progress, "doing");
    assert_eq!(s.monday_in_progress, "In Progress");
    assert_eq!(s.branch_prefix, "s/");
    assert_eq!(s.worktree_root, Some(PathBuf::from("/tmp/slices")));
    assert_eq!((cfg.harvest.project.as_str(), cfg.harvest.task.as_str()), ("P", "T"));

    assert!(Config::parse("typo = 1").is_err(), "unknown keys are rejected");
}
