mod config;
mod git;

use std::io::{self, BufRead, Write};
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
    /// Merge a super branch's configured branches into the currently checked out branches
    Merge {
        /// Name of the super branch defined in gitman.toml
        name: String,
    },
    /// Print the active gitman.toml
    List,
    /// Open the active gitman.toml in the default editor ($VISUAL/$EDITOR, falls back to vi)
    Edit,
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
    /// Replace an existing super branch with the currently checked out branches
    Update {
        /// Name of the super branch defined in gitman.toml
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
    match &cli.command {
        // `build` may create gitman.toml, so a missing file is fine there.
        Command::Build { name } => {
            let config = Config::load_or_default(&cli.dir)?;
            return cmd_build(&cli.dir, &config, name);
        }
        // `list` and `edit` work on the raw file, so they must not require a
        // successful parse (`edit` is how you fix a file that no longer parses).
        Command::List => return cmd_list(&cli.dir),
        Command::Edit => return cmd_edit(&cli.dir),
        _ => {}
    }

    let config = Config::load(&cli.dir)?;
    match &cli.command {
        Command::Checkout { name } => cmd_checkout(&cli.dir, &config, name),
        Command::Pull { name } => cmd_sync(&cli.dir, &config, name, SyncAction::Pull),
        Command::Push { name } => cmd_sync(&cli.dir, &config, name, SyncAction::Push),
        Command::Merge { name } => cmd_merge(&cli.dir, &config, name),
        Command::Status { name } => cmd_status(&cli.dir, &config, name.as_deref()),
        Command::Update { name } => cmd_update(&cli.dir, &config, name),
        Command::List | Command::Edit | Command::Build { .. } => unreachable!("handled above"),
    }
}

fn cmd_list(root: &Path) -> Result<bool> {
    let path = root.join(config::CONFIG_FILE);
    let content = std::fs::read_to_string(&path)
        .map_err(|err| anyhow::anyhow!("cannot read config file {}: {err}", path.display()))?;
    let color = use_color();
    for line in content.lines() {
        if color {
            println!("{}", highlight_toml_line(line));
        } else {
            println!("{line}");
        }
    }
    Ok(true)
}

/// Color only when stdout is a terminal and NO_COLOR (https://no-color.org) is unset.
fn use_color() -> bool {
    use std::io::IsTerminal;
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const BOLD_CYAN: &str = "\x1b[1;36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";

/// Minimal TOML syntax highlighting: comments dim, table headers bold cyan,
/// keys green, values yellow. Anything unrecognized passes through unchanged.
fn highlight_toml_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    if trimmed.starts_with('#') {
        return format!("{indent}{DIM}{trimmed}{RESET}");
    }
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return format!("{indent}{BOLD_CYAN}{trimmed}{RESET}");
    }
    if let Some(eq) = find_unquoted(trimmed, '=') {
        let (key, rest) = trimmed.split_at(eq);
        let value = &rest[1..];
        return format!("{indent}{GREEN}{key}{RESET}={YELLOW}{value}{RESET}");
    }
    line.to_string()
}

/// Byte index of the first `needle` outside a double-quoted string,
/// so `"a=b" = "x"` splits at the right '='.
fn find_unquoted(line: &str, needle: char) -> Option<usize> {
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            c if c == needle && !in_string => return Some(i),
            _ => {}
        }
    }
    None
}

fn cmd_edit(root: &Path) -> Result<bool> {
    let path = root.join(config::CONFIG_FILE);
    if !path.exists() {
        bail!(
            "config file not found: {} (run 'gitman build <name>' to create one)",
            path.display()
        );
    }

    // $VISUAL, then $EDITOR, then vi — the same fallback order git uses.
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "vi".to_string());

    // The value may contain arguments (e.g. EDITOR="code --wait"), so run it
    // through the shell; the path goes in as "$1" to survive spaces.
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg(&editor)
        .arg(&path)
        .status()
        .map_err(|err| anyhow::anyhow!("cannot run editor '{editor}': {err}"))?;
    if !status.success() {
        bail!("editor '{editor}' exited with {status}");
    }

    // Warn (but keep the edit) if the file no longer parses.
    if let Err(err) = Config::load(root) {
        eprintln!("warning: {err:#}");
        return Ok(false);
    }
    Ok(true)
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

fn cmd_merge(root: &Path, config: &Config, name: &str) -> Result<bool> {
    let repos = config.superbranch(name)?;
    let mut ok = true;

    // Resolve the plan: what gets merged into what, per repo.
    struct Plan {
        repo: String,
        branch: String,  // configured branch (merge source)
        current: String, // checked out branch (merge target)
    }
    let mut plans: Vec<Plan> = Vec::new();
    for (repo, branch) in repos {
        let state = repo_path(root, repo).and_then(|path| {
            let current = git::current_branch(&path)?;
            let dirty = git::is_dirty(&path)?;
            Ok((current, dirty))
        });
        match state {
            Ok((None, _)) => println!("[{repo}] detached HEAD, skipping"),
            Ok((Some(current), _)) if current == *branch => {
                println!("[{repo}] already on '{branch}', skipping");
            }
            Ok((Some(current), dirty)) => {
                let dirty_mark = if dirty { " (dirty!)" } else { "" };
                println!("[{repo}] merge '{branch}' into '{current}'{dirty_mark}");
                plans.push(Plan {
                    repo: repo.clone(),
                    branch: branch.clone(),
                    current,
                });
            }
            Err(err) => {
                eprintln!("[{repo}] error: {err}");
                ok = false;
            }
        }
    }
    if plans.is_empty() {
        println!("nothing to merge");
        return Ok(ok);
    }

    // Explicit confirmation: only the literal word "yes" proceeds.
    print!("\nType 'yes' to merge, anything else aborts: ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    if answer.trim() != "yes" {
        println!("aborted, no changes made");
        return Ok(false);
    }

    println!();
    for plan in &plans {
        let Plan { repo, branch, current } = plan;
        let path = root.join(repo);
        match git::merge(&path, branch)? {
            git::MergeOutcome::Merged => println!("[{repo}] merged '{branch}' into '{current}'"),
            git::MergeOutcome::UpToDate => println!("[{repo}] already up to date"),
            git::MergeOutcome::Conflict => {
                println!("[{repo}] CONFLICT merging '{branch}' — resolve, then 'git add' and 'git commit':");
                match git::conflicted_files(&path) {
                    Ok(files) => {
                        for file in files {
                            println!("    {file}");
                        }
                    }
                    Err(err) => eprintln!("[{repo}] error listing conflicts: {err}"),
                }
                ok = false;
            }
            git::MergeOutcome::Failed(msg) => {
                eprintln!("[{repo}] failed: {msg}");
                ok = false;
            }
        }
    }
    Ok(ok)
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
        bail!(
            "super branch '{name}' already exists in {} (use 'gitman update {name}' to overwrite it)",
            config::CONFIG_FILE
        );
    }

    let entries = snapshot_branches(root)?;
    config::append_superbranch(root, name, &entries)?;
    println!("added super branch '{name}' to {}:", config::CONFIG_FILE);
    for (repo, branch) in &entries {
        println!("  {repo} -> {branch}");
    }
    Ok(true)
}

fn cmd_update(root: &Path, config: &Config, name: &str) -> Result<bool> {
    if !config.superbranches.contains_key(name) {
        bail!(
            "unknown super branch '{name}' (use 'gitman build {name}' to create it)"
        );
    }

    let entries = snapshot_branches(root)?;
    config::update_superbranch(root, name, &entries)?;
    println!("updated super branch '{name}' in {}:", config::CONFIG_FILE);
    for (repo, branch) in &entries {
        println!("  {repo} -> {branch}");
    }
    Ok(true)
}

/// The currently checked out branch of every git repo directly under `root`.
/// Repos with a detached HEAD are skipped with a warning.
fn snapshot_branches(root: &Path) -> Result<Vec<(String, String)>> {
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
    Ok(entries)
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
