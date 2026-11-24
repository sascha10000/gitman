use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Run a git command inside `repo` and return its stdout verbatim.
/// Fails with git's stderr if the command exits non-zero.
fn run_raw(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .context("failed to spawn git — is git installed and on the PATH?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Like `run_raw`, but with trimmed stdout — for single-value outputs.
fn run(repo: &Path, args: &[&str]) -> Result<String> {
    run_raw(repo, args).map(|stdout| stdout.trim().to_string())
}

pub fn is_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// The current branch name, or None when HEAD is detached.
pub fn current_branch(repo: &Path) -> Result<Option<String>> {
    match run(repo, &["symbolic-ref", "--short", "-q", "HEAD"]) {
        Ok(branch) if !branch.is_empty() => Ok(Some(branch)),
        _ => Ok(None),
    }
}

/// The current branch name, or the commit SHA when HEAD is detached.
/// Restoring either later is a plain `git checkout <ref>`.
pub fn current_ref(repo: &Path) -> Result<String> {
    match current_branch(repo)? {
        Some(branch) => Ok(branch),
        None => run(repo, &["rev-parse", "HEAD"]).context("cannot determine current branch"),
    }
}

/// True when the working tree has uncommitted changes (staged or unstaged).
pub fn is_dirty(repo: &Path) -> Result<bool> {
    Ok(!run(repo, &["status", "--porcelain"])?.is_empty())
}

/// One entry of `git status --porcelain`: the two-letter XY code plus paths.
pub struct FileChange {
    /// X: state in the index (staged).
    pub index: char,
    /// Y: state in the working tree (unstaged).
    pub worktree: char,
    pub path: String,
    /// Source path of a rename or copy.
    pub orig: Option<String>,
}

impl FileChange {
    pub fn is_untracked(&self) -> bool {
        self.index == '?'
    }

    /// Unmerged paths: either side 'U', or both sides added / deleted.
    pub fn is_conflict(&self) -> bool {
        matches!(
            (self.index, self.worktree),
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D')
        )
    }
}

/// Every pending change in the working tree, untracked files included.
pub fn changed_files(repo: &Path) -> Result<Vec<FileChange>> {
    // -z: NUL-separated, unquoted paths; a rename/copy entry is followed by its source.
    // run_raw: the leading space of a ` M file` entry is the staged/unstaged marker.
    let stdout = run_raw(repo, &["status", "--porcelain", "-z"])?;
    let mut tokens = stdout.split('\0').filter(|t| !t.is_empty());
    let mut changes = Vec::new();
    while let Some(entry) = tokens.next() {
        let mut status = entry.chars();
        let index = status.next().unwrap_or(' ');
        let worktree = status.next().unwrap_or(' ');
        let path = entry.get(3..).unwrap_or_default().to_string();
        let orig = if matches!(index, 'R' | 'C') {
            tokens.next().map(str::to_string)
        } else {
            None
        };
        changes.push(FileChange { index, worktree, path, orig });
    }
    Ok(changes)
}

/// Counts of (staged, unstaged) pending changes; untracked files count as unstaged.
pub fn pending_changes(repo: &Path) -> Result<(usize, usize)> {
    let mut staged = 0;
    let mut unstaged = 0;
    for change in changed_files(repo)? {
        if change.is_untracked() {
            unstaged += 1;
            continue;
        }
        if change.index != ' ' {
            staged += 1;
        }
        if change.worktree != ' ' {
            unstaged += 1;
        }
    }
    Ok((staged, unstaged))
}

pub fn checkout(repo: &Path, branch: &str) -> Result<()> {
    run(repo, &["checkout", branch]).map(|_| ())
}

/// True when a local branch `name` exists. Only fails when git cannot be spawned.
pub fn branch_exists(repo: &Path, name: &str) -> Result<bool> {
    // `show-ref --verify --quiet` reports "missing" through its exit code,
    // which `run` would turn into an error — so check the status directly.
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", &format!("refs/heads/{name}")])
        .status()
        .context("failed to spawn git — is git installed and on the PATH?")?;
    Ok(status.success())
}

/// Create branch `name` at `start` and check it out.
pub fn create_branch(repo: &Path, name: &str, start: &str) -> Result<()> {
    run(repo, &["checkout", "-b", name, start]).map(|_| ())
}

pub fn pull(repo: &Path) -> Result<()> {
    run(repo, &["pull", "--ff-only"]).map(|_| ())
}

/// Outcome of `git merge <branch>` — a conflict is an expected, reportable
/// state, not an error to propagate.
pub enum MergeOutcome {
    /// A merge commit or fast-forward was created.
    Merged,
    /// Nothing to merge.
    UpToDate,
    /// The merge started but left unmerged paths to resolve.
    Conflict,
    /// git refused to merge (dirty tree, unknown ref, ...) — carries stderr.
    Failed(String),
}

/// Merge `branch` into the currently checked out branch.
/// Only fails when git itself cannot be spawned.
pub fn merge(repo: &Path, branch: &str) -> Result<MergeOutcome> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge", branch])
        .output()
        .context("failed to spawn git — is git installed and on the PATH?")?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("Already up to date") {
            return Ok(MergeOutcome::UpToDate);
        }
        return Ok(MergeOutcome::Merged);
    }

    // Non-zero exit: unmerged paths mean a conflict, anything else a refusal.
    if !run(repo, &["ls-files", "-u"])?.is_empty() {
        return Ok(MergeOutcome::Conflict);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(MergeOutcome::Failed(stderr.trim().to_string()))
}

/// Paths that currently have merge conflicts.
pub fn conflicted_files(repo: &Path) -> Result<Vec<String>> {
    let stdout = run(repo, &["diff", "--name-only", "--diff-filter=U"])?;
    Ok(stdout.lines().map(str::to_string).collect())
}

pub fn push(repo: &Path) -> Result<()> {
    run(repo, &["push"]).map(|_| ())
}
