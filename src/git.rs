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

pub fn push(repo: &Path) -> Result<()> {
    run(repo, &["push"]).map(|_| ())
}
