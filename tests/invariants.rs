//! Property tests for the SLICE invariants over generated starting states,
//! requested tickets, injected write failures and unreachable sources.
//! Marked `// invariant: <ID>` rather than with a Spool annotation until
//! Spool has an ID format for invariants.

mod common;

use std::path::PathBuf;

use common::*;
use proptest::prelude::*;
use slice::ports::*;

#[derive(Clone, Debug)]
struct Start {
    /// Ticket of the active slice and whether its timer is running.
    active: Option<(&'static str, bool)>,
    /// A running timer that is not the slice's: None, or Some(ticket or no ticket).
    /// Only generated when the slice's own timer is not running, so at most
    /// one timer runs at the start, as in Harvest.
    stray: Option<Option<&'static str>>,
    request: &'static str,
    fail: Option<&'static str>,
    unreachable: Option<Source>,
}

impl Start {
    fn build(&self) -> Fake {
        let mut fake = Fake::new();
        if let Some((ticket, running)) = self.active {
            fake.with_active_slice(ticket, running);
        }
        if let Some(stray) = self.stray {
            fake.with_running_timer(stray);
        }
        fake.fail = self.fail;
        fake.unreachable = self.unreachable.into_iter().collect();
        fake
    }

    fn request(&self) -> OpenRequest {
        OpenRequest {
            ticket: tref(self.request),
            repo: PathBuf::from(REPO),
            base: "HEAD".to_string(),
        }
    }
}

const FORWARD_WRITES: [&str; 6] = [
    "stop",
    "start",
    "set_status",
    "create_worktree",
    "write_manifest",
    "set_active",
];

fn start() -> impl Strategy<Value = Start> {
    let active = prop_oneof![
        Just(None),
        (prop::sample::select(vec![T, U]), any::<bool>()).prop_map(Some)
    ];
    let stray = prop_oneof![
        Just(None),
        prop::sample::select(vec![Some(T), Some(U), None]).prop_map(Some)
    ];
    let request = prop::sample::select(vec![T, U, MONDAY, UNKNOWN]);
    let fail = prop_oneof![Just(None), prop::sample::select(FORWARD_WRITES.to_vec()).prop_map(Some)];
    let unreachable = prop_oneof![
        3 => Just(None),
        1 => prop::sample::select(vec![
            Source::Harvest,
            Source::TicketSystem(TicketSystemKind::ClickUp),
            Source::TicketSystem(TicketSystemKind::Monday),
            Source::LocalState,
            Source::Git,
        ])
        .prop_map(Some),
    ];
    (active, stray, request, fail, unreachable).prop_map(|(active, stray, request, fail, unreachable)| {
        let slice_timer_running = matches!(active, Some((_, true)));
        Start {
            active,
            stray: if slice_timer_running { None } else { stray },
            request,
            fail,
            unreachable,
        }
    })
}

proptest! {
    // invariant: SLICE-1-I1
    #[test]
    fn status_is_read_only(s in start()) {
        let fake = s.build();
        let before = fake.snapshot();

        let _ = status(&fake.deps());

        prop_assert_eq!(fake.writes(), Vec::<String>::new());
        prop_assert_eq!(fake.snapshot(), before);
    }

    // invariant: SLICE-2-I1
    #[test]
    fn at_most_one_running_timer(s in start()) {
        let fake = s.build();

        let _ = open(&fake.deps(), &s.request());

        let w = fake.snapshot();
        prop_assert!(w.running().len() <= 1, "running: {:?}", w.running());
    }

    // invariant: SLICE-2-I2
    #[test]
    fn failed_open_leaves_no_trace(s in start()) {
        let fake = s.build();
        let before = fake.snapshot();

        if open(&fake.deps(), &s.request()).is_err() {
            let after = fake.snapshot();
            let before_ids: Vec<_> = before.entries.keys().collect();
            let after_ids: Vec<_> = after.entries.keys().collect();
            prop_assert_eq!(after_ids, before_ids, "no time entry it started still exists");
            for (id, entry) in &before.entries {
                prop_assert_eq!(after.entries[id].running, entry.running, "timer {} running state restored", id);
            }
            prop_assert_eq!(&after.manifests, &before.manifests, "no manifest for the slice");
            for (r, ticket) in &before.tickets {
                prop_assert_eq!(&after.tickets[r].status, &ticket.status, "status of {} unchanged", r);
            }
        }
    }
}
