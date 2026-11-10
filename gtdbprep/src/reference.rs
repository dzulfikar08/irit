//! The reference data the laptop side reads: `markers/`, `masks/` and `metadata/`.

use crate::hash::{sha1_dir, sha256_file, sha256_tree, walk_files};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Folders `identify` and `align --skip_gtdb_refs` read from `GTDBTK_DATA_PATH`.
///
/// `taxonomy/` is only read to refuse input genome IDs that equal a GTDB accession
/// (`_genomes_to_process` in GTDB-Tk's `main.py`). Opaque IDs never do, so its contents don't
/// affect the result: GTDB's mockup ships a 5-line stub that works. It is hashed and recorded,
/// not enforced.
pub const CLIENT_FOLDERS: [&str; 4] = ["markers", "masks", "metadata", "taxonomy"];

/// Folders whose `REF_HASHES` must match before prep runs.
pub fn enforced(folder: &str) -> bool {
    folder != "taxonomy"
}

/// `REF_HASHES` for R232 in GTDB-Tk 2.8.0 (`gtdbtk/config/common.py`, commit `155d989`).
pub fn expected_sha1(release: &str, folder: &str) -> Option<&'static str> {
    match (release, folder) {
        ("r232", "markers") => Some("163f542c3f0a40f59df45d453aa235b39aa96e27"),
        ("r232", "masks") => Some("84c7f7a17adc134db4161b15db10ae58389a90c1"),
        ("r232", "metadata") => Some("1febdba7d2513a8f43c409423aee83ed99df7a78"),
        ("r232", "taxonomy") => Some("c1e1766fa610229cb7cb45edc9a8fa0eafb44c96"),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FolderHash {
    /// GTDB-Tk's own folder hash (`sha1_dir`), comparable with `REF_HASHES`.
    pub gtdbtk_sha1: String,
    /// The `REF_HASHES` value this GTDB-Tk release expects, when known.
    pub expected_sha1: Option<String>,
    /// `gtdbtk_sha1 == expected_sha1`; `None` if there is no expected value.
    pub matches_gtdbtk: Option<bool>,
    /// Whether a mismatch stops prep (false for `taxonomy/`, see `CLIENT_FOLDERS`).
    pub enforced: bool,
    /// SHA-256 of the folder's `sha256sum`-style listing (covers names and contents).
    pub sha256_tree: String,
    pub files: usize,
    /// Per-file SHA-256 for small folders (masks, metadata). Markers has hundreds of HMM files,
    /// so it is covered by `sha256_tree` only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub file_sha256: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReferenceReport {
    /// `VERSION_DATA` from `metadata/metadata.txt`, e.g. `r232`.
    pub gtdb_release: String,
    pub folders: BTreeMap<String, FolderHash>,
}

impl ReferenceReport {
    pub fn all_match(&self) -> bool {
        self.folders
            .values()
            .all(|f| !f.enforced || f.matches_gtdbtk != Some(false))
    }
}

/// Read `VERSION_DATA=...` from GTDB-Tk's metadata file.
pub fn parse_release(metadata: &str) -> Option<String> {
    metadata
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("VERSION_DATA=")
                .map(|v| v.trim().to_string())
        })
        .filter(|v| !v.is_empty())
}

pub fn check(data: &Path) -> Result<ReferenceReport> {
    let meta_path = data.join("metadata/metadata.txt");
    let meta = fs::read_to_string(&meta_path).with_context(|| {
        format!(
            "reading {} (is --data a GTDB-Tk reference folder?)",
            meta_path.display()
        )
    })?;
    let Some(release) = parse_release(&meta) else {
        bail!("{} has no VERSION_DATA line", meta_path.display());
    };
    let mut folders = BTreeMap::new();
    for name in CLIENT_FOLDERS {
        let dir = data.join(name);
        if !dir.is_dir() {
            bail!("reference folder {} is missing", dir.display());
        }
        let sha1 = sha1_dir(&dir)?;
        let expected = expected_sha1(&release, name).map(str::to_string);
        let (tree, files) = sha256_tree(&dir)?;
        let mut file_sha256 = BTreeMap::new();
        if name != "markers" {
            for rel in walk_files(&dir)? {
                file_sha256.insert(format!("{name}/{rel}"), sha256_file(&dir.join(&rel))?.0);
            }
        }
        folders.insert(
            name.to_string(),
            FolderHash {
                matches_gtdbtk: expected.as_ref().map(|e| *e == sha1),
                enforced: enforced(name),
                gtdbtk_sha1: sha1,
                expected_sha1: expected,
                sha256_tree: tree,
                files,
                file_sha256,
            },
        );
    }
    Ok(ReferenceReport {
        gtdb_release: release,
        folders,
    })
}

/// GTDB-Tk's split bacterial trees. The R232 mockup ships an empty `split/`, so it only works
/// with `--full_tree`.
pub fn has_split_trees(data: &Path) -> bool {
    fs::read_dir(data.join("split"))
        .map(|mut d| d.next().is_some())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_from_metadata() {
        let m = "RED_DIST_BAC_DICT={}\nVERSION_DATA=r232\n";
        assert_eq!(parse_release(m).as_deref(), Some("r232"));
        assert_eq!(parse_release("X=1\n"), None);
    }

    #[test]
    fn check_reports_mismatch_for_fake_data() {
        let d = tempfile::tempdir().unwrap();
        for f in CLIENT_FOLDERS {
            fs::create_dir_all(d.path().join(f)).unwrap();
        }
        fs::write(
            d.path().join("metadata/metadata.txt"),
            "VERSION_DATA=r232\n",
        )
        .unwrap();
        fs::write(d.path().join("masks/gtdb_r232_bac120.mask"), "0101\n").unwrap();
        fs::write(
            d.path().join("taxonomy/gtdb_taxonomy.tsv"),
            "GB_GCA_1\td__Bacteria\n",
        )
        .unwrap();
        fs::write(d.path().join("markers/x.hmm"), "HMM\n").unwrap();
        let r = check(d.path()).unwrap();
        assert_eq!(r.gtdb_release, "r232");
        assert_eq!(r.folders["masks"].matches_gtdbtk, Some(false));
        assert!(!r.all_match());
        assert!(
            r.folders["masks"]
                .file_sha256
                .contains_key("masks/gtdb_r232_bac120.mask")
        );
        assert!(r.folders["markers"].file_sha256.is_empty());
        assert_eq!(r.folders["markers"].files, 1);
        // A stub taxonomy is recorded as different but does not count against the data.
        assert_eq!(r.folders["taxonomy"].matches_gtdbtk, Some(false));
        assert!(!r.folders["taxonomy"].enforced);
    }

    #[test]
    fn unknown_release_has_no_expectation() {
        let d = tempfile::tempdir().unwrap();
        for f in CLIENT_FOLDERS {
            fs::create_dir_all(d.path().join(f)).unwrap();
        }
        fs::write(
            d.path().join("metadata/metadata.txt"),
            "VERSION_DATA=r999\n",
        )
        .unwrap();
        let r = check(d.path()).unwrap();
        assert!(r.folders.values().all(|f| f.matches_gtdbtk.is_none()));
        assert!(r.all_match());
    }
}
