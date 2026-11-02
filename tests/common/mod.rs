//! In-memory fakes of the four ports. One `Fake` holds the whole world and
//! implements every trait, so a test can inspect timers, tickets, worktrees
//! and state after a run. Every write is appended to `log`.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use slice::ports::*;

pub const T: &str = "ENG-1";
pub const U: &str = "ENG-2";
pub const MONDAY: &str = "TBOARD-3";
pub const UNKNOWN: &str = "ENG-404";
pub const BASE_COMMIT: &str = "abc1234";
pub const REPO: &str = "/work/app";

pub fn tref(s: &str) -> TicketRef {
    TicketRef(s.to_string())
}

pub fn settings() -> Settings {
    Settings {
        clickup_in_progress: "development".to_string(),
        monday_in_progress: "In Progress".to_string(),
        branch_prefix: "slice/".to_string(),
        worktree_root: None,
    }
}

pub fn system_of(r: &TicketRef) -> TicketSystemKind {
    if r.0.starts_with("TBOARD-") {
        TicketSystemKind::Monday
    } else {
        TicketSystemKind::ClickUp
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: TimerId,
    pub ticket: Option<TicketRef>,
    pub notes: String,
    pub running: bool,
}

/// Everything the ports can observe or change.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct World {
    pub entries: BTreeMap<i64, Entry>,
    pub tickets: BTreeMap<String, Ticket>,
    /// (branch, worktree path)
    pub worktrees: BTreeSet<(String, PathBuf)>,
    pub active: Option<ActiveSlice>,
    pub manifests: BTreeMap<PathBuf, Manifest>,
    pub slice_counts: BTreeMap<String, u32>,
}

impl World {
    pub fn running(&self) -> Vec<&Entry> {
        self.entries.values().filter(|e| e.running).collect()
    }
}

pub struct Fake {
    pub world: RefCell<World>,
    pub log: RefCell<Vec<String>>,
    /// A write operation that fails, by name: "start", "stop", "restart",
    /// "delete", "set_status", "create_worktree", "remove_worktree",
    /// "write_manifest", "remove_manifest", "set_active", "clear_active".
    pub fail: Option<&'static str>,
    /// Sources whose reads fail.
    pub unreachable: Vec<Source>,
    pub settings: Settings,
}

impl Fake {
    pub fn new() -> Fake {
        let mut world = World::default();
        for (r, title, status) in [
            (T, "Login", "to do"),
            (U, "Logout", "to do"),
            (MONDAY, "Board", "Working on it"),
        ] {
            world.tickets.insert(
                r.to_string(),
                Ticket {
                    reference: tref(r),
                    system: system_of(&tref(r)),
                    title: title.to_string(),
                    status: status.to_string(),
                },
            );
        }
        Fake {
            world: RefCell::new(world),
            log: RefCell::new(Vec::new()),
            fail: None,
            unreachable: Vec::new(),
            settings: settings(),
        }
    }

    pub fn deps(&self) -> Deps<'_> {
        Deps {
            timer: self,
            tickets: self,
            vcs: self,
            state: self,
            settings: &self.settings,
        }
    }

    /// Add a running timer, optionally for a ticket, and return its id.
    pub fn with_running_timer(&self, ticket: Option<&str>) -> TimerId {
        self.add_entry(ticket, true)
    }

    pub fn add_entry(&self, ticket: Option<&str>, running: bool) -> TimerId {
        let mut w = self.world.borrow_mut();
        let id = TimerId(w.entries.keys().max().copied().unwrap_or(100) + 1);
        let notes = match ticket {
            Some(t) => format!("{t} - work"),
            None => "admin".to_string(),
        };
        w.entries.insert(
            id.0,
            Entry {
                id,
                ticket: ticket.map(tref),
                notes,
                running,
            },
        );
        id
    }

    /// Record an active slice for `ticket` as a previous open would have,
    /// with its time entry running or stopped.
    pub fn with_active_slice(&self, ticket: &str, timer_running: bool) -> ActiveSlice {
        let timer = self.add_entry(Some(ticket), timer_running);
        let id = SliceId(format!("{ticket}-1"));
        self.world.borrow_mut().entries.get_mut(&timer.0).unwrap().notes = format!("{ticket} - work [slice:{}]", id.0);
        let worktree = PathBuf::from(format!("/work/app.slices/{}", id.0));
        let slice = ActiveSlice {
            id: id.clone(),
            ticket: tref(ticket),
            repo: PathBuf::from(REPO),
            branch: format!("slice/{}", id.0),
            worktree: worktree.clone(),
            manifest: worktree.join(format!(".slices/{}.yaml", id.0)),
            timer,
        };
        let mut w = self.world.borrow_mut();
        w.worktrees.insert((slice.branch.clone(), slice.worktree.clone()));
        w.manifests.insert(
            slice.manifest.clone(),
            Manifest {
                id,
                ticket: tref(ticket),
                ticket_system: system_of(&tref(ticket)),
                base_commit: BASE_COMMIT.to_string(),
                status: ManifestStatus::Open,
                acceptance_criteria: vec![],
                invariants: vec![],
                spec_hash: None,
                test_hash: None,
                budgets: vec![],
            },
        );
        w.slice_counts.insert(ticket.to_string(), 1);
        w.active = Some(slice.clone());
        slice
    }

    pub fn snapshot(&self) -> World {
        self.world.borrow().clone()
    }

    pub fn writes(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    fn write(&self, op: &'static str, detail: String, source: Source) -> PortResult<()> {
        if self.fail == Some(op) {
            return Err(PortError {
                source,
                message: format!("{op} failed (injected)"),
            });
        }
        self.log.borrow_mut().push(format!("{op} {detail}"));
        Ok(())
    }

    fn read(&self, source: Source) -> PortResult<()> {
        if self.unreachable.contains(&source) {
            return Err(PortError {
                source,
                message: "unreachable (injected)".to_string(),
            });
        }
        Ok(())
    }
}

impl TimeTracker for Fake {
    fn running(&self) -> PortResult<Option<Timer>> {
        self.read(Source::Harvest)?;
        let w = self.world.borrow();
        Ok(w.running().first().map(|e| Timer {
            id: e.id,
            ticket: e.ticket.clone(),
            notes: e.notes.clone(),
        }))
    }

    fn start(&self, ticket: &Ticket, notes: &str) -> PortResult<Timer> {
        self.write("start", ticket.reference.0.clone(), Source::Harvest)?;
        let mut w = self.world.borrow_mut();
        let id = TimerId(w.entries.keys().max().copied().unwrap_or(100) + 1);
        let entry = Entry {
            id,
            ticket: Some(ticket.reference.clone()),
            notes: notes.to_string(),
            running: true,
        };
        w.entries.insert(id.0, entry);
        Ok(Timer {
            id,
            ticket: Some(ticket.reference.clone()),
            notes: notes.to_string(),
        })
    }

    fn stop(&self, id: TimerId) -> PortResult<()> {
        self.write("stop", id.0.to_string(), Source::Harvest)?;
        self.world
            .borrow_mut()
            .entries
            .get_mut(&id.0)
            .expect("stop: no such entry")
            .running = false;
        Ok(())
    }

    fn restart(&self, id: TimerId) -> PortResult<TimerId> {
        self.write("restart", id.0.to_string(), Source::Harvest)?;
        self.world
            .borrow_mut()
            .entries
            .get_mut(&id.0)
            .expect("restart: no such entry")
            .running = true;
        Ok(id)
    }

    fn delete(&self, id: TimerId) -> PortResult<()> {
        self.write("delete", id.0.to_string(), Source::Harvest)?;
        self.world
            .borrow_mut()
            .entries
            .remove(&id.0)
            .expect("delete: no such entry");
        Ok(())
    }
}

impl TicketSystem for Fake {
    fn fetch(&self, reference: &TicketRef) -> PortResult<Option<Ticket>> {
        self.read(Source::TicketSystem(system_of(reference)))?;
        Ok(self.world.borrow().tickets.get(&reference.0).cloned())
    }

    fn set_status(&self, reference: &TicketRef, status: &str) -> PortResult<()> {
        self.write(
            "set_status",
            format!("{} {status}", reference.0),
            Source::TicketSystem(system_of(reference)),
        )?;
        self.world
            .borrow_mut()
            .tickets
            .get_mut(&reference.0)
            .expect("set_status: no such ticket")
            .status = status.to_string();
        Ok(())
    }
}

impl Vcs for Fake {
    fn resolve(&self, _repo: &Path, _rev: &str) -> PortResult<String> {
        self.read(Source::Git)?;
        Ok(BASE_COMMIT.to_string())
    }

    fn create_worktree(&self, _repo: &Path, branch: &str, path: &Path, base: &str) -> PortResult<()> {
        self.write(
            "create_worktree",
            format!("{branch} {} {base}", path.display()),
            Source::Git,
        )?;
        self.world
            .borrow_mut()
            .worktrees
            .insert((branch.to_string(), path.to_path_buf()));
        Ok(())
    }

    fn remove_worktree(&self, _repo: &Path, branch: &str, path: &Path) -> PortResult<()> {
        self.write("remove_worktree", format!("{branch} {}", path.display()), Source::Git)?;
        self.world
            .borrow_mut()
            .worktrees
            .remove(&(branch.to_string(), path.to_path_buf()));
        Ok(())
    }
}

impl StateStore for Fake {
    fn active(&self) -> PortResult<Option<ActiveSlice>> {
        self.read(Source::LocalState)?;
        Ok(self.world.borrow().active.clone())
    }

    fn next_slice_id(&self, ticket: &TicketRef) -> PortResult<SliceId> {
        self.read(Source::LocalState)?;
        let n = self.world.borrow().slice_counts.get(&ticket.0).copied().unwrap_or(0) + 1;
        Ok(SliceId(format!("{}-{n}", ticket.0)))
    }

    fn set_active(&self, slice: &ActiveSlice) -> PortResult<()> {
        self.write("set_active", slice.id.0.clone(), Source::LocalState)?;
        let mut w = self.world.borrow_mut();
        *w.slice_counts.entry(slice.ticket.0.clone()).or_insert(0) += 1;
        w.active = Some(slice.clone());
        Ok(())
    }

    fn clear_active(&self) -> PortResult<()> {
        self.write("clear_active", String::new(), Source::LocalState)?;
        self.world.borrow_mut().active = None;
        Ok(())
    }

    fn write_manifest(&self, path: &Path, manifest: &Manifest) -> PortResult<()> {
        self.write("write_manifest", path.display().to_string(), Source::LocalState)?;
        self.world
            .borrow_mut()
            .manifests
            .insert(path.to_path_buf(), manifest.clone());
        Ok(())
    }

    fn remove_manifest(&self, path: &Path) -> PortResult<()> {
        self.write("remove_manifest", path.display().to_string(), Source::LocalState)?;
        self.world.borrow_mut().manifests.remove(path);
        Ok(())
    }
}
