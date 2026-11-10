//! `manifest.json`: what the bundle contains and how it was made.

use crate::hash::{all_files, sha256_file};
use crate::reference::ReferenceReport;
use crate::tools::ToolVersion;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const FORMAT: &str = "gtdbprep-bundle/1";
pub const MANIFEST: &str = "manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub format: String,
    pub client: Client,
    /// RFC 3339 UTC.
    pub created: String,
    pub started: String,
    pub gtdb_release: String,
    /// Versions reported by the tools that ran on the laptop (gtdbtk, prodigal, hmmer, skani).
    pub tools: BTreeMap<String, ToolVersion>,
    /// GTDB-Tk commands run, with local paths replaced by `<renamed>`, `<work>` placeholders.
    pub commands: Vec<Vec<String>>,
    pub reference: ReferenceReport,
    /// The GTDB-Tk output prefix used for file names in the bundle.
    pub prefix: String,
    pub genomes: Vec<GenomeEntry>,
    /// Input files left out before GTDB-Tk ran (empty or not FASTA). Only the count: their
    /// names and reasons stay in the local mapping.
    #[serde(default)]
    pub inputs_excluded: usize,
    pub sketches: Option<SketchInfo>,
    /// Every other file in the bundle: path relative to the bundle root → size and SHA-256.
    pub files: BTreeMap<String, FileEntry>,
    pub privacy: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Client {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GenomeEntry {
    /// Opaque ID; the only genome name in the bundle.
    pub id: String,
    /// SHA-256 of the renamed FASTA GTDB-Tk read (opaque headers, original sequence). Lets
    /// someone who already holds the genome confirm a match; reveals nothing otherwise.
    pub fasta_sha256: String,
    pub contigs: usize,
    /// `bac120` or `ar53`: GTDB-Tk's domain call from single-copy marker counts.
    pub domain: Option<String>,
    pub bac120_single_copy: Option<usize>,
    pub ar53_single_copy: Option<usize>,
    /// Genetic code chosen by GTDB-Tk's Prodigal step (11 or 4).
    pub translation_table: Option<u32>,
    /// Present in a user MSA file of the bundle.
    pub in_msa: Option<String>,
    /// Listed in GTDB-Tk's filtered/failed files (too few amino acids, no markers, Prodigal failure).
    pub excluded: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SketchInfo {
    pub skani_version: String,
    /// `skani sketch` parameters (defaults; must match GTDB-Tk's skani database).
    pub c: u32,
    pub marker_c: u32,
    pub k: u32,
    pub warning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileEntry {
    pub bytes: u64,
    pub sha256: String,
}

/// Hash every file in the bundle except the manifest itself.
pub fn hash_bundle_files(bundle: &Path) -> Result<BTreeMap<String, FileEntry>> {
    let mut files = BTreeMap::new();
    for path in all_files(bundle)? {
        let rel = path
            .strip_prefix(bundle)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel == MANIFEST {
            continue;
        }
        let (sha256, bytes) = sha256_file(&path)?;
        files.insert(rel, FileEntry { bytes, sha256 });
    }
    Ok(files)
}

impl Manifest {
    pub fn write(&self, bundle: &Path) -> Result<()> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        fs::write(bundle.join(MANIFEST), text)?;
        Ok(())
    }

    pub fn read(bundle: &Path) -> Result<Manifest> {
        let p = bundle.join(MANIFEST);
        let text = fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let m: Manifest =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", p.display()))?;
        if m.format != FORMAT {
            bail!(
                "{}: bundle format {} is not {FORMAT}",
                p.display(),
                m.format
            );
        }
        Ok(m)
    }

    /// Check that the bundle's files are exactly those listed, with the listed hashes.
    pub fn verify_files(&self, bundle: &Path) -> Result<()> {
        let actual = hash_bundle_files(bundle)?;
        let mut problems = Vec::new();
        for (path, want) in &self.files {
            match actual.get(path) {
                None => problems.push(format!("missing {path}")),
                Some(got) if got != want => problems.push(format!("changed {path}")),
                _ => {}
            }
        }
        for path in actual.keys() {
            if !self.files.contains_key(path) {
                problems.push(format!("unlisted {path}"));
            }
        }
        if !problems.is_empty() {
            bail!(
                "bundle {} does not match its manifest: {}",
                bundle.display(),
                problems.join(", ")
            );
        }
        Ok(())
    }

    /// Bytes of everything sent, including the manifest as written.
    pub fn total_bytes(&self, bundle: &Path) -> u64 {
        let m = fs::metadata(bundle.join(MANIFEST))
            .map(|m| m.len())
            .unwrap_or(0);
        m + self.files.values().map(|f| f.bytes).sum::<u64>()
    }

    pub fn sketch_bytes(&self) -> u64 {
        self.files
            .iter()
            .filter(|(p, _)| p.starts_with("sketches/"))
            .map(|(_, f)| f.bytes)
            .sum()
    }
}

pub fn now_rfc3339() -> String {
    humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string()
}

#[cfg(test)]
pub(crate) fn sample(files: BTreeMap<String, FileEntry>) -> Manifest {
    Manifest {
        format: FORMAT.into(),
        client: Client {
            name: "gtdbprep".into(),
            version: "0.1.0".into(),
        },
        created: "2026-11-10T00:00:00Z".into(),
        started: "2026-11-10T00:00:00Z".into(),
        gtdb_release: "r232".into(),
        tools: BTreeMap::new(),
        commands: vec![vec!["gtdbtk".into(), "identify".into()]],
        reference: ReferenceReport {
            gtdb_release: "r232".into(),
            folders: BTreeMap::new(),
        },
        prefix: "gtdbtk".into(),
        genomes: vec![GenomeEntry {
            id: "g0123456789abcdef".into(),
            fasta_sha256: "00".into(),
            contigs: 2,
            domain: Some("bac120".into()),
            bac120_single_copy: Some(119),
            ar53_single_copy: Some(7),
            translation_table: Some(11),
            in_msa: Some("bac120".into()),
            excluded: None,
        }],
        inputs_excluded: 0,
        sketches: None,
        files,
        privacy: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_file_check() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("align")).unwrap();
        fs::write(d.path().join("align/x.fasta.gz"), b"abc").unwrap();
        let files = hash_bundle_files(d.path()).unwrap();
        assert_eq!(files["align/x.fasta.gz"].bytes, 3);
        let m = sample(files);
        m.write(d.path()).unwrap();
        let back = Manifest::read(d.path()).unwrap();
        assert_eq!(back, m);
        back.verify_files(d.path()).unwrap();
        assert_eq!(
            back.total_bytes(d.path()),
            3 + fs::metadata(d.path().join(MANIFEST)).unwrap().len()
        );

        fs::write(d.path().join("align/x.fasta.gz"), b"abd").unwrap();
        let e = back.verify_files(d.path()).unwrap_err().to_string();
        assert!(e.contains("changed align/x.fasta.gz"), "{e}");
        fs::write(d.path().join("extra"), b"").unwrap();
        assert!(
            back.verify_files(d.path())
                .unwrap_err()
                .to_string()
                .contains("unlisted extra")
        );
    }

    #[test]
    fn rejects_other_formats() {
        let d = tempfile::tempdir().unwrap();
        let mut m = sample(BTreeMap::new());
        m.format = "something/9".into();
        m.write(d.path()).unwrap();
        assert!(Manifest::read(d.path()).is_err());
    }
}
