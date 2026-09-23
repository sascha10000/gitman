use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

pub const CONFIG_FILE: &str = "gitman.toml";

/// The gitman.toml file, located in the parent directory of the repositories.
///
/// ```toml
/// [superbranches.feature-login]
/// auth-service = "feature/login"
/// web-frontend = "feature/login-ui"
///
/// [superbranches.hotfix-42]
/// auth-service = "hotfix/42"
/// ```
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// super branch name -> (repo directory name -> branch name)
    #[serde(default)]
    pub superbranches: BTreeMap<String, BTreeMap<String, String>>,
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(CONFIG_FILE);
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read config file {}", path.display()))?;
        let config: Config = toml::from_str(&raw)
            .with_context(|| format!("invalid config file {}", path.display()))?;
        Ok(config)
    }

    /// Like `load`, but a missing file yields an empty config
    /// (used by `build`, which may create the file).
    pub fn load_or_default(root: &Path) -> Result<Self> {
        if root.join(CONFIG_FILE).exists() {
            Self::load(root)
        } else {
            Ok(Config {
                superbranches: BTreeMap::new(),
            })
        }
    }

    /// Look up a super branch by name, with a helpful error listing the known ones.
    pub fn superbranch(&self, name: &str) -> Result<&BTreeMap<String, String>> {
        match self.superbranches.get(name) {
            Some(repos) if repos.is_empty() => {
                bail!("super branch '{name}' does not list any repositories")
            }
            Some(repos) => Ok(repos),
            None => {
                let known: Vec<&str> = self.superbranches.keys().map(String::as_str).collect();
                if known.is_empty() {
                    bail!("unknown super branch '{name}' (the config defines no super branches)");
                }
                bail!(
                    "unknown super branch '{name}' (known: {})",
                    known.join(", ")
                );
            }
        }
    }
}

/// Append a new `[superbranches.<name>]` block to gitman.toml, creating the
/// file if needed. Appending as text (instead of re-serializing the whole
/// config) preserves any comments and formatting in the existing file.
pub fn append_superbranch(root: &Path, name: &str, entries: &[(String, String)]) -> Result<()> {
    let path = root.join(CONFIG_FILE);
    let mut content = if path.exists() {
        std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read config file {}", path.display()))?
    } else {
        String::new()
    };

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    if !content.is_empty() {
        content.push('\n');
    }
    content.push_str(&format!("[superbranches.{}]\n", toml_key(name)));
    for (repo, branch) in entries {
        content.push_str(&format!("{} = {}\n", toml_key(repo), toml_string(branch)));
    }

    // Validation round-trip: never write a file gitman itself cannot parse.
    toml::from_str::<Config>(&content)
        .context("internal error: generated config would be invalid")?;

    std::fs::write(&path, content)
        .with_context(|| format!("cannot write config file {}", path.display()))?;
    Ok(())
}

/// Render a TOML key: bare if possible, quoted otherwise.
fn toml_key(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        key.to_string()
    } else {
        toml_string(key)
    }
}

/// Render a TOML basic string with escaping.
fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
