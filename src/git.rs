use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Run a git command inside `repo` and return its trimmed stdout.
/// Fails with git's stderr if the command exits non-zero.
fn run(repo: &Path, args: &[&str]) -> Result<String> {
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
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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

pub fn checkout(repo: &Path, branch: &str) -> Result<()> {
    run(repo, &["checkout", branch]).map(|_| ())
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
