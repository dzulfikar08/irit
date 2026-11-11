//! The catalog: databases dblock knows how to fetch. Built in, with an optional user file
//! for in-house or newer databases (same JSON format, entries with the same id win).

use anyhow::{Context, Result, bail};
use md5::Digest;
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
    /// Tried in order; later ones are mirrors. Empty for multi-part entries (see `parts`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
    pub format: Format,
    /// Download size in bytes; for multi-part entries the sum of the parts (filled in if left out).
    #[serde(default)]
    pub size: u64,
    /// Size once extracted, in bytes (approximate).
    pub extracted: u64,
    /// The archive's MD5. For multi-part entries, the MD5 of the parts' `MD5SUMS` lines
    /// (`<md5>  <file>\n`, sorted by file name), filled in if left out: it names the exact set.
    #[serde(default)]
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
    /// Prefix for parts given by `file`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Several archives that unpack into one database folder, in order. Each is downloaded and
    /// checked on its own (resumable per part); the folder takes its final name only after all of
    /// them succeed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
}

/// One archive of a multi-part entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Part {
    /// Full URL; or leave it out and give `file`, which is appended to the entry's `base_url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub size: u64,
    pub md5: String,
    /// Checked as well as the MD5 when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// One archive to download and check: the entry itself, or one of its parts.
#[derive(Debug, Clone)]
pub struct Source {
    /// File name, used for the partial download and in messages.
    pub name: String,
    /// Tried in order.
    pub urls: Vec<String>,
    pub size: u64,
    pub md5: String,
    pub sha256: Option<String>,
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

    pub fn is_multi_part(&self) -> bool {
        !self.parts.is_empty()
    }

    /// The archives to download, in order: one for a plain entry, one per part otherwise.
    pub fn sources(&self) -> Vec<Source> {
        if self.parts.is_empty() {
            return vec![Source {
                name: last_segment(&self.urls[0]).unwrap_or(&self.id).to_string(),
                urls: self.urls.clone(),
                size: self.size,
                md5: self.md5.clone(),
                sha256: None,
            }];
        }
        self.parts
            .iter()
            .map(|p| {
                let url = p.url.clone().unwrap_or_else(|| {
                    format!("{}/{}", self.base_url.as_deref().unwrap_or("").trim_end_matches('/'), p.file.as_deref().unwrap_or(""))
                });
                Source {
                    name: p.file.clone().or_else(|| last_segment(&url).map(str::to_string)).unwrap_or_default(),
                    urls: vec![url],
                    size: p.size,
                    md5: p.md5.clone(),
                    sha256: p.sha256.clone(),
                }
            })
            .collect()
    }
}

fn last_segment(url: &str) -> Option<&str> {
    url.rsplit('/').next().filter(|s| !s.is_empty())
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Resolve each part's URL, check its checksums, and fill in the entry's size and combined MD5.
fn check_parts(e: &mut Entry) -> Result<()> {
    if e.format == Format::File {
        bail!("catalog entry {}: `parts` needs an archive format, not `file`", e.id);
    }
    if !e.urls.is_empty() {
        bail!("catalog entry {}: give either `urls` or `parts`, not both", e.id);
    }
    let mut names = std::collections::BTreeSet::new();
    let mut listing = Vec::new();
    let mut total = 0u64;
    for (i, p) in e.parts.iter_mut().enumerate() {
        let url = match (&p.url, &p.file, &e.base_url) {
            (Some(url), None, _) => url.clone(),
            (None, Some(file), Some(base)) => format!("{}/{file}", base.trim_end_matches('/')),
            (None, Some(_), None) => bail!("catalog entry {}: part {} gives `file` but the entry has no `base_url`", e.id, i + 1),
            _ => bail!("catalog entry {}: part {} needs exactly one of `url` or `file`", e.id, i + 1),
        };
        let name = p.file.clone().or_else(|| last_segment(&url).map(str::to_string)).unwrap_or_default();
        let name_ok = !name.is_empty()
            && !name.starts_with('.')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !name_ok {
            bail!("catalog entry {}: part file name `{name}` must use only A-Z a-z 0-9 . _ - and not start with `.`", e.id);
        }
        if !names.insert(name.clone()) {
            bail!("catalog entry {}: two parts are named `{name}`", e.id);
        }
        if p.size == 0 {
            bail!("catalog entry {}: part `{name}` needs its `size` in bytes", e.id);
        }
        p.md5 = p.md5.to_ascii_lowercase();
        if !is_hex(&p.md5, 32) {
            bail!("catalog entry {}: part `{name}` has a malformed md5", e.id);
        }
        if let Some(sha) = &mut p.sha256 {
            *sha = sha.to_ascii_lowercase();
            if !is_hex(sha, 64) {
                bail!("catalog entry {}: part `{name}` has a malformed sha256", e.id);
            }
        }
        listing.push((name, p.md5.clone()));
        total += p.size;
    }
    if e.size != 0 && e.size != total {
        bail!("catalog entry {}: size {} is not the sum of its parts ({total})", e.id, e.size);
    }
    e.size = total;
    // Sorted by file name, as `md5sum * > MD5SUMS` writes it, so part order doesn't matter.
    listing.sort();
    let listing: String = listing.iter().map(|(name, md5)| format!("{md5}  {name}\n")).collect();
    let combined = crate::fetch::hex(&md5::Md5::digest(listing.as_bytes()));
    if !e.md5.is_empty() && !e.md5.eq_ignore_ascii_case(&combined) {
        bail!("catalog entry {}: md5 must be the MD5 of its parts' MD5SUMS lines ({combined}) or left out", e.id);
    }
    e.md5 = combined;
    Ok(())
}

/// A plain relative path inside a database: no root, no `..`, not empty.
pub fn plain_relative(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && path.components().any(|c| matches!(c, Component::Normal(_)))
        && path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// `./a/./b` -> `a/b`, for comparing paths component by component.
fn clean(p: &str) -> std::path::PathBuf {
    Path::new(p).components().filter(|c| !matches!(c, Component::CurDir)).collect()
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
    /// Zstandard, including archives made with a long window (`zstd --long=N`, up to 2 GiB).
    #[serde(rename = "tar.zst")]
    TarZst,
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
        if e.is_multi_part() {
            check_parts(e)?;
        } else if e.urls.is_empty() {
            bail!("catalog entry {} has no URL", e.id);
        } else if e.size == 0 {
            bail!("catalog entry {} needs its download `size` in bytes", e.id);
        } else if e.base_url.is_some() {
            bail!("catalog entry {}: `base_url` is only for entries with `parts`", e.id);
        }
        if !is_hex(&e.md5, 32) {
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
            // With `keep`, a folder outside it would never be written, so its hash can't match.
            if !e.keep.is_empty() && !e.keep.iter().any(|k| clean(dir).starts_with(clean(k))) {
                bail!("catalog entry {}: verify_dirs folder `{dir}` is not inside any `keep` path", e.id);
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
        // With `keep`, verify_dirs folders must be inside a kept path (component-wise).
        let h = "163f542c3f0a40f59df45d453aa235b39aa96e27";
        let v = |keep: &str, dir: &str| with(&format!(r#""keep":["{keep}"],"verify_dirs":{{"{dir}":"{h}"}}"#));
        assert!(v("db/markers", "db/markers").is_ok());
        assert!(v("./db", "db/markers").is_ok());
        assert!(v("db/markers", "./db/markers/sub").is_ok());
        let err = format!("{:#}", v("db/markers", "db/masks").unwrap_err());
        assert!(err.contains("`db/masks` is not inside any `keep` path"), "{err}");
        assert!(v("db/markers", "db/markers2").is_err());
        assert!(v("db/markers", "db").is_err());
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
    fn placement_entries_are_multi_part() {
        let c = load(None).unwrap();
        let full = find(&c, "gtdbtk-r232-placement").unwrap();
        let lite = find(&c, "gtdbtk-r232-placement-lite").unwrap();
        assert_eq!((full.parts.len(), full.size, full.on_disk()), (12, 279_636_147, 7_368_346_876));
        assert_eq!((lite.parts.len(), lite.size), (11, 175_495_212));
        assert!(full.sources().iter().all(|s| s.urls[0].starts_with(
            "https://github.com/dzulfikar08/irit-data/releases/download/gtdbtk-r232-placement-v1/gtdbtk-r232-"
        ) && s.sha256.is_some()));
        assert_eq!(full.verify_dirs["release232/pplacer"], "4e8ffa1133f10bde827bddb81413d06d62de341e");
        assert!(!lite.verify_dirs.contains_key("release232/pplacer"));
        assert_ne!(full.md5, lite.md5, "different sets of parts");
    }

    #[test]
    fn ani_screen_entry_is_multi_part() {
        let c = load(None).unwrap();
        let e = find(&c, "gtdbtk-r232-ani-screen").unwrap();
        assert_eq!((e.parts.len(), e.size, e.on_disk()), (3, 2_101_861_800, 2_329_014_415));
        assert!(e.sources().iter().all(|s| s.urls[0].starts_with(
            "https://github.com/dzulfikar08/irit-data/releases/download/gtdbtk-r232-ani-screen-v1/gtdbtk-r232-ani-screen-"
        ) && s.sha256.is_some()));
        assert!(e.env.is_none());
    }

    #[test]
    fn single_url_entries_need_a_size_and_no_base_url() {
        let ok = entry_json("one", &"0".repeat(32));
        assert!(user_catalog(&ok).is_ok());
        let err = format!("{:#}", user_catalog(&ok.replace(r#""size":1,"#, "")).unwrap_err());
        assert!(err.contains("needs its download `size`"), "{err}");
        let err = format!("{:#}", user_catalog(&ok.replace(r#""size":1,"#, r#""size":1,"base_url":"https://x/","#)).unwrap_err());
        assert!(err.contains("only for entries with `parts`"), "{err}");
    }

    fn parts_json(extra: &str, parts: &str) -> Result<Vec<Entry>> {
        user_catalog(&format!(
            r#"[{{"id":"mp","name":"t","version":"1","tool":"t","format":"tar.zst","extracted":1,"md5_source":"t","env":null,"compat":null,"licence":"x"{extra},"parts":[{parts}]}}]"#
        ))
    }

    #[test]
    fn parts_are_checked() {
        let a = r#"{"file":"a.tar.zst","size":3,"md5":"E3113558D456816BA2C70706932F02C8","sha256":"AB"}"#;
        let ok_a = r#"{"file":"a.tar.zst","size":3,"md5":"E3113558D456816BA2C70706932F02C8"}"#;
        let b = r#"{"url":"https://m.example/x/b.tar.zst","size":4,"md5":"0123456789abcdef0123456789abcdef"}"#;
        let base = r#","base_url":"https://e.example/rel/""#;
        let c = parts_json(base, &format!("{ok_a},{b}")).unwrap();
        let e = find(&c, "mp").unwrap();
        assert_eq!(e.size, 7);
        let s = e.sources();
        assert_eq!(s[0].urls, ["https://e.example/rel/a.tar.zst"]);
        assert_eq!((s[1].name.as_str(), s[1].urls[0].as_str()), ("b.tar.zst", "https://m.example/x/b.tar.zst"));
        assert_eq!(s[0].md5, "e3113558d456816ba2c70706932f02c8");
        // The entry's md5 names the exact set of parts: MD5 of their MD5SUMS lines.
        let listing = "e3113558d456816ba2c70706932f02c8  a.tar.zst\n0123456789abcdef0123456789abcdef  b.tar.zst\n";
        assert_eq!(e.md5, crate::fetch::hex(&md5::Md5::digest(listing.as_bytes())));
        assert!(parts_json(&format!(r#"{base},"md5":"{}""#, e.md5), &format!("{ok_a},{b}")).is_ok());
        // Part order doesn't change it (lines are sorted by file name, like `md5sum *`).
        assert_eq!(find(&parts_json(base, &format!("{b},{ok_a}")).unwrap(), "mp").unwrap().md5, e.md5);
        // A loaded entry can be written out and loaded again.
        let again = user_catalog(&serde_json::to_string(&vec![e.clone()]).unwrap()).unwrap();
        assert_eq!(find(&again, "mp").unwrap().md5, e.md5);
        let bad = |extra: &str, parts: &str| format!("{:#}", parts_json(extra, parts).unwrap_err());
        assert!(bad(base, a).contains("malformed sha256"));
        assert!(bad("", ok_a).contains("no `base_url`"));
        assert!(bad(base, &format!("{ok_a},{ok_a}")).contains("two parts are named"));
        assert!(bad(&format!(r#"{base},"size":8"#), &format!("{ok_a},{b}")).contains("not the sum"));
        assert!(bad(&format!(r#"{base},"md5":"{}""#, "0".repeat(32)), ok_a).contains("MD5SUMS lines"));
        assert!(bad(r#","urls":["https://x/y.tar.zst"]"#, b).contains("not both"));
        assert!(bad(base, r#"{"file":"../a","size":1,"md5":"e3113558d456816ba2c70706932f02c8"}"#).contains("file name"));
        assert!(bad(base, r#"{"size":1,"md5":"e3113558d456816ba2c70706932f02c8"}"#).contains("exactly one of"));
        assert!(bad(base, r#"{"file":"a.tar.zst","size":0,"md5":"e3113558d456816ba2c70706932f02c8"}"#).contains("needs its `size`"));
    }

    #[test]
    fn sizes() {
        assert_eq!(human(26_324_952), "26 MB");
        assert_eq!(human(60_806_405_195), "60.8 GB");
    }
}
