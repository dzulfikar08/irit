//! `dblock.toml` (what a project wants) and `dblock.lock` (exactly what it got).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PROJECT_FILE: &str = "dblock.toml";
pub const LOCK_FILE: &str = "dblock.lock";
/// Written inside each database folder so other projects can reuse it.
pub const MARKER: &str = ".dblock.toml";
/// Per-file SHA-256 sums written next to the marker at fetch time (for `verify --deep`).
pub const SUMS: &str = ".dblock.sha256";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Project {
    /// Where databases are stored. Shared between projects; `~` is expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    #[serde(default)]
    pub databases: Vec<String>,
}

impl Project {
    pub fn load(dir: &Path) -> Result<Option<Self>> {
        let path = dir.join(PROJECT_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)?;
        Ok(Some(toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?))
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        let text = format!(
            "# Reference databases this project uses. Edit freely, then run `dblock fetch`.\n# `dblock catalog` lists the ids.\n\n{body}"
        );
        std::fs::write(dir.join(PROJECT_FILE), text)?;
        Ok(())
    }

    pub fn store_dir(&self) -> Result<PathBuf> {
        if let Some(s) = &self.store {
            return Ok(expand(s));
        }
        default_store()
    }
}

pub fn default_store() -> Result<PathBuf> {
    if let Some(s) = std::env::var_os("DBLOCK_STORE") {
        return Ok(PathBuf::from(s));
    }
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(x).join("dblock"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set; set DBLOCK_STORE")?;
    Ok(PathBuf::from(home).join(".local/share/dblock"))
}

fn expand(s: &str) -> PathBuf {
    match (s.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(s),
    }
}

/// One fetched database, as recorded in the lock file and in the database's own folder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Locked {
    pub id: String,
    pub name: String,
    pub version: String,
    pub tool: String,
    pub url: String,
    pub archive_md5: String,
    pub archive_bytes: u64,
    pub fetched: String,
    /// Folder that holds the database.
    pub path: String,
    /// What to hand the tool (a folder inside `path`, or one file).
    pub data_path: String,
    pub files: u64,
    pub bytes: u64,
    /// md5 of the sorted `relative path<TAB>size` list: changes if any file is added,
    /// removed or changes size.
    pub manifest_md5: String,
    /// SHA-256 of the folder's `.dblock.sha256` list (every file's SHA-256, size and path).
    /// Missing for databases fetched by dblock 0.1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    pub licence: String,
    #[serde(default)]
    pub non_commercial: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Lock {
    #[serde(default, rename = "database")]
    pub databases: Vec<Locked>,
}

impl Lock {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(LOCK_FILE);
        if !path.exists() {
            return Ok(Lock::default());
        }
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        let text = format!(
            "# Written by dblock. Commit it with your analysis: it records exactly which\n# reference databases (URL, checksum, date, contents) the results came from.\n\n{body}"
        );
        std::fs::write(dir.join(LOCK_FILE), text)?;
        Ok(())
    }

    pub fn upsert(&mut self, entry: Locked) {
        self.databases.retain(|d| d.id != entry.id);
        self.databases.push(entry);
        self.databases.sort_by(|a, b| a.id.cmp(&b.id));
    }
}

pub fn read_marker(db_dir: &Path) -> Option<Locked> {
    let text = std::fs::read_to_string(db_dir.join(MARKER)).ok()?;
    toml::from_str(&text).ok()
}

pub fn write_marker(db_dir: &Path, entry: &Locked) -> Result<()> {
    std::fs::write(db_dir.join(MARKER), toml::to_string_pretty(entry)?)?;
    Ok(())
}
