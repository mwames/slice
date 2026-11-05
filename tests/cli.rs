//! The slice binary's argument handling, run with HOME, XDG_STATE_HOME and
//! XDG_CONFIG_HOME in a fresh temp dir and no other environment, so it can
//! never reach real configuration, state, Harvest or a ticket system.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use slice::adapters::state::FileState;
use slice::ports::*;

/// A fresh home with an active slice and its manifest in local state, and a
/// config file that fails to load, so a run that gets past argument checks
/// stops there rather than reaching Harvest or a ticket system.
/// A temporary directory, deleted with everything under it when it goes out
/// of scope, even when the test fails. Derefs to `path`, which may be a
/// subdirectory of the root that gets deleted.
struct TempDir {
    root: PathBuf,
    path: PathBuf,
}

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn home(name: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!("slice-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config/slice")).unwrap();
    std::fs::write(dir.join("config/slice/config.toml"), "not = [valid toml\n").unwrap();

    let state = FileState::new(dir.join("state/slice"));
    let branch = "feature/20261101-ENG-1-login";
    let slice = ActiveSlice {
        id: SliceId("ENG-1-1".into()),
        ticket: TicketRef("ENG-1".into()),
        repo: PathBuf::from("/work/app"),
        branch: branch.into(),
        worktree: PathBuf::from("/work/app.slices/feature-20261101-ENG-1-login"),
    };
    state.set_active(&slice).unwrap();
    state
        .write_manifest(&Manifest {
            id: slice.id.clone(),
            ticket: slice.ticket.clone(),
            ticket_system: TicketSystemKind::ClickUp,
            branch: branch.into(),
            base_commit: "def5678".into(),
            status: ManifestStatus::Open,
            acceptance_criteria: vec![],
            invariants: vec![],
            spec_hash: None,
            test_hash: None,
            budgets: vec![],
            abandon_reason: None,
        })
        .unwrap();
    TempDir {
        root: dir.clone(),
        path: dir,
    }
}

fn slice(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_slice"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .current_dir(home)
        .output()
        .unwrap()
}

/// Every file under `dir` and its contents.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.insert(path.clone(), std::fs::read(&path).unwrap());
        }
    }
    out
}

/// A usage error: exit 2, a message on stderr that is not about the config
/// file, nothing on stdout, and no file changed.
fn assert_usage_error(name: &str, args: &[&str]) {
    let home = home(name);
    let before = files(&home);

    let out = slice(&home, args);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{args:?}: exit code; stderr: {stderr}");
    assert!(!stderr.trim().is_empty(), "{args:?}: an error is printed");
    assert!(
        !stderr.contains("config.toml"),
        "{args:?}: refused before config is loaded: {stderr}"
    );
    assert!(
        out.stdout.is_empty(),
        "{args:?}: stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(files(&home), before, "{args:?}: nothing changed");
}

// SLICE-7-3
#[test]
fn abandon_with_empty_reason_is_a_usage_error() {
    for (i, reason) in ["", " ", "   ", "\t", " \n "].into_iter().enumerate() {
        assert_usage_error(&format!("empty-{i}"), &["abandon", "--reason", reason]);
        assert_usage_error(&format!("empty-json-{i}"), &["abandon", "--reason", reason, "--json"]);
    }
}

// SLICE-7-3
#[test]
fn abandon_with_reason_last_is_a_usage_error() {
    assert_usage_error("last", &["abandon", "--reason"]);
    assert_usage_error("last-json", &["abandon", "--json", "--reason"]);
}

// SLICE-7-4
#[test]
fn reason_on_other_commands_is_a_usage_error() {
    let commands: [&[&str]; 7] = [
        &["status"],
        &["status", "--json"],
        &["open", "ENG-1", "--type", "feature"],
        &["pause"],
        &["resume"],
        &["close"],
        &["close", "--done"],
    ];
    for (i, command) in commands.into_iter().enumerate() {
        let mut args = command.to_vec();
        args.extend(["--reason", "fixed by ENG-14800"]);
        assert_usage_error(&format!("other-{i}"), &args);
    }
}
