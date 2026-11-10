//! The catalog: databases dblock knows how to fetch. Built in, with an optional user file
//! for in-house or newer databases (same JSON format, entries with the same id win).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path};

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
    /// Only unpack archive entries under these paths (e.g. `release232_mockup/markers`). Other
    /// entries are still read, so the archive checksum covers the whole file, but not written.
    /// Empty: unpack everything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keep: Vec<String>,
    /// Size of what `keep` keeps, in bytes. Falls back to `extracted` when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_size: Option<u64>,
    /// Folder (relative to the database folder) -> GTDB-Tk folder hash (`sha1_dir`) it must have.
    /// Checked after unpacking and by `verify --deep`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub verify_dirs: BTreeMap<String, String>,
}

impl Entry {
    /// Bytes the database takes once in the store.
    pub fn on_disk(&self) -> u64 {
        match self.format {
            Format::File => self.size,
            _ if !self.keep.is_empty() => self.kept_size.unwrap_or(self.extracted),
            _ => self.extracted,
        }
    }
}

/// A plain relative path inside a database: no root, no `..`, not empty.
pub fn plain_relative(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && path.components().any(|c| matches!(c, Component::Normal(_)))
        && path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
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
    for e in &mut entries {
        // Ids become folder and file names in the store.
        let id_ok = !e.id.is_empty()
            && !e.id.starts_with('.')
            && !e.id.contains("..")
            && e.id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !id_ok {
            bail!("catalog entry id `{}` must use only A-Z a-z 0-9 . _ - and not start with `.` or contain `..`", e.id);
        }
        // Publishers print MD5s in either case; downloads are compared in lower case.
        e.md5 = e.md5.to_ascii_lowercase();
        if e.urls.is_empty() {
            bail!("catalog entry {} has no URL", e.id);
        }
        if e.md5.len() != 32 || !e.md5.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("catalog entry {} has a malformed md5", e.id);
        }
        if !e.keep.is_empty() && e.format == Format::File {
            bail!("catalog entry {}: `keep` needs an archive format, not `file`", e.id);
        }
        if let Some(bad) = e.keep.iter().find(|k| !plain_relative(k)) {
            bail!("catalog entry {}: keep path `{bad}` must be relative, without `..`", e.id);
        }
        for (dir, hash) in &mut e.verify_dirs {
            if !plain_relative(dir) {
                bail!("catalog entry {}: verify_dirs folder `{dir}` must be relative, without `..`", e.id);
            }
            *hash = hash.to_ascii_lowercase();
            if hash.len() != 40 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                bail!("catalog entry {}: verify_dirs hash for `{dir}` is not a SHA-1", e.id);
            }
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

    fn user_catalog(json: &str) -> Result<Vec<Entry>> {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.json");
        std::fs::write(&p, json).unwrap();
        load(Some(&p))
    }

    fn entry_json(id: &str, md5: &str) -> String {
        format!(
            r#"[{{"id":"{id}","name":"t","version":"1","tool":"t","urls":["http://127.0.0.1:1/x.tar.gz"],"format":"tar.gz","size":1,"extracted":1,"md5":"{md5}","md5_source":"t","env":null,"compat":null,"licence":"x"}}]"#
        )
    }

    #[test]
    fn uppercase_md5_is_lowercased() {
        let c = user_catalog(&entry_json("mydb", "E3113558D456816BA2C70706932F02C8")).unwrap();
        assert_eq!(find(&c, "mydb").unwrap().md5, "e3113558d456816ba2c70706932f02c8");
    }

    #[test]
    fn ids_that_are_not_plain_names_are_refused() {
        for bad in ["../evil", "a/b", "..", ".hidden", "a..b", "a b", ""] {
            assert!(user_catalog(&entry_json(bad, "e3113558d456816ba2c70706932f02c8")).is_err(), "{bad}");
        }
        assert!(user_catalog(&entry_json("ok.db_v1-2", "e3113558d456816ba2c70706932f02c8")).is_ok());
    }

    #[test]
    fn keep_and_verify_dirs_are_checked() {
        let base = entry_json("mydb", "e3113558d456816ba2c70706932f02c8");
        let with = |extra: &str| user_catalog(&base.replace(r#""licence":"x""#, &format!(r#""licence":"x",{extra}"#)));
        let ok = with(r#""keep":["db/a","./db/b"],"kept_size":5,"verify_dirs":{"db/a":"163F542C3F0A40F59DF45D453AA235B39AA96E27"}"#).unwrap();
        let e = find(&ok, "mydb").unwrap();
        assert_eq!(e.on_disk(), 5);
        assert_eq!(e.verify_dirs["db/a"], "163f542c3f0a40f59df45d453aa235b39aa96e27");
        for bad in [r#""keep":["../x"]"#, r#""keep":["/abs"]"#, r#""keep":[""]"#, r#""keep":["."]"#] {
            assert!(with(bad).is_err(), "{bad}");
        }
        assert!(with(r#""verify_dirs":{"a/../../b":"163f542c3f0a40f59df45d453aa235b39aa96e27"}"#).is_err());
        assert!(with(r#""verify_dirs":{"a":"abc"}"#).is_err());
        // Without kept_size, `keep` falls back to `extracted`.
        assert_eq!(find(&with(r#""keep":["db"]"#).unwrap(), "mydb").unwrap().on_disk(), 1);
    }

    #[test]
    fn client_entry_keeps_only_what_gtdbprep_reads() {
        let c = load(None).unwrap();
        let e = find(&c, "gtdbtk-r232-client").unwrap();
        assert_eq!(e.keep.len(), 4);
        assert_eq!(e.verify_dirs.len(), 3);
        assert!(e.on_disk() < 100_000_000);
    }

    #[test]
    fn sizes() {
        assert_eq!(human(26_324_952), "26 MB");
        assert_eq!(human(60_806_405_195), "60.8 GB");
    }
}
