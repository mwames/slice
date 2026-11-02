mod common;

use std::path::PathBuf;

use common::*;
use slice::ports::*;

fn request(ticket: &str) -> OpenRequest {
    OpenRequest {
        ticket: tref(ticket),
        repo: PathBuf::from(REPO),
        base: "HEAD".to_string(),
    }
}

/// Everything SLICE-2-1 requires of a completed open.
fn assert_opened(fake: &Fake, outcome: OpenOutcome, ticket: &str, in_progress: &str) -> ActiveSlice {
    let OpenOutcome::Opened(slice) = outcome else {
        panic!("want Opened, got {outcome:?}")
    };
    let w = fake.snapshot();

    assert_eq!(slice.ticket, tref(ticket));
    let running = w.running();
    assert_eq!(running.len(), 1, "exactly one running timer");
    assert_eq!(running[0].id, slice.timer);
    assert_eq!(running[0].ticket, Some(tref(ticket)));
    assert!(
        running[0].notes.contains(&slice.id.0),
        "notes {:?} name slice {}",
        running[0].notes,
        slice.id.0
    );

    assert_eq!(w.tickets[ticket].status, in_progress);
    assert!(
        w.worktrees.contains(&(slice.branch.clone(), slice.worktree.clone())),
        "branch and worktree exist"
    );

    let manifest = w.manifests.get(&slice.manifest).expect("manifest written");
    assert_eq!(manifest.id, slice.id);
    assert_eq!(manifest.ticket, tref(ticket));
    assert_eq!(manifest.base_commit, BASE_COMMIT);

    assert_eq!(w.active.as_ref(), Some(&slice));
    slice
}

// SLICE-2-1
#[test]
fn clean_open() {
    let fake = Fake::new();

    let outcome = open(&fake.deps(), &request(T)).expect("open");

    assert_opened(&fake, outcome, T, "development");
}

// SLICE-2-1
#[test]
fn clean_open_monday() {
    let fake = Fake::new();

    let outcome = open(&fake.deps(), &request(MONDAY)).expect("open");

    assert_opened(&fake, outcome, MONDAY, "In Progress");
}

// SLICE-2-2
#[test]
fn stray_timer_running() {
    for stray_ticket in [Some(U), None] {
        let fake = Fake::new();
        let stray = fake.with_running_timer(stray_ticket);

        let outcome = open(&fake.deps(), &request(T)).expect("open");

        assert_opened(&fake, outcome, T, "development");
        assert!(
            !fake.snapshot().entries[&stray.0].running,
            "stray timer for {stray_ticket:?} stopped"
        );
        let log = fake.writes();
        let stop = log
            .iter()
            .position(|w| *w == format!("stop {}", stray.0))
            .expect("stray stopped");
        let start = log.iter().position(|w| w.starts_with("start ")).expect("timer started");
        assert!(stop < start, "stray stopped before the new timer started: {log:?}");
    }
}

// SLICE-2-3
#[test]
fn another_slice_active() {
    let fake = Fake::new();
    let other = fake.with_active_slice(U, true);
    let before = fake.snapshot();

    let err = open(&fake.deps(), &request(T)).expect_err("open must fail");

    assert_eq!(err, OpenError::AnotherSliceActive(Box::new(other)));
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

    let outcome = open(&fake.deps(), &request(T)).expect("open succeeds");

    assert_eq!(outcome, OpenOutcome::AlreadyOpen(slice));
    assert_eq!(fake.snapshot(), before, "nothing changed");
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// SLICE-2-5
#[test]
fn unknown_ticket() {
    let fake = Fake::new();
    let before = fake.snapshot();

    let err = open(&fake.deps(), &request(UNKNOWN)).expect_err("open must fail");

    assert_eq!(err, OpenError::UnknownTicket(tref(UNKNOWN)));
    assert_ne!(err.exit_code(), 0);
    assert_eq!(fake.snapshot(), before, "nothing changed");
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// SLICE-2-6
#[test]
fn partial_failure_undoes_earlier_steps() {
    // Each failing write, the step it belongs to, and the writes before it
    // that must be undone. A stray timer is running so StopStrayTimer has
    // something to undo.
    let cases = [
        ("start", OpenStep::StartTimer),
        ("set_status", OpenStep::SetInProgress),
        ("create_worktree", OpenStep::CreateWorktree),
        ("write_manifest", OpenStep::WriteManifest),
        ("set_active", OpenStep::SetActive),
    ];
    for (fail, step) in cases {
        let mut fake = Fake::new();
        let stray = fake.with_running_timer(Some(U));
        fake.fail = Some(fail);
        let before = fake.snapshot();

        let err = open(&fake.deps(), &request(T)).expect_err("open must fail");

        let OpenError::StepFailed {
            step: failed,
            undo_failures,
            ..
        } = &err
        else {
            panic!("{fail}: want StepFailed, got {err:?}")
        };
        assert_eq!(*failed, step, "{fail}: failed step named");
        assert!(undo_failures.is_empty(), "{fail}: undo succeeded");
        assert_ne!(err.exit_code(), 0);
        assert_eq!(fake.snapshot(), before, "{fail}: world restored");
        assert!(
            fake.writes().contains(&format!("restart {}", stray.0)),
            "{fail}: stray timer restarted"
        );
    }
}

// SLICE-2-6
#[test]
fn partial_failure_undoes_in_reverse_order() {
    let mut fake = Fake::new();
    fake.with_running_timer(Some(U));
    fake.fail = Some("set_active");

    open(&fake.deps(), &request(T)).expect_err("open must fail");

    let ops: Vec<String> = fake
        .writes()
        .iter()
        .map(|w| w.split(' ').next().unwrap().to_string())
        .collect();
    assert_eq!(
        ops,
        [
            "stop",
            "start",
            "set_status",
            "create_worktree",
            "write_manifest",
            "remove_manifest",
            "remove_worktree",
            "set_status",
            "delete",
            "restart",
        ]
    );
}

// SLICE-2-7
#[test]
fn same_slice_timer_stopped() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, false);
    let before = fake.snapshot();

    let outcome = open(&fake.deps(), &request(T)).expect("open succeeds");

    let OpenOutcome::Resumed(resumed) = outcome else {
        panic!("want Resumed, got {outcome:?}")
    };
    let after = fake.snapshot();
    let running = after.running();
    assert_eq!(running.len(), 1, "exactly one running timer");
    assert_eq!(running[0].id, resumed.timer);
    assert_eq!(running[0].ticket, Some(tref(T)));
    assert!(running[0].notes.contains(&slice.id.0), "notes name the slice");

    assert_eq!(resumed.id, slice.id);
    assert_eq!(resumed.branch, slice.branch);
    assert_eq!(resumed.worktree, slice.worktree);
    assert_eq!(after.tickets, before.tickets, "ticket unchanged");
    assert_eq!(after.worktrees, before.worktrees, "branch and worktree unchanged");
    assert_eq!(after.manifests, before.manifests, "manifest unchanged");
}
