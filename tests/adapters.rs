//! The local adapters against real git and a real filesystem in temp dirs.
//! Harvest, ClickUp and monday are not exercised here: they need live
//! accounts.

use std::path::{Path, PathBuf};
use std::process::Command;

use slice::adapters::git::Git;
use slice::adapters::state::{FileState, manifest_yaml, parse_manifest_yaml};
use slice::config::Config;
use slice::ports::*;

/// A temporary directory, deleted with everything under it when it goes out
/// of scope, even when the test fails. Derefs to `path`, which may be a
/// subdirectory of the root that gets deleted.
struct TempDir {
    root: PathBuf,
    path: PathBuf,
}

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn temp_dir(name: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!("slice-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TempDir {
        root: dir.clone(),
        path: dir,
    }
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

/// A git repository with one empty commit, at `app` inside a temporary root
/// that also holds its worktrees.
fn repo(name: &str) -> TempDir {
    let mut dir = temp_dir(name);
    dir.path = dir.path.join("app");
    std::fs::create_dir_all(&*dir).unwrap();
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
        repos: vec![SliceRepo {
            repo: PathBuf::from("/work/app"),
            branch: BRANCH.into(),
            worktree: PathBuf::from("/work/app.slices/feature-20261102-ENG-1-login"),
            base_commit: "abc1234".into(),
        }],
        status: ManifestStatus::Open,
        acceptance_criteria: vec![],
        invariants: vec![],
        spec_hash: None,
        test_hash: None,
        budgets: vec![],
        abandon_reason: None,
    }
}

const BRANCH: &str = "feature/20261102-ENG-1-login";

fn branch_exists(repo: &Path, branch: &str) -> bool {
    !git(repo, &["branch", "--list", branch]).is_empty()
}

#[test]
fn git_resolves_commits_and_branches() {
    let repo = repo("resolve");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["branch", BRANCH]);

    assert_eq!(Git.resolve(&repo, "HEAD").unwrap(), head);
    assert_eq!(Git.resolve(&repo, BRANCH).unwrap(), head);
    let err = Git.resolve(&repo, "no-such-ref").unwrap_err();
    assert_eq!(err.source, Source::Git);
}

#[test]
fn git_lists_local_branches() {
    let repo = repo("branches");
    git(&repo, &["branch", BRANCH]);
    git(&repo, &["branch", "bug/20261020-ENG-1-crash"]);

    let mut branches = Git.branches(&repo).unwrap();
    branches.sort();

    assert_eq!(branches, ["bug/20261020-ENG-1-crash", BRANCH, "main"]);
}

#[test]
fn git_creates_and_removes_a_branch_with_worktree() {
    let repo = repo("create");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let worktree = repo.parent().unwrap().join("app.slices/x");

    Git.create_branch(&repo, BRANCH, &worktree, &head).unwrap();

    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&worktree, &["branch", "--show-current"]), BRANCH);
    let upstream = Command::new("git")
        .arg("-C")
        .arg(&worktree)
        .args(["rev-parse", "--abbrev-ref", "@{upstream}"])
        .output()
        .unwrap();
    assert!(!upstream.status.success(), "branch has no upstream");
    let found = Git.worktree_of(&repo, BRANCH).unwrap().expect("worktree found");
    assert_eq!(found.canonicalize().unwrap(), worktree.canonicalize().unwrap());

    Git.remove_branch(&repo, BRANCH, &worktree).unwrap();
    assert!(!worktree.exists());
    assert!(!branch_exists(&repo, BRANCH));
}

#[test]
fn git_adds_and_removes_a_worktree_for_an_existing_branch() {
    let repo = repo("add");
    git(&repo, &["branch", BRANCH]);
    let worktree = repo.parent().unwrap().join("app.slices/x");
    assert_eq!(Git.worktree_of(&repo, BRANCH).unwrap(), None);

    Git.add_worktree(&repo, BRANCH, &worktree).unwrap();
    assert_eq!(git(&worktree, &["branch", "--show-current"]), BRANCH);
    assert!(Git.worktree_of(&repo, BRANCH).unwrap().is_some());

    Git.remove_worktree(&repo, &worktree).unwrap();
    assert!(!worktree.exists());
    assert!(branch_exists(&repo, BRANCH), "branch kept");
    assert_eq!(Git.worktree_of(&repo, BRANCH).unwrap(), None);
}

#[test]
fn git_worktree_of_the_main_checkout() {
    let repo = repo("main");

    let found = Git.worktree_of(&repo, "main").unwrap().expect("main is checked out");

    assert_eq!(found.canonicalize().unwrap(), repo.canonicalize().unwrap());
}

/// Found in a live run: `git worktree add -b` creates the branch before it
/// notices the path exists, so the adapter must not leave a branch behind.
#[test]
fn git_failed_create_leaves_no_branch() {
    let repo = repo("blocked");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let worktree = repo.parent().unwrap().join("app.slices/x");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::write(worktree.join("file"), "in the way").unwrap();

    let err = Git.create_branch(&repo, BRANCH, &worktree, &head).unwrap_err();

    assert_eq!(err.source, Source::Git);
    assert!(!branch_exists(&repo, BRANCH), "no branch left behind");
}

/// The cleanup above must never delete a branch that existed before.
#[test]
fn git_failed_create_keeps_existing_branch() {
    let repo = repo("existing");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["branch", BRANCH]);
    let worktree = repo.parent().unwrap().join("app.slices/x");

    Git.create_branch(&repo, BRANCH, &worktree, &head).unwrap_err();

    assert!(branch_exists(&repo, BRANCH));
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
        repos: vec![SliceRepo {
            repo: PathBuf::from("/work/app"),
            branch: BRANCH.into(),
            worktree: PathBuf::from("/work/app.slices/x"),
            base_commit: "abc1234".into(),
        }],
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

    let err = FileState::new(dir.to_path_buf()).active().unwrap_err();

    assert_eq!(err.source, Source::LocalState);
}

// SLICE-10-9
#[test]
fn manifests_kept_in_state_by_slice_id() {
    let dir = temp_dir("manifest");
    let state = FileState::new(dir.join("state"));
    let id = SliceId("ENG-1-1".into());
    assert_eq!(state.read_manifest(&id).unwrap(), None);

    state.write_manifest(&manifest()).unwrap();
    let path = dir.join("state/manifests/ENG-1-1.yaml");
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, manifest_yaml(&manifest()));
    assert!(text.contains("ticket: \"ENG-1\"\n"));
    assert_eq!(state.read_manifest(&id).unwrap(), Some(manifest()));

    let closed = Manifest {
        status: ManifestStatus::Closed,
        ..manifest()
    };
    state.write_manifest(&closed).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("status: closed\n"));
    assert_eq!(state.read_manifest(&id).unwrap(), Some(closed));

    let abandoned = Manifest {
        status: ManifestStatus::Abandoned,
        ..manifest()
    };
    state.write_manifest(&abandoned).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("status: abandoned\n"));
    assert_eq!(state.read_manifest(&id).unwrap(), Some(abandoned));

    state.remove_manifest(&id).unwrap();
    assert!(!path.exists());
    state.remove_manifest(&id).unwrap();
}

#[test]
fn config_defaults_and_overrides() {
    let defaults = Config::parse("").unwrap().settings().unwrap();
    assert_eq!(
        defaults.in_progress,
        Statuses {
            clickup: "development".into(),
            monday: "In Progress".into()
        }
    );
    assert_eq!(
        defaults.review,
        Statuses {
            clickup: "dev complete".into(),
            monday: "Waiting for review".into()
        }
    );
    assert_eq!(
        defaults.done,
        Statuses {
            clickup: "complete".into(),
            monday: "Done".into()
        }
    );
    assert_eq!(defaults.worktree_root, None);

    let cfg = Config::parse(
        "worktree_root = \"/tmp/slices\"\n[status.in_progress]\nclickup = \"doing\"\n[status.done]\nmonday = \"Shipped\"\n[harvest]\nproject = \"P\"\ntask = \"T\"\n",
    )
    .unwrap();
    let s = cfg.settings().unwrap();
    assert_eq!(s.in_progress.clickup, "doing");
    assert_eq!(s.in_progress.monday, "In Progress");
    assert_eq!(s.done.monday, "Shipped");
    assert_eq!(s.worktree_root, Some(PathBuf::from("/tmp/slices")));
    assert_eq!((cfg.harvest.project.as_str(), cfg.harvest.task.as_str()), ("P", "T"));

    assert!(
        Config::parse("branch_prefix = \"s/\"").is_err(),
        "removed and unknown keys are rejected"
    );
}

/// A manifest as slice wrote it before SLICE-7, with no abandon reason line.
const OLDER_MANIFEST: &str = "id: \"ENG-1-2\"
ticket: \"ENG-1\"
ticket_system: clickup
branch: \"feature/20261102-ENG-1-login\"
base_commit: \"abc1234\"
status: abandoned
acceptance_criteria: [\"ENG-1-1\",\"ENG-1-2\"]
invariants: [\"ENG-1-INV-1\"]
spec_hash: \"s123\"
test_hash: null
budgets:
  \"tokens\": \"100k\"
";

// SLICE-7-5
// SLICE-10-9
#[test]
fn older_manifest_reads_back_without_a_reason() {
    let dir = temp_dir("older-manifest");
    let state = FileState::new(dir.join("state"));
    std::fs::create_dir_all(dir.join("state/manifests")).unwrap();
    std::fs::write(dir.join("state/manifests/ENG-1-2.yaml"), OLDER_MANIFEST).unwrap();

    let read = state.read_manifest(&SliceId("ENG-1-2".into())).unwrap();

    assert_eq!(
        read,
        Some(Manifest {
            id: SliceId("ENG-1-2".into()),
            status: ManifestStatus::Abandoned,
            acceptance_criteria: vec!["ENG-1-1".into(), "ENG-1-2".into()],
            invariants: vec!["ENG-1-INV-1".into()],
            spec_hash: Some("s123".into()),
            budgets: vec![("tokens".into(), "100k".into())],
            abandon_reason: None,
            repos: vec![earlier_repo()],
            ..manifest()
        })
    );
}

// SLICE-7-6
#[test]
fn abandon_reason_round_trips_through_the_manifest() {
    let dir = temp_dir("reason");
    let state = FileState::new(dir.join("state"));
    let reasons = [
        "said \"won't fix\"",
        "fixed by: ENG-14800",
        "path C:\\work\\app",
        "first line\nsecond line",
        "déjà vu — 修正済み ✓",
        "a \"quoted\": C:\\x\\\"y\"\nstatus: closed\nbudgets: café 🚧",
    ];
    for reason in reasons {
        let abandoned = Manifest {
            status: ManifestStatus::Abandoned,
            abandon_reason: Some(reason.to_string()),
            ..manifest()
        };

        state.write_manifest(&abandoned).unwrap();
        let read = state.read_manifest(&abandoned.id).unwrap().expect("manifest");

        assert_eq!(read.abandon_reason.as_deref(), Some(reason), "{reason:?}");
        assert_eq!(read, abandoned, "{reason:?}: every field as written");
    }
}

// ---------------------------------------------------------------------------
// Several repositories in a slice
// ---------------------------------------------------------------------------

/// The one repository a manifest written before SLICE-10 reads as: its
/// branch and base commit, with no repository path or worktree, which that
/// manifest never recorded.
fn earlier_repo() -> SliceRepo {
    SliceRepo {
        repo: PathBuf::new(),
        branch: BRANCH.into(),
        worktree: PathBuf::new(),
        base_commit: "abc1234".into(),
    }
}

/// A manifest as slice wrote it after SLICE-7 and before SLICE-10: one
/// repository's branch and base commit at the top level, and a reason.
const EARLIER_MANIFEST: &str = "id: \"ENG-1-3\"
ticket: \"ENG-1\"
ticket_system: clickup
branch: \"feature/20261102-ENG-1-login\"
base_commit: \"abc1234\"
status: abandoned
acceptance_criteria: []
invariants: []
spec_hash: null
test_hash: null
abandon_reason: \"fixed by: ENG-14800\"
budgets: {}
";

/// Repositories whose paths, branches and worktrees need quoting.
fn awkward_repos() -> Vec<SliceRepo> {
    vec![
        SliceRepo {
            repo: PathBuf::from("/work/app"),
            branch: BRANCH.into(),
            worktree: PathBuf::from("/work/app.slices/feature-20261102-ENG-1-login"),
            base_commit: "abc1234".into(),
        },
        SliceRepo {
            repo: PathBuf::from("/home/me/my repos/api: \"v2\""),
            branch: "bug/20261020-ENG-1-crash".into(),
            worktree: PathBuf::from("/home/me/my repos/api: \"v2\".slices/bug-20261020-ENG-1-crash"),
            base_commit: "0123456789abcdef0123456789abcdef01234567".into(),
        },
        SliceRepo {
            repo: PathBuf::from("/srv/café/web"),
            branch: BRANCH.into(),
            worktree: PathBuf::from("/srv/café/web.slices/feature-20261102-ENG-1-login"),
            base_commit: "def5678".into(),
        },
    ]
}

// SLICE-10-9
#[test]
fn manifest_round_trips_every_repository() {
    let dir = temp_dir("manifest-repos");
    let state = FileState::new(dir.join("state"));
    let all = awkward_repos();
    for n in 1..=all.len() {
        for (status, reason) in [
            (ManifestStatus::Open, None),
            (ManifestStatus::Closed, None),
            (
                ManifestStatus::Abandoned,
                Some("said \"no\": status: closed\nbudgets: {}".to_string()),
            ),
        ] {
            let m = Manifest {
                repos: all[..n].to_vec(),
                status,
                abandon_reason: reason,
                acceptance_criteria: vec!["ENG-1-1".into()],
                budgets: vec![("tokens".into(), "100k".into())],
                ..manifest()
            };

            assert_eq!(
                parse_manifest_yaml(&manifest_yaml(&m)),
                Ok(m.clone()),
                "{n} repositories"
            );
            state.write_manifest(&m).unwrap();
            assert_eq!(state.read_manifest(&m.id).unwrap(), Some(m), "{n} repositories");
        }
    }
}

// SLICE-10-9
#[test]
fn active_slice_round_trips_every_repository() {
    let dir = temp_dir("active-repos");
    let state = FileState::new(dir.join("state"));
    let all = awkward_repos();
    for n in 1..=all.len() {
        let slice = ActiveSlice {
            id: SliceId("ENG-1-1".into()),
            ticket: TicketRef("ENG-1".into()),
            repos: all[..n].to_vec(),
        };

        state.set_active(&slice).unwrap();

        assert_eq!(state.active().unwrap(), Some(slice), "{n} repositories");
    }
    assert_eq!(
        state.next_slice_id(&TicketRef("ENG-1".into())).unwrap(),
        SliceId("ENG-1-2".into()),
        "adding repositories does not count as another slice"
    );
}

// SLICE-10-9
#[test]
fn earlier_manifests_read_as_one_repository() {
    let want = Manifest {
        id: SliceId("ENG-1-3".into()),
        status: ManifestStatus::Abandoned,
        abandon_reason: Some("fixed by: ENG-14800".into()),
        repos: vec![earlier_repo()],
        ..manifest()
    };
    assert_eq!(parse_manifest_yaml(EARLIER_MANIFEST), Ok(want.clone()));

    let dir = temp_dir("earlier-manifest");
    let state = FileState::new(dir.join("state"));
    std::fs::create_dir_all(dir.join("state/manifests")).unwrap();
    std::fs::write(dir.join("state/manifests/ENG-1-3.yaml"), EARLIER_MANIFEST).unwrap();
    std::fs::write(dir.join("state/manifests/ENG-1-2.yaml"), OLDER_MANIFEST).unwrap();

    assert_eq!(state.read_manifest(&want.id).unwrap(), Some(want));
    let older = state
        .read_manifest(&SliceId("ENG-1-2".into()))
        .unwrap()
        .expect("manifest");
    assert_eq!(older.repos, [earlier_repo()]);
}

/// A worktree of `repo` for a new branch, at `<root>/elsewhere/wt`, outside
/// the repository and its `.slices` directory.
fn worktree_of_repo(repo: &Path) -> PathBuf {
    let worktree = repo.parent().unwrap().join("elsewhere/wt");
    git(
        repo,
        &["worktree", "add", "-q", "-b", "scratch", &worktree.to_string_lossy()],
    );
    worktree
}

// SLICE-10-7
#[test]
fn git_repository_from_the_main_working_tree() {
    let repo = repo("repository-main");
    let sub = repo.join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();

    for dir in [repo.to_path_buf(), sub] {
        let found = Git::repository(&dir).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            repo.canonicalize().unwrap(),
            "from {}",
            dir.display()
        );
    }
}

// SLICE-10-7
#[test]
fn git_repository_from_a_worktree() {
    let repo = repo("repository-worktree");
    let worktree = worktree_of_repo(&repo);
    let sub = worktree.join("src");
    std::fs::create_dir_all(&sub).unwrap();
    let slices = repo.parent().unwrap().join("app.slices/x");
    git(
        &repo,
        &["worktree", "add", "-q", "-b", BRANCH, &slices.to_string_lossy()],
    );

    for dir in [worktree, sub, slices] {
        let found = Git::repository(&dir).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            repo.canonicalize().unwrap(),
            "from {}",
            dir.display()
        );
    }
}
