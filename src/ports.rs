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

/// A ticket's branch name taken apart: the parts a [`BranchTemplate`] writes
/// and recognizes (SLICE-11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchName {
    /// `None` when the template has no `{type}`.
    pub branch_type: Option<BranchType>,
    /// `yyyymmdd`, the day the branch was created. `None` unless the
    /// template gives a full date: `{date}`, or `{yyyy}`, `{mm}` and `{dd}`.
    pub date: Option<String>,
    pub ticket: TicketRef,
    pub short_title: String,
}

impl BranchName {
    /// The parts for a new branch of `ticket` created on `date`.
    pub fn new(branch_type: Option<&BranchType>, date: &str, ticket: &Ticket) -> BranchName {
        BranchName {
            branch_type: branch_type.cloned(),
            date: Some(date.to_string()),
            ticket: ticket.reference.clone(),
            short_title: short_title(&ticket.title),
        }
    }
}

/// A branch name template: literal text with the placeholders `{type}`,
/// `{date}`, `{yyyy}`, `{mm}`, `{dd}`, `{ticket}` and `{title}` (SLICE-11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchTemplate {
    text: String,
}

impl BranchTemplate {
    /// The template for a repository no config entry matches (SLICE-11-1).
    pub const DEFAULT: &str = "{type}/{date}-{ticket}-{title}";

    /// Check a template, or say what is wrong with it (SLICE-11-7).
    pub fn parse(text: &str) -> Result<BranchTemplate, String> {
        let _ = text;
        todo!("SLICE-11-7")
    }

    /// Whether names from this template carry a type (SLICE-11-6).
    pub fn has_type(&self) -> bool {
        todo!("SLICE-11-6")
    }

    /// The branch name for `name`'s parts. `name` has a type when the
    /// template has `{type}`, and a date when it has any date placeholder.
    /// An empty short title drops `{title}` and the literal text directly
    /// before it (SLICE-11-8).
    pub fn render(&self, name: &BranchName) -> String {
        let _ = name;
        todo!("SLICE-11-2")
    }

    /// `branch`'s parts when the whole name matches the template
    /// (SLICE-11-3, 11-4).
    pub fn recognize(&self, branch: &str) -> Option<BranchName> {
        let _ = branch;
        todo!("SLICE-11-3")
    }
}

impl Default for BranchTemplate {
    fn default() -> BranchTemplate {
        BranchTemplate::parse(BranchTemplate::DEFAULT).expect("the default template is valid")
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

/// One repository of a slice (SLICE-10). `repo` is the repository's main
/// working tree, never one of its worktrees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceRepo {
    pub repo: PathBuf,
    pub branch: String,
    pub worktree: PathBuf,
    pub base_commit: String,
}

/// The active slice, as recorded in local state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveSlice {
    pub id: SliceId,
    pub ticket: TicketRef,
    /// In the order they were added; the first is the one the slice was
    /// opened in, whose branch name every later repository takes.
    pub repos: Vec<SliceRepo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestStatus {
    Open,
    Closed,
    /// Ended by abandon: the ticket was left as it was (SLICE-6).
    Abandoned,
}

/// The slice manifest, kept in local state by slice id. Everything after
/// `repos` is filled in by later pipeline stages; open leaves it empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub id: SliceId,
    pub ticket: TicketRef,
    pub ticket_system: TicketSystemKind,
    /// The same repositories, in the same order, as the active slice.
    pub repos: Vec<SliceRepo>,
    pub status: ManifestStatus,
    pub acceptance_criteria: Vec<String>,
    pub invariants: Vec<String>,
    pub spec_hash: Option<String>,
    pub test_hash: Option<String>,
    pub budgets: Vec<(String, String)>,
    /// Why the slice was abandoned, when a reason was given (SLICE-7).
    pub abandon_reason: Option<String>,
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
    /// Branch templates by repository path pattern, in config order, with
    /// `~` already expanded (SLICE-11-5).
    pub branch_templates: Vec<(PathBuf, BranchTemplate)>,
}

impl Settings {
    /// The template of the first entry whose pattern matches `repo`, a
    /// main working tree, as a whole path; `*` matches within one path
    /// segment. The default when none does (SLICE-11-1, 11-5).
    pub fn template_for(&self, repo: &Path) -> BranchTemplate {
        let _ = repo;
        todo!("SLICE-11-5")
    }
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
/// compare them. Never writes to any port (SLICE-1-INV-1).
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
    /// The request's repository was added to the active slice, and is the
    /// last of its `repos` (SLICE-10-1, 10-3, 10-4).
    Added(ActiveSlice),
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

    let existing = existing_branch(deps, &request.repo, &ticket.reference)?;
    let worktree_root = worktree_root(deps.settings, &request.repo);
    let (branch, worktree, base_commit, checkout) = match existing {
        Some(branch) => plan_existing(deps, request, branch, &worktree_root)?,
        None => {
            let template = deps.settings.template_for(&request.repo);
            let Some(branch_type) = &request.branch_type else {
                return Err(SliceError::TypeRequired);
            };
            let branch = template.render(&BranchName::new(Some(branch_type), deps.today, &ticket));
            plan_new(deps, request, branch, &worktree_root)?
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

        make_worktree(
            deps,
            &request.repo,
            &branch,
            &worktree,
            &base_commit,
            &checkout,
            &mut undo,
        )
        .map_err(|e| (Step::CreateWorktree, e))?;

        let repo = SliceRepo {
            repo: request.repo.clone(),
            branch: branch.clone(),
            worktree: worktree.clone(),
            base_commit: base_commit.clone(),
        };
        let manifest = Manifest {
            id: id.clone(),
            ticket: ticket.reference.clone(),
            ticket_system: ticket.system,
            repos: vec![repo.clone()],
            status: ManifestStatus::Open,
            acceptance_criteria: Vec::new(),
            invariants: Vec::new(),
            spec_hash: None,
            test_hash: None,
            budgets: Vec::new(),
            abandon_reason: None,
        };
        deps.state
            .write_manifest(&manifest)
            .map_err(|e| (Step::WriteManifest, e))?;
        undo.push(Step::WriteManifest, Action::RemoveManifest(id.clone()));

        let slice = ActiveSlice {
            id: id.clone(),
            ticket: ticket.reference.clone(),
            repos: vec![repo],
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

/// The one branch in `repo` for the ticket, if any; more than one is
/// refused (SLICE-2-12).
fn existing_branch(deps: &Deps, repo: &Path, ticket: &TicketRef) -> Result<Option<String>, SliceError> {
    let template = deps.settings.template_for(repo);
    let mut existing: Vec<String> = deps
        .vcs
        .branches(repo)
        .map_err(|e| failed(Step::Preflight, e))?
        .into_iter()
        .filter(|b| template.recognize(b).is_some_and(|n| n.ticket == *ticket))
        .collect();
    if existing.len() > 1 {
        existing.sort();
        return Err(SliceError::AmbiguousBranch(existing));
    }
    Ok(existing.pop())
}

/// (branch, worktree, base commit, how to make the worktree)
type Plan = (String, PathBuf, String, Checkout);

/// Use `branch`, which exists in the request's repository: refused with
/// --base or a --type other than its own (SLICE-2-10).
fn plan_existing(deps: &Deps, request: &OpenRequest, branch: String, worktree_root: &Path) -> Result<Plan, SliceError> {
    let preflight = |e| failed(Step::Preflight, e);
    let template = deps.settings.template_for(&request.repo);
    let parsed = template.recognize(&branch).expect("filtered on recognize");
    let type_differs = request
        .branch_type
        .as_ref()
        .is_some_and(|t| Some(t) != parsed.branch_type.as_ref());
    if request.base.is_some() || type_differs {
        return Err(SliceError::BranchOptionsConflict { branch });
    }
    let base_commit = deps.vcs.resolve(&request.repo, &branch).map_err(preflight)?;
    match deps.vcs.worktree_of(&request.repo, &branch).map_err(preflight)? {
        Some(path) => Ok((branch, path, base_commit, Checkout::Existing)),
        None => {
            let path = worktree_root.join(branch.replace('/', "-"));
            Ok((branch, path, base_commit, Checkout::AddWorktree))
        }
    }
}

/// Create `branch` in the request's repository, at --base or `HEAD`.
fn plan_new(deps: &Deps, request: &OpenRequest, branch: String, worktree_root: &Path) -> Result<Plan, SliceError> {
    let base = request.base.as_deref().unwrap_or("HEAD");
    let base_commit = deps
        .vcs
        .resolve(&request.repo, base)
        .map_err(|e| failed(Step::Preflight, e))?;
    let path = worktree_root.join(branch.replace('/', "-"));
    Ok((branch, path, base_commit, Checkout::CreateBranch))
}

/// Make the worktree the checkout calls for, pushing the matching undo.
fn make_worktree(
    deps: &Deps,
    repo: &Path,
    branch: &str,
    worktree: &Path,
    base_commit: &str,
    checkout: &Checkout,
    undo: &mut Undo,
) -> PortResult<()> {
    match checkout {
        Checkout::Existing => {}
        Checkout::AddWorktree => {
            deps.vcs.add_worktree(repo, branch, worktree)?;
            undo.push(
                Step::CreateWorktree,
                Action::RemoveWorktree(repo.to_path_buf(), worktree.to_path_buf()),
            );
        }
        Checkout::CreateBranch => {
            deps.vcs.create_branch(repo, branch, worktree, base_commit)?;
            undo.push(
                Step::CreateWorktree,
                Action::RemoveBranch(repo.to_path_buf(), branch.to_string(), worktree.to_path_buf()),
            );
        }
    }
    Ok(())
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
    if !slice.repos.iter().any(|r| r.repo == request.repo) {
        return add_repo(deps, request, slice, report);
    }
    match resume_active(deps, slice, report)? {
        ResumeOutcome::Resumed(slice) => Ok(OpenOutcome::Resumed(slice)),
        ResumeOutcome::AlreadyRunning(slice) => Ok(OpenOutcome::AlreadyOpen(slice)),
    }
}

/// Add the request's repository to the active slice for the same ticket:
/// use or create its branch, named as the first repository's, and worktree,
/// run the ticket's entry for today if it is stopped, and record the
/// repository in the manifest and the active slice (SLICE-10).
fn add_repo(
    deps: &Deps,
    request: &OpenRequest,
    slice: &ActiveSlice,
    report: &StatusReport,
) -> Result<OpenOutcome, SliceError> {
    if report.mismatch.is_some() {
        return Err(SliceError::Inconsistent(Box::new(report.clone())));
    }
    let preflight = |e| failed(Step::Preflight, e);
    let first = &slice.repos[0];
    let worktree_root = worktree_root(deps.settings, &request.repo);
    let (branch, worktree, base_commit, checkout) = match existing_branch(deps, &request.repo, &slice.ticket)? {
        Some(branch) => plan_existing(deps, request, branch, &worktree_root)?,
        None => {
            // A new branch takes the slice's name, and so its type.
            let first_name = deps.settings.template_for(&first.repo).recognize(&first.branch);
            let type_differs = request
                .branch_type
                .as_ref()
                .zip(first_name)
                .is_some_and(|(t, n)| Some(t) != n.branch_type.as_ref());
            if type_differs {
                return Err(SliceError::BranchOptionsConflict {
                    branch: first.branch.clone(),
                });
            }
            plan_new(deps, request, first.branch.clone(), &worktree_root)?
        }
    };
    let manifest = match deps.state.read_manifest(&slice.id) {
        Ok(Some(manifest)) => manifest,
        Ok(None) => {
            let error = PortError {
                source: Source::LocalState,
                message: format!("no manifest for slice {}", slice.id.0),
            };
            return Err(preflight(error));
        }
        Err(e) => return Err(preflight(e)),
    };
    let entry = match report.timer {
        Some(_) => None,
        None => deps.timer.entry_today(&slice.ticket).map_err(preflight)?,
    };
    let ticket = match (&report.timer, &report.ticket) {
        (None, None) => return Err(SliceError::UnknownTicket(slice.ticket.clone())),
        (_, ticket) => ticket.as_ref(),
    };

    let mut undo = Undo::default();
    let result = (|| {
        if let (None, Some(ticket)) = (&report.timer, ticket) {
            run_entry(deps, ticket, entry, &mut undo).map_err(|e| (Step::StartTimer, e))?;
        }

        make_worktree(
            deps,
            &request.repo,
            &branch,
            &worktree,
            &base_commit,
            &checkout,
            &mut undo,
        )
        .map_err(|e| (Step::CreateWorktree, e))?;

        let mut repos = slice.repos.clone();
        repos.push(SliceRepo {
            repo: request.repo.clone(),
            branch: branch.clone(),
            worktree: worktree.clone(),
            base_commit: base_commit.clone(),
        });
        let added = Manifest {
            repos: repos.clone(),
            ..manifest.clone()
        };
        deps.state
            .write_manifest(&added)
            .map_err(|e| (Step::WriteManifest, e))?;
        undo.push(Step::WriteManifest, Action::WriteManifest(Box::new(manifest.clone())));

        let updated = ActiveSlice { repos, ..slice.clone() };
        deps.state.set_active(&updated).map_err(|e| (Step::SetActive, e))?;
        Ok(updated)
    })();

    match result {
        Ok(slice) => Ok(OpenOutcome::Added(slice)),
        Err((step, error)) => Err(SliceError::StepFailed {
            step,
            error,
            undo_failures: undo.unwind(deps),
        }),
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbandonRequest {
    /// Why the slice is being abandoned, already checked by [`abandon_reason`].
    pub reason: Option<String>,
}

/// A slice that abandon ended, and the reason recorded for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Abandoned {
    pub slice: ActiveSlice,
    pub reason: Option<String>,
}

impl Abandoned {
    /// The line printed without --json.
    pub fn message(&self) -> String {
        let line = format!(
            "abandoned {}; {} left as it was; {} kept",
            self.slice.id.0,
            self.slice.ticket.0,
            kept_branches(&self.slice)
        );
        match &self.reason {
            Some(reason) => format!("{line}; reason: {reason}"),
            None => line,
        }
    }

    /// The object printed with --json.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "outcome": "abandoned", "slice": slice_json(&self.slice), "reason": self.reason,
        })
    }
}

/// The value of `--reason` with surrounding whitespace removed, or an error
/// when nothing is left (SLICE-7-3).
pub fn abandon_reason(raw: &str) -> Result<String, String> {
    let reason = raw.trim();
    if reason.is_empty() {
        return Err("--reason needs a non-empty value".to_string());
    }
    Ok(reason.to_string())
}

/// A slice as `--json` output reports it: id, ticket, and each repository
/// with its path, branch and worktree, in the order they were added
/// (SLICE-10-8).
pub fn slice_json(s: &ActiveSlice) -> serde_json::Value {
    let repos: Vec<_> = s
        .repos
        .iter()
        .map(|r| {
            serde_json::json!({
                "repo": r.repo.display().to_string(),
                "branch": r.branch,
                "worktree": r.worktree.display().to_string(),
            })
        })
        .collect();
    serde_json::json!({ "id": s.id.0, "ticket": s.ticket.0, "repos": repos })
}

/// The branches close and abandon leave in place, for their messages.
pub fn kept_branches(s: &ActiveSlice) -> String {
    s.repos
        .iter()
        .map(|r| format!("{} in {} ({})", r.branch, r.repo.display(), r.worktree.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Stop the timer, mark the manifest abandoned with the request's reason, and
/// clear the active slice, without writing the ticket's status. The Harvest
/// entry, branch and worktree stay, and the reason goes nowhere but the
/// manifest (SLICE-6, SLICE-7).
pub fn abandon(deps: &Deps, request: &AbandonRequest) -> Result<Abandoned, SliceError> {
    let (slice, report) = active_slice(deps)?;
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

    // Close without its status step: the ticket is never written.
    let mut undo = Undo::default();
    let result = (|| {
        if let Some(timer) = &report.timer {
            deps.timer.stop(timer.id).map_err(|e| (Step::StopTimer, e))?;
            undo.push(Step::StopTimer, Action::Restart(timer.id));
        }

        let abandoned = Manifest {
            status: ManifestStatus::Abandoned,
            abandon_reason: request.reason.clone(),
            ..manifest.clone()
        };
        deps.state
            .write_manifest(&abandoned)
            .map_err(|e| (Step::AbandonManifest, e))?;
        undo.push(Step::AbandonManifest, Action::WriteManifest(Box::new(manifest.clone())));

        deps.state.clear_active().map_err(|e| (Step::ClearActive, e))
    })();

    match result {
        Ok(()) => Ok(Abandoned {
            slice,
            reason: request.reason.clone(),
        }),
        Err((step, error)) => Err(SliceError::StepFailed {
            step,
            error,
            undo_failures: undo.unwind(deps),
        }),
    }
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
