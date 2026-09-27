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

/// Counts of (staged, unstaged) pending changes; untracked files count as unstaged.
pub fn pending_changes(repo: &Path) -> Result<(usize, usize)> {
    // run_raw: the leading space of a ` M file` line is the staged/unstaged marker.
    let stdout = run_raw(repo, &["status", "--porcelain"])?;
    let mut staged = 0;
    let mut unstaged = 0;
    for line in stdout.lines() {
        let mut status = line.chars();
        let index = status.next().unwrap_or(' ');
        let worktree = status.next().unwrap_or(' ');
        if index == '?' {
            unstaged += 1;
            continue;
        }
        if index != ' ' {
            staged += 1;
        }
        if worktree != ' ' {
            unstaged += 1;
        }
    }
    Ok((staged, unstaged))
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
