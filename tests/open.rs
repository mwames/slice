mod common;

use std::path::PathBuf;

use common::*;
use slice::ports::*;

fn request(ticket: &str, branch_type: Option<&str>) -> OpenRequest {
    OpenRequest {
        ticket: tref(ticket),
        repo: PathBuf::from(REPO),
        branch_type: branch_type.map(|t| BranchType(t.to_string())),
        base: None,
    }
}

fn feature(ticket: &str) -> OpenRequest {
    request(ticket, Some("feature"))
}

/// Everything SLICE-2-1 requires of a completed open, given the branch and
/// base commit the slice should be on.
fn assert_opened(
    fake: &Fake,
    outcome: OpenOutcome,
    ticket: &str,
    in_progress: &str,
    branch: &str,
    base: &str,
) -> ActiveSlice {
    let OpenOutcome::Opened(slice) = outcome else {
        panic!("want Opened, got {outcome:?}")
    };
    let w = fake.snapshot();

    assert_eq!(slice.ticket, tref(ticket));
    let running = w.running();
    assert_eq!(running.len(), 1, "exactly one running timer");
    assert_eq!(running[0].ticket, Some(tref(ticket)));
    let title = &w.tickets[ticket].title;
    assert_eq!(
        running[0].notes,
        format!("{ticket} - {title}"),
        "notes carry the ticket and title only"
    );

    assert_eq!(w.tickets[ticket].status, in_progress);
    let [repo] = slice.repos.as_slice() else {
        panic!("want one repository, got {:?}", slice.repos)
    };
    assert_eq!(repo.repo, PathBuf::from(REPO));
    assert_eq!(repo.branch, branch);
    assert!(w.branches.contains_key(branch), "branch {branch} exists");
    assert_eq!(w.worktrees.get(branch), Some(&repo.worktree), "worktree checked out");

    let manifest = w.manifests.get(&slice.id).expect("manifest written");
    assert_eq!(manifest.ticket, tref(ticket));
    assert_eq!(manifest.repos, slice.repos);
    assert_eq!(manifest.repos[0].base_commit, base);
    assert_eq!(manifest.status, ManifestStatus::Open);

    assert_eq!(w.active.as_ref(), Some(&slice));
    slice
}

// SLICE-2-1
#[test]
fn clean_open() {
    let fake = Fake::new();

    let outcome = open(&fake.deps(), &feature(T)).expect("open");

    let slice = assert_opened(&fake, outcome, T, "development", T_BRANCH_NEW, BASE_COMMIT);
    assert_eq!(fake.snapshot().entries.len(), 1, "one new entry");
    assert!(
        !slice.repos[0].branch.contains(&slice.id.0),
        "branch name has no slice id"
    );
}

// SLICE-2-1
#[test]
fn clean_open_monday_bug() {
    let fake = Fake::new();

    let outcome = open(&fake.deps(), &request(MONDAY, Some("bug"))).expect("open");

    assert_opened(
        &fake,
        outcome,
        MONDAY,
        "In Progress",
        "bug/20261102-TBOARD-3-board",
        BASE_COMMIT,
    );
}

// SLICE-2-1
#[test]
fn clean_open_from_base() {
    let fake = Fake::new();
    fake.with_branch("dev", "0dev000", false);
    let mut req = feature(T);
    req.base = Some("dev".into());

    let outcome = open(&fake.deps(), &req).expect("open");

    assert_opened(&fake, outcome, T, "development", T_BRANCH_NEW, "0dev000");
}

// SLICE-2-2
#[test]
fn stray_timer_running() {
    for stray_ticket in [Some(U), None] {
        let fake = Fake::new();
        let stray = fake.with_running_timer(stray_ticket);

        let outcome = open(&fake.deps(), &feature(T)).expect("open");

        assert_opened(&fake, outcome, T, "development", T_BRANCH_NEW, BASE_COMMIT);
        assert!(
            !fake.snapshot().entries[&stray.0].running,
            "stray timer for {stray_ticket:?} stopped"
        );
        let ops = fake.write_ops();
        let stop = ops.iter().position(|o| o == "stop").expect("stray stopped");
        let start = ops.iter().position(|o| o == "start").expect("timer started");
        assert!(stop < start, "stray stopped before the new timer started: {ops:?}");
    }
}

// SLICE-2-3
#[test]
fn another_slice_active() {
    let fake = Fake::new();
    let other = fake.with_active_slice(U, true);
    let before = fake.snapshot();

    let err = open(&fake.deps(), &feature(T)).expect_err("open must fail");

    assert_eq!(err, SliceError::AnotherSliceActive(Box::new(other)));
    assert_ne!(err.exit_code(), 0);
    assert_eq!(fake.snapshot(), before, "nothing changed");
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// SLICE-2-4
#[test]
fn same_slice_already_open() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, true);
    let before = fake.snapshot();

    let outcome = open(&fake.deps(), &request(T, None)).expect("open succeeds");

    assert_eq!(outcome, OpenOutcome::AlreadyOpen(slice));
    assert_eq!(fake.snapshot(), before, "nothing changed");
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// SLICE-2-5
#[test]
fn unknown_ticket() {
    let fake = Fake::new();
    let before = fake.snapshot();

    let err = open(&fake.deps(), &feature(UNKNOWN)).expect_err("open must fail");

    assert_eq!(err, SliceError::UnknownTicket(tref(UNKNOWN)));
    assert_ne!(err.exit_code(), 0);
    assert_eq!(fake.snapshot(), before, "nothing changed");
    assert_eq!(fake.writes(), Vec::<String>::new());
}

fn assert_failed_at(err: &SliceError, step: Step, fail: &str) {
    let SliceError::StepFailed {
        step: failed,
        undo_failures,
        ..
    } = err
    else {
        panic!("{fail}: want StepFailed, got {err:?}")
    };
    assert_eq!(*failed, step, "{fail}: failed step named");
    assert!(undo_failures.is_empty(), "{fail}: undo succeeded");
    assert_ne!(err.exit_code(), 0);
}

/// The parts of the world a failed open must leave as they were.
fn assert_restored(fake: &Fake, before: &World, case: &str) {
    let after = fake.snapshot();
    assert_eq!(
        after.entry_states(),
        before.entry_states(),
        "{case}: entries and running states restored"
    );
    assert_eq!(after.tickets, before.tickets, "{case}: ticket status restored");
    assert_eq!(after.branches, before.branches, "{case}: branches restored");
    assert_eq!(after.worktrees, before.worktrees, "{case}: worktrees restored");
    assert_eq!(after.manifests, before.manifests, "{case}: no manifest");
    assert_eq!(after.active, before.active, "{case}: no active slice");
}

// SLICE-2-6
#[test]
fn partial_failure_new_entry_and_branch() {
    let cases = [
        ("start", Step::StartTimer),
        ("set_status", Step::SetInProgress),
        ("create_branch", Step::CreateWorktree),
        ("write_manifest", Step::WriteManifest),
        ("set_active", Step::SetActive),
    ];
    for (fail, step) in cases {
        let mut fake = Fake::new();
        let stray = fake.with_running_timer(Some(U));
        fake.fail = Some(fail);
        let before = fake.snapshot();

        let err = open(&fake.deps(), &feature(T)).expect_err("open must fail");

        assert_failed_at(&err, step, fail);
        assert_restored(&fake, &before, fail);
        assert!(
            fake.writes().contains(&format!("restart {}", stray.0)),
            "{fail}: stray timer restarted"
        );
    }
}

// SLICE-2-6
#[test]
fn partial_failure_keeps_existing_entry_and_branch() {
    for fail in ["set_status", "add_worktree", "write_manifest", "set_active"] {
        let mut fake = Fake::new();
        let entry = fake.add_entry(Some(T), false, true);
        fake.with_branch(T_BRANCH_OLD, T_BRANCH_OLD_HEAD, false);
        fake.fail = Some(fail);
        let before = fake.snapshot();

        open(&fake.deps(), &request(T, None)).expect_err("open must fail");

        assert_restored(&fake, &before, fail);
        assert!(
            !fake.write_ops().contains(&"delete".to_string()),
            "{fail}: existing entry {} not deleted",
            entry.0
        );
        assert!(
            !fake.write_ops().contains(&"remove_branch".to_string()),
            "{fail}: existing branch not removed"
        );
    }
}

// SLICE-2-6
#[test]
fn partial_failure_undoes_in_reverse_order() {
    let mut fake = Fake::new();
    fake.with_running_timer(Some(U));
    fake.fail = Some("set_active");

    open(&fake.deps(), &feature(T)).expect_err("open must fail");

    assert_eq!(
        fake.write_ops(),
        [
            "stop",
            "start",
            "set_status",
            "create_branch",
            "write_manifest",
            "remove_manifest",
            "remove_branch",
            "set_status",
            "delete",
            "restart",
        ]
    );
}

// SLICE-2-6
#[test]
fn partial_failure_stops_a_restarted_entry() {
    let mut fake = Fake::new();
    fake.add_entry(Some(T), false, true);
    fake.with_branch(T_BRANCH_OLD, T_BRANCH_OLD_HEAD, true);
    fake.fail = Some("set_active");

    open(&fake.deps(), &request(T, None)).expect_err("open must fail");

    assert_eq!(
        fake.write_ops(),
        [
            "restart",
            "set_status",
            "write_manifest",
            "remove_manifest",
            "set_status",
            "stop"
        ]
    );
}

// SLICE-2-7
#[test]
fn same_slice_timer_stopped() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, false);
    let entry = fake.entry_today(&tref(T)).unwrap().unwrap();
    let before = fake.snapshot();

    let outcome = open(&fake.deps(), &request(T, None)).expect("open succeeds");

    assert_eq!(outcome, OpenOutcome::Resumed(slice));
    let after = fake.snapshot();
    assert_eq!(
        after.running().iter().map(|e| e.id).collect::<Vec<_>>(),
        [entry],
        "today's entry restarted"
    );
    assert_eq!(after.entries.len(), before.entries.len(), "no new entry");
    assert_eq!(after.tickets, before.tickets, "ticket unchanged");
    assert_eq!(after.branches, before.branches, "branch unchanged");
    assert_eq!(after.worktrees, before.worktrees, "worktree unchanged");
    assert_eq!(after.manifests, before.manifests, "manifest unchanged");
}

// SLICE-2-7
#[test]
fn same_slice_paused_since_yesterday() {
    let fake = Fake::new();
    fake.with_active_slice(T, false);
    // The slice's entry is from an earlier day: today has none for T.
    fake.world
        .borrow_mut()
        .entries
        .values_mut()
        .for_each(|e| e.today = false);
    let before = fake.snapshot();

    let outcome = open(&fake.deps(), &request(T, None)).expect("open succeeds");

    assert!(matches!(outcome, OpenOutcome::Resumed(_)), "got {outcome:?}");
    let after = fake.snapshot();
    let running = after.running();
    assert_eq!(running.len(), 1);
    assert!(
        running[0].today && running[0].ticket == Some(tref(T)),
        "a new entry for T today"
    );
    assert_eq!(after.entries.len(), before.entries.len() + 1);
}

// SLICE-2-8
#[test]
fn ticket_already_tracked_today() {
    let fake = Fake::new();
    let entry = fake.add_entry(Some(T), false, true);

    let outcome = open(&fake.deps(), &feature(T)).expect("open");

    assert_opened(&fake, outcome, T, "development", T_BRANCH_NEW, BASE_COMMIT);
    let after = fake.snapshot();
    assert_eq!(after.entries.len(), 1, "no new entry");
    assert!(after.entries[&entry.0].running, "today's entry restarted");
}

// SLICE-2-9
#[test]
fn ticket_branch_exists_with_worktree() {
    for branch_type in [None, Some("feature")] {
        let fake = Fake::new();
        fake.with_branch(T_BRANCH_OLD, T_BRANCH_OLD_HEAD, true);
        let branches_before = fake.snapshot().branches;

        let outcome = open(&fake.deps(), &request(T, branch_type)).expect("open");

        assert_opened(&fake, outcome, T, "development", T_BRANCH_OLD, T_BRANCH_OLD_HEAD);
        assert_eq!(fake.snapshot().branches, branches_before, "no other branch created");
        assert!(
            !fake
                .write_ops()
                .iter()
                .any(|o| o == "add_worktree" || o == "create_branch")
        );
    }
}

// SLICE-2-9
#[test]
fn ticket_branch_exists_without_worktree() {
    let fake = Fake::new();
    fake.with_branch(T_BRANCH_OLD, T_BRANCH_OLD_HEAD, false);

    let outcome = open(&fake.deps(), &request(T, None)).expect("open");

    assert_opened(&fake, outcome, T, "development", T_BRANCH_OLD, T_BRANCH_OLD_HEAD);
    assert!(
        fake.write_ops().contains(&"add_worktree".to_string()),
        "worktree created"
    );
}

// SLICE-2-9
#[test]
fn another_tickets_branch_is_not_reused() {
    let fake = Fake::new();
    fake.with_branch("feature/20261101-ENG-12-other", "0other0", true);

    let outcome = open(&fake.deps(), &feature(T)).expect("open");

    assert_opened(&fake, outcome, T, "development", T_BRANCH_NEW, BASE_COMMIT);
}

// SLICE-2-10
#[test]
fn branch_options_conflict() {
    let mut with_base = request(T, None);
    with_base.base = Some("main".into());
    for req in [with_base, request(T, Some("bug"))] {
        let fake = Fake::new();
        fake.with_branch(T_BRANCH_OLD, T_BRANCH_OLD_HEAD, true);
        let before = fake.snapshot();

        let err = open(&fake.deps(), &req).expect_err("open must fail");

        assert_eq!(
            err,
            SliceError::BranchOptionsConflict {
                branch: T_BRANCH_OLD.into()
            },
            "{req:?}"
        );
        assert_ne!(err.exit_code(), 0);
        assert_eq!(fake.snapshot(), before, "nothing changed");
        assert_eq!(fake.writes(), Vec::<String>::new());
    }
}

// SLICE-2-11
#[test]
fn type_required_for_new_branch() {
    let fake = Fake::new();
    let before = fake.snapshot();

    let err = open(&fake.deps(), &request(T, None)).expect_err("open must fail");

    assert_eq!(err, SliceError::TypeRequired);
    assert_ne!(err.exit_code(), 0);
    assert_eq!(fake.snapshot(), before, "nothing changed");
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// SLICE-2-12
#[test]
fn ambiguous_ticket_branch() {
    let fake = Fake::new();
    fake.with_branch(T_BRANCH_OLD, T_BRANCH_OLD_HEAD, true);
    fake.with_branch("bug/20261020-ENG-1-login-crash", "0bug000", false);
    let before = fake.snapshot();

    let err = open(&fake.deps(), &request(T, None)).expect_err("open must fail");

    let SliceError::AmbiguousBranch(mut branches) = err.clone() else {
        panic!("want AmbiguousBranch, got {err:?}")
    };
    branches.sort();
    assert_eq!(branches, ["bug/20261020-ENG-1-login-crash", T_BRANCH_OLD]);
    assert_ne!(err.exit_code(), 0);
    assert_eq!(fake.snapshot(), before, "nothing changed");
}
