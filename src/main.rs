use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};
use slice::adapters::{git::Git, harvest::Harvest, state::FileState, tickets::Tickets};
use slice::config::Config;
use slice::ports::*;

const USAGE: &str = "usage:
  slice status [--json]
  slice open <TICKET> [--type TYPE] [--repo PATH] [--base REF] [--json]
  slice pause [--json]
  slice resume [--json]
  slice close [--done] [--json]
  slice abandon [--json]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("slice: {message}");
            ExitCode::from(2)
        }
    }
}

struct Flags {
    json: bool,
    repo: Option<PathBuf>,
    base: Option<String>,
    branch_type: Option<String>,
    done: bool,
    positional: Vec<String>,
}

fn parse(args: &[String]) -> Result<Flags, String> {
    let mut flags = Flags {
        json: false,
        repo: None,
        base: None,
        branch_type: None,
        done: false,
        positional: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--json" => flags.json = true,
            "--done" => flags.done = true,
            "--repo" => flags.repo = Some(PathBuf::from(value("--repo")?)),
            "--base" => flags.base = Some(value("--base")?),
            "--type" => flags.branch_type = Some(value("--type")?),
            a if a.starts_with('-') => return Err(format!("unknown flag {a}\n{USAGE}")),
            a => flags.positional.push(a.to_string()),
        }
    }
    Ok(flags)
}

fn run(args: &[String]) -> Result<u8, String> {
    let Some((command, rest)) = args.split_first() else {
        return Err(USAGE.to_string());
    };
    if matches!(command.as_str(), "help" | "-h" | "--help") {
        println!("{USAGE}");
        return Ok(0);
    }
    let flags = parse(rest)?;

    let config = Config::load()?;
    let settings = config.settings();
    let harvest = Harvest::new(&config.harvest.project, &config.harvest.task);
    let tickets = Tickets::new();
    let state = FileState::new(FileState::default_dir());
    let today = harvest::Date::today().to_string().replace('-', "");
    let deps = Deps {
        timer: &harvest,
        tickets: &tickets,
        vcs: &Git,
        state: &state,
        settings: &settings,
        today: &today,
    };

    match command.as_str() {
        "status" => {
            if !flags.positional.is_empty() {
                return Err(USAGE.to_string());
            }
            Ok(print_status(status(&deps), flags.json))
        }
        "open" => {
            let [ticket] = flags.positional.as_slice() else {
                return Err(USAGE.to_string());
            };
            let dir = match flags.repo {
                Some(repo) => repo,
                None => std::env::current_dir().map_err(|e| e.to_string())?,
            };
            let repo = Git::toplevel(&dir).map_err(|e| e.to_string())?;
            let request = OpenRequest {
                ticket: TicketRef(ticket.trim().to_uppercase()),
                repo,
                branch_type: flags.branch_type.map(|t| BranchType(t.trim().to_lowercase())),
                base: flags.base,
            };
            Ok(print_result(open(&deps, &request), flags.json, |o| match o {
                OpenOutcome::Opened(s) => (
                    "opened",
                    format!(
                        "opened {} on branch {}\n  worktree {}",
                        s.id.0,
                        s.branch,
                        s.worktree.display()
                    ),
                    s,
                ),
                OpenOutcome::AlreadyOpen(s) => ("already_open", format!("{} is already open", s.id.0), s),
                OpenOutcome::Resumed(s) => (
                    "resumed",
                    format!("{} was paused; its timer is running again", s.id.0),
                    s,
                ),
            }))
        }
        "pause" | "resume" | "close" | "abandon" if !flags.positional.is_empty() => Err(USAGE.to_string()),
        "pause" => Ok(print_result(pause(&deps), flags.json, |o| match o {
            PauseOutcome::Paused(s) => ("paused", format!("paused {}", s.id.0), s),
            PauseOutcome::AlreadyPaused(s) => ("already_paused", format!("{} is already paused", s.id.0), s),
        })),
        "resume" => Ok(print_result(resume(&deps), flags.json, |o| match o {
            ResumeOutcome::Resumed(s) => ("resumed", format!("resumed {}", s.id.0), s),
            ResumeOutcome::AlreadyRunning(s) => ("already_running", format!("{} is already running", s.id.0), s),
        })),
        "close" => {
            let done = flags.done;
            Ok(print_result(close(&deps, &CloseRequest { done }), flags.json, |s| {
                let to = if done { "done" } else { "review" };
                (
                    "closed",
                    format!(
                        "closed {}; {} moved to {to}; branch {} kept",
                        s.id.0, s.ticket.0, s.branch
                    ),
                    s,
                )
            }))
        }
        "abandon" => Ok(print_result(abandon(&deps), flags.json, |s| {
            (
                "abandoned",
                format!(
                    "abandoned {}; {} left as it was; branch {} kept",
                    s.id.0, s.ticket.0, s.branch
                ),
                s,
            )
        })),
        other => Err(format!("unknown command {other}\n{USAGE}")),
    }
}

fn opt_ref(r: &Option<TicketRef>) -> String {
    r.as_ref()
        .map(|r| r.0.clone())
        .unwrap_or_else(|| "no ticket".to_string())
}

fn slice_json(s: &ActiveSlice) -> Value {
    json!({
        "id": s.id.0, "ticket": s.ticket.0, "repo": s.repo, "branch": s.branch,
        "worktree": s.worktree,
    })
}

fn error_json(e: &PortError) -> Value {
    json!({ "source": e.source.to_string(), "message": e.message })
}

fn print_status(result: PortResult<StatusReport>, as_json: bool) -> u8 {
    let report = match result {
        Ok(report) => report,
        Err(e) => {
            if as_json {
                println!("{}", json!({ "error": error_json(&e) }));
            } else {
                eprintln!("slice: cannot read {}: {}", e.source, e.message);
            }
            return 1;
        }
    };
    let mismatch = report.mismatch.as_ref().map(mismatch_text);
    if as_json {
        let out = json!({
            "slice": report.active.as_ref().map(slice_json),
            "timer": report.timer.as_ref().map(|t| json!({ "id": t.id.0, "ticket": t.ticket.as_ref().map(|r| &r.0), "notes": t.notes })),
            "ticket": report.ticket.as_ref().map(|t| json!({ "ref": t.reference.0, "title": t.title, "status": t.status })),
            "mismatch": mismatch,
        });
        println!("{out}");
    } else {
        match &report.active {
            Some(s) => println!(
                "slice:  {} ({})  branch {}  worktree {}",
                s.id.0,
                s.ticket.0,
                s.branch,
                s.worktree.display()
            ),
            None => println!("slice:  none active"),
        }
        match &report.timer {
            Some(t) => println!("timer:  running, entry {}: {}", t.id.0, t.notes.trim()),
            None => println!("timer:  none running"),
        }
        if let Some(t) = &report.ticket {
            println!("ticket: {} {}  [{}]", t.reference.0, t.title, t.status);
        }
        if let Some(m) = &mismatch {
            println!("mismatch: {m}");
        }
    }
    report.exit_code() as u8
}

fn step_name(step: Step) -> &'static str {
    match step {
        Step::Preflight => "preflight",
        Step::ResolveTicket => "resolve ticket",
        Step::StopStrayTimer => "stop stray timer",
        Step::StartTimer => "start timer",
        Step::SetInProgress => "set ticket in progress",
        Step::CreateWorktree => "create branch and worktree",
        Step::WriteManifest => "write manifest",
        Step::SetActive => "record active slice",
        Step::StopTimer => "stop timer",
        Step::SetClosedStatus => "set ticket status",
        Step::CloseManifest => "close manifest",
        Step::AbandonManifest => "mark manifest abandoned",
        Step::ClearActive => "clear active slice",
    }
}

fn mismatch_text(m: &Mismatch) -> String {
    match m {
        Mismatch::DifferentTicket { slice, timer } => {
            format!(
                "the running timer is for {} but the active slice is for {}",
                opt_ref(timer),
                slice.0
            )
        }
        Mismatch::TimerWithoutSlice { timer } => {
            format!("a timer is running for {} with no active slice", opt_ref(timer))
        }
    }
}

fn describe_error(e: &SliceError) -> (String, Value) {
    match e {
        SliceError::NoActiveSlice => ("no slice is active".to_string(), json!({ "error": "no_active_slice" })),
        SliceError::AnotherSliceActive(s) => (
            format!(
                "slice {} for {} is active; close it before opening another",
                s.id.0, s.ticket.0
            ),
            json!({ "error": "another_slice_active", "slice": slice_json(s) }),
        ),
        SliceError::UnknownTicket(r) => (
            format!("no ticket {}", r.0),
            json!({ "error": "unknown_ticket", "ticket": r.0 }),
        ),
        SliceError::Inconsistent(report) => {
            let mismatch = report.mismatch.as_ref().map(mismatch_text).unwrap_or_default();
            (
                format!("{mismatch}; nothing changed"),
                json!({ "error": "inconsistent", "mismatch": mismatch, "slice": report.active.as_ref().map(slice_json) }),
            )
        }
        SliceError::BranchOptionsConflict { branch } => (
            format!("{branch} already exists; --base and a different --type apply only to a new branch"),
            json!({ "error": "branch_options_conflict", "branch": branch }),
        ),
        SliceError::TypeRequired => (
            "no branch exists for this ticket; pass --type to name one".to_string(),
            json!({ "error": "type_required" }),
        ),
        SliceError::AmbiguousBranch(branches) => (
            format!("more than one branch for this ticket: {}", branches.join(", ")),
            json!({ "error": "ambiguous_branch", "branches": branches }),
        ),
        SliceError::StepFailed {
            step,
            error,
            undo_failures,
        } => {
            let mut message = format!("{} failed: {error}; earlier steps undone", step_name(*step));
            for (s, e) in undo_failures {
                message.push_str(&format!("\n  could not undo {}: {e}", step_name(*s)));
            }
            let undo: Vec<Value> = undo_failures
                .iter()
                .map(|(s, e)| json!({ "step": step_name(*s), "error": error_json(e) }))
                .collect();
            (
                message,
                json!({ "error": "step_failed", "step": step_name(*step), "cause": error_json(error), "undo_failures": undo }),
            )
        }
    }
}

/// Print an operation's result and return its exit code. `ok` gives the
/// outcome's JSON name, its message, and the slice it concerns.
fn print_result<T>(
    result: Result<T, SliceError>,
    as_json: bool,
    ok: impl Fn(&T) -> (&'static str, String, &ActiveSlice),
) -> u8 {
    let (code, message, value) = match &result {
        Ok(outcome) => {
            let (name, message, slice) = ok(outcome);
            (0, message, json!({ "outcome": name, "slice": slice_json(slice) }))
        }
        Err(e) => {
            let (message, value) = describe_error(e);
            (e.exit_code() as u8, message, value)
        }
    };
    if as_json {
        println!("{value}");
    } else if code == 0 {
        println!("{message}");
    } else {
        eprintln!("slice: {message}");
    }
    code
}
