//! Property tests for the SLICE invariants over generated starting states,
//! requests, injected write failures and unreachable sources.

mod common;

use std::path::PathBuf;

use common::*;
use proptest::prelude::*;
use slice::ports::*;

#[derive(Clone, Debug)]
enum Branches {
    None,
    WithWorktree,
    WithoutWorktree,
    Two,
}

#[derive(Clone, Debug)]
struct Start {
    /// Ticket of the active slice and whether its timer is running.
    active: Option<(&'static str, bool)>,
    /// A running timer that is not the slice's, for a ticket or none. Only
    /// when the slice's own timer is not running: Harvest runs one at a time.
    stray: Option<Option<&'static str>>,
    /// A stopped entry dated today for the requested ticket.
    entry_today: bool,
    /// Existing branches for the requested ticket.
    branches: Branches,
    request: &'static str,
    branch_type: Option<&'static str>,
    base: Option<&'static str>,
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
        if self.entry_today {
            fake.add_entry(Some(self.request), false, true);
        }
        let title = fake
            .snapshot()
            .tickets
            .get(self.request)
            .map(|t| t.title.to_lowercase())
            .unwrap_or_default();
        let branch = format!("feature/20261101-{}-{title}", self.request);
        match self.branches {
            Branches::None => {}
            Branches::WithWorktree => fake.with_branch(&branch, T_BRANCH_OLD_HEAD, true),
            Branches::WithoutWorktree => fake.with_branch(&branch, T_BRANCH_OLD_HEAD, false),
            Branches::Two => {
                fake.with_branch(&branch, T_BRANCH_OLD_HEAD, true);
                fake.with_branch(&format!("bug/20261020-{}-crash", self.request), "0bug000", false);
            }
        }
        fake.fail = self.fail;
        fake.unreachable = self.unreachable.into_iter().collect();
        fake
    }

    fn request(&self) -> OpenRequest {
        OpenRequest {
            ticket: tref(self.request),
            repo: PathBuf::from(REPO),
            branch_type: self.branch_type.map(|t| BranchType(t.into())),
            base: self.base.map(String::from),
        }
    }
}

const WRITES: [&str; 9] = [
    "stop",
    "start",
    "restart",
    "set_status",
    "create_branch",
    "add_worktree",
    "write_manifest",
    "set_active",
    "clear_active",
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
    let branches = prop_oneof![
        Just(Branches::None),
        Just(Branches::WithWorktree),
        Just(Branches::WithoutWorktree),
        Just(Branches::Two),
    ];
    let request = prop::sample::select(vec![T, U, MONDAY, UNKNOWN]);
    let branch_type = prop::sample::select(vec![None, Some("feature"), Some("bug")]);
    let base = prop::sample::select(vec![None, Some("main")]);
    let fail = prop_oneof![Just(None), prop::sample::select(WRITES.to_vec()).prop_map(Some)];
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
    (
        active,
        stray,
        any::<bool>(),
        branches,
        request,
        branch_type,
        base,
        fail,
        unreachable,
    )
        .prop_map(
            |(active, stray, entry_today, branches, request, branch_type, base, fail, unreachable)| {
                let slice_timer_running = matches!(active, Some((_, true)));
                Start {
                    active,
                    stray: if slice_timer_running { None } else { stray },
                    entry_today,
                    branches,
                    request,
                    branch_type,
                    base,
                    fail,
                    unreachable,
                }
            },
        )
}

proptest! {
    // SLICE-1-INV-1
    #[test]
    fn status_is_read_only(s in start()) {
        let fake = s.build();
        let before = fake.snapshot();

        let _ = status(&fake.deps());

        prop_assert_eq!(fake.writes(), Vec::<String>::new());
        prop_assert_eq!(fake.snapshot(), before);
    }

    // SLICE-2-INV-1
    #[test]
    fn open_leaves_at_most_one_running_timer(s in start()) {
        let fake = s.build();

        let _ = open(&fake.deps(), &s.request());

        let w = fake.snapshot();
        prop_assert!(w.running().len() <= 1, "running: {:?}", w.running());
    }

    // SLICE-2-INV-2
    #[test]
    fn failed_open_leaves_no_trace(s in start()) {
        let fake = s.build();
        let before = fake.snapshot();

        if open(&fake.deps(), &s.request()).is_err() {
            let after = fake.snapshot();
            prop_assert_eq!(after.entry_states(), before.entry_states(), "entries and running states");
            prop_assert_eq!(&after.branches, &before.branches, "branches");
            prop_assert_eq!(&after.worktrees, &before.worktrees, "worktrees");
            prop_assert_eq!(&after.manifests, &before.manifests, "manifests");
            prop_assert_eq!(&after.tickets, &before.tickets, "ticket statuses");
        }
    }

    // SLICE-3-INV-1
    #[test]
    fn pause_never_starts_or_deletes(s in start()) {
        let fake = s.build();
        let before = fake.snapshot();

        let _ = pause(&fake.deps());

        let after = fake.snapshot();
        prop_assert_eq!(after.entries.keys().collect::<Vec<_>>(), before.entries.keys().collect::<Vec<_>>());
        for (id, e) in &after.entries {
            prop_assert!(!e.running || before.entries[id].running, "entry {} started", id);
        }
        prop_assert_eq!(&after.tickets, &before.tickets);
        prop_assert_eq!(&after.active, &before.active);
        prop_assert_eq!(&after.manifests, &before.manifests);
    }

    // SLICE-4-INV-1
    #[test]
    fn resume_keeps_one_timer(s in start()) {
        let fake = s.build();
        let before = fake.snapshot();

        let _ = resume(&fake.deps());

        let after = fake.snapshot();
        prop_assert!(after.running().len() <= 1, "running: {:?}", after.running());
        for id in before.entries.keys() {
            prop_assert!(after.entries.contains_key(id), "entry {} deleted", id);
        }
    }

    // SLICE-5-INV-1
    #[test]
    fn failed_close_leaves_no_trace(s in start(), done in any::<bool>()) {
        let fake = s.build();
        let before = fake.snapshot();

        if close(&fake.deps(), &CloseRequest { done }).is_err() {
            let after = fake.snapshot();
            prop_assert_eq!(&after.active, &before.active, "slice still active");
            prop_assert_eq!(after.entry_states(), before.entry_states(), "timer running state");
            prop_assert_eq!(&after.tickets, &before.tickets, "ticket status");
            prop_assert_eq!(&after.manifests, &before.manifests, "manifest");
        }
    }

    // SLICE-6-INV-1
    #[test]
    fn abandon_never_touches_the_ticket(s in start()) {
        let fake = s.build();
        let before = fake.snapshot();

        let result = abandon(&fake.deps(), &AbandonRequest { reason: None });

        let after = fake.snapshot();
        prop_assert!(!fake.write_ops().contains(&"set_status".to_string()), "ticket status written");
        prop_assert_eq!(&after.tickets, &before.tickets, "ticket status");
        if result.is_err() {
            prop_assert_eq!(&after.active, &before.active, "slice still active");
            prop_assert_eq!(after.entry_states(), before.entry_states(), "timer running state");
            prop_assert_eq!(&after.manifests, &before.manifests, "manifest");
        }
    }

    // SLICE-7-INV-1
    #[test]
    fn abandon_reason_stays_local(
        s in start(),
        reason in prop::sample::select(vec!["fixed by ENG-14800", "product retired", "duplicate of ENG-2: see notes"]),
    ) {
        let fake = s.build();
        let before = fake.snapshot();
        let request = AbandonRequest { reason: Some(reason.to_string()) };

        let _ = abandon(&fake.deps(), &request);

        let after = fake.snapshot();
        prop_assert_eq!(&after.tickets, &before.tickets, "tickets unchanged");
        for (id, e) in &after.entries {
            prop_assert!(!e.notes.contains(reason), "entry {} notes: {:?}", id, e.notes);
            if let Some(old) = before.entries.get(id) {
                prop_assert_eq!(&e.notes, &old.notes, "entry {} notes changed", id);
            }
        }
        for write in fake.writes() {
            prop_assert!(!write.contains(reason), "reason written: {}", write);
        }
        let slice = before.active.as_ref().map(|a| a.id.clone());
        for (id, m) in &after.manifests {
            if Some(id) != slice.as_ref() {
                prop_assert_eq!(Some(m), before.manifests.get(id), "manifest {} changed", &id.0);
            }
        }
    }
}
