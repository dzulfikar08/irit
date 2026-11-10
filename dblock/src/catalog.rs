//! The catalog: databases dblock knows how to fetch. Built in, with an optional user file
//! for in-house or newer databases (same JSON format, entries with the same id win).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

const BUILT_IN: &str = include_str!("catalog.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub tool: String,
    /// Tried in order; later ones are mirrors.
    pub urls: Vec<String>,
    pub format: Format,
    /// Download size in bytes.
    pub size: u64,
    /// Size once extracted, in bytes (approximate).
    pub extracted: u64,
    pub md5: String,
    pub md5_source: String,
    /// Environment variable the tool reads, if any.
    pub env: Option<String>,
    /// What the database path should point at: `dir`, or `glob:*.dmnd` for one file inside.
    #[serde(default = "default_points_to")]
    pub env_points_to: String,
    pub compat: Option<Compat>,
    pub licence: String,
    #[serde(default)]
    pub non_commercial: bool,
    #[serde(default)]
    pub notes: String,
}

fn default_points_to() -> String {
    "dir".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Compat {
    /// Command that prints the tool's version, e.g. `["gtdbtk", "--version"]`.
    pub command: Vec<String>,
    pub min: String,
    #[serde(default)]
    pub max_exclusive: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum Format {
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "tar.xz")]
    TarXz,
    #[serde(rename = "tar.bz2")]
    TarBz2,
    /// Kept as downloaded (e.g. a QIIME 2 `.qza`).
    #[serde(rename = "file")]
    File,
}

pub fn load(extra: Option<&Path>) -> Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = serde_json::from_str(BUILT_IN).context("built-in catalog")?;
    if let Some(path) = extra {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let more: Vec<Entry> = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        for e in more {
            entries.retain(|x| x.id != e.id);
            entries.push(e);
        }
    }
    for e in &entries {
        if e.urls.is_empty() {
            bail!("catalog entry {} has no URL", e.id);
        }
        if e.md5.len() != 32 || !e.md5.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("catalog entry {} has a malformed md5", e.id);
        }
    }
    Ok(entries)
}

pub fn find<'a>(entries: &'a [Entry], id: &str) -> Result<&'a Entry> {
    entries.iter().find(|e| e.id == id).with_context(|| {
        let close: Vec<_> = entries
            .iter()
            .filter(|e| e.id.contains(id) || id.contains(&e.tool.to_ascii_lowercase()))
            .map(|e| e.id.as_str())
            .collect();
        if close.is_empty() {
            format!("no database `{id}` in the catalog; `dblock catalog` lists them")
        } else {
            format!("no database `{id}`; did you mean {}?", close.join(", "))
        }
    })
}

/// "1.7 GB" style sizes (decimal units, like the download pages use).
pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    match bytes {
        0..1_000_000 => format!("{:.0} kB", b / 1e3),
        1_000_000..1_000_000_000 => format!("{:.0} MB", b / 1e6),
        _ => format!("{:.1} GB", b / 1e9),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_catalog_parses() {
        let c = load(None).unwrap();
        assert!(c.len() >= 10);
        assert!(find(&c, "checkm2-db-v3").is_ok());
        assert!(find(&c, "nope").is_err());
    }

    #[test]
    fn sizes() {
        assert_eq!(human(26_324_952), "26 MB");
        assert_eq!(human(60_806_405_195), "60.8 GB");
    }
}
