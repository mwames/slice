//! slice pause (SLICE-3), resume (SLICE-4) and close (SLICE-5).

mod common;

use common::*;
use slice::ports::*;

fn today_entry(fake: &Fake, ticket: &str) -> TimerId {
    fake.entry_today(&tref(ticket)).unwrap().expect("entry for today")
}

/// A refusal that must change nothing.
fn assert_refused(fake: &Fake, before: &World, err: &SliceError, want: &SliceError) {
    assert_eq!(err, want);
    assert_ne!(err.exit_code(), 0);
    assert_eq!(&fake.snapshot(), before, "nothing changed");
    assert_eq!(fake.writes(), Vec::<String>::new());
}

fn mismatched(fake_ticket: Option<&str>) -> (Fake, World) {
    let fake = Fake::new();
    fake.with_active_slice(T, false);
    fake.with_running_timer(fake_ticket);
    let before = fake.snapshot();
    (fake, before)
}

// ---------------------------------------------------------------------------
// pause
// ---------------------------------------------------------------------------

// SLICE-3-1
#[test]
fn pause_running_slice() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, true);
    let entry = today_entry(&fake, T);
    let before = fake.snapshot();

    let outcome = pause(&fake.deps()).expect("pause");

    assert_eq!(outcome, PauseOutcome::Paused(slice));
    let after = fake.snapshot();
    assert!(!after.entries[&entry.0].running, "timer stopped");
    assert_eq!(after.active, before.active, "slice still active");
    assert_eq!(after.tickets, before.tickets);
    assert_eq!(after.branches, before.branches);
    assert_eq!(after.worktrees, before.worktrees);
    assert_eq!(after.manifests, before.manifests);
    assert_eq!(fake.write_ops(), ["stop"]);
}

// SLICE-3-2
#[test]
fn pause_already_paused() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, false);
    let before = fake.snapshot();

    let outcome = pause(&fake.deps()).expect("pause succeeds");

    assert_eq!(outcome, PauseOutcome::AlreadyPaused(slice));
    assert_eq!(fake.snapshot(), before);
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// SLICE-3-3
#[test]
fn nothing_to_pause() {
    let fake = Fake::new();
    let before = fake.snapshot();

    let err = pause(&fake.deps()).expect_err("pause must fail");

    assert_refused(&fake, &before, &err, &SliceError::NoActiveSlice);
}

// SLICE-3-4
#[test]
fn pause_mismatch() {
    for stray in [Some(U), None] {
        let (fake, before) = mismatched(stray);

        let err = pause(&fake.deps()).expect_err("pause must fail");

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

// ---------------------------------------------------------------------------
// resume
// ---------------------------------------------------------------------------

/// SLICE-4-1's "the ticket, branch, worktree, and manifest are unchanged".
fn assert_only_timer_changed(fake: &Fake, before: &World) {
    let after = fake.snapshot();
    assert_eq!(after.active, before.active);
    assert_eq!(after.tickets, before.tickets);
    assert_eq!(after.branches, before.branches);
    assert_eq!(after.worktrees, before.worktrees);
    assert_eq!(after.manifests, before.manifests);
}

// SLICE-4-1
#[test]
fn resume_restarts_todays_entry() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, false);
    let entry = today_entry(&fake, T);
    let before = fake.snapshot();

    let outcome = resume(&fake.deps()).expect("resume");

    assert_eq!(outcome, ResumeOutcome::Resumed(slice));
    let after = fake.snapshot();
    assert_eq!(after.running().iter().map(|e| e.id).collect::<Vec<_>>(), [entry]);
    assert_eq!(after.entries.len(), before.entries.len(), "no new entry");
    assert_only_timer_changed(&fake, &before);
}

// SLICE-4-1
#[test]
fn resume_starts_an_entry_when_today_has_none() {
    let fake = Fake::new();
    fake.with_active_slice(T, false);
    fake.world
        .borrow_mut()
        .entries
        .values_mut()
        .for_each(|e| e.today = false);
    let before = fake.snapshot();

    resume(&fake.deps()).expect("resume");

    let after = fake.snapshot();
    let running = after.running();
    assert_eq!(running.len(), 1);
    assert!(running[0].today && running[0].ticket == Some(tref(T)));
    assert_eq!(running[0].notes, "ENG-1 - Login");
    assert_eq!(after.entries.len(), before.entries.len() + 1);
    assert_only_timer_changed(&fake, &before);
}

// SLICE-4-2
#[test]
fn resume_already_running() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, true);
    let before = fake.snapshot();

    let outcome = resume(&fake.deps()).expect("resume succeeds");

    assert_eq!(outcome, ResumeOutcome::AlreadyRunning(slice));
    assert_eq!(fake.snapshot(), before);
    assert_eq!(fake.writes(), Vec::<String>::new());
}

// SLICE-4-3
#[test]
fn nothing_to_resume() {
    let fake = Fake::new();
    let before = fake.snapshot();

    let err = resume(&fake.deps()).expect_err("resume must fail");

    assert_refused(&fake, &before, &err, &SliceError::NoActiveSlice);
}

// SLICE-4-4
#[test]
fn resume_mismatch() {
    for stray in [Some(U), None] {
        let (fake, before) = mismatched(stray);

        let err = resume(&fake.deps()).expect_err("resume must fail");

        assert!(matches!(&err, SliceError::Inconsistent(_)), "got {err:?}");
        assert_refused(&fake, &before, &err, &err.clone());
    }
}

// ---------------------------------------------------------------------------
// close
// ---------------------------------------------------------------------------

/// Everything SLICE-5-1 requires of a completed close.
fn assert_closed(fake: &Fake, closed: &ActiveSlice, slice: &ActiveSlice, before: &World, status: &str) {
    assert_eq!(closed, slice);
    let after = fake.snapshot();
    assert!(after.running().is_empty(), "timer not running");
    assert_eq!(after.tickets[&slice.ticket.0].status, status);
    assert_eq!(after.manifests[&slice.id].status, ManifestStatus::Closed);
    assert_eq!(after.active, None, "no slice active");
    assert_eq!(after.branches, before.branches, "branch kept");
    assert_eq!(after.worktrees, before.worktrees, "worktree kept");
}

// SLICE-5-1
#[test]
fn close_to_review() {
    for (ticket, status) in [(T, "dev complete"), (MONDAY, "Waiting for review")] {
        let fake = Fake::new();
        let slice = fake.with_active_slice(ticket, true);
        let before = fake.snapshot();

        let closed = close(&fake.deps(), &CloseRequest { done: false }).expect("close");

        assert_closed(&fake, &closed, &slice, &before, status);
    }
}

// SLICE-5-2
#[test]
fn close_to_done() {
    for (ticket, status) in [(T, "complete"), (MONDAY, "Done")] {
        let fake = Fake::new();
        let slice = fake.with_active_slice(ticket, true);
        let before = fake.snapshot();

        let closed = close(&fake.deps(), &CloseRequest { done: true }).expect("close");

        assert_closed(&fake, &closed, &slice, &before, status);
    }
}

// SLICE-5-3
#[test]
fn close_paused_slice() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, false);
    let before = fake.snapshot();

    let closed = close(&fake.deps(), &CloseRequest { done: false }).expect("close");

    assert_closed(&fake, &closed, &slice, &before, "dev complete");
    let ops = fake.write_ops();
    assert!(
        !ops.iter().any(|o| ["stop", "start", "restart"].contains(&o.as_str())),
        "no timer touched: {ops:?}"
    );
}

// SLICE-5-4
#[test]
fn nothing_to_close() {
    let fake = Fake::new();
    let before = fake.snapshot();

    let err = close(&fake.deps(), &CloseRequest { done: false }).expect_err("close must fail");

    assert_refused(&fake, &before, &err, &SliceError::NoActiveSlice);
}

// SLICE-5-5
#[test]
fn close_mismatch() {
    for stray in [Some(U), None] {
        let (fake, before) = mismatched(stray);

        let err = close(&fake.deps(), &CloseRequest { done: false }).expect_err("close must fail");

        assert!(matches!(&err, SliceError::Inconsistent(_)), "got {err:?}");
        assert_refused(&fake, &before, &err, &err.clone());
    }
}

// SLICE-5-6
#[test]
fn close_partial_failure() {
    let cases = [
        ("stop", Step::StopTimer),
        ("set_status", Step::SetClosedStatus),
        ("write_manifest", Step::CloseManifest),
        ("clear_active", Step::ClearActive),
    ];
    for (fail, step) in cases {
        let mut fake = Fake::new();
        fake.with_active_slice(T, true);
        fake.fail = Some(fail);
        let before = fake.snapshot();

        let err = close(&fake.deps(), &CloseRequest { done: false }).expect_err("close must fail");

        let SliceError::StepFailed {
            step: failed,
            undo_failures,
            ..
        } = &err
        else {
            panic!("{fail}: want StepFailed, got {err:?}")
        };
        assert_eq!(*failed, step, "{fail}: failed step named");
        assert!(undo_failures.is_empty());
        assert_ne!(err.exit_code(), 0);
        let after = fake.snapshot();
        assert_eq!(
            after.entry_states(),
            before.entry_states(),
            "{fail}: timer running again"
        );
        assert_eq!(after.tickets, before.tickets, "{fail}: status restored");
        assert_eq!(after.manifests, before.manifests, "{fail}: manifest restored");
        assert_eq!(after.active, before.active, "{fail}: slice still active");
    }
}

// SLICE-5-6
#[test]
fn close_partial_failure_undoes_in_reverse_order() {
    let mut fake = Fake::new();
    let slice = fake.with_active_slice(T, true);
    let entry = today_entry(&fake, T);
    fake.fail = Some("clear_active");

    close(&fake.deps(), &CloseRequest { done: false }).expect_err("close must fail");

    assert_eq!(
        fake.writes(),
        [
            format!("stop {}", entry.0),
            format!("set_status {T} dev complete"),
            format!("write_manifest {} Closed", slice.id.0),
            format!("write_manifest {} Open", slice.id.0),
            format!("set_status {T} to do"),
            format!("restart {}", entry.0),
        ]
    );
}
