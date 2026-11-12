//! The slice binary with a slice in several repositories (SLICE-10): what
//! status and open print, and open run from a worktree. Harvest and ClickUp
//! are a stub server on localhost; git repositories are real, in temp dirs.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use slice::adapters::state::FileState;
use slice::ports::*;

/// The active slice's branch.
const BRANCH: &str = "feature/20261101-ENG-1-login";

/// A temporary directory, deleted with everything under it when it goes out
/// of scope, even when the test fails.
struct TempDir {
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
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn temp_dir(name: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!("slice-multi-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TempDir {
        path: dir.canonicalize().unwrap(),
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A git repository at `<root>/<name>` with one empty commit on `main`.
fn repo(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(
        &dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "base",
        ],
    );
    dir
}

// ---------------------------------------------------------------------------
// Harvest and ClickUp
// ---------------------------------------------------------------------------

/// A Harvest time entry dated today.
struct Entry {
    id: i64,
    notes: String,
    running: bool,
}

/// What the stub server holds: Harvest's entries for today and ticket
/// ENG-1's ClickUp status.
struct Remote {
    entries: Vec<Entry>,
    status: String,
}

impl Remote {
    /// T's entry for today, running or stopped, and the ticket in progress.
    fn with_entry(running: bool) -> Remote {
        Remote {
            entries: vec![Entry {
                id: 501,
                notes: "ENG-1 - Login".into(),
                running,
            }],
            status: "development".into(),
        }
    }

    fn entry_json(e: &Entry) -> Value {
        json!({
            "id": e.id,
            "spent_date": harvest::Date::today().to_string(),
            "notes": e.notes,
            "is_running": e.running,
            "created_at": "2026-11-12T09:00:00Z",
            "updated_at": "2026-11-12T09:00:00Z",
            "project": { "id": 1, "name": "Acme Product" },
            "task": { "id": 2, "name": "New Development" },
        })
    }

    fn task_json(&self) -> Value {
        json!({
            "id": "86abc",
            "custom_id": "ENG-1",
            "name": "Login",
            "status": { "status": self.status },
            "list": { "id": "L1", "name": "Dev" },
        })
    }

    /// The response to one request: status code and JSON body.
    fn respond(&mut self, method: &str, target: &str, body: &str) -> (u16, Value) {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
        match (method, segments.as_slice()) {
            ("GET", ["users", "me"]) => (200, json!({ "id": 1, "first_name": "T", "is_active": true })),
            ("GET", ["time_entries"]) => {
                let running_only = query.split('&').any(|p| p == "is_running=true");
                let entries: Vec<Value> = self
                    .entries
                    .iter()
                    .filter(|e| e.running || !running_only)
                    .map(Remote::entry_json)
                    .collect();
                (200, json!({ "time_entries": entries, "next_page": null }))
            }
            ("GET", ["time_entries", id]) => match self.entries.iter().find(|e| e.id.to_string() == *id) {
                Some(e) => (200, Remote::entry_json(e)),
                None => (404, json!({ "message": "not found" })),
            },
            ("PATCH", ["time_entries", id, action @ ("restart" | "stop")]) => {
                match self.entries.iter_mut().find(|e| e.id.to_string() == *id) {
                    Some(e) => {
                        e.running = *action == "restart";
                        (200, Remote::entry_json(e))
                    }
                    None => (404, json!({ "message": "not found" })),
                }
            }
            ("GET", ["task", "ENG-1"]) => (200, self.task_json()),
            ("PUT", ["task", "ENG-1"]) => {
                let body: Value = serde_json::from_str(body).unwrap_or_default();
                if let Some(status) = body["status"].as_str() {
                    self.status = status.to_string();
                }
                (200, self.task_json())
            }
            ("GET", ["list", "L1"]) => (
                200,
                json!({ "id": "L1", "name": "Dev", "statuses": [
                    { "status": "to do" },
                    { "status": "development" },
                    { "status": "dev complete" },
                    { "status": "complete" },
                ] }),
            ),
            _ => (404, json!({ "err": "not found", "ECODE": "ITEM_015" })),
        }
    }
}

/// Serve `remote` on localhost for the rest of the process; returns the base
/// URL.
fn serve(remote: Remote) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let remote = Arc::new(Mutex::new(remote));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or_default().to_string();
            let target = parts.next().unwrap_or_default().to_string();
            let (code, value) = remote
                .lock()
                .unwrap()
                .respond(&method, &target, &String::from_utf8_lossy(&body));
            let body = value.to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 {code} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                if code == 200 { "OK" } else { "Not Found" },
                body.len()
            );
            let _ = stream.flush();
        }
    });
    url
}

// ---------------------------------------------------------------------------
// The binary
// ---------------------------------------------------------------------------

/// A home for the binary: local state, an empty config, and the stub
/// serving `remote`.
struct Home {
    dir: TempDir,
    url: String,
}

impl Home {
    fn new(name: &str, remote: Remote) -> Home {
        let dir = temp_dir(name);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        Home {
            dir,
            url: serve(remote),
        }
    }

    fn state(&self) -> FileState {
        FileState::new(self.dir.join("home/state/slice"))
    }

    /// Record `slice` as active, with its manifest.
    fn with_active(&self, slice: &ActiveSlice) {
        let state = self.state();
        state.set_active(slice).unwrap();
        state
            .write_manifest(&Manifest {
                id: slice.id.clone(),
                ticket: slice.ticket.clone(),
                ticket_system: TicketSystemKind::ClickUp,
                repos: slice.repos.clone(),
                status: ManifestStatus::Open,
                acceptance_criteria: vec![],
                invariants: vec![],
                spec_hash: None,
                test_hash: None,
                budgets: vec![],
                abandon_reason: None,
            })
            .unwrap();
    }

    /// Run slice in `cwd` with nothing from the environment but `PATH`.
    fn slice(&self, cwd: &Path, args: &[&str]) -> Output {
        let home = self.dir.join("home");
        Command::new(env!("CARGO_BIN_EXE_slice"))
            .args(args)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home)
            .env("XDG_STATE_HOME", home.join("state"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("HARVEST_ACCOUNT_ID", "1")
            .env("HARVEST_ACCESS_TOKEN", "token")
            .env("HARVEST_API_URL", &self.url)
            .env("HARVEST_CLICKUP_PREFIXES", "ENG")
            .env("PERSONAL_KEY", "token")
            .env("COMPANY_ID", "9")
            .env("CLICKUP_API_URL", &self.url)
            .current_dir(cwd)
            .output()
            .unwrap()
    }
}

/// stdout, after checking the run exited zero.
fn succeeded(out: &Output, what: &str) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{what}: stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

fn json_of(stdout: &str) -> Value {
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("not JSON: {e}: {stdout}"))
}

/// Each repository's path, branch and worktree appear in `text`, one
/// repository after another in the order they were added.
fn assert_lists_repos(text: &str, repos: &[SliceRepo]) {
    let mut from = 0;
    for r in repos {
        let mut end = from;
        for item in [
            r.repo.display().to_string(),
            r.branch.clone(),
            r.worktree.display().to_string(),
        ] {
            let at = text[from..]
                .find(&item)
                .unwrap_or_else(|| panic!("{item} missing after byte {from} of {text:?}"));
            end = end.max(from + at + item.len());
        }
        from = end;
    }
}

/// The slice's JSON has a `repos` array listing each repository's path,
/// branch and worktree in order.
fn assert_json_lists_repos(json: &Value, repos: &[SliceRepo]) {
    let got = json["repos"]
        .as_array()
        .unwrap_or_else(|| panic!("no repos array: {json}"));
    assert_eq!(got.len(), repos.len(), "{json}");
    for (got, want) in got.iter().zip(repos) {
        assert_eq!(got["repo"], want.repo.display().to_string(), "{json}");
        assert_eq!(got["branch"], want.branch, "{json}");
        assert_eq!(got["worktree"], want.worktree.display().to_string(), "{json}");
    }
}

/// A slice whose repositories need not exist: status only reads them.
fn invented_slice(n: usize) -> ActiveSlice {
    let repos = [
        ("/src/app", "/trees/one", "def5678"),
        ("/src/api", "/trees/two", "0api000"),
        ("/src/web", "/trees/three", "0web000"),
    ];
    ActiveSlice {
        id: SliceId("ENG-1-1".into()),
        ticket: TicketRef("ENG-1".into()),
        repos: repos[..n]
            .iter()
            .map(|(repo, worktree, base)| SliceRepo {
                repo: PathBuf::from(repo),
                branch: BRANCH.into(),
                worktree: PathBuf::from(worktree),
                base_commit: base.to_string(),
            })
            .collect(),
    }
}

/// Two real repositories, `app` with the slice's branch checked out in its
/// worktree and `api` with nothing for the ticket, and a home whose active
/// slice is in `app` with its timer running.
fn slice_in_app(name: &str) -> (Home, PathBuf, PathBuf, ActiveSlice) {
    let home = Home::new(name, Remote::with_entry(true));
    let app = repo(&home.dir, "app");
    let api = repo(&home.dir, "api");
    let worktree = home.dir.join("app.slices/feature-20261101-ENG-1-login");
    git(
        &app,
        &["worktree", "add", "-q", "-b", BRANCH, &worktree.to_string_lossy()],
    );
    let slice = ActiveSlice {
        id: SliceId("ENG-1-1".into()),
        ticket: TicketRef("ENG-1".into()),
        repos: vec![SliceRepo {
            repo: app.clone(),
            branch: BRANCH.into(),
            worktree,
            base_commit: git(&app, &["rev-parse", "HEAD"]),
        }],
    };
    home.with_active(&slice);
    (home, app, api, slice)
}

/// A worktree of `repo` on a branch of its own, outside the repository, with
/// a subdirectory.
fn scratch_worktree(repo: &Path) -> PathBuf {
    let name = repo.file_name().unwrap().to_string_lossy();
    let worktree = repo.parent().unwrap().join(format!("elsewhere/{name}-scratch"));
    git(
        repo,
        &["worktree", "add", "-q", "-b", "scratch", &worktree.to_string_lossy()],
    );
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    worktree
}

/// `api`, added to `slice`'s state: the last repository, on the slice's
/// branch, checked out in a worktree of `api` outside `scratch`.
fn assert_api_added(home: &Home, slice: &ActiveSlice, api: &Path, scratch: &Path) -> ActiveSlice {
    let now = home.state().active().unwrap().expect("still active");
    assert_eq!(now.id, slice.id);
    assert_eq!(now.repos.len(), 2, "{now:?}");
    assert_eq!(now.repos[0], slice.repos[0], "app kept");
    let added = &now.repos[1];
    assert_eq!(
        added.repo.canonicalize().unwrap(),
        api.canonicalize().unwrap(),
        "the repository, not its worktree"
    );
    assert_eq!(added.branch, BRANCH, "the slice's branch name");
    assert_eq!(added.base_commit, git(api, &["rev-parse", "HEAD"]));
    assert!(
        !added.worktree.starts_with(scratch),
        "worktree {} not inside the worktree it was run from",
        added.worktree.display()
    );
    assert_eq!(git(&added.worktree, &["branch", "--show-current"]), BRANCH);
    assert_eq!(
        PathBuf::from(git(
            &added.worktree,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"]
        ))
        .canonicalize()
        .unwrap(),
        api.join(".git").canonicalize().unwrap(),
        "a worktree of api"
    );
    let manifest = home.state().read_manifest(&slice.id).unwrap().expect("manifest");
    assert_eq!(manifest.repos, now.repos);
    now
}

// SLICE-10-8
#[test]
fn status_lists_every_repository() {
    for n in [1, 2, 3] {
        let home = Home::new(&format!("status-{n}"), Remote::with_entry(true));
        let slice = invented_slice(n);
        home.with_active(&slice);

        let text = succeeded(&home.slice(&home.dir, &["status"]), "status");
        assert_lists_repos(&text, &slice.repos);

        let json = json_of(&succeeded(
            &home.slice(&home.dir, &["status", "--json"]),
            "status --json",
        ));
        assert_json_lists_repos(&json["slice"], &slice.repos);
    }
}

// SLICE-10-8
#[test]
fn pause_and_resume_json_list_every_repository() {
    let home = Home::new("pause", Remote::with_entry(true));
    let slice = invented_slice(2);
    home.with_active(&slice);

    let json = json_of(&succeeded(&home.slice(&home.dir, &["pause", "--json"]), "pause"));
    assert_json_lists_repos(&json["slice"], &slice.repos);
    let json = json_of(&succeeded(&home.slice(&home.dir, &["resume", "--json"]), "resume"));
    assert_json_lists_repos(&json["slice"], &slice.repos);
}

// SLICE-10-8
#[test]
fn open_reports_the_new_slice() {
    for as_json in [false, true] {
        let home = Home::new(&format!("opened-{as_json}"), Remote::with_entry(false));
        let app = repo(&home.dir, "app");
        let mut args = vec!["open", "ENG-1", "--type", "feature"];
        if as_json {
            args.push("--json");
        }

        let stdout = succeeded(&home.slice(&app, &args), "open");

        let slice = home.state().active().unwrap().expect("opened");
        assert_eq!(slice.repos.len(), 1);
        assert_eq!(slice.repos[0].repo.canonicalize().unwrap(), app);
        if as_json {
            assert_json_lists_repos(&json_of(&stdout)["slice"], &slice.repos);
        } else {
            assert_lists_repos(&stdout, &slice.repos);
        }
    }
}

// SLICE-10-1
// SLICE-10-7
// SLICE-10-8
#[test]
fn open_in_a_worktree_adds_its_repository() {
    let (home, _app, api, slice) = slice_in_app("add-cwd");
    let scratch = scratch_worktree(&api);

    let text = succeeded(&home.slice(&scratch.join("src"), &["open", "ENG-1"]), "open");

    let now = assert_api_added(&home, &slice, &api, &scratch);
    assert_lists_repos(&text, &now.repos[1..]);
}

// SLICE-10-1
// SLICE-10-7
// SLICE-10-8
#[test]
fn open_with_repo_in_a_worktree_adds_its_repository() {
    let (home, _app, api, slice) = slice_in_app("add-flag");
    let scratch = scratch_worktree(&api);
    let dir = scratch.join("src").display().to_string();

    let stdout = succeeded(
        &home.slice(&home.dir, &["open", "ENG-1", "--repo", &dir, "--json"]),
        "open",
    );

    let now = assert_api_added(&home, &slice, &api, &scratch);
    assert_json_lists_repos(&json_of(&stdout)["slice"], &now.repos);
}

// SLICE-10-7
#[test]
fn open_in_a_worktree_finds_the_repositorys_branch() {
    let (home, _app, api, slice) = slice_in_app("add-existing");
    git(&api, &["branch", "bug/20261020-ENG-1-crash"]);
    let head = git(&api, &["rev-parse", "HEAD"]);
    let scratch = scratch_worktree(&api);

    succeeded(&home.slice(&scratch, &["open", "ENG-1"]), "open");

    let now = home.state().active().unwrap().expect("still active");
    assert_eq!(now.repos.len(), 2, "{now:?}");
    assert_eq!(now.repos[0], slice.repos[0]);
    let added = &now.repos[1];
    assert_eq!(added.repo.canonicalize().unwrap(), api.canonicalize().unwrap());
    assert_eq!(
        added.branch, "bug/20261020-ENG-1-crash",
        "api's own branch for the ticket"
    );
    assert_eq!(added.base_commit, head);
    assert!(!added.worktree.starts_with(&scratch), "{}", added.worktree.display());
    assert_eq!(
        git(&added.worktree, &["branch", "--show-current"]),
        "bug/20261020-ENG-1-crash"
    );
    assert!(
        git(&api, &["branch", "--list", "feature/*"]).is_empty(),
        "no branch created in api"
    );
}

// SLICE-10-7
#[test]
fn open_in_a_worktree_of_a_repository_in_the_slice() {
    let (home, app, _api, slice) = slice_in_app("present");
    let scratch = scratch_worktree(&app);
    let dirs = [slice.repos[0].worktree.clone(), scratch.clone(), scratch.join("src")];
    let branches = git(&app, &["branch", "--list"]);
    let worktrees = git(&app, &["worktree", "list", "--porcelain"]);

    for dir in &dirs {
        succeeded(&home.slice(dir, &["open", "ENG-1"]), &dir.display().to_string());
        let flag = dir.display().to_string();
        succeeded(&home.slice(&home.dir, &["open", "ENG-1", "--repo", &flag]), &flag);

        assert_eq!(
            home.state().active().unwrap(),
            Some(slice.clone()),
            "{}: slice unchanged",
            dir.display()
        );
        let manifest = home.state().read_manifest(&slice.id).unwrap().expect("manifest");
        assert_eq!(manifest.repos, slice.repos, "{}: manifest unchanged", dir.display());
        assert_eq!(git(&app, &["branch", "--list"]), branches, "{}", dir.display());
        assert_eq!(
            git(&app, &["worktree", "list", "--porcelain"]),
            worktrees,
            "{}",
            dir.display()
        );
    }
}
