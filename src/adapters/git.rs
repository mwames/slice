//! `Vcs` over the git CLI.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ports::{PortError, PortResult, Source, Vcs};

pub struct Git;

fn err(message: impl Into<String>) -> PortError {
    PortError {
        source: Source::Git,
        message: message.into(),
    }
}

/// Run git in `repo` and return trimmed stdout, or stderr as the error.
fn git(repo: &Path, args: &[&str]) -> PortResult<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| err(format!("run git: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(err(format!("git {}: {stderr}", args.join(" "))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Git {
    /// The top level of the repository containing `dir`.
    pub fn toplevel(dir: &Path) -> PortResult<PathBuf> {
        git(dir, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
    }

    /// The main working tree of the repository containing `dir`, which may
    /// be inside one of its worktrees (SLICE-10-7).
    pub fn repository(dir: &Path) -> PortResult<PathBuf> {
        // The first worktree listed is the main one, from any of them.
        let out = git(dir, &["worktree", "list", "--porcelain"])?;
        out.lines()
            .find_map(|line| line.strip_prefix("worktree "))
            .map(PathBuf::from)
            .ok_or_else(|| err(format!("no worktree listed for {}", dir.display())))
    }
}

impl Vcs for Git {
    fn resolve(&self, repo: &Path, rev: &str) -> PortResult<String> {
        git(
            repo,
            &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")],
        )
        .map_err(|_| err(format!("{rev} is not a commit in {}", repo.display())))
    }

    fn create_branch(&self, repo: &Path, branch: &str, path: &Path, base: &str) -> PortResult<()> {
        let path = path.to_string_lossy();
        let branch_ref = format!("refs/heads/{branch}");
        let existed = git(repo, &["rev-parse", "--verify", "--quiet", &branch_ref]).is_ok();
        let result = git(repo, &["worktree", "add", "--no-track", "-b", branch, &path, base]).map(|_| ());
        // `worktree add -b` creates the branch before checking the path, so a
        // failure can leave a branch behind. Remove it unless it was already there.
        if result.is_err() && !existed && git(repo, &["rev-parse", "--verify", "--quiet", &branch_ref]).is_ok() {
            let _ = git(repo, &["branch", "-D", branch]);
        }
        result
    }

    fn branches(&self, repo: &Path) -> PortResult<Vec<String>> {
        let out = git(repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads/"])?;
        Ok(out.lines().map(str::to_string).collect())
    }

    fn worktree_of(&self, repo: &Path, branch: &str) -> PortResult<Option<PathBuf>> {
        // Porcelain output is blocks of "worktree <path>" ... "branch refs/heads/<name>".
        let out = git(repo, &["worktree", "list", "--porcelain"])?;
        let wanted = format!("branch refs/heads/{branch}");
        let mut path = None;
        for line in out.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if line == wanted {
                return Ok(path);
            }
        }
        Ok(None)
    }

    fn add_worktree(&self, repo: &Path, branch: &str, path: &Path) -> PortResult<()> {
        let path = path.to_string_lossy();
        git(repo, &["worktree", "add", &path, branch]).map(|_| ())
    }

    fn remove_worktree(&self, repo: &Path, path: &Path) -> PortResult<()> {
        let path = path.to_string_lossy();
        git(repo, &["worktree", "remove", "--force", &path]).map(|_| ())
    }

    fn remove_branch(&self, repo: &Path, branch: &str, path: &Path) -> PortResult<()> {
        let path = path.to_string_lossy();
        git(repo, &["worktree", "remove", "--force", &path])?;
        git(repo, &["branch", "-D", branch]).map(|_| ())
    }
}
