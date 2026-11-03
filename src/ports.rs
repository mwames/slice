//! The slice interface: the four ports slice talks to the world through, and
//! the operations built on them. Real adapters wrap the worktools crates and
//! the git CLI; tests use in-memory fakes.
//!
//! Harvest is tracked per ticket per day, and a branch belongs to a ticket;
//! slices are internal steps within both (see SLICE-2's rationale).

use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// A ticket reference such as `ENG-1234` (ClickUp) or `TBOARD-56` (monday),
/// normalized to uppercase.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TicketRef(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TicketSystemKind {
    ClickUp,
    Monday,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub reference: TicketRef,
    pub system: TicketSystemKind,
    pub title: String,
    pub status: String,
}

/// A Harvest time entry id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(pub i64);

/// A running Harvest timer. `ticket` is the ref found in its notes, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timer {
    pub id: TimerId,
    pub ticket: Option<TicketRef>,
    pub notes: String,
}

/// `<TICKET>-<n>`, with n counting slices opened for that ticket. Internal:
/// never written to Harvest, a ticket, or a branch name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SliceId(pub String);

/// The branch type chosen by the caller from the ticket's content, such as
/// `feature`, `bug` or `refactor`: one lowercase word.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BranchType(pub String);

/// A ticket's branch name, `<type>/<yyyymmdd>-<REF>-<short title>`, taken
/// apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchName {
    pub branch_type: BranchType,
    /// `yyyymmdd`, the day the branch was created.
    pub date: String,
    pub ticket: TicketRef,
    pub short_title: String,
}

impl BranchName {
    /// Name the branch for a new slice of `ticket` created on `date`.
    pub fn new(branch_type: &BranchType, date: &str, ticket: &Ticket) -> BranchName {
        BranchName {
            branch_type: branch_type.clone(),
            date: date.to_string(),
            ticket: ticket.reference.clone(),
            short_title: short_title(&ticket.title),
        }
    }

    /// Parse a branch name; `None` unless it has the ticket-branch shape.
    pub fn parse(name: &str) -> Option<BranchName> {
        let (branch_type, rest) = name.split_once('/')?;
        if branch_type.is_empty() || !branch_type.bytes().all(|b| b.is_ascii_lowercase()) || rest.contains('/') {
            return None;
        }
        let (date, rest) = rest.split_at_checked(8)?;
        let rest = rest.strip_prefix('-')?;
        if !date.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // <PREFIX>-<digits>, then optionally -<short title>.
        let (prefix, rest) = rest.split_once('-')?;
        let mut chars = prefix.bytes();
        if !chars.next().is_some_and(|b| b.is_ascii_uppercase())
            || !chars.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return None;
        }
        let (number, short_title) = rest.split_once('-').unwrap_or((rest, ""));
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(BranchName {
            branch_type: BranchType(branch_type.to_string()),
            date: date.to_string(),
            ticket: TicketRef(format!("{prefix}-{number}")),
            short_title: short_title.to_string(),
        })
    }
}

/// The ticket title lowercased, with runs of anything but ASCII letters and
/// digits turned into single hyphens, cut at a word to at most 40 characters.
fn short_title(title: &str) -> String {
    const MAX: usize = 40;
    let lowered = title.to_lowercase();
    let words = lowered
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty());
    let mut out = String::new();
    for word in words {
        let extra = if out.is_empty() { word.len() } else { word.len() + 1 };
        if out.len() + extra > MAX {
            if out.is_empty() {
                out.push_str(&word[..MAX]);
            }
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(word);
    }
    out
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}-{}", self.branch_type.0, self.date, self.ticket.0)?;
        if !self.short_title.is_empty() {
            write!(f, "-{}", self.short_title)?;
        }
        Ok(())
    }
}

/// The active slice, as recorded in local state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveSlice {
    pub id: SliceId,
    pub ticket: TicketRef,
    pub repo: PathBuf,
    pub branch: String,
    pub worktree: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestStatus {
    Open,
    Closed,
    /// Ended by abandon: the ticket was left as it was (SLICE-6).
    Abandoned,
}

/// The slice manifest, kept in local state by slice id. Everything after
/// `base_commit` is filled in by later pipeline stages; open leaves it empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub id: SliceId,
    pub ticket: TicketRef,
    pub ticket_system: TicketSystemKind,
    pub branch: String,
    pub base_commit: String,
    pub status: ManifestStatus,
    pub acceptance_criteria: Vec<String>,
    pub invariants: Vec<String>,
    pub spec_hash: Option<String>,
    pub test_hash: Option<String>,
    pub budgets: Vec<(String, String)>,
}

/// A ticket status per system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statuses {
    pub clickup: String,
    pub monday: String,
}

impl Statuses {
    pub fn for_system(&self, system: TicketSystemKind) -> &str {
        match system {
            TicketSystemKind::ClickUp => &self.clickup,
            TicketSystemKind::Monday => &self.monday,
        }
    }
}

/// Settings read from `~/.config/slice/config.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Set by open: `development` / `In Progress`.
    pub in_progress: Statuses,
    /// Set by close: `dev complete` / `Waiting for review`.
    pub review: Statuses,
    /// Set by close --done: `complete` / `Done`.
    pub done: Statuses,
    /// Directory for worktrees; `None` means `<repo>/../<repo>.slices/`.
    pub worktree_root: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// The external source a port talks to. Errors always name one (SLICE-1-4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Harvest,
    TicketSystem(TicketSystemKind),
    Git,
    LocalState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortError {
    pub source: Source,
    pub message: String,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Source::Harvest => "Harvest",
            Source::TicketSystem(TicketSystemKind::ClickUp) => "ClickUp",
            Source::TicketSystem(TicketSystemKind::Monday) => "monday",
            Source::Git => "git",
            Source::LocalState => "local state",
        })
    }
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.source, self.message)
    }
}

impl std::error::Error for PortError {}

pub type PortResult<T> = Result<T, PortError>;

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

/// Harvest timers. Each ticket has at most one entry per day that slice uses.
pub trait TimeTracker {
    /// The running timer, if any.
    fn running(&self) -> PortResult<Option<Timer>>;
    /// The ticket's entry for today, running or stopped: the most recently
    /// updated entry dated today whose notes name the ticket.
    fn entry_today(&self, ticket: &TicketRef) -> PortResult<Option<TimerId>>;
    /// Create and start a new entry for the ticket, noted `<REF> - <title>`.
    fn start(&self, ticket: &Ticket) -> PortResult<TimerId>;
    fn stop(&self, id: TimerId) -> PortResult<()>;
    /// Restart a stopped entry dated today.
    fn restart(&self, id: TimerId) -> PortResult<()>;
    /// Delete an entry. Only ever used on an entry the same run created.
    fn delete(&self, id: TimerId) -> PortResult<()>;
}

/// ClickUp and monday, routed by ref prefix inside the adapter.
pub trait TicketSystem {
    /// The ticket, or `None` if no such ticket exists (SLICE-2-5).
    fn fetch(&self, reference: &TicketRef) -> PortResult<Option<Ticket>>;
    fn set_status(&self, reference: &TicketRef, status: &str) -> PortResult<()>;
}

/// Git, through the git CLI.
pub trait Vcs {
    /// Resolve a revision such as `HEAD` or a branch name to a commit hash.
    fn resolve(&self, repo: &Path, rev: &str) -> PortResult<String>;
    /// Every local branch name in the repo; open picks out the ticket's.
    fn branches(&self, repo: &Path) -> PortResult<Vec<String>>;
    /// Where `branch` is checked out in a worktree, if anywhere.
    fn worktree_of(&self, repo: &Path, branch: &str) -> PortResult<Option<PathBuf>>;
    /// Create `branch` at `base` with no upstream, checked out in a new
    /// worktree at `path`. Leaves nothing behind if it fails.
    fn create_branch(&self, repo: &Path, branch: &str, path: &Path, base: &str) -> PortResult<()>;
    /// Check out an existing `branch` in a new worktree at `path`.
    fn add_worktree(&self, repo: &Path, branch: &str, path: &Path) -> PortResult<()>;
    /// Remove a worktree; undoes `add_worktree`.
    fn remove_worktree(&self, repo: &Path, path: &Path) -> PortResult<()>;
    /// Remove the worktree and delete the branch; undoes `create_branch`.
    fn remove_branch(&self, repo: &Path, branch: &str, path: &Path) -> PortResult<()>;
}

/// Local slice state (`~/.local/state/slice/`), manifests included.
pub trait StateStore {
    fn active(&self) -> PortResult<Option<ActiveSlice>>;
    /// The id the next slice for this ticket would get.
    fn next_slice_id(&self, ticket: &TicketRef) -> PortResult<SliceId>;
    fn set_active(&self, slice: &ActiveSlice) -> PortResult<()>;
    fn clear_active(&self) -> PortResult<()>;
    fn read_manifest(&self, id: &SliceId) -> PortResult<Option<Manifest>>;
    fn write_manifest(&self, manifest: &Manifest) -> PortResult<()>;
    fn remove_manifest(&self, id: &SliceId) -> PortResult<()>;
}

/// Everything the operations need.
pub struct Deps<'a> {
    pub timer: &'a dyn TimeTracker,
    pub tickets: &'a dyn TicketSystem,
    pub vcs: &'a dyn Vcs,
    pub state: &'a dyn StateStore,
    pub settings: &'a Settings,
    /// Today as `yyyymmdd`, for naming new branches.
    pub today: &'a str,
}

// ---------------------------------------------------------------------------
// slice status (SLICE-1)
// ---------------------------------------------------------------------------

/// What status found. Built only when every source answered; an unreachable
/// source is an `Err`, never an empty field (SLICE-1-4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusReport {
    pub active: Option<ActiveSlice>,
    pub timer: Option<Timer>,
    /// The active slice's ticket, fetched for its current status.
    pub ticket: Option<Ticket>,
    pub mismatch: Option<Mismatch>,
}

/// The running timer disagrees with the active slice (SLICE-1-3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// A timer is running for a different ticket than the active slice's.
    DifferentTicket { slice: TicketRef, timer: Option<TicketRef> },
    /// A timer is running with no active slice, whether or not its notes
    /// name a ticket.
    TimerWithoutSlice { timer: Option<TicketRef> },
}

impl StatusReport {
    pub fn exit_code(&self) -> i32 {
        if self.mismatch.is_some() { 1 } else { 0 }
    }
}

/// Read the active slice, the running timer and the ticket's status, and
/// compare them. Never writes to any port (SLICE-1-I1).
pub fn status(deps: &Deps) -> PortResult<StatusReport> {
    let active = deps.state.active()?;
    let timer = deps.timer.running()?;
    let ticket = match &active {
        Some(slice) => deps.tickets.fetch(&slice.ticket)?,
        None => None,
    };
    let mismatch = match (&active, &timer) {
        (Some(slice), Some(timer)) if timer.ticket.as_ref() != Some(&slice.ticket) => Some(Mismatch::DifferentTicket {
            slice: slice.ticket.clone(),
            timer: timer.ticket.clone(),
        }),
        (None, Some(timer)) => Some(Mismatch::TimerWithoutSlice {
            timer: timer.ticket.clone(),
        }),
        _ => None,
    };
    Ok(StatusReport {
        active,
        timer,
        ticket,
        mismatch,
    })
}

// ---------------------------------------------------------------------------
// Errors shared by open, pause, resume and close
// ---------------------------------------------------------------------------

/// The steps the operations take. Each successful step that changes
/// something pushes its undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Preflight,
    ResolveTicket,
    StopStrayTimer,
    /// Start a new entry or restart today's.
    StartTimer,
    SetInProgress,
    /// Create the branch and worktree, or the worktree for an existing branch.
    CreateWorktree,
    WriteManifest,
    SetActive,
    StopTimer,
    /// Set the review or done status.
    SetClosedStatus,
    CloseManifest,
    /// Mark the manifest abandoned.
    AbandonManifest,
    ClearActive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SliceError {
    /// No slice is active (SLICE-3-3, SLICE-4-3, SLICE-5-4).
    NoActiveSlice,
    /// A different ticket's slice is active (SLICE-2-3).
    AnotherSliceActive(Box<ActiveSlice>),
    /// The ticket does not exist (SLICE-2-5).
    UnknownTicket(TicketRef),
    /// The running timer does not match the active slice (SLICE-3-4,
    /// SLICE-4-4, SLICE-5-5).
    Inconsistent(Box<StatusReport>),
    /// --base, or a --type other than the branch's, given for a ticket whose
    /// branch exists (SLICE-2-10).
    BranchOptionsConflict { branch: String },
    /// No branch exists for the ticket and no --type was given (SLICE-2-11).
    TypeRequired,
    /// More than one branch matches the ticket (SLICE-2-12).
    AmbiguousBranch(Vec<String>),
    /// A step failed. Earlier steps were undone in reverse order; any undo
    /// that itself failed is listed (SLICE-2-6, SLICE-5-6).
    StepFailed {
        step: Step,
        error: PortError,
        undo_failures: Vec<(Step, PortError)>,
    },
}

impl SliceError {
    pub fn exit_code(&self) -> i32 {
        1
    }
}

// ---------------------------------------------------------------------------
// slice open (SLICE-2)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    pub ticket: TicketRef,
    pub repo: PathBuf,
    /// Required when the ticket has no branch yet; must match when it does.
    pub branch_type: Option<BranchType>,
    /// Where a new branch starts; `HEAD` when `None`. Refused for an
    /// existing branch.
    pub base: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenOutcome {
    /// A new slice was opened (SLICE-2-1, 2-2, 2-8, 2-9).
    Opened(ActiveSlice),
    /// The slice for this ticket was already open and timing (SLICE-2-4).
    AlreadyOpen(ActiveSlice),
    /// The slice for this ticket was open but paused; its timer is running
    /// again (SLICE-2-7).
    Resumed(ActiveSlice),
}

/// Open a slice for the requested ticket: stop any stray timer, run the
/// ticket's entry for today, set it in progress, use or create its branch and
/// worktree, write the manifest and record the slice as active.
pub fn open(deps: &Deps, request: &OpenRequest) -> Result<OpenOutcome, SliceError> {
    let preflight = |e| failed(Step::Preflight, e);

    // Reads first: nothing below changes anything until every check passes.
    let report = status(deps).map_err(preflight)?;
    if let Some(slice) = &report.active {
        return reopen(deps, request, slice, &report);
    }
    let ticket = match deps.tickets.fetch(&request.ticket) {
        Ok(Some(ticket)) => ticket,
        Ok(None) => return Err(SliceError::UnknownTicket(request.ticket.clone())),
        Err(e) => return Err(failed(Step::ResolveTicket, e)),
    };

    let mut existing: Vec<String> = deps
        .vcs
        .branches(&request.repo)
        .map_err(preflight)?
        .into_iter()
        .filter(|b| BranchName::parse(b).is_some_and(|n| n.ticket == ticket.reference))
        .collect();
    if existing.len() > 1 {
        existing.sort();
        return Err(SliceError::AmbiguousBranch(existing));
    }
    let worktree_root = worktree_root(deps.settings, &request.repo);
    // (branch, worktree, base commit, how to make the worktree)
    let (branch, worktree, base_commit, checkout) = match existing.pop() {
        Some(branch) => {
            let parsed = BranchName::parse(&branch).expect("filtered on parse");
            let type_differs = request.branch_type.as_ref().is_some_and(|t| *t != parsed.branch_type);
            if request.base.is_some() || type_differs {
                return Err(SliceError::BranchOptionsConflict { branch });
            }
            let base_commit = deps.vcs.resolve(&request.repo, &branch).map_err(preflight)?;
            match deps.vcs.worktree_of(&request.repo, &branch).map_err(preflight)? {
                Some(path) => (branch, path, base_commit, Checkout::Existing),
                None => {
                    let path = worktree_root.join(branch.replace('/', "-"));
                    (branch, path, base_commit, Checkout::AddWorktree)
                }
            }
        }
        None => {
            let Some(branch_type) = &request.branch_type else {
                return Err(SliceError::TypeRequired);
            };
            let branch = BranchName::new(branch_type, deps.today, &ticket).to_string();
            let base = request.base.as_deref().unwrap_or("HEAD");
            let base_commit = deps.vcs.resolve(&request.repo, base).map_err(preflight)?;
            let path = worktree_root.join(branch.replace('/', "-"));
            (branch, path, base_commit, Checkout::CreateBranch)
        }
    };
    let id = deps.state.next_slice_id(&ticket.reference).map_err(preflight)?;
    let entry = deps.timer.entry_today(&ticket.reference).map_err(preflight)?;
    let in_progress = deps.settings.in_progress.for_system(ticket.system);

    let mut undo = Undo::default();
    let result = (|| {
        if let Some(stray) = &report.timer {
            deps.timer.stop(stray.id).map_err(|e| (Step::StopStrayTimer, e))?;
            undo.push(Step::StopStrayTimer, Action::Restart(stray.id));
        }

        run_entry(deps, &ticket, entry, &mut undo).map_err(|e| (Step::StartTimer, e))?;

        deps.tickets
            .set_status(&ticket.reference, in_progress)
            .map_err(|e| (Step::SetInProgress, e))?;
        undo.push(
            Step::SetInProgress,
            Action::SetStatus(ticket.reference.clone(), ticket.status.clone()),
        );

        match checkout {
            Checkout::Existing => {}
            Checkout::AddWorktree => {
                deps.vcs
                    .add_worktree(&request.repo, &branch, &worktree)
                    .map_err(|e| (Step::CreateWorktree, e))?;
                undo.push(
                    Step::CreateWorktree,
                    Action::RemoveWorktree(request.repo.clone(), worktree.clone()),
                );
            }
            Checkout::CreateBranch => {
                deps.vcs
                    .create_branch(&request.repo, &branch, &worktree, &base_commit)
                    .map_err(|e| (Step::CreateWorktree, e))?;
                undo.push(
                    Step::CreateWorktree,
                    Action::RemoveBranch(request.repo.clone(), branch.clone(), worktree.clone()),
                );
            }
        }

        let manifest = Manifest {
            id: id.clone(),
            ticket: ticket.reference.clone(),
            ticket_system: ticket.system,
            branch: branch.clone(),
            base_commit: base_commit.clone(),
            status: ManifestStatus::Open,
            acceptance_criteria: Vec::new(),
            invariants: Vec::new(),
            spec_hash: None,
            test_hash: None,
            budgets: Vec::new(),
        };
        deps.state
            .write_manifest(&manifest)
            .map_err(|e| (Step::WriteManifest, e))?;
        undo.push(Step::WriteManifest, Action::RemoveManifest(id.clone()));

        let slice = ActiveSlice {
            id: id.clone(),
            ticket: ticket.reference.clone(),
            repo: request.repo.clone(),
            branch: branch.clone(),
            worktree: worktree.clone(),
        };
        deps.state.set_active(&slice).map_err(|e| (Step::SetActive, e))?;
        Ok(slice)
    })();

    match result {
        Ok(slice) => Ok(OpenOutcome::Opened(slice)),
        Err((step, error)) => Err(SliceError::StepFailed {
            step,
            error,
            undo_failures: undo.unwind(deps),
        }),
    }
}

/// How open gets a worktree for the slice's branch.
enum Checkout {
    /// The branch is already checked out in a worktree.
    Existing,
    /// The branch exists; check it out in a new worktree.
    AddWorktree,
    /// Create the branch and its worktree.
    CreateBranch,
}

/// Open for a ticket while a slice is active: the same ticket's slice is
/// already open (SLICE-2-4) or paused and resumed (SLICE-2-7); a different
/// ticket's slice blocks the open (SLICE-2-3).
fn reopen(
    deps: &Deps,
    request: &OpenRequest,
    slice: &ActiveSlice,
    report: &StatusReport,
) -> Result<OpenOutcome, SliceError> {
    if slice.ticket != request.ticket {
        return Err(SliceError::AnotherSliceActive(Box::new(slice.clone())));
    }
    match resume_active(deps, slice, report)? {
        ResumeOutcome::Resumed(slice) => Ok(OpenOutcome::Resumed(slice)),
        ResumeOutcome::AlreadyRunning(slice) => Ok(OpenOutcome::AlreadyOpen(slice)),
    }
}

/// Run the ticket's entry for today: restart it if there is one, start one
/// otherwise, pushing the matching undo.
fn run_entry(deps: &Deps, ticket: &Ticket, entry: Option<TimerId>, undo: &mut Undo) -> PortResult<()> {
    match entry {
        Some(id) => {
            deps.timer.restart(id)?;
            undo.push(Step::StartTimer, Action::Stop(id));
        }
        None => {
            let id = deps.timer.start(ticket)?;
            undo.push(Step::StartTimer, Action::Delete(id));
        }
    }
    Ok(())
}

fn failed(step: Step, error: PortError) -> SliceError {
    SliceError::StepFailed {
        step,
        error,
        undo_failures: Vec::new(),
    }
}

/// The active slice from a status report, refusing when there is none or the
/// running timer does not match it.
fn active_slice(deps: &Deps) -> Result<(ActiveSlice, StatusReport), SliceError> {
    let report = status(deps).map_err(|e| failed(Step::Preflight, e))?;
    let Some(slice) = report.active.clone() else {
        return Err(SliceError::NoActiveSlice);
    };
    if report.mismatch.is_some() {
        return Err(SliceError::Inconsistent(Box::new(report)));
    }
    Ok((slice, report))
}

/// `<repo>/../<repo name>.slices/` unless configured.
fn worktree_root(settings: &Settings, repo: &Path) -> PathBuf {
    if let Some(root) = &settings.worktree_root {
        return root.clone();
    }
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    repo.parent().unwrap_or(repo).join(format!("{name}.slices"))
}

/// The compensating action for a completed step.
enum Action {
    Restart(TimerId),
    Stop(TimerId),
    Delete(TimerId),
    SetStatus(TicketRef, String),
    RemoveWorktree(PathBuf, PathBuf),
    RemoveBranch(PathBuf, String, PathBuf),
    RemoveManifest(SliceId),
    WriteManifest(Box<Manifest>),
}

#[derive(Default)]
struct Undo(Vec<(Step, Action)>);

impl Undo {
    fn push(&mut self, step: Step, action: Action) {
        self.0.push((step, action));
    }

    /// Run every compensating action in reverse order, continuing past
    /// failures, and return the ones that failed.
    fn unwind(self, deps: &Deps) -> Vec<(Step, PortError)> {
        let mut failures = Vec::new();
        for (step, action) in self.0.into_iter().rev() {
            let result = match action {
                Action::Restart(id) => deps.timer.restart(id),
                Action::Stop(id) => deps.timer.stop(id),
                Action::Delete(id) => deps.timer.delete(id),
                Action::SetStatus(r, status) => deps.tickets.set_status(&r, &status),
                Action::RemoveWorktree(repo, path) => deps.vcs.remove_worktree(&repo, &path),
                Action::RemoveBranch(repo, branch, path) => deps.vcs.remove_branch(&repo, &branch, &path),
                Action::RemoveManifest(id) => deps.state.remove_manifest(&id),
                Action::WriteManifest(manifest) => deps.state.write_manifest(&manifest),
            };
            if let Err(e) = result {
                failures.push((step, e));
            }
        }
        failures
    }
}

// ---------------------------------------------------------------------------
// slice pause (SLICE-3), resume (SLICE-4), close (SLICE-5)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PauseOutcome {
    Paused(ActiveSlice),
    AlreadyPaused(ActiveSlice),
}

/// Stop the active slice's timer; nothing else changes.
pub fn pause(deps: &Deps) -> Result<PauseOutcome, SliceError> {
    let (slice, report) = active_slice(deps)?;
    let Some(timer) = &report.timer else {
        return Ok(PauseOutcome::AlreadyPaused(slice));
    };
    deps.timer.stop(timer.id).map_err(|e| failed(Step::StopTimer, e))?;
    Ok(PauseOutcome::Paused(slice))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeOutcome {
    Resumed(ActiveSlice),
    AlreadyRunning(ActiveSlice),
}

/// Run the active slice's ticket's entry for today, restarting it or
/// starting one.
pub fn resume(deps: &Deps) -> Result<ResumeOutcome, SliceError> {
    let (slice, report) = active_slice(deps)?;
    resume_active(deps, &slice, &report)
}

/// Resume `slice`, which is active, given the status report that found it.
fn resume_active(deps: &Deps, slice: &ActiveSlice, report: &StatusReport) -> Result<ResumeOutcome, SliceError> {
    if report.mismatch.is_some() {
        return Err(SliceError::Inconsistent(Box::new(report.clone())));
    }
    if report.timer.is_some() {
        return Ok(ResumeOutcome::AlreadyRunning(slice.clone()));
    }
    let Some(ticket) = &report.ticket else {
        return Err(SliceError::UnknownTicket(slice.ticket.clone()));
    };
    let entry = deps
        .timer
        .entry_today(&ticket.reference)
        .map_err(|e| failed(Step::Preflight, e))?;
    // A single write: nothing earlier to undo if it fails.
    run_entry(deps, ticket, entry, &mut Undo::default()).map_err(|e| failed(Step::StartTimer, e))?;
    Ok(ResumeOutcome::Resumed(slice.clone()))
}

/// Stop the timer, mark the manifest abandoned and clear the active slice,
/// without writing the ticket's status. The Harvest entry, branch and worktree
/// stay. Returns the slice that was abandoned (SLICE-6).
pub fn abandon(deps: &Deps) -> Result<ActiveSlice, SliceError> {
    let _ = deps;
    todo!("SLICE-6")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseRequest {
    /// Move the ticket to done rather than review.
    pub done: bool,
}

/// Stop the timer, move the ticket to review or done, mark the manifest
/// closed and clear the active slice. The branch and worktree stay. Returns
/// the slice that was closed.
pub fn close(deps: &Deps, request: &CloseRequest) -> Result<ActiveSlice, SliceError> {
    let (slice, report) = active_slice(deps)?;
    let Some(ticket) = &report.ticket else {
        return Err(SliceError::UnknownTicket(slice.ticket.clone()));
    };
    let manifest = match deps.state.read_manifest(&slice.id) {
        Ok(Some(manifest)) => manifest,
        Ok(None) => {
            let error = PortError {
                source: Source::LocalState,
                message: format!("no manifest for slice {}", slice.id.0),
            };
            return Err(failed(Step::Preflight, error));
        }
        Err(e) => return Err(failed(Step::Preflight, e)),
    };
    let statuses = if request.done {
        &deps.settings.done
    } else {
        &deps.settings.review
    };

    let mut undo = Undo::default();
    let result = (|| {
        if let Some(timer) = &report.timer {
            deps.timer.stop(timer.id).map_err(|e| (Step::StopTimer, e))?;
            undo.push(Step::StopTimer, Action::Restart(timer.id));
        }

        deps.tickets
            .set_status(&ticket.reference, statuses.for_system(ticket.system))
            .map_err(|e| (Step::SetClosedStatus, e))?;
        undo.push(
            Step::SetClosedStatus,
            Action::SetStatus(ticket.reference.clone(), ticket.status.clone()),
        );

        let closed = Manifest {
            status: ManifestStatus::Closed,
            ..manifest.clone()
        };
        deps.state
            .write_manifest(&closed)
            .map_err(|e| (Step::CloseManifest, e))?;
        undo.push(Step::CloseManifest, Action::WriteManifest(Box::new(manifest.clone())));

        deps.state.clear_active().map_err(|e| (Step::ClearActive, e))
    })();

    match result {
        Ok(()) => Ok(slice),
        Err((step, error)) => Err(SliceError::StepFailed {
            step,
            error,
            undo_failures: undo.unwind(deps),
        }),
    }
}
