mod common;

use common::*;
use slice::ports::*;

// SLICE-1-1
#[test]
fn nothing_active() {
    let fake = Fake::new();

    let report = status(&fake.deps()).expect("status");

    assert_eq!(report.active, None);
    assert_eq!(report.timer, None);
    assert_eq!(report.mismatch, None);
    assert_eq!(report.exit_code(), 0);
}

// SLICE-1-2
#[test]
fn active_slice() {
    let fake = Fake::new();
    let slice = fake.with_active_slice(T, true);

    let report = status(&fake.deps()).expect("status");

    assert_eq!(report.active.as_ref(), Some(&slice));
    let timer = report.timer.as_ref().expect("running timer reported");
    assert_eq!(timer.id, slice.timer);
    assert_eq!(timer.ticket, Some(tref(T)));
    let ticket = report.ticket.as_ref().expect("ticket reported");
    assert_eq!(ticket.reference, tref(T));
    assert_eq!(ticket.status, "to do");
    assert_eq!(report.mismatch, None);
    assert_eq!(report.exit_code(), 0);
}

// SLICE-1-3
#[test]
fn timer_for_another_ticket() {
    let fake = Fake::new();
    fake.with_active_slice(T, false);
    fake.with_running_timer(Some(U));

    let report = status(&fake.deps()).expect("status");

    assert_eq!(
        report.mismatch,
        Some(Mismatch::DifferentTicket {
            slice: tref(T),
            timer: Some(tref(U))
        })
    );
    assert_ne!(report.exit_code(), 0);
}

// SLICE-1-3
#[test]
fn timer_without_slice() {
    for ticket in [Some(U), None] {
        let fake = Fake::new();
        fake.with_running_timer(ticket);

        let report = status(&fake.deps()).expect("status");

        assert_eq!(
            report.mismatch,
            Some(Mismatch::TimerWithoutSlice {
                timer: ticket.map(tref)
            }),
            "timer for {ticket:?}"
        );
        assert_ne!(report.exit_code(), 0);
    }
}

// SLICE-1-4
#[test]
fn source_unavailable() {
    let cases = [
        (Source::Harvest, T),
        (Source::TicketSystem(TicketSystemKind::ClickUp), T),
        (Source::TicketSystem(TicketSystemKind::Monday), MONDAY),
        (Source::LocalState, T),
    ];
    for (source, ticket) in cases {
        let mut fake = Fake::new();
        fake.with_active_slice(ticket, true);
        fake.unreachable = vec![source];

        let err = status(&fake.deps()).expect_err("status must fail, not report an empty source");

        assert_eq!(err.source, source);
    }
}
