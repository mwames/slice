//! The slice interface: the four ports slice talks to the world through, and
//! the two operations built on them. Real adapters wrap the worktools crates
//! and the git CLI; tests use in-memory fakes.

use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// A ticket reference such as `ENG-1234` (ClickUp) or `TBOARD-56` (monday),
/// normalized to uppercase.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimerId(pub i64);

/// A running Harvest timer. `ticket` is the ref found in its notes, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timer {
    pub id: TimerId,
    pub ticket: Option<TicketRef>,
    pub notes: String,
}

/// `<TICKET>-<n>`, with n counting slices opened for that ticket.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SliceId(pub String);

/// The active slice, as recorded in local state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveSlice {
    pub id: SliceId,
    pub ticket: TicketRef,
    pub repo: PathBuf,
    pub branch: String,
    pub worktree: PathBuf,
    pub manifest: PathBuf,
    /// The slice's time entry, restarted when the slice is reopened (SLICE-2-7).
    pub timer: TimerId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestStatus {
    Open,
}

/// The slice manifest written into the worktree. Everything after
/// `base_commit` is filled in by later pipeline stages; open leaves it empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub id: SliceId,
    pub ticket: TicketRef,
    pub ticket_system: TicketSystemKind,
    pub base_commit: String,
    pub status: ManifestStatus,
    pub acceptance_criteria: Vec<String>,
    pub invariants: Vec<String>,
    pub spec_hash: Option<String>,
    pub test_hash: Option<String>,
    pub budgets: Vec<(String, String)>,
}

/// Settings read from `~/.config/slice/config.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Status that marks a ClickUp ticket in progress, e.g. `development`.
    pub clickup_in_progress: String,
    /// Status that marks a monday item in progress, e.g. `In Progress`.
    pub monday_in_progress: String,
    /// Prefix for slice branches, e.g. `slice/`.
    pub branch_prefix: String,
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

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.source, self.message)
    }
}

impl std::error::Error for PortError {}

pub type PortResult<T> = Result<T, PortError>;

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

/// Harvest timers.
pub trait TimeTracker {
    /// The running timer, if any.
    fn running(&self) -> PortResult<Option<Timer>>;
    /// Start a timer for the ticket with the given notes.
    fn start(&self, ticket: &Ticket, notes: &str) -> PortResult<Timer>;
    fn stop(&self, id: TimerId) -> PortResult<()>;
    /// Restart a stopped timer; undoes `stop`. Returns the id of the running
    /// timer, which differs from `id` when the entry cannot be restarted (for
    /// example, it is from an earlier day) and a new one with the same notes
    /// is started instead.
    fn restart(&self, id: TimerId) -> PortResult<TimerId>;
    /// Delete an entry; undoes `start`.
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
    /// Resolve a revision such as `HEAD` to a commit hash.
    fn resolve(&self, repo: &Path, rev: &str) -> PortResult<String>;
    /// Create `branch` at `base` and check it out in a new worktree at `path`,
    /// with no upstream set.
    fn create_worktree(&self, repo: &Path, branch: &str, path: &Path, base: &str) -> PortResult<()>;
    /// Remove the worktree and delete the branch; undoes `create_worktree`.
    fn remove_worktree(&self, repo: &Path, branch: &str, path: &Path) -> PortResult<()>;
}

/// Local slice state (`~/.local/state/slice/`) and manifests.
pub trait StateStore {
    fn active(&self) -> PortResult<Option<ActiveSlice>>;
    /// The id the next slice for this ticket would get.
    fn next_slice_id(&self, ticket: &TicketRef) -> PortResult<SliceId>;
    fn set_active(&self, slice: &ActiveSlice) -> PortResult<()>;
    fn clear_active(&self) -> PortResult<()>;
    fn write_manifest(&self, path: &Path, manifest: &Manifest) -> PortResult<()>;
    fn remove_manifest(&self, path: &Path) -> PortResult<()>;
}

/// Everything status and open need.
pub struct Deps<'a> {
    pub timer: &'a dyn TimeTracker,
    pub tickets: &'a dyn TicketSystem,
    pub vcs: &'a dyn Vcs,
    pub state: &'a dyn StateStore,
    pub settings: &'a Settings,
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
        todo!()
    }
}

/// Read the active slice, the running timer and the ticket's status, and
/// compare them. Never writes to any port (SLICE-1-I1).
pub fn status(deps: &Deps) -> PortResult<StatusReport> {
    let _ = deps;
    todo!()
}

// ---------------------------------------------------------------------------
// slice open (SLICE-2)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    pub ticket: TicketRef,
    pub repo: PathBuf,
    /// Revision the slice branch starts from; `HEAD` by default.
    pub base: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenOutcome {
    /// A new slice was opened (SLICE-2-1, SLICE-2-2).
    Opened(ActiveSlice),
    /// The slice for this ticket was already open; nothing changed (SLICE-2-4).
    AlreadyOpen(ActiveSlice),
    /// The slice for this ticket was open with no timer running; its timer
    /// was restarted (SLICE-2-7).
    Resumed(ActiveSlice),
}

/// The steps of open, in order. Each successful step pushes its undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenStep {
    Preflight,
    ResolveTicket,
    StopStrayTimer,
    StartTimer,
    SetInProgress,
    CreateWorktree,
    WriteManifest,
    SetActive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// A different ticket's slice is active; nothing changed (SLICE-2-3).
    AnotherSliceActive(Box<ActiveSlice>),
    /// The ticket does not exist; nothing changed (SLICE-2-5).
    UnknownTicket(TicketRef),
    /// Preflight status found a mismatch; nothing changed.
    Inconsistent(Box<StatusReport>),
    /// A step failed. Earlier steps were undone in reverse order, including
    /// restarting a stopped timer; any undo that itself failed is listed
    /// (SLICE-2-6).
    StepFailed {
        step: OpenStep,
        error: PortError,
        undo_failures: Vec<(OpenStep, PortError)>,
    },
}

impl OpenError {
    pub fn exit_code(&self) -> i32 {
        todo!()
    }
}

/// Open a slice for the requested ticket: stop any stray timer, start one
/// for the ticket, set it in progress, create the branch and worktree, write
/// the manifest and record the slice as active.
pub fn open(deps: &Deps, request: &OpenRequest) -> Result<OpenOutcome, OpenError> {
    let _ = (deps, request);
    todo!()
}
