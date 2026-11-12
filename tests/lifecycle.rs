//! slice pause (SLICE-3), resume (SLICE-4), close (SLICE-5) and abandon (SLICE-6).

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

// ---------------------------------------------------------------------------
// abandon
// ---------------------------------------------------------------------------

/// Everything SLICE-6-1 requires of a completed abandon.
fn assert_abandoned(fake: &Fake, abandoned: &ActiveSlice, slice: &ActiveSlice, before: &World) {
    assert_eq!(abandoned, slice);
    let after = fake.snapshot();
    assert!(after.running().is_empty(), "timer not running");
    assert_eq!(
        after.entries.keys().collect::<Vec<_>>(),
        before.entries.keys().collect::<Vec<_>>(),
        "Harvest entries kept"
    );
    assert_eq!(after.tickets, before.tickets, "ticket unchanged");
    assert!(
        !fake.write_ops().contains(&"set_status".to_string()),
        "ticket status not written: {:?}",
        fake.writes()
    );
    assert_eq!(after.manifests[&slice.id].status, ManifestStatus::Abandoned);
    assert_eq!(after.active, None, "no slice active");
    assert_eq!(after.branches, before.branches, "branch kept");
    assert_eq!(after.worktrees, before.worktrees, "worktree kept");
}

// SLICE-6-1
#[test]
fn abandon_running_slice() {
    for ticket in [T, MONDAY] {
        let fake = Fake::new();
        let slice = fake.with_active_slice(ticket, true);
        let before = fake.snapshot();

        let abandoned = abandon(&fake.deps(), &AbandonRequest { reason: None }).expect("abandon");

        assert_abandoned(&fake, &abandoned.slice, &slice, &before);
    }
}

// SLICE-6-2
#[test]
fn abandon_paused_slice() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, false);
    let before = fake.snapshot();

    let abandoned = abandon(&fake.deps(), &AbandonRequest { reason: None }).expect("abandon");

    assert_abandoned(&fake, &abandoned.slice, &slice, &before);
    let ops = fake.write_ops();
    assert!(
        !ops.iter().any(|o| ["stop", "start", "restart"].contains(&o.as_str())),
        "no timer touched: {ops:?}"
    );
}

// SLICE-6-3
#[test]
fn nothing_to_abandon() {
    let fake = Fake::new();
    let before = fake.snapshot();

    let err = abandon(&fake.deps(), &AbandonRequest { reason: None }).expect_err("abandon must fail");

    assert_refused(&fake, &before, &err, &SliceError::NoActiveSlice);
}

// SLICE-6-4
#[test]
fn abandon_mismatch() {
    for stray in [Some(U), None] {
        let (fake, before) = mismatched(stray);

        let err = abandon(&fake.deps(), &AbandonRequest { reason: None }).expect_err("abandon must fail");

        assert!(matches!(&err, SliceError::Inconsistent(_)), "got {err:?}");
        assert_refused(&fake, &before, &err, &err.clone());
    }
}

// SLICE-6-5
#[test]
fn abandon_partial_failure() {
    let cases = [
        ("stop", Step::StopTimer),
        ("write_manifest", Step::AbandonManifest),
        ("clear_active", Step::ClearActive),
    ];
    for (fail, step) in cases {
        let mut fake = Fake::new();
        fake.with_active_slice(T, true);
        fake.fail = Some(fail);
        let before = fake.snapshot();

        let err = abandon(&fake.deps(), &AbandonRequest { reason: None }).expect_err("abandon must fail");

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
        assert_eq!(after.tickets, before.tickets, "{fail}: ticket unchanged");
        assert_eq!(after.manifests, before.manifests, "{fail}: manifest restored");
        assert_eq!(after.active, before.active, "{fail}: slice still active");
    }
}

// SLICE-6-5
#[test]
fn abandon_partial_failure_undoes_in_reverse_order() {
    let mut fake = Fake::new();
    let slice = fake.with_active_slice(T, true);
    let entry = today_entry(&fake, T);
    fake.fail = Some("clear_active");

    abandon(&fake.deps(), &AbandonRequest { reason: None }).expect_err("abandon must fail");

    assert_eq!(
        fake.writes(),
        [
            format!("stop {}", entry.0),
            format!("write_manifest {} Abandoned", slice.id.0),
            format!("write_manifest {} Open", slice.id.0),
            format!("restart {}", entry.0),
        ]
    );
}

// SLICE-6-6
#[test]
fn reopen_after_abandon() {
    let fake = Fake::new();
    let first = fake.with_active_slice(T, true);
    abandon(&fake.deps(), &AbandonRequest { reason: None }).expect("abandon");

    let request = OpenRequest {
        ticket: tref(T),
        repo: first.repos[0].repo.clone(),
        branch_type: None,
        base: None,
    };
    let OpenOutcome::Opened(second) = open(&fake.deps(), &request).expect("open") else {
        panic!("want Opened")
    };

    assert_eq!(second.id, SliceId(format!("{T}-2")), "the next slice id");
    assert_eq!(second.repos.len(), 1, "the repository it was opened in");
    assert_eq!(second.repos[0].branch, first.repos[0].branch, "the ticket's branch");
    assert_eq!(
        second.repos[0].worktree, first.repos[0].worktree,
        "the ticket's worktree"
    );
    let after = fake.snapshot();
    assert_eq!(after.active, Some(second.clone()));
    assert_eq!(
        after.manifests[&first.id].status,
        ManifestStatus::Abandoned,
        "old manifest kept"
    );
    assert_eq!(after.manifests[&second.id].status, ManifestStatus::Open);
}

// ---------------------------------------------------------------------------
// abandon --reason
// ---------------------------------------------------------------------------

// SLICE-7-1
#[test]
fn abandon_records_the_trimmed_reason() {
    let reason = abandon_reason("  fixed by ENG-14800  ").expect("a reason");
    assert_eq!(reason, "fixed by ENG-14800", "surrounding whitespace removed");

    for running in [true, false] {
        let fake = Fake::new();
        let slice = fake.with_active_slice(T, running);
        let before = fake.snapshot();
        let request = AbandonRequest {
            reason: Some(reason.clone()),
        };

        let abandoned = abandon(&fake.deps(), &request).expect("abandon");

        assert_abandoned(&fake, &abandoned.slice, &slice, &before);
        assert_eq!(abandoned.reason.as_deref(), Some("fixed by ENG-14800"));
        let manifest = &fake.snapshot().manifests[&slice.id];
        assert_eq!(manifest.abandon_reason.as_deref(), Some("fixed by ENG-14800"));
        assert_eq!(
            Manifest {
                abandon_reason: None,
                status: ManifestStatus::Open,
                ..manifest.clone()
            },
            before.manifests[&slice.id],
            "the manifest is otherwise as it was"
        );
        assert!(
            abandoned.message().contains("fixed by ENG-14800"),
            "printed line includes the reason: {}",
            abandoned.message()
        );
        let json = abandoned.to_json();
        assert_eq!(json["reason"], "fixed by ENG-14800", "json: {json}");
        assert_eq!(json["outcome"], "abandoned", "json: {json}");
        assert_eq!(json["slice"], slice_json(&slice), "json: {json}");
    }
}

// SLICE-7-2
#[test]
fn abandon_without_reason_records_none() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, true);
    let before = fake.snapshot();

    let abandoned = abandon(&fake.deps(), &AbandonRequest { reason: None }).expect("abandon");

    assert_abandoned(&fake, &abandoned.slice, &slice, &before);
    assert_eq!(abandoned.reason, None);
    assert_eq!(fake.snapshot().manifests[&slice.id].abandon_reason, None);
    let json = abandoned.to_json();
    let object = json.as_object().expect("a JSON object");
    assert_eq!(object.get("reason"), Some(&serde_json::Value::Null), "json: {json}");
    assert_eq!(json["outcome"], "abandoned", "json: {json}");
    assert_eq!(json["slice"], slice_json(&slice), "json: {json}");
}

// SLICE-7-3
#[test]
fn empty_reason_refused() {
    for raw in ["", " ", "   ", "\t", "\n", " \t\r\n "] {
        assert!(abandon_reason(raw).is_err(), "{raw:?} refused");
    }
}
