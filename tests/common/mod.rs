//! In-memory fakes of the four ports. One `Fake` holds the whole world and
//! implements every trait, so a test can inspect timers, tickets, branches,
//! worktrees and state after a run. Every write is appended to `log`.

#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use slice::ports::*;

pub mod agent;

pub const T: &str = "ENG-1";
pub const U: &str = "ENG-2";
pub const MONDAY: &str = "TBOARD-3";
pub const UNKNOWN: &str = "ENG-404";
pub const BASE_COMMIT: &str = "abc1234";
pub const REPO: &str = "/work/app";
/// Repositories other than `REPO`, for slices that span several (SLICE-10),
/// and the commit each one's HEAD is at.
pub const API: &str = "/work/api";
pub const API_HEAD: &str = "0api000";
pub const WEB: &str = "/work/web";
pub const WEB_HEAD: &str = "0web000";
pub const TODAY: &str = "20261102";

/// T's branch as a new feature opened today, and as an earlier slice left it.
pub const T_BRANCH_NEW: &str = "feature/20261102-ENG-1-login";
pub const T_BRANCH_OLD: &str = "feature/20261101-ENG-1-login";
pub const T_BRANCH_OLD_HEAD: &str = "def5678";

pub fn tref(s: &str) -> TicketRef {
    TicketRef(s.to_string())
}

pub fn settings() -> Settings {
    Settings {
        in_progress: Statuses {
            clickup: "development".into(),
            monday: "In Progress".into(),
        },
        review: Statuses {
            clickup: "dev complete".into(),
            monday: "Waiting for review".into(),
        },
        done: Statuses {
            clickup: "complete".into(),
            monday: "Done".into(),
        },
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

/// Where the default worktree for a branch lives: `<repo>/../app.slices/<dir>`.
pub fn worktree_for(branch: &str) -> PathBuf {
    worktree_in(REPO, branch)
}

/// Where the default worktree for a branch lives in any repository:
/// `<repo>/../<repo name>.slices/<dir>`.
pub fn worktree_in(repo: &str, branch: &str) -> PathBuf {
    let repo = Path::new(repo);
    let name = repo.file_name().unwrap().to_string_lossy();
    repo.parent()
        .unwrap()
        .join(format!("{name}.slices"))
        .join(branch.replace('/', "-"))
}

/// A repository other than `REPO`: its HEAD, branches and worktrees.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Repo {
    pub head: String,
    /// Branch name to head commit.
    pub branches: BTreeMap<String, String>,
    /// Branch name to worktree path.
    pub worktrees: BTreeMap<String, PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: TimerId,
    pub ticket: Option<TicketRef>,
    pub notes: String,
    pub running: bool,
    /// Dated today, as opposed to an earlier day.
    pub today: bool,
    /// Order of last update; higher is more recent.
    pub updated: u64,
}

/// Everything the ports can observe or change.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct World {
    pub entries: BTreeMap<i64, Entry>,
    pub tickets: BTreeMap<String, Ticket>,
    /// `REPO`'s branches: branch name to head commit. Its HEAD is at
    /// `BASE_COMMIT`.
    pub branches: BTreeMap<String, String>,
    /// `REPO`'s worktrees: branch name to worktree path.
    pub worktrees: BTreeMap<String, PathBuf>,
    /// Every other repository, by path. Git fails on a path that is neither
    /// `REPO` nor one of these.
    pub repos: BTreeMap<PathBuf, Repo>,
    pub active: Option<ActiveSlice>,
    pub manifests: BTreeMap<SliceId, Manifest>,
    pub slice_counts: BTreeMap<String, u32>,
    pub clock: u64,
}

impl World {
    pub fn running(&self) -> Vec<&Entry> {
        self.entries.values().filter(|e| e.running).collect()
    }

    /// Entries as (id, running), ignoring update order.
    pub fn entry_states(&self) -> BTreeMap<i64, bool> {
        self.entries.iter().map(|(id, e)| (*id, e.running)).collect()
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
}

pub struct Fake {
    pub world: RefCell<World>,
    pub log: RefCell<Vec<String>>,
    /// A write operation that fails, by name: "start", "stop", "restart",
    /// "delete", "set_status", "create_branch", "add_worktree",
    /// "remove_worktree", "remove_branch", "write_manifest",
    /// "remove_manifest", "set_active", "clear_active". Only the first call
    /// fails, so an undo that uses the same operation still succeeds.
    pub fail: Option<&'static str>,
    failed: Cell<bool>,
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
        world.branches.insert("main".into(), BASE_COMMIT.into());
        Fake {
            world: RefCell::new(world),
            log: RefCell::new(Vec::new()),
            fail: None,
            failed: Cell::new(false),
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
            today: TODAY,
        }
    }

    /// Add an entry and return its id.
    pub fn add_entry(&self, ticket: Option<&str>, running: bool, today: bool) -> TimerId {
        let mut w = self.world.borrow_mut();
        let id = TimerId(w.entries.keys().max().copied().unwrap_or(100) + 1);
        let notes = match ticket {
            Some(t) => format!(
                "{t} - {}",
                w.tickets.get(t).map(|k| k.title.clone()).unwrap_or_default()
            ),
            None => "admin".to_string(),
        };
        let updated = w.tick();
        w.entries.insert(
            id.0,
            Entry {
                id,
                ticket: ticket.map(tref),
                notes,
                running,
                today,
                updated,
            },
        );
        id
    }

    /// A running timer dated today.
    pub fn with_running_timer(&self, ticket: Option<&str>) -> TimerId {
        self.add_entry(ticket, true, true)
    }

    /// A branch with the given head, optionally checked out in its worktree.
    pub fn with_branch(&self, name: &str, head: &str, with_worktree: bool) {
        let mut w = self.world.borrow_mut();
        w.branches.insert(name.to_string(), head.to_string());
        if with_worktree {
            w.worktrees.insert(name.to_string(), worktree_for(name));
        }
    }

    /// A repository other than `REPO`, with HEAD and `main` at `head` and no
    /// other branch.
    pub fn with_repo(&self, repo: &str, head: &str) {
        let mut r = Repo {
            head: head.to_string(),
            ..Repo::default()
        };
        r.branches.insert("main".into(), head.to_string());
        self.world.borrow_mut().repos.insert(PathBuf::from(repo), r);
    }

    /// A branch in `repo`, which is `REPO` or one made by `with_repo`,
    /// optionally checked out in its worktree.
    pub fn with_branch_in(&self, repo: &str, name: &str, head: &str, with_worktree: bool) {
        if repo == REPO {
            return self.with_branch(name, head, with_worktree);
        }
        let mut w = self.world.borrow_mut();
        let r = w.repos.get_mut(Path::new(repo)).expect("with_branch_in: no such repo");
        r.branches.insert(name.to_string(), head.to_string());
        if with_worktree {
            r.worktrees.insert(name.to_string(), worktree_in(repo, name));
        }
    }

    /// Add `repo`, made by `with_repo`, to the active slice as an earlier add
    /// would have left it: on the slice's branch, checked out in its
    /// worktree, based on the repository's HEAD.
    pub fn with_slice_repo(&self, repo: &str) -> SliceRepo {
        let (branch, head) = {
            let w = self.world.borrow();
            let slice = w.active.as_ref().expect("with_slice_repo: no active slice");
            (slice.repos[0].branch.clone(), w.repos[Path::new(repo)].head.clone())
        };
        self.with_branch_in(repo, &branch, &head, true);
        let added = SliceRepo {
            repo: PathBuf::from(repo),
            branch: branch.clone(),
            worktree: worktree_in(repo, &branch),
            base_commit: head,
        };
        let mut w = self.world.borrow_mut();
        let slice = w.active.as_mut().unwrap();
        slice.repos.push(added.clone());
        let id = slice.id.clone();
        w.manifests.get_mut(&id).unwrap().repos.push(added.clone());
        added
    }

    /// Record an active slice for `ticket` as an earlier open would have left
    /// it, on its branch from yesterday, with an entry for the ticket today
    /// that is running or stopped.
    pub fn with_active_slice(&self, ticket: &str, timer_running: bool) -> ActiveSlice {
        self.add_entry(Some(ticket), timer_running, true);
        let title = self.world.borrow().tickets[ticket].title.to_lowercase();
        let branch = format!("feature/20261101-{ticket}-{title}");
        self.with_branch(&branch, T_BRANCH_OLD_HEAD, true);
        let id = SliceId(format!("{ticket}-1"));
        let repo = SliceRepo {
            repo: PathBuf::from(REPO),
            branch: branch.clone(),
            worktree: worktree_for(&branch),
            base_commit: T_BRANCH_OLD_HEAD.to_string(),
        };
        let slice = ActiveSlice {
            id: id.clone(),
            ticket: tref(ticket),
            repos: vec![repo.clone()],
        };
        let mut w = self.world.borrow_mut();
        w.manifests.insert(
            id.clone(),
            Manifest {
                id,
                ticket: tref(ticket),
                ticket_system: system_of(&tref(ticket)),
                repos: vec![repo],
                status: ManifestStatus::Open,
                acceptance_criteria: vec![],
                invariants: vec![],
                spec_hash: None,
                test_hash: None,
                budgets: vec![],
                abandon_reason: None,
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

    /// Just the operation names of the writes, in order.
    pub fn write_ops(&self) -> Vec<String> {
        self.writes()
            .iter()
            .map(|w| w.split(' ').next().unwrap().to_string())
            .collect()
    }

    fn write(&self, op: &'static str, detail: String, source: Source) -> PortResult<()> {
        if self.fail == Some(op) && !self.failed.replace(true) {
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

    fn entry_today(&self, ticket: &TicketRef) -> PortResult<Option<TimerId>> {
        self.read(Source::Harvest)?;
        let w = self.world.borrow();
        Ok(w.entries
            .values()
            .filter(|e| e.today && e.ticket.as_ref() == Some(ticket))
            .max_by_key(|e| e.updated)
            .map(|e| e.id))
    }

    fn start(&self, ticket: &Ticket) -> PortResult<TimerId> {
        self.write("start", ticket.reference.0.clone(), Source::Harvest)?;
        let mut w = self.world.borrow_mut();
        let id = TimerId(w.entries.keys().max().copied().unwrap_or(100) + 1);
        let updated = w.tick();
        let notes = format!("{} - {}", ticket.reference.0, ticket.title);
        w.entries.insert(
            id.0,
            Entry {
                id,
                ticket: Some(ticket.reference.clone()),
                notes,
                running: true,
                today: true,
                updated,
            },
        );
        Ok(id)
    }

    fn stop(&self, id: TimerId) -> PortResult<()> {
        self.write("stop", id.0.to_string(), Source::Harvest)?;
        let mut w = self.world.borrow_mut();
        let updated = w.tick();
        let e = w.entries.get_mut(&id.0).expect("stop: no such entry");
        assert!(e.running, "stop: entry {} is not running", id.0);
        e.running = false;
        e.updated = updated;
        Ok(())
    }

    fn restart(&self, id: TimerId) -> PortResult<()> {
        self.write("restart", id.0.to_string(), Source::Harvest)?;
        let mut w = self.world.borrow_mut();
        let updated = w.tick();
        let e = w.entries.get_mut(&id.0).expect("restart: no such entry");
        assert!(e.today, "restart: entry {} is from an earlier day", id.0);
        e.running = true;
        e.updated = updated;
        Ok(())
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

impl Fake {
    /// Run `f` on `repo`'s HEAD, branches and worktrees, or fail as git does
    /// outside a repository.
    fn in_repo<R>(
        &self,
        repo: &Path,
        f: impl FnOnce(&str, &mut BTreeMap<String, String>, &mut BTreeMap<String, PathBuf>) -> R,
    ) -> PortResult<R> {
        let mut w = self.world.borrow_mut();
        let w = &mut *w;
        if repo == Path::new(REPO) {
            return Ok(f(BASE_COMMIT, &mut w.branches, &mut w.worktrees));
        }
        match w.repos.get_mut(repo) {
            Some(r) => Ok(f(&r.head, &mut r.branches, &mut r.worktrees)),
            None => Err(PortError {
                source: Source::Git,
                message: format!("{} is not a git repository", repo.display()),
            }),
        }
    }

    /// Every worktree path, in every repository.
    fn worktree_paths(&self) -> Vec<PathBuf> {
        let w = self.world.borrow();
        w.worktrees
            .values()
            .chain(w.repos.values().flat_map(|r| r.worktrees.values()))
            .cloned()
            .collect()
    }
}

impl Vcs for Fake {
    fn resolve(&self, repo: &Path, rev: &str) -> PortResult<String> {
        self.read(Source::Git)?;
        self.in_repo(repo, |head, branches, _| match rev {
            "HEAD" => Ok(head.to_string()),
            r => branches.get(r).cloned().ok_or(PortError {
                source: Source::Git,
                message: format!("no {r}"),
            }),
        })?
    }

    fn branches(&self, repo: &Path) -> PortResult<Vec<String>> {
        self.read(Source::Git)?;
        self.in_repo(repo, |_, branches, _| branches.keys().cloned().collect())
    }

    fn worktree_of(&self, repo: &Path, branch: &str) -> PortResult<Option<PathBuf>> {
        self.read(Source::Git)?;
        self.in_repo(repo, |_, _, worktrees| worktrees.get(branch).cloned())
    }

    fn create_branch(&self, repo: &Path, branch: &str, path: &Path, base: &str) -> PortResult<()> {
        self.in_repo(repo, |_, _, _| ())?;
        self.write(
            "create_branch",
            format!("{branch} {} {base}", path.display()),
            Source::Git,
        )?;
        let taken = self.worktree_paths().contains(&path.to_path_buf());
        assert!(!taken, "create_branch: a worktree is already at {}", path.display());
        self.in_repo(repo, |_, branches, worktrees| {
            assert!(!branches.contains_key(branch), "create_branch: {branch} exists");
            branches.insert(branch.to_string(), base.to_string());
            worktrees.insert(branch.to_string(), path.to_path_buf());
        })
    }

    fn add_worktree(&self, repo: &Path, branch: &str, path: &Path) -> PortResult<()> {
        self.in_repo(repo, |_, _, _| ())?;
        self.write("add_worktree", format!("{branch} {}", path.display()), Source::Git)?;
        let taken = self.worktree_paths().contains(&path.to_path_buf());
        assert!(!taken, "add_worktree: a worktree is already at {}", path.display());
        self.in_repo(repo, |_, branches, worktrees| {
            assert!(branches.contains_key(branch), "add_worktree: no branch {branch}");
            assert!(
                !worktrees.contains_key(branch),
                "add_worktree: {branch} already checked out"
            );
            worktrees.insert(branch.to_string(), path.to_path_buf());
        })
    }

    fn remove_worktree(&self, repo: &Path, path: &Path) -> PortResult<()> {
        self.in_repo(repo, |_, _, _| ())?;
        self.write("remove_worktree", path.display().to_string(), Source::Git)?;
        self.in_repo(repo, |_, _, worktrees| worktrees.retain(|_, p| p != path))
    }

    fn remove_branch(&self, repo: &Path, branch: &str, path: &Path) -> PortResult<()> {
        self.in_repo(repo, |_, _, _| ())?;
        self.write("remove_branch", format!("{branch} {}", path.display()), Source::Git)?;
        self.in_repo(repo, |_, branches, worktrees| {
            worktrees.remove(branch);
            branches.remove(branch);
        })
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

    fn read_manifest(&self, id: &SliceId) -> PortResult<Option<Manifest>> {
        self.read(Source::LocalState)?;
        Ok(self.world.borrow().manifests.get(id).cloned())
    }

    fn write_manifest(&self, manifest: &Manifest) -> PortResult<()> {
        self.write(
            "write_manifest",
            format!("{} {:?}", manifest.id.0, manifest.status),
            Source::LocalState,
        )?;
        self.world
            .borrow_mut()
            .manifests
            .insert(manifest.id.clone(), manifest.clone());
        Ok(())
    }

    fn remove_manifest(&self, id: &SliceId) -> PortResult<()> {
        self.write("remove_manifest", id.0.clone(), Source::LocalState)?;
        self.world.borrow_mut().manifests.remove(id);
        Ok(())
    }
}
