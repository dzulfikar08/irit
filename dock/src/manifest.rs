//! `manifest.json`: what was run, with what, on what.
//!
//! A docking score is meaningless without the box, the seed and the versions that produced
//! it, and those are exactly the things people forget to write down. Same shape as the
//! gtdbprep bundle manifest, including its privacy rule: version strings that contain a
//! path are dropped rather than recorded, so a manifest can be attached to a paper.

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

pub const FORMAT: &str = "dock-run/1";

#[derive(Serialize)]
pub struct Client {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Serialize)]
pub struct FileEntry {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Serialize)]
pub struct Manifest {
    pub format: &'static str,
    pub client: Client,
    pub started: String,
    pub created: String,
    /// Tool versions, with anything path-shaped removed.
    pub tools: BTreeMap<String, String>,
    /// Every external command, argv form, in order.
    pub commands: Vec<Vec<String>>,
    pub parameters: serde_json::Value,
    pub results: serde_json::Value,
    pub files: BTreeMap<String, FileEntry>,
}

pub fn now() -> String {
    humantime::format_rfc3339_seconds(SystemTime::now()).to_string()
}

/// Drop a version string that leaks a local path.
pub fn clean_version(v: &str) -> Option<String> {
    let v = v.trim();
    if v.is_empty() || v.contains('/') || v.contains('\\') {
        None
    } else {
        Some(v.to_string())
    }
}

/// Replace the user's home directory with `~` wherever it appears.
///
/// The manifest is meant to be publishable. Recorded argv and parameters are full paths,
/// which on a personal machine carry the account name, so they are rewritten before they
/// are written out. This is not anonymisation — a path below the home directory still shows
/// the layout of someone's disk — it only keeps the account name out of a published file.
pub fn redact_home(s: &str) -> String {
    match std::env::var_os("HOME").map(std::path::PathBuf::from) {
        Some(home) => {
            let home = home.to_string_lossy().to_string();
            if home.is_empty() || home == "/" {
                s.to_string()
            } else {
                s.replace(&home, "~")
            }
        }
        None => s.to_string(),
    }
}

/// Apply [`redact_home`] to every string in a JSON value.
pub fn redact_value(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::String(s) => Value::String(redact_home(s)),
        Value::Array(a) => Value::Array(a.iter().map(redact_value).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, x)| (k.clone(), redact_value(x))).collect())
        }
        other => other.clone(),
    }
}

pub fn hash_file(path: &Path) -> Result<FileEntry> {
    let bytes = std::fs::read(path).with_context(|| format!("hashing {}", path.display()))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(FileEntry { bytes: bytes.len() as u64, sha256: format!("{:x}", h.finalize()) })
}

impl Manifest {
    pub fn write(&self, dir: &Path) -> Result<()> {
        let path = dir.join("manifest.json");
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_lines_with_paths_are_dropped() {
        assert_eq!(clean_version("1.2.5"), Some("1.2.5".to_string()));
        assert_eq!(clean_version(" 2026.9.2 "), Some("2026.9.2".to_string()));
        assert_eq!(clean_version("/home/someone/.cache/dock/venv"), None);
        assert_eq!(clean_version(""), None);
    }

    #[test]
    fn home_is_replaced_in_recorded_paths() {
        // SAFETY: single-threaded test, restored immediately.
        let prev = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", "/home/someone") };
        assert_eq!(redact_home("/home/someone/data/x.pdb"), "~/data/x.pdb");
        assert_eq!(redact_home("/opt/shared/x.pdb"), "/opt/shared/x.pdb");
        let v = serde_json::json!({
            "receptor": "/home/someone/r.pdb",
            "nested": ["/home/someone/a", 3, {"b": "/home/someone/b"}],
            "number": 7,
        });
        let r = redact_value(&v);
        assert_eq!(r["receptor"], "~/r.pdb");
        assert_eq!(r["nested"][0], "~/a");
        assert_eq!(r["nested"][2]["b"], "~/b");
        assert_eq!(r["number"], 7, "non-strings must survive untouched");
        match prev {
            Some(p) => unsafe { std::env::set_var("HOME", p) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }

    #[test]
    fn hashes_are_stable() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, b"hello").unwrap();
        let e = hash_file(&f).unwrap();
        assert_eq!(e.bytes, 5);
        assert_eq!(
            e.sha256,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
