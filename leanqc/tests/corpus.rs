//! leanqc against FastQC 0.12.1 on the synthetic corpus in `tests/corpus/`.
//!
//! FastQC's verdicts are recorded in `fastqc_verdicts.tsv` (by `scripts/fastqc_corpus.sh`), so
//! this test needs neither Java nor Docker. Every module both tools report must agree, except the
//! rows in `expected_differences.tsv`; files FastQC can't read must make leanqc fail too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// FastQC module → leanqc check.
const MODULES: &[(&str, &str)] = &[
    ("Per base sequence quality", "Per-position quality"),
    ("Per tile sequence quality", "Per-tile quality"),
    ("Per sequence quality scores", "Per-read quality"),
    ("Per base sequence content", "Per-position base content"),
    ("Per sequence GC content", "Per-read GC"),
    ("Per base N content", "N content"),
    ("Sequence Length Distribution", "Read length"),
    ("Sequence Duplication Levels", "Duplication"),
    ("Overrepresented sequences", "Overrepresented sequences"),
    ("Adapter Content", "Adapter content"),
];

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

fn rows(name: &str) -> Vec<Vec<String>> {
    std::fs::read_to_string(corpus().join(name))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .skip(1)
        .map(|l| l.split('\t').map(String::from).collect())
        .collect()
}

#[test]
fn verdicts_match_fastqc() {
    let mut fastqc: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for r in rows("fastqc_verdicts.tsv") {
        fastqc.entry(r[0].clone()).or_default().push((r[1].clone(), r[2].clone()));
    }
    let expected: BTreeSet<(String, String, String, String)> =
        rows("expected_differences.tsv").into_iter().map(|r| (r[0].clone(), r[1].clone(), r[2].clone(), r[3].clone())).collect();

    let out = tempfile::tempdir().unwrap();
    let mut compared = 0;
    let mut problems = Vec::new();
    let mut seen_expected = BTreeSet::new();
    for (file, verdicts) in &fastqc {
        let dir = out.path().join(file);
        let run = Command::new(env!("CARGO_BIN_EXE_leanqc"))
            .arg(corpus().join(file))
            .args(["--no-html", "-o"])
            .arg(&dir)
            .output()
            .unwrap();
        if verdicts.iter().any(|(_, v)| v == "error") {
            if run.status.success() {
                problems.push(format!("{file}: FastQC can't read it, but leanqc reported no error"));
            }
            continue;
        }
        assert!(run.status.success(), "{file}: {}", String::from_utf8_lossy(&run.stderr));
        let stem = file.trim_end_matches(".gz").trim_end_matches(".fastq");
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{stem}_leanqc.json"))).unwrap()).unwrap();
        let ours: BTreeMap<&str, &str> =
            json["checks"].as_array().unwrap().iter().map(|c| (c["name"].as_str().unwrap(), c["status"].as_str().unwrap())).collect();
        for (module, check) in MODULES {
            let theirs = verdicts.iter().find(|(m, _)| m == module).map(|(_, v)| v.as_str());
            let mine = ours.get(check).copied();
            match (theirs, mine) {
                (Some(t), Some(m)) => {
                    compared += 1;
                    if t != m {
                        let key = (file.clone(), module.to_string(), t.to_string(), m.to_string());
                        if expected.contains(&key) {
                            seen_expected.insert(key);
                        } else {
                            problems.push(format!("{file}: {module}: FastQC {t}, leanqc {m}"));
                        }
                    }
                }
                (None, None) => {}
                (t, m) => problems.push(format!("{file}: {module} reported by only one tool (FastQC {t:?}, leanqc {m:?})")),
            }
        }
    }
    for e in expected.difference(&seen_expected) {
        problems.push(format!("expected difference no longer seen (update expected_differences.tsv): {e:?}"));
    }
    assert!(compared >= 90, "only {compared} verdicts compared");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
