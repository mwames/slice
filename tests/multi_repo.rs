//! slice open adding a repository to the active slice (SLICE-10), and the
//! slice as reports and local state carry it.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use proptest::prelude::*;
use slice::adapters::state::FileState;
use slice::ports::*;

/// The active slice's branch, from `with_active_slice(T, ..)`.
const A_BRANCH: &str = "feature/20261101-ENG-1-login";
/// A branch for T in another repository, with a date and type of its own.
const R_BRANCH: &str = "bug/20261020-ENG-1-crash";
const R_BRANCH_HEAD: &str = "0crash0";

fn request_in(repo: &str, branch_type: Option<&str>) -> OpenRequest {
    OpenRequest {
        ticket: tref(T),
        repo: PathBuf::from(repo),
        branch_type: branch_type.map(|t| BranchType(t.to_string())),
        base: None,
    }
}

/// An active slice for T opened in `REPO`, its timer running or not, and
/// `API`, not in the slice, with no branch for T.
fn slice_and_api(timer_running: bool) -> (Fake, ActiveSlice) {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, timer_running);
    fake.with_repo(API, API_HEAD);
    (fake, slice)
}

fn repo<'a>(w: &'a World, path: &str) -> &'a Repo {
    &w.repos[Path::new(path)]
}

fn manifest_of(w: &World, slice: &ActiveSlice) -> Manifest {
    w.manifests.get(&slice.id).expect("manifest").clone()
}

/// Everything SLICE-10-1 requires of the slice once `repo` was added on
/// `branch` at `base`, other than the timer: the outcome, the active slice
/// and the manifest list the earlier repositories and then `repo`, `repo`
/// has the branch checked out in its worktree, and nothing else changed.
/// Returns the added repository.
fn assert_added(
    fake: &Fake,
    before: &World,
    outcome: OpenOutcome,
    repo_path: &str,
    branch: &str,
    base: &str,
) -> SliceRepo {
    let OpenOutcome::Added(slice) = outcome else {
        panic!("want Added, got {outcome:?}")
    };
    let was = before.active.clone().expect("a slice was active");
    let after = fake.snapshot();

    assert_eq!(slice.id, was.id, "the same slice");
    assert_eq!(slice.ticket, was.ticket);
    assert_eq!(slice.repos.len(), was.repos.len() + 1, "one repository added");
    assert_eq!(
        slice.repos[..was.repos.len()],
        was.repos[..],
        "earlier repositories kept"
    );
    let added = slice.repos.last().unwrap().clone();
    assert_eq!(added.repo, PathBuf::from(repo_path), "the request's repository, last");
    assert_eq!(added.branch, branch);
    assert_eq!(added.base_commit, base);

    let r = repo(&after, repo_path);
    assert!(r.branches.contains_key(branch), "{branch} exists in {repo_path}");
    assert_eq!(r.worktrees.get(branch), Some(&added.worktree), "worktree checked out");
    assert_ne!(added.worktree, PathBuf::from(repo_path), "a worktree of its own");

    assert_eq!(
        after.active.as_ref(),
        Some(&slice),
        "the active slice lists every repository"
    );
    let was_manifest = manifest_of(before, &was);
    assert_eq!(
        manifest_of(&after, &slice),
        Manifest {
            repos: slice.repos.clone(),
            ..was_manifest
        },
        "the manifest lists every repository and is otherwise as it was"
    );

    assert_eq!(after.branches, before.branches, "REPO's branches unchanged");
    assert_eq!(after.worktrees, before.worktrees, "REPO's worktrees unchanged");
    for (path, r) in &before.repos {
        if path != Path::new(repo_path) {
            assert_eq!(after.repos.get(path), Some(r), "{} unchanged", path.display());
        }
    }
    assert_eq!(after.tickets, before.tickets, "ticket status unchanged");
    assert!(
        !fake.write_ops().contains(&"set_status".to_string()),
        "ticket not written: {:?}",
        fake.writes()
    );
    added
}

/// The timer as it was: every entry still there, running as it was, and no
/// timer write.
fn assert_timer_unchanged(fake: &Fake, before: &World) {
    let after = fake.snapshot();
    assert_eq!(after.entries, before.entries, "Harvest entries unchanged");
    let ops = fake.write_ops();
    assert!(
        !ops.iter()
            .any(|o| ["start", "stop", "restart", "delete"].contains(&o.as_str())),
        "no timer written: {ops:?}"
    );
}

/// A refusal: the error, a non-zero exit, and nothing changed.
fn assert_refused(fake: &Fake, before: &World, err: &SliceError, want: &SliceError) {
    assert_eq!(err, want);
    assert_ne!(err.exit_code(), 0);
    assert_eq!(&fake.snapshot(), before, "nothing changed");
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// ---------------------------------------------------------------------------
// SLICE-10-1: repository added
// ---------------------------------------------------------------------------

// SLICE-10-1
#[test]
fn repository_added() {
    let (fake, _) = slice_and_api(true);
    let before = fake.snapshot();

    let outcome = open(&fake.deps(), &request_in(API, None)).expect("open");

    assert_added(&fake, &before, outcome, API, A_BRANCH, API_HEAD);
    assert_timer_unchanged(&fake, &before);
    assert!(
        fake.write_ops().contains(&"create_branch".to_string()),
        "branch created: {:?}",
        fake.writes()
    );
}

// SLICE-10-1
#[test]
fn repository_added_from_base() {
    let (fake, _) = slice_and_api(true);
    fake.with_branch_in(API, "dev", "0apidev", false);
    let before = fake.snapshot();
    let mut request = request_in(API, None);
    request.base = Some("dev".into());

    let outcome = open(&fake.deps(), &request).expect("open");

    assert_added(&fake, &before, outcome, API, A_BRANCH, "0apidev");
    assert_eq!(repo(&fake.snapshot(), API).branches[A_BRANCH], "0apidev");
    assert_timer_unchanged(&fake, &before);
}

// SLICE-10-1
#[test]
fn repository_added_after_another() {
    let (fake, _) = slice_and_api(true);
    fake.with_slice_repo(API);
    fake.with_repo(WEB, WEB_HEAD);
    let before = fake.snapshot();

    let outcome = open(&fake.deps(), &request_in(WEB, None)).expect("open");

    let added = assert_added(&fake, &before, outcome, WEB, A_BRANCH, WEB_HEAD);
    let slice = fake.snapshot().active.unwrap();
    let paths: Vec<_> = slice.repos.iter().map(|r| r.repo.clone()).collect();
    assert_eq!(paths, [PathBuf::from(REPO), PathBuf::from(API), PathBuf::from(WEB)]);
    assert_eq!(
        added.branch, slice.repos[0].branch,
        "the first repository's branch name"
    );
    assert_timer_unchanged(&fake, &before);
}

// SLICE-10-1
#[test]
fn repository_added_to_a_monday_slice() {
    let fake = Fake::new();
    fake.with_active_slice(MONDAY, true);
    fake.with_repo(API, API_HEAD);
    let before = fake.snapshot();
    let request = OpenRequest {
        ticket: tref(MONDAY),
        ..request_in(API, None)
    };

    let outcome = open(&fake.deps(), &request).expect("open");

    assert_added(
        &fake,
        &before,
        outcome,
        API,
        "feature/20261101-TBOARD-3-board",
        API_HEAD,
    );
    assert_timer_unchanged(&fake, &before);
}

// ---------------------------------------------------------------------------
// SLICE-10-2: type taken from the slice
// ---------------------------------------------------------------------------

// SLICE-10-2
#[test]
fn type_taken_from_the_slice() {
    for branch_type in [None, Some("feature")] {
        let (fake, _) = slice_and_api(true);
        let before = fake.snapshot();

        let outcome = open(&fake.deps(), &request_in(API, branch_type)).expect("open");

        assert_added(&fake, &before, outcome, API, A_BRANCH, API_HEAD);
        assert_timer_unchanged(&fake, &before);
    }
}

// SLICE-10-2
#[test]
fn other_type_refused() {
    for branch_type in ["bug", "refactor"] {
        let (fake, _) = slice_and_api(true);
        let before = fake.snapshot();

        let err = open(&fake.deps(), &request_in(API, Some(branch_type))).expect_err("open must fail");

        assert_refused(
            &fake,
            &before,
            &err,
            &SliceError::BranchOptionsConflict {
                branch: A_BRANCH.into(),
            },
        );
    }
}

// ---------------------------------------------------------------------------
// SLICE-10-3: existing branch in the added repository
// ---------------------------------------------------------------------------

// SLICE-10-3
#[test]
fn existing_branch_with_worktree_used() {
    let (fake, _) = slice_and_api(true);
    fake.with_branch_in(API, R_BRANCH, R_BRANCH_HEAD, true);
    let before = fake.snapshot();

    let outcome = open(&fake.deps(), &request_in(API, None)).expect("open");

    let added = assert_added(&fake, &before, outcome, API, R_BRANCH, R_BRANCH_HEAD);
    assert_eq!(added.worktree, worktree_in(API, R_BRANCH), "its existing worktree");
    assert_eq!(repo(&fake.snapshot(), API), repo(&before, API), "API unchanged");
    assert_timer_unchanged(&fake, &before);
    let ops = fake.write_ops();
    assert!(
        !ops.iter().any(|o| o == "create_branch" || o == "add_worktree"),
        "nothing created: {ops:?}"
    );
}

// SLICE-10-3
#[test]
fn existing_branch_without_worktree_used() {
    let (fake, _) = slice_and_api(true);
    fake.with_branch_in(API, R_BRANCH, R_BRANCH_HEAD, false);
    let before = fake.snapshot();

    let outcome = open(&fake.deps(), &request_in(API, None)).expect("open");

    assert_added(&fake, &before, outcome, API, R_BRANCH, R_BRANCH_HEAD);
    let after = fake.snapshot();
    assert_eq!(
        repo(&after, API).branches,
        repo(&before, API).branches,
        "no branch created"
    );
    assert_timer_unchanged(&fake, &before);
    let ops = fake.write_ops();
    assert!(ops.contains(&"add_worktree".to_string()), "worktree created: {ops:?}");
    assert!(
        !ops.contains(&"create_branch".to_string()),
        "no branch created: {ops:?}"
    );
}

// SLICE-10-3
#[test]
fn existing_branch_with_base_refused() {
    for with_worktree in [true, false] {
        let (fake, _) = slice_and_api(true);
        fake.with_branch_in(API, R_BRANCH, R_BRANCH_HEAD, with_worktree);
        let before = fake.snapshot();
        let mut request = request_in(API, None);
        request.base = Some("main".into());

        let err = open(&fake.deps(), &request).expect_err("open must fail");

        assert_refused(
            &fake,
            &before,
            &err,
            &SliceError::BranchOptionsConflict {
                branch: R_BRANCH.into(),
            },
        );
    }
}

// SLICE-10-3
#[test]
fn ambiguous_branch_in_added_repository() {
    let (fake, _) = slice_and_api(true);
    fake.with_branch_in(API, R_BRANCH, R_BRANCH_HEAD, true);
    fake.with_branch_in(API, "feature/20261030-ENG-1-login", "0feat00", false);
    let before = fake.snapshot();

    let err = open(&fake.deps(), &request_in(API, None)).expect_err("open must fail");

    let SliceError::AmbiguousBranch(mut branches) = err.clone() else {
        panic!("want AmbiguousBranch, got {err:?}")
    };
    branches.sort();
    assert_eq!(branches, [R_BRANCH, "feature/20261030-ENG-1-login"]);
    assert_ne!(err.exit_code(), 0);
    assert_eq!(fake.snapshot(), before, "nothing changed");
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// ---------------------------------------------------------------------------
// SLICE-10-4: adding restarts a stopped timer
// ---------------------------------------------------------------------------

/// The ways SLICE-10-4 adds a repository: (branch for T already in API, the
/// branch API ends up on, its base commit).
fn add_cases() -> [(bool, &'static str, &'static str); 2] {
    [(false, A_BRANCH, API_HEAD), (true, R_BRANCH, R_BRANCH_HEAD)]
}

// SLICE-10-4
#[test]
fn adding_restarts_todays_entry() {
    for (existing, branch, base) in add_cases() {
        let (fake, _) = slice_and_api(false);
        if existing {
            fake.with_branch_in(API, R_BRANCH, R_BRANCH_HEAD, false);
        }
        let entry = fake.entry_today(&tref(T)).unwrap().expect("T's entry today");
        let before = fake.snapshot();

        let outcome = open(&fake.deps(), &request_in(API, None)).expect("open");

        assert_added(&fake, &before, outcome, API, branch, base);
        let after = fake.snapshot();
        assert_eq!(
            after.running().iter().map(|e| e.id).collect::<Vec<_>>(),
            [entry],
            "today's entry restarted"
        );
        assert_eq!(after.entries.len(), before.entries.len(), "no new entry");
    }
}

// SLICE-10-4
#[test]
fn adding_starts_an_entry_when_today_has_none() {
    for (existing, branch, base) in add_cases() {
        let (fake, _) = slice_and_api(false);
        if existing {
            fake.with_branch_in(API, R_BRANCH, R_BRANCH_HEAD, true);
        }
        fake.world
            .borrow_mut()
            .entries
            .values_mut()
            .for_each(|e| e.today = false);
        let before = fake.snapshot();

        let outcome = open(&fake.deps(), &request_in(API, None)).expect("open");

        assert_added(&fake, &before, outcome, API, branch, base);
        let after = fake.snapshot();
        let running = after.running();
        assert_eq!(running.len(), 1, "one running timer");
        assert!(
            running[0].today && running[0].ticket == Some(tref(T)),
            "T's entry for today"
        );
        assert_eq!(running[0].notes, "ENG-1 - Login");
        assert_eq!(after.entries.len(), before.entries.len() + 1, "one new entry");
    }
}

// ---------------------------------------------------------------------------
// SLICE-10-5: adding refused on mismatch
// ---------------------------------------------------------------------------

// SLICE-10-5
#[test]
fn adding_refused_on_mismatch() {
    for stray in [Some(U), None] {
        for existing in [false, true] {
            let (fake, _) = slice_and_api(false);
            fake.with_running_timer(stray);
            if existing {
                fake.with_branch_in(API, R_BRANCH, R_BRANCH_HEAD, false);
            }
            let before = fake.snapshot();

            let err = open(&fake.deps(), &request_in(API, None)).expect_err("open must fail");

            let SliceError::Inconsistent(report) = &err else {
                panic!("want Inconsistent, got {err:?}")
            };
            assert_eq!(
                report.mismatch,
                Some(Mismatch::DifferentTicket {
                    slice: tref(T),
                    timer: stray.map(tref)
                }),
                "both values reported"
            );
            assert_refused(&fake, &before, &err, &err.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// SLICE-10-6: failed add undone
// ---------------------------------------------------------------------------

/// The write that undoes each write an add makes.
fn undo_of(op: &str) -> &'static str {
    match op {
        "start" => "delete",
        "restart" => "stop",
        "create_branch" => "remove_branch",
        "add_worktree" => "remove_worktree",
        "write_manifest" => "write_manifest",
        "set_active" => "set_active",
        other => panic!("an add does not write {other}"),
    }
}

/// The parts of the world a failed add must leave as they were.
fn assert_restored(fake: &Fake, before: &World, case: &str) {
    let after = fake.snapshot();
    assert_eq!(after.entry_states(), before.entry_states(), "{case}: entries restored");
    assert_eq!(after.tickets, before.tickets, "{case}: ticket status unchanged");
    assert_eq!(after.branches, before.branches, "{case}: REPO's branches");
    assert_eq!(after.worktrees, before.worktrees, "{case}: REPO's worktrees");
    assert_eq!(
        after.repos, before.repos,
        "{case}: other repositories' branches and worktrees"
    );
    assert_eq!(after.manifests, before.manifests, "{case}: manifest lists what it did");
    assert_eq!(after.active, before.active, "{case}: active slice lists what it did");
}

// SLICE-10-6
#[test]
fn failed_add_undone() {
    // (timer running, T has an entry today, API has a branch for T, the
    // writes that add makes in that state)
    let cases: [(bool, bool, bool, &[&str]); 4] = [
        (true, true, false, &["create_branch", "write_manifest", "set_active"]),
        (true, true, true, &["add_worktree", "write_manifest", "set_active"]),
        (
            false,
            true,
            true,
            &["restart", "add_worktree", "write_manifest", "set_active"],
        ),
        (
            false,
            false,
            false,
            &["start", "create_branch", "write_manifest", "set_active"],
        ),
    ];
    for (running, entry_today, existing, writes) in cases {
        for &fail in writes {
            let case = format!("running {running}, entry {entry_today}, branch {existing}, {fail} fails");
            let mut fake = Fake::new();
            fake.with_active_slice(T, running);
            fake.with_repo(API, API_HEAD);
            if !entry_today {
                fake.world
                    .borrow_mut()
                    .entries
                    .values_mut()
                    .for_each(|e| e.today = false);
            }
            if existing {
                fake.with_branch_in(API, R_BRANCH, R_BRANCH_HEAD, false);
            }
            fake.fail = Some(fail);
            let before = fake.snapshot();

            let err = open(&fake.deps(), &request_in(API, None)).expect_err(&case);

            let SliceError::StepFailed {
                step, undo_failures, ..
            } = &err
            else {
                panic!("{case}: want StepFailed, got {err:?}")
            };
            let want = match fail {
                "start" | "restart" => Step::StartTimer,
                "create_branch" | "add_worktree" => Step::CreateWorktree,
                "write_manifest" => Step::WriteManifest,
                _ => Step::SetActive,
            };
            assert_eq!(*step, want, "{case}: failed step named");
            assert!(undo_failures.is_empty(), "{case}: undo succeeded");
            assert_ne!(err.exit_code(), 0);
            assert_restored(&fake, &before, &case);

            let ops = fake.write_ops();
            assert_eq!(ops.len() % 2, 0, "{case}: one undo per step done: {ops:?}");
            let (done, undone) = ops.split_at(ops.len() / 2);
            let reversed: Vec<&str> = done.iter().rev().map(|o| undo_of(o)).collect();
            assert_eq!(undone, reversed, "{case}: undone in reverse order: {ops:?}");
        }
    }
}

// SLICE-10-6
#[test]
fn failed_add_keeps_what_it_did_not_create() {
    for fail in ["write_manifest", "set_active"] {
        let mut fake = Fake::new();
        fake.with_active_slice(T, true);
        fake.with_repo(API, API_HEAD);
        fake.with_branch_in(API, R_BRANCH, R_BRANCH_HEAD, true);
        fake.fail = Some(fail);
        let before = fake.snapshot();

        open(&fake.deps(), &request_in(API, None)).expect_err("open must fail");

        assert_restored(&fake, &before, fail);
        let ops = fake.write_ops();
        assert!(
            !ops.iter()
                .any(|o| ["remove_branch", "remove_worktree", "stop", "delete"].contains(&o.as_str())),
            "{fail}: nothing removed or stopped that this run did not create: {ops:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// SLICE-2-4, SLICE-2-7: a repository already in the slice
// ---------------------------------------------------------------------------

// SLICE-2-4
#[test]
fn already_open_in_an_added_repository() {
    for repo in [REPO, API] {
        let (fake, _) = slice_and_api(true);
        fake.with_slice_repo(API);
        let slice = fake.snapshot().active.unwrap();
        let before = fake.snapshot();

        let outcome = open(&fake.deps(), &request_in(repo, None)).expect("open succeeds");

        assert_eq!(outcome, OpenOutcome::AlreadyOpen(slice), "{repo}");
        assert_eq!(fake.snapshot(), before, "{repo}: nothing changed");
        assert_eq!(fake.writes(), Vec::<String>::new());
    }
}

// SLICE-2-7
#[test]
fn timer_restarted_in_an_added_repository() {
    for repo in [REPO, API] {
        let (fake, _) = slice_and_api(false);
        fake.with_slice_repo(API);
        let slice = fake.snapshot().active.unwrap();
        let entry = fake.entry_today(&tref(T)).unwrap().unwrap();
        let before = fake.snapshot();

        let outcome = open(&fake.deps(), &request_in(repo, None)).expect("open succeeds");

        assert_eq!(outcome, OpenOutcome::Resumed(slice), "{repo}");
        let after = fake.snapshot();
        assert_eq!(after.running().iter().map(|e| e.id).collect::<Vec<_>>(), [entry]);
        assert_eq!(after.entries.len(), before.entries.len(), "no new entry");
        assert_eq!(after.tickets, before.tickets, "ticket unchanged");
        assert_eq!(after.branches, before.branches, "branches unchanged");
        assert_eq!(after.worktrees, before.worktrees, "worktrees unchanged");
        assert_eq!(after.repos, before.repos, "other repositories unchanged");
        assert_eq!(after.manifests, before.manifests, "manifest unchanged");
        assert_eq!(after.active, before.active, "active slice unchanged");
    }
}

// ---------------------------------------------------------------------------
// SLICE-10-8: reports list every repository
// ---------------------------------------------------------------------------

/// A slice in two repositories whose paths, branch and worktrees are none of
/// them part of another.
fn two_repo_slice() -> ActiveSlice {
    ActiveSlice {
        id: SliceId("ENG-1-1".into()),
        ticket: tref(T),
        repos: vec![
            SliceRepo {
                repo: PathBuf::from("/src/app"),
                branch: A_BRANCH.into(),
                worktree: PathBuf::from("/trees/one"),
                base_commit: "def5678".into(),
            },
            SliceRepo {
                repo: PathBuf::from("/src/api"),
                branch: A_BRANCH.into(),
                worktree: PathBuf::from("/trees/two"),
                base_commit: "0api000".into(),
            },
        ],
    }
}

fn one_repo_slice() -> ActiveSlice {
    let mut slice = two_repo_slice();
    slice.repos.truncate(1);
    slice
}

/// Each repository's path, branch and worktree appear in `text`, one
/// repository after another in the order they were added.
fn assert_lists_repos(text: &str, slice: &ActiveSlice) {
    let mut from = 0;
    for r in &slice.repos {
        let mut end = from;
        for item in [
            r.repo.display().to_string(),
            r.branch.clone(),
            r.worktree.display().to_string(),
        ] {
            let at = text[from..]
                .find(&item)
                .unwrap_or_else(|| panic!("{item} missing after byte {from} of {text:?}"));
            end = end.max(from + at + item.len());
        }
        from = end;
    }
}

/// The slice's JSON has a `repos` array listing each repository's path,
/// branch and worktree in order.
fn assert_json_lists_repos(json: &serde_json::Value, slice: &ActiveSlice) {
    let repos = json["repos"]
        .as_array()
        .unwrap_or_else(|| panic!("no repos array: {json}"));
    assert_eq!(repos.len(), slice.repos.len(), "{json}");
    for (got, want) in repos.iter().zip(&slice.repos) {
        assert_eq!(got["repo"], want.repo.display().to_string(), "{json}");
        assert_eq!(got["branch"], want.branch, "{json}");
        assert_eq!(got["worktree"], want.worktree.display().to_string(), "{json}");
    }
}

// SLICE-10-8
#[test]
fn slice_json_lists_repositories() {
    for slice in [one_repo_slice(), two_repo_slice()] {
        let json = slice_json(&slice);
        assert_eq!(json["id"], "ENG-1-1", "{json}");
        assert_eq!(json["ticket"], "ENG-1", "{json}");
        assert_json_lists_repos(&json, &slice);
    }
}

// SLICE-10-8
#[test]
fn kept_branches_lists_repositories() {
    for slice in [one_repo_slice(), two_repo_slice()] {
        assert_lists_repos(&kept_branches(&slice), &slice);
    }
}

// SLICE-10-8
#[test]
fn abandon_report_lists_repositories() {
    for slice in [one_repo_slice(), two_repo_slice()] {
        for reason in [None, Some("fixed by ENG-14800".to_string())] {
            let abandoned = Abandoned {
                slice: slice.clone(),
                reason,
            };
            assert_lists_repos(&abandoned.message(), &slice);
            assert_json_lists_repos(&abandoned.to_json()["slice"], &slice);
        }
    }
}

// SLICE-10-8
#[test]
fn close_and_abandon_report_the_slice_with_its_repositories() {
    let (fake, _) = slice_and_api(true);
    fake.with_slice_repo(API);
    let slice = fake.snapshot().active.unwrap();

    let closed = close(&fake.deps(), &CloseRequest { done: false }).expect("close");
    assert_eq!(closed, slice, "close reports every repository");

    let (fake, _) = slice_and_api(true);
    fake.with_slice_repo(API);
    let abandoned = abandon(&fake.deps(), &AbandonRequest { reason: None }).expect("abandon");
    assert_eq!(abandoned.slice, slice, "abandon reports every repository");
    assert_lists_repos(&abandoned.message(), &slice);
}

// SLICE-10-8
#[test]
fn pause_and_resume_report_the_slice_with_its_repositories() {
    let (fake, _) = slice_and_api(true);
    fake.with_slice_repo(API);
    let slice = fake.snapshot().active.unwrap();

    assert_eq!(pause(&fake.deps()).expect("pause"), PauseOutcome::Paused(slice.clone()));
    assert_eq!(
        resume(&fake.deps()).expect("resume"),
        ResumeOutcome::Resumed(slice.clone())
    );
    let report = status(&fake.deps()).expect("status");
    assert_eq!(report.active, Some(slice.clone()));
    assert_json_lists_repos(&slice_json(&slice), &slice);
}

// ---------------------------------------------------------------------------
// SLICE-10-9: state written before SLICE-10
// ---------------------------------------------------------------------------

/// `active.json` as slice wrote it before SLICE-10: one repository, as
/// top-level repo, branch and worktree.
const EARLIER_ACTIVE: &str = r#"{
  "id": "ENG-1-1",
  "ticket": "ENG-1",
  "repo": "/work/app",
  "branch": "feature/20261101-ENG-1-login",
  "worktree": "/work/app.slices/feature-20261101-ENG-1-login"
}
"#;

/// That slice's manifest as slice wrote it before SLICE-10: top-level branch
/// and base commit, and no repository path.
const EARLIER_MANIFEST: &str = "id: \"ENG-1-1\"
ticket: \"ENG-1\"
ticket_system: clickup
branch: \"feature/20261101-ENG-1-login\"
base_commit: \"def5678\"
status: open
acceptance_criteria: []
invariants: []
spec_hash: null
test_hash: null
budgets: {}
";

/// Local state as slice left it before SLICE-10, with the slice above active.
fn earlier_state(name: &str) -> (common::agent::TempDir, FileState) {
    let dir = common::agent::temp_dir(name);
    let state_dir = dir.join("state");
    std::fs::create_dir_all(state_dir.join("manifests")).unwrap();
    std::fs::write(state_dir.join("active.json"), EARLIER_ACTIVE).unwrap();
    std::fs::write(state_dir.join("counts.json"), "{\n  \"ENG-1\": 1\n}\n").unwrap();
    std::fs::write(state_dir.join("manifests/ENG-1-1.yaml"), EARLIER_MANIFEST).unwrap();
    (dir, FileState::new(state_dir))
}

/// The one repository the earlier state holds.
fn earlier_repo() -> SliceRepo {
    SliceRepo {
        repo: PathBuf::from(REPO),
        branch: A_BRANCH.into(),
        worktree: worktree_for(A_BRANCH),
        base_commit: T_BRANCH_OLD_HEAD.into(),
    }
}

/// The fake world for the earlier state: T's branch in `REPO` checked out
/// in its worktree, T's entry for today, and `API` with no branch for T.
fn earlier_world(timer_running: bool) -> Fake {
    let fake = Fake::new();
    fake.add_entry(Some(T), timer_running, true);
    fake.with_branch(A_BRANCH, T_BRANCH_OLD_HEAD, true);
    fake.with_repo(API, API_HEAD);
    fake
}

fn deps_with<'a>(fake: &'a Fake, state: &'a FileState) -> Deps<'a> {
    Deps { state, ..fake.deps() }
}

// SLICE-10-9
#[test]
fn earlier_active_slice_reads_as_one_repository() {
    let (_dir, state) = earlier_state("earlier-active");
    let want = ActiveSlice {
        id: SliceId("ENG-1-1".into()),
        ticket: tref(T),
        repos: vec![earlier_repo()],
    };

    assert_eq!(state.active().unwrap(), Some(want.clone()));

    let fake = earlier_world(true);
    let report = status(&deps_with(&fake, &state)).expect("status");
    assert_eq!(report.active, Some(want));
    assert_eq!(state.next_slice_id(&tref(T)).unwrap(), SliceId("ENG-1-2".into()));
}

// SLICE-10-9
#[test]
fn earlier_slice_already_open_in_its_repository() {
    let (_dir, state) = earlier_state("earlier-reopen");
    let fake = earlier_world(true);

    let outcome = open(&deps_with(&fake, &state), &request_in(REPO, None)).expect("open");

    let OpenOutcome::AlreadyOpen(slice) = outcome else {
        panic!("want AlreadyOpen, got {outcome:?}")
    };
    assert_eq!(slice.repos, [earlier_repo()]);
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// SLICE-10-9
#[test]
fn repository_added_to_an_earlier_slice() {
    for timer_running in [true, false] {
        let (_dir, state) = earlier_state(&format!("earlier-add-{timer_running}"));
        let fake = earlier_world(timer_running);

        let outcome = open(&deps_with(&fake, &state), &request_in(API, None)).expect("open");

        let OpenOutcome::Added(slice) = outcome else {
            panic!("want Added, got {outcome:?}")
        };
        let api = SliceRepo {
            repo: PathBuf::from(API),
            branch: A_BRANCH.into(),
            worktree: slice.repos.last().unwrap().worktree.clone(),
            base_commit: API_HEAD.into(),
        };
        assert_eq!(slice.repos, [earlier_repo(), api]);
        assert_eq!(state.active().unwrap(), Some(slice.clone()), "active slice written");
        let manifest = state.read_manifest(&slice.id).unwrap().expect("manifest");
        assert_eq!(manifest.repos, slice.repos, "manifest lists both repositories");
        assert_eq!(manifest.status, ManifestStatus::Open);
        assert_eq!(manifest.ticket, tref(T));
        assert_eq!(manifest.ticket_system, TicketSystemKind::ClickUp);
        let w = fake.snapshot();
        assert_eq!(w.running().len(), 1, "T's timer running");
        assert_eq!(
            w.worktrees.get(A_BRANCH),
            Some(&worktree_for(A_BRANCH)),
            "REPO untouched"
        );
    }
}

// ---------------------------------------------------------------------------
// Invariants over adds
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Branches {
    None,
    WithWorktree,
    WithoutWorktree,
    Two,
}

#[derive(Clone, Debug)]
struct Start {
    /// Whether the slice's timer is running.
    running: bool,
    /// A running timer that is not the slice's, for a ticket or none. Only
    /// when the slice's own timer is not running.
    stray: Option<Option<&'static str>>,
    /// T's entry for today, rather than one from an earlier day.
    entry_today: bool,
    /// API is in the slice before the run.
    api_in_slice: bool,
    /// The repository open is run in.
    repo: &'static str,
    /// Branches for T in WEB, and in API when it is not in the slice.
    branches: Branches,
    branch_type: Option<&'static str>,
    base: Option<&'static str>,
    fail: Option<&'static str>,
    unreachable: Option<Source>,
}

impl Start {
    fn build(&self) -> Fake {
        let mut fake = Fake::new();
        fake.with_active_slice(T, self.running);
        if !self.entry_today {
            fake.world
                .borrow_mut()
                .entries
                .values_mut()
                .for_each(|e| e.today = false);
        }
        if let Some(stray) = self.stray {
            fake.with_running_timer(stray);
        }
        fake.with_repo(API, API_HEAD);
        fake.with_repo(WEB, WEB_HEAD);
        if self.api_in_slice {
            fake.with_slice_repo(API);
        }
        for repo in [API, WEB] {
            if repo == API && self.api_in_slice {
                continue;
            }
            match self.branches {
                Branches::None => {}
                Branches::WithWorktree => fake.with_branch_in(repo, R_BRANCH, R_BRANCH_HEAD, true),
                Branches::WithoutWorktree => fake.with_branch_in(repo, R_BRANCH, R_BRANCH_HEAD, false),
                Branches::Two => {
                    fake.with_branch_in(repo, R_BRANCH, R_BRANCH_HEAD, true);
                    fake.with_branch_in(repo, "feature/20261030-ENG-1-login", "0feat00", false);
                }
            }
        }
        fake.fail = self.fail;
        fake.unreachable = self.unreachable.into_iter().collect();
        fake
    }

    fn request(&self) -> OpenRequest {
        OpenRequest {
            ticket: tref(T),
            repo: PathBuf::from(self.repo),
            branch_type: self.branch_type.map(|t| BranchType(t.into())),
            base: self.base.map(String::from),
        }
    }
}

const WRITES: [&str; 11] = [
    "stop",
    "start",
    "restart",
    "delete",
    "set_status",
    "create_branch",
    "add_worktree",
    "remove_worktree",
    "remove_branch",
    "write_manifest",
    "set_active",
];

fn start() -> impl Strategy<Value = Start> {
    let stray = prop_oneof![
        2 => Just(None),
        1 => prop::sample::select(vec![Some(U), None]).prop_map(Some)
    ];
    let branches = prop_oneof![
        Just(Branches::None),
        Just(Branches::WithWorktree),
        Just(Branches::WithoutWorktree),
        Just(Branches::Two),
    ];
    let repo = prop::sample::select(vec![REPO, API, WEB]);
    let branch_type = prop::sample::select(vec![None, Some("feature"), Some("bug")]);
    let base = prop::sample::select(vec![None, Some("main")]);
    let fail = prop_oneof![Just(None), prop::sample::select(WRITES.to_vec()).prop_map(Some)];
    let unreachable = prop_oneof![
        3 => Just(None),
        1 => prop::sample::select(vec![
            Source::Harvest,
            Source::TicketSystem(TicketSystemKind::ClickUp),
            Source::LocalState,
            Source::Git,
        ])
        .prop_map(Some),
    ];
    (
        any::<bool>(),
        stray,
        any::<bool>(),
        any::<bool>(),
        repo,
        branches,
        branch_type,
        base,
        fail,
        unreachable,
    )
        .prop_map(
            |(running, stray, entry_today, api_in_slice, repo, branches, branch_type, base, fail, unreachable)| Start {
                running,
                stray: if running { None } else { stray },
                entry_today,
                api_in_slice,
                repo,
                branches,
                branch_type,
                base,
                fail,
                unreachable,
            },
        )
}

/// A repository's branches and worktrees, `REPO`'s included.
fn on_disk(
    w: &World,
    path: &Path,
) -> (
    std::collections::BTreeMap<String, String>,
    std::collections::BTreeMap<String, PathBuf>,
) {
    if path == Path::new(REPO) {
        return (w.branches.clone(), w.worktrees.clone());
    }
    let r = &w.repos[path];
    (r.branches.clone(), r.worktrees.clone())
}

proptest! {
    // SLICE-10-INV-1
    #[test]
    fn other_repositories_untouched(s in start()) {
        let fake = s.build();
        let before = fake.snapshot();
        let was = before.active.clone().unwrap();

        let _ = open(&fake.deps(), &s.request());

        let after = fake.snapshot();
        let now = after.active.clone().expect("the slice is still active");
        let manifest = &after.manifests[&was.id];
        prop_assert!(now.repos.len() >= was.repos.len(), "repositories dropped: {:?}", now.repos);
        prop_assert_eq!(&now.repos[..was.repos.len()], &was.repos[..], "active slice's repositories");
        prop_assert!(manifest.repos.len() >= was.repos.len(), "repositories dropped: {:?}", manifest.repos);
        prop_assert_eq!(&manifest.repos[..was.repos.len()], &was.repos[..], "manifest's repositories");
        for r in &was.repos {
            let (branches, worktrees) = on_disk(&after, &r.repo);
            let (old_branches, old_worktrees) = on_disk(&before, &r.repo);
            prop_assert_eq!(branches.get(&r.branch), old_branches.get(&r.branch), "{} branch", r.repo.display());
            prop_assert_eq!(worktrees.get(&r.branch), old_worktrees.get(&r.branch), "{} worktree", r.repo.display());
        }
    }

    // SLICE-2-INV-1
    #[test]
    fn adding_leaves_at_most_one_running_timer(s in start()) {
        let fake = s.build();

        let _ = open(&fake.deps(), &s.request());

        let w = fake.snapshot();
        prop_assert!(w.running().len() <= 1, "running: {:?}", w.running());
    }

    // SLICE-2-INV-2
    // SLICE-10-6
    #[test]
    fn failed_add_leaves_no_trace(s in start()) {
        let fake = s.build();
        let before = fake.snapshot();

        if open(&fake.deps(), &s.request()).is_err() {
            let after = fake.snapshot();
            prop_assert_eq!(after.entry_states(), before.entry_states(), "entries and running states");
            prop_assert_eq!(&after.branches, &before.branches, "REPO's branches");
            prop_assert_eq!(&after.worktrees, &before.worktrees, "REPO's worktrees");
            prop_assert_eq!(&after.repos, &before.repos, "other repositories");
            prop_assert_eq!(&after.manifests, &before.manifests, "manifests");
            prop_assert_eq!(&after.active, &before.active, "active slice");
            prop_assert_eq!(&after.tickets, &before.tickets, "ticket statuses");
        }
    }
}
