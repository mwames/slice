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
        let _ = (branch_type, date, ticket);
        todo!()
    }

    /// Parse a branch name; `None` unless it has the ticket-branch shape.
    pub fn parse(name: &str) -> Option<BranchName> {
        let _ = name;
        todo!()
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
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
    let _ = (deps, request);
    todo!()
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
    let _ = deps;
    todo!()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeOutcome {
    Resumed(ActiveSlice),
    AlreadyRunning(ActiveSlice),
}

/// Run the active slice's ticket's entry for today, restarting it or
/// starting one.
pub fn resume(deps: &Deps) -> Result<ResumeOutcome, SliceError> {
    let _ = deps;
    todo!()
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
    let _ = (deps, request);
    todo!()
}
