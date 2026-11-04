//! `StateStore` on the local filesystem: the active slice and per-ticket slice
//! counts as JSON under `$XDG_STATE_HOME/slice` (default
//! `~/.local/state/slice`), and manifests as YAML in `manifests/` there.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::home;
use crate::ports::{
    ActiveSlice, Manifest, ManifestStatus, PortError, PortResult, SliceId, Source, StateStore, TicketRef,
    TicketSystemKind,
};

pub struct FileState {
    dir: PathBuf,
}

fn err(path: &Path, e: impl std::fmt::Display) -> PortError {
    PortError {
        source: Source::LocalState,
        message: format!("{}: {e}", path.display()),
    }
}

/// The on-disk form of [`ActiveSlice`].
#[derive(Serialize, Deserialize)]
struct ActiveFile {
    id: String,
    ticket: String,
    repo: PathBuf,
    branch: String,
    worktree: PathBuf,
}

impl FileState {
    pub fn new(dir: PathBuf) -> FileState {
        FileState { dir }
    }

    pub fn default_dir() -> PathBuf {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/state"));
        base.join("slice")
    }

    fn active_path(&self) -> PathBuf {
        self.dir.join("active.json")
    }

    fn manifest_path(&self, id: &SliceId) -> PathBuf {
        self.dir.join("manifests").join(format!("{}.yaml", id.0))
    }

    fn counts_path(&self) -> PathBuf {
        self.dir.join("counts.json")
    }

    fn counts(&self) -> PortResult<BTreeMap<String, u32>> {
        let path = self.counts_path();
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| err(&path, e)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(err(&path, e)),
        }
    }

    fn write_json(&self, path: &Path, value: &impl Serialize) -> PortResult<()> {
        fs::create_dir_all(&self.dir).map_err(|e| err(&self.dir, e))?;
        let text = serde_json::to_string_pretty(value).map_err(|e| err(path, e))?;
        // Write then rename, so a crash never leaves half a file behind.
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, text + "\n").map_err(|e| err(&tmp, e))?;
        fs::rename(&tmp, path).map_err(|e| err(path, e))
    }
}

impl StateStore for FileState {
    fn active(&self) -> PortResult<Option<ActiveSlice>> {
        let path = self.active_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(err(&path, e)),
        };
        let f: ActiveFile = serde_json::from_str(&text).map_err(|e| err(&path, e))?;
        Ok(Some(ActiveSlice {
            id: SliceId(f.id),
            ticket: TicketRef(f.ticket),
            repo: f.repo,
            branch: f.branch,
            worktree: f.worktree,
        }))
    }

    fn next_slice_id(&self, ticket: &TicketRef) -> PortResult<SliceId> {
        let n = self.counts()?.get(&ticket.0).copied().unwrap_or(0) + 1;
        Ok(SliceId(format!("{}-{n}", ticket.0)))
    }

    fn set_active(&self, slice: &ActiveSlice) -> PortResult<()> {
        let mut counts = self.counts()?;
        let n: u32 = slice.id.0.rsplit('-').next().and_then(|n| n.parse().ok()).unwrap_or(0);
        let count = counts.entry(slice.ticket.0.clone()).or_insert(0);
        *count = (*count).max(n);
        self.write_json(&self.counts_path(), &counts)?;
        let f = ActiveFile {
            id: slice.id.0.clone(),
            ticket: slice.ticket.0.clone(),
            repo: slice.repo.clone(),
            branch: slice.branch.clone(),
            worktree: slice.worktree.clone(),
        };
        self.write_json(&self.active_path(), &f)
    }

    fn clear_active(&self) -> PortResult<()> {
        let path = self.active_path();
        match fs::remove_file(&path) {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(err(&path, e)),
            _ => Ok(()),
        }
    }

    fn read_manifest(&self, id: &SliceId) -> PortResult<Option<Manifest>> {
        let path = self.manifest_path(id);
        match fs::read_to_string(&path) {
            Ok(text) => parse_manifest_yaml(&text).map(Some).map_err(|e| err(&path, e)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(err(&path, e)),
        }
    }

    fn write_manifest(&self, manifest: &Manifest) -> PortResult<()> {
        let path = self.manifest_path(&manifest.id);
        let dir = self.dir.join("manifests");
        fs::create_dir_all(&dir).map_err(|e| err(&dir, e))?;
        let tmp = path.with_extension("yaml.tmp");
        fs::write(&tmp, manifest_yaml(manifest)).map_err(|e| err(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| err(&path, e))
    }

    fn remove_manifest(&self, id: &SliceId) -> PortResult<()> {
        let path = self.manifest_path(id);
        match fs::remove_file(&path) {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(err(&path, e)),
            _ => Ok(()),
        }
    }
}

/// The manifest as YAML. Values are JSON-quoted strings, which YAML accepts.
pub fn manifest_yaml(m: &Manifest) -> String {
    let q = |s: &str| serde_json::to_string(s).expect("string serializes");
    let list = |items: &[String]| serde_json::to_string(items).expect("strings serialize");
    let opt = |o: &Option<String>| o.as_deref().map(q).unwrap_or_else(|| "null".to_string());
    let system = match m.ticket_system {
        TicketSystemKind::ClickUp => "clickup",
        TicketSystemKind::Monday => "monday",
    };
    let status = match m.status {
        ManifestStatus::Open => "open",
        ManifestStatus::Closed => "closed",
        ManifestStatus::Abandoned => "abandoned",
    };
    let budgets = if m.budgets.is_empty() {
        " {}".to_string()
    } else {
        m.budgets
            .iter()
            .map(|(k, v)| format!("\n  {}: {}", q(k), q(v)))
            .collect()
    };
    format!(
        "id: {}\nticket: {}\nticket_system: {system}\nbranch: {}\nbase_commit: {}\nstatus: {status}\nacceptance_criteria: {}\ninvariants: {}\nspec_hash: {}\ntest_hash: {}\nbudgets:{budgets}\n",
        q(&m.id.0),
        q(&m.ticket.0),
        q(&m.branch),
        q(&m.base_commit),
        list(&m.acceptance_criteria),
        list(&m.invariants),
        opt(&m.spec_hash),
        opt(&m.test_hash),
    )
}

/// Read back what [`manifest_yaml`] writes. Not a general YAML parser: each
/// top-level line is `key: value` with a JSON value or a bare word, and
/// budgets are indented `"key": "value"` lines.
pub fn parse_manifest_yaml(text: &str) -> Result<Manifest, String> {
    let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
    let mut budgets = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if let Some(entry) = line.strip_prefix("  ") {
            let (k, v) = entry
                .split_once(": ")
                .ok_or_else(|| format!("bad budget line {line:?}"))?;
            budgets.push((json::<String>(k)?, json::<String>(v)?));
        } else {
            let (k, v) = line.split_once(':').ok_or_else(|| format!("bad line {line:?}"))?;
            fields.insert(k, v.trim());
        }
    }
    let field = |k: &str| fields.get(k).copied().ok_or_else(|| format!("missing {k}"));
    let ticket_system = match field("ticket_system")? {
        "clickup" => TicketSystemKind::ClickUp,
        "monday" => TicketSystemKind::Monday,
        other => return Err(format!("unknown ticket_system {other}")),
    };
    let status = match field("status")? {
        "open" => ManifestStatus::Open,
        "closed" => ManifestStatus::Closed,
        "abandoned" => ManifestStatus::Abandoned,
        other => return Err(format!("unknown status {other}")),
    };
    Ok(Manifest {
        id: SliceId(json(field("id")?)?),
        ticket: TicketRef(json(field("ticket")?)?),
        ticket_system,
        branch: json(field("branch")?)?,
        base_commit: json(field("base_commit")?)?,
        status,
        acceptance_criteria: json(field("acceptance_criteria")?)?,
        invariants: json(field("invariants")?)?,
        spec_hash: json(field("spec_hash")?)?,
        test_hash: json(field("test_hash")?)?,
        budgets,
        abandon_reason: None,
    })
}

fn json<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, String> {
    serde_json::from_str(s).map_err(|e| format!("{s}: {e}"))
}
