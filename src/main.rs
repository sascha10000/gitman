mod config;
mod git;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use config::Config;

#[derive(Parser)]
#[command(
    name = "gitman",
    version,
    about = "Manage multiple git repositories with super branches",
    long_about = "Manage multiple git repositories that live in the same parent directory.\n\
                  A gitman.toml in that directory defines super branches: named sets of\n\
                  (repository, branch) pairs. Repositories not listed in a super branch\n\
                  are left untouched.\n\n\
                  Example gitman.toml:\n\n  \
                  [superbranches.feature-login]\n  \
                  auth-service = \"feature/login\"\n  \
                  web-frontend = \"feature/login-ui\""
)]
struct Cli {
    /// Parent directory containing the repositories and gitman.toml
    #[arg(short = 'C', long = "dir", default_value = ".", global = true)]
    dir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Checkout the configured branch in every repo of a super branch
    Checkout {
        /// Name of the super branch defined in gitman.toml
        name: String,
    },
    /// Checkout the configured branch, pull, then restore the previous branch
    Pull {
        /// Name of the super branch defined in gitman.toml
        name: String,
    },
    /// Checkout the configured branch, push, then restore the previous branch
    Push {
        /// Name of the super branch defined in gitman.toml
        name: String,
    },
    /// List the super branches defined in gitman.toml
    List,
    /// Show the current branch of every repo, or compare against a super branch
    Status {
        /// Optional super branch to compare the checked out branches against
        name: Option<String>,
    },
    /// Create a super branch from the currently checked out branches and append it to gitman.toml
    Build {
        /// Name for the new super branch
        name: String,
    },
}

/// What to run in a repo after checking out its configured branch.
#[derive(Clone, Copy)]
enum SyncAction {
    Pull,
    Push,
}

impl SyncAction {
    fn run(self, repo: &Path) -> Result<()> {
        match self {
            SyncAction::Pull => git::pull(repo),
            SyncAction::Push => git::push(repo),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Returns Ok(false) when individual repos failed but execution continued.
fn run(cli: &Cli) -> Result<bool> {
    // `build` may create gitman.toml, so a missing file is fine there.
    if let Command::Build { name } = &cli.command {
        let config = Config::load_or_default(&cli.dir)?;
        return cmd_build(&cli.dir, &config, name);
    }

    let config = Config::load(&cli.dir)?;
    match &cli.command {
        Command::Checkout { name } => cmd_checkout(&cli.dir, &config, name),
        Command::Pull { name } => cmd_sync(&cli.dir, &config, name, SyncAction::Pull),
        Command::Push { name } => cmd_sync(&cli.dir, &config, name, SyncAction::Push),
        Command::List => {
            cmd_list(&config);
            Ok(true)
        }
        Command::Status { name } => cmd_status(&cli.dir, &config, name.as_deref()),
        Command::Build { .. } => unreachable!("handled above"),
    }
}

fn cmd_list(config: &Config) {
    if config.superbranches.is_empty() {
        println!("no super branches defined in {}", config::CONFIG_FILE);
        return;
    }
    for (name, repos) in &config.superbranches {
        println!("{name}");
        for (repo, branch) in repos {
            println!("  {repo} -> {branch}");
        }
    }
}

fn cmd_checkout(root: &Path, config: &Config, name: &str) -> Result<bool> {
    let repos = config.superbranch(name)?;
    let mut ok = true;
    for (repo, branch) in repos {
        let result = repo_path(root, repo).and_then(|path| git::checkout(&path, branch));
        match result {
            Ok(()) => println!("[{repo}] checked out '{branch}'"),
            Err(err) => {
                eprintln!("[{repo}] error: {err}");
                ok = false;
            }
        }
    }
    Ok(ok)
}

fn cmd_sync(root: &Path, config: &Config, name: &str, action: SyncAction) -> Result<bool> {
    let repos = config.superbranch(name)?;
    let mut ok = true;
    for (repo, branch) in repos {
        if let Err(err) = sync_repo(root, repo, branch, action) {
            eprintln!("[{repo}] error: {err}");
            ok = false;
        }
    }
    Ok(ok)
}

/// Checkout the configured branch, run pull/push, then restore whatever
/// branch (or detached commit) was checked out before.
fn sync_repo(root: &Path, repo: &str, branch: &str, action: SyncAction) -> Result<()> {
    let path = repo_path(root, repo)?;
    let previous = git::current_ref(&path)?;

    git::checkout(&path, branch)?;
    let result = action.run(&path);

    if previous != *branch {
        if let Err(err) = git::checkout(&path, &previous) {
            // Report the restore failure, but don't let it mask a pull/push error.
            eprintln!("[{repo}] warning: could not restore '{previous}': {err}");
        }
    }

    result?;
    match action {
        SyncAction::Pull => println!("[{repo}] pulled '{branch}' (back on '{previous}')"),
        SyncAction::Push => println!("[{repo}] pushed '{branch}' (back on '{previous}')"),
    }
    Ok(())
}

fn cmd_status(root: &Path, config: &Config, name: Option<&str>) -> Result<bool> {
    let mut ok = true;
    match name {
        // Compare the listed repos against the super branch definition.
        Some(name) => {
            let repos = config.superbranch(name)?;
            for (repo, expected) in repos {
                let state = repo_path(root, repo).and_then(|path| {
                    let branch = git::current_ref(&path)?;
                    let dirty = git::is_dirty(&path)?;
                    Ok((branch, dirty))
                });
                match state {
                    Ok((branch, dirty)) => {
                        let dirty_mark = if dirty { ", dirty" } else { "" };
                        if branch == *expected {
                            println!("[{repo}] {branch} (matches{dirty_mark})");
                        } else {
                            println!("[{repo}] {branch} (expected '{expected}'{dirty_mark})");
                            ok = false;
                        }
                    }
                    Err(err) => {
                        eprintln!("[{repo}] error: {err}");
                        ok = false;
                    }
                }
            }
        }
        // No super branch given: show every git repo in the parent directory.
        None => {
            for (repo, path) in discover_repos(root)? {
                match git::current_ref(&path) {
                    Ok(branch) => {
                        let dirty = git::is_dirty(&path).unwrap_or(false);
                        let dirty_mark = if dirty { " (dirty)" } else { "" };
                        println!("[{repo}] {branch}{dirty_mark}");
                    }
                    Err(err) => {
                        eprintln!("[{repo}] error: {err}");
                        ok = false;
                    }
                }
            }
        }
    }
    Ok(ok)
}

fn cmd_build(root: &Path, config: &Config, name: &str) -> Result<bool> {
    if config.superbranches.contains_key(name) {
        bail!("super branch '{name}' already exists in {}", config::CONFIG_FILE);
    }

    let mut entries = Vec::new();
    for (repo, path) in discover_repos(root)? {
        match git::current_branch(&path)? {
            Some(branch) => entries.push((repo, branch)),
            None => eprintln!("[{repo}] skipped: detached HEAD, no branch to record"),
        }
    }
    if entries.is_empty() {
        bail!("no git repositories with a checked out branch found in {}", root.display());
    }

    config::append_superbranch(root, name, &entries)?;
    println!("added super branch '{name}' to {}:", config::CONFIG_FILE);
    for (repo, branch) in &entries {
        println!("  {repo} -> {branch}");
    }
    Ok(true)
}

/// All direct subdirectories of `root` that are git repositories, sorted by name.
fn discover_repos(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut repos = Vec::new();
    let dir = std::fs::read_dir(root)
        .map_err(|err| anyhow::anyhow!("cannot read directory {}: {err}", root.display()))?;
    for entry in dir {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && git::is_repo(&path) {
            repos.push((entry.file_name().to_string_lossy().into_owned(), path));
        }
    }
    repos.sort();
    Ok(repos)
}

fn repo_path(root: &Path, repo: &str) -> Result<PathBuf> {
    let path = root.join(repo);
    if !path.is_dir() {
        bail!("directory not found: {}", path.display());
    }
    if !git::is_repo(&path) {
        bail!("not a git repository: {}", path.display());
    }
    Ok(path)
}
