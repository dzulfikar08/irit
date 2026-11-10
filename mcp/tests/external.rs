//! fastp, Bakta, CheckM2 and GTDB-Tk through irit-mcp over stdio, with shell-script stand-ins
//! for the real programs on PATH (so no tool or database is needed), then replayed.
//!
//! The stand-ins check the flags they are given against the ones the wrappers may send, write
//! files shaped like the real outputs, and print a `--version` line like the real tool's.
#![cfg(unix)]

mod common;

use common::{Client, sha256};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Shared helpers for the stand-ins: refuse an unknown flag like the real tools do.
const PRELUDE: &str = r#"#!/bin/sh
set -eu
die() { echo "$0: $*" >&2; exit 2; }
"#;

const FASTP: &str = r#"
[ "${1:-}" = --version ] && { echo "fastp 1.4.0"; exit 0; }
in1= ; json= ; html= ; out1= ; out2=
for a in "$@"; do
  case "$a" in
    --in1=*) in1=${a#--in1=} ;;
    --in2=*|--report_title=*|--thread=*|--compression=*|--adapter_sequence=*|--adapter_sequence_r2=*) ;;
    --trim_front1=*|--trim_tail1=*|--cut_window_size=*|--cut_mean_quality=*|--qualified_quality_phred=*) ;;
    --unqualified_percent_limit=*|--n_base_limit=*|--average_qual=*|--length_required=*) ;;
    --json=*) json=${a#--json=} ;;
    --html=*) html=${a#--html=} ;;
    --out1=*) out1=${a#--out1=} ;;
    --out2=*) out2=${a#--out2=} ;;
    --phred64|--detect_adapter_for_pe|--disable_adapter_trimming|--trim_poly_g|--disable_trim_poly_g) ;;
    --cut_front|--cut_tail|--cut_right|--correction|--dedup|--overrepresentation_analysis) ;;
    *) die "unknown option $a" ;;
  esac
done
case "$in1" in *fail*) echo "ERROR: $in1 is not a valid FASTQ" >&2; exit 255 ;; esac
cat > "$json" <<EOF
{"summary": {"before_filtering": {"total_reads": 600, "total_bases": 36000},
 "after_filtering": {"total_reads": 580, "total_bases": 34000, "q30_rate": 0.93, "q30_bases": 31620}},
 "duplication": {"rate": 0.01}, "command": "fastp --in1 $in1"}
EOF
echo "<html>fastp report</html>" > "$html"
[ -n "$out1" ] && cp "$in1" "$out1"
[ -n "$out2" ] && cp "$in1" "$out2"
exit 0
"#;

const BAKTA: &str = r#"
[ "${1:-}" = --version ] && { echo "bakta 1.12.2"; exit 0; }
out= ; prefix= ; db= ; genome=
while [ $# -gt 0 ]; do
  case "$1" in
    --db=*) db=${1#--db=} ;;
    --output=*) out=${1#--output=} ;;
    --prefix=*) prefix=${1#--prefix=} ;;
    --force|--complete|--keep-contig-headers|--compliant|--meta|--skip-*) ;;
    --min-contig-length=*|--genus=*|--species=*|--strain=*|--plasmid=*|--translation-table=*|--gram=*) ;;
    --locus=*|--locus-tag=*|--threads=*) ;;
    --) shift; genome=$1; break ;;
    *) die "unknown option $1" ;;
  esac
  shift
done
[ -f "$db/version.json" ] || die "database version file (version.json) not readable"
[ -f "$genome" ] || die "genome not found"
mkdir -p "$out"
[ "$prefix" = nowrite ] && exit 0
for ext in json tsv gff3 gbff embl fna ffn faa; do echo "$ext of $genome" > "$out/$prefix.$ext"; done
printf 'Sequence(s):\nLength: 4641652\nCount: 1\nGC: 50.8\nN50: 4641652\n\nAnnotation:\ntRNAs: 86\nCDSs: 4310\nhypotheticals: 112\n\nBakta:\nSoftware: v1.12.2\nDatabase: v6.0, light\n' > "$out/$prefix.txt"
"#;

const CHECKM2: &str = r#"
[ "${1:-}" = --version ] && { echo "1.1.0"; exit 0; }
[ "${1:-}" = predict ] || die "expected predict"
shift
out= ; inputs=
while [ $# -gt 0 ]; do
  case "$1" in
    --output-directory=*) out=${1#--output-directory=} ;;
    --database_path=*) [ -f "${1#--database_path=}" ] || die "no database" ;;
    --force|--lowmem|--general|--specific|--allmodels|--genes|--remove_intermediates) ;;
    --threads=*|--ttable=*|--extension=*) ;;
    --input) shift; inputs="$*"; break ;;
    *) die "unknown option $1" ;;
  esac
  shift
done
mkdir -p "$out"
printf 'Name\tCompleteness\tContamination\tCompleteness_Model_Used\n' > "$out/quality_report.tsv"
for i in $inputs; do
  if [ -d "$i" ]; then set -- "$i"/*.fna; else set -- "$i"; fi
  for f in "$@"; do b=$(basename "$f"); printf '%s\t99.1\t0.4\tGradient Boost\n' "${b%.*}" >> "$out/quality_report.tsv"; done
done
"#;

const GTDBTK: &str = r#"
case "${1:-}" in --version) echo "gtdbtk: version 2.7.2 Copyright 2017 Pierre-Alain Chaumeil"; echo "There is a newer version of GTDB-Tk available: v2.8.0"; exit 0 ;; esac
[ "$1" = classify_wf ] || die "expected classify_wf"
shift
[ -f "${GTDBTK_DATA_PATH:-}/metadata/metadata.txt" ] || die "GTDBTK_DATA_PATH not set"
out= ; dir= ; ext=fna
for a in "$@"; do
  case "$a" in
    --genome_dir=*) dir=${a#--genome_dir=} ;;
    --out_dir=*) out=${a#--out_dir=} ;;
    --extension=*) ext=${a#--extension=} ;;
    --prefix=*|--cpus=*|--pplacer_cpus=*|--scratch_dir=*|--min_af=*|--min_perc_aa=*) ;;
    --full_tree|--place_species|--keep_intermediates|--write_single_copy_genes|--force|--genes) ;;
    *) die "unknown option $a" ;;
  esac
done
mkdir -p "$out/classify"
s="$out/classify/gtdbtk.bac120.summary.tsv"
printf 'user_genome\tclassification\tclosest_genome_reference\tclosest_genome_ani\tclassification_method\tnote\twarnings\n' > "$s"
for f in "$dir"/*."$ext"; do b=$(basename "$f" ".$ext"); printf '%s\td__Bacteria;p__Pseudomonadota;s__Escherichia coli\tGCF_003697165.2\t98.9\tANI\tN/A\tN/A\n' "$b" >> "$s"; done
ln -sf classify/gtdbtk.bac120.summary.tsv "$out/gtdbtk.bac120.summary.tsv"
"#;

fn script(dir: &Path, name: &str, body: &str) {
    let p = dir.join(name);
    std::fs::write(&p, format!("{PRELUDE}{body}")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn write(dir: &Path, name: &str, text: &str) {
    let p = dir.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Same marker files Bakta 1.12.2's `db.check` wants, as empty stand-ins.
fn fake_bakta_db(dir: &Path) {
    write(dir, "version.json", r#"{"date": "2025-02-24", "major": 6, "minor": 0, "type": "light"}"#);
    for f in [
        "antifam.h3f", "antifam.h3i", "antifam.h3m", "antifam.h3p", "bakta.db", "expert-protein-sequences.dmnd", "ncRNA-genes.i1f",
        "ncRNA-genes.i1i", "ncRNA-genes.i1m", "ncRNA-genes.i1p", "ncRNA-regions.i1f", "ncRNA-regions.i1i", "ncRNA-regions.i1m",
        "ncRNA-regions.i1p", "oric.fna", "orit.fna", "pfam.h3f", "pfam.h3i", "pfam.h3m", "pfam.h3p", "rfam-go.tsv", "rRNA.i1f", "rRNA.i1i",
        "rRNA.i1m", "rRNA.i1p", "sorf.dmnd", "pscc.dmnd",
    ] {
        write(dir, f, "");
    }
}

struct Setup {
    _tmp: tempfile::TempDir,
    work: PathBuf,
    path: String,
}

fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let work = std::fs::canonicalize(tmp.path()).unwrap();
    let tools = work.join("fake-bin");
    std::fs::create_dir(&tools).unwrap();
    for (n, b) in [("fastp", FASTP), ("bakta", BAKTA), ("checkm2", CHECKM2), ("gtdbtk", GTDBTK)] {
        script(&tools, n, b);
    }
    for (n, seq) in [("reads/s1_R1.fastq", "@r1\nACGT\n+\nIIII\n"), ("reads/s1_R2.fastq", "@r1\nTTGA\n+\nIIII\n")] {
        write(&work, n, seq);
    }
    write(&work, "reads/fail_R1.fastq", "@r1\nACGT\n+\nIIII\n");
    write(&work, "asm/s1.fna", ">contig_1\nACGTACGT\n");
    write(&work, "asm/s2.fna", ">contig_1\nGGGTACGT\n");
    fake_bakta_db(&work.join("db/bakta/db-light"));
    write(&work, "db/checkm2/CheckM2_database/uniref100.KO.1.dmnd", "");
    write(&work, "db/gtdbtk/release232/metadata/metadata.txt", "VERSION_DATA=r232\n");
    for d in ["markers", "masks", "mrca_red", "pplacer", "radii", "split", "taxonomy", "skani"] {
        std::fs::create_dir_all(work.join("db/gtdbtk/release232").join(d)).unwrap();
    }
    let path = format!("{}:{}", tools.display(), std::env::var("PATH").unwrap_or_default());
    Setup { _tmp: tmp, work, path }
}

fn outputs(call: &Value) -> Vec<(PathBuf, String)> {
    call["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| Some((PathBuf::from(o["path"].as_str()?), o["sha256"].as_str()?.to_string())))
        .collect()
}

#[test]
fn isolate_path_with_stand_in_tools_is_recorded_and_replayable() {
    let s = setup();
    let work = &s.work;
    let bakta_db = work.join("db/bakta/db-light");
    let mut c = Client::spawn(work, &[], &[("PATH", &s.path), ("BAKTA_DB", bakta_db.to_str().unwrap())]);

    let tools = c.request("tools/list", json!({}));
    let list = tools["result"]["tools"].as_array().unwrap();
    for want in ["fastp", "bakta", "checkm2", "gtdbtk_classify_wf"] {
        let t = list.iter().find(|t| t["name"] == want).unwrap_or_else(|| panic!("{want} missing"));
        assert_eq!(t["inputSchema"]["additionalProperties"], false, "{want}");
    }

    // 1: fastp, paired-end
    let r = c.call(
        "fastp",
        json!({"in1": "reads/s1_R1.fastq", "in2": "reads/s1_R2.fastq", "out1": "trim/s1_R1.fastq", "out2": "trim/s1_R2.fastq",
               "json": "trim/s1.fastp.json", "detect_adapter_for_pe": true}),
    );
    assert_eq!(r["isError"], true, "trim/ doesn't exist yet: {r}");
    assert_eq!(r["structuredContent"]["field"], "out1");
    std::fs::create_dir(work.join("trim")).unwrap();
    let r = c.call(
        "fastp",
        json!({"in1": "reads/s1_R1.fastq", "in2": "reads/s1_R2.fastq", "out1": "trim/s1_R1.fastq", "out2": "trim/s1_R2.fastq",
               "json": "trim/s1.fastp.json", "detect_adapter_for_pe": true}),
    );
    assert_ne!(r["isError"], true, "{r}");
    let v = &r["structuredContent"];
    assert_eq!(v["summary"]["reads_raw"], 600, "{v}");
    assert_eq!(v["summary"]["reads_trimmed"], 580);
    assert_eq!(v["summary"]["q30_trimmed"], 93.0);
    assert_eq!(v["report"]["html"], "trim/s1.fastp.html");
    assert_eq!(v["version"], "fastp 1.4.0");
    assert_eq!(v["warnings"], json!([]));

    // 2: fastp refuses an output that exists, and reports a failing run as a failure
    let r = c.call("fastp", json!({"in1": "reads/s1_R1.fastq", "json": "trim/s1.fastp.json"}));
    assert_eq!(r["structuredContent"]["field"], "json", "{r}");
    let r = c.call("fastp", json!({"in1": "reads/fail_R1.fastq", "json": "trim/fail.fastp.json"}));
    assert_eq!(r["structuredContent"]["error"], "tool_failed", "{r}");
    assert!(r["structuredContent"]["stderr"].as_str().unwrap().contains("not a valid FASTQ"));

    // 3: Bakta with the database from $BAKTA_DB
    std::fs::create_dir(work.join("bakta")).unwrap();
    let r = c.call("bakta", json!({"genome": "asm/s1.fna", "outdir": "bakta/s1", "genus": "Escherichia", "skip": ["plot"]}));
    assert_ne!(r["isError"], true, "{r}");
    let v = &r["structuredContent"];
    assert_eq!(v["summary"]["annotation"]["cdss"], 4310, "{v}");
    assert_eq!(v["summary"]["sequences"]["gc"], 50.8);
    assert!(v["command"].as_str().unwrap().contains(&format!("--db={}", bakta_db.display())), "{v}");

    // 4: exit 0 without the summary files is not a success
    let r = c.call("bakta", json!({"genome": "asm/s2.fna", "outdir": "bakta/s2", "prefix": "nowrite"}));
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["error"], "missing_outputs");
    assert!(r["structuredContent"]["message"].as_str().unwrap().contains("nowrite.txt"));

    // 5: a reads file given as a genome is refused before anything runs
    let r = c.call("bakta", json!({"genome": "reads/s1_R1.fastq", "outdir": "bakta/x"}));
    assert_eq!(r["structuredContent"]["field"], "genome", "{r}");

    // 6: CheckM2 on the assembly folder
    let r = c.call("checkm2", json!({"genome_dir": "asm", "outdir": "checkm2", "db": "db/checkm2", "lowmem": true, "threads": 2}));
    assert_ne!(r["isError"], true, "{r}");
    let g = &r["structuredContent"]["genomes"];
    assert_eq!(g, &json!([{"genome": "s1", "completeness": 99.1, "contamination": 0.4}, {"genome": "s2", "completeness": 99.1, "contamination": 0.4}]));

    // 7: GTDB-Tk; the stand-in claims 2.7.2, so the result carries a version warning
    let r = c.call(
        "gtdbtk_classify_wf",
        json!({"genome_dir": "asm", "out_dir": "gtdbtk", "data_path": "db/gtdbtk/release232", "cpus": 2, "pplacer_cpus": 1}),
    );
    assert_ne!(r["isError"], true, "{r}");
    let v = &r["structuredContent"];
    assert_eq!(v["genomes"][0]["genome"], "s1", "{v}");
    assert_eq!(v["genomes"][0]["gtdb"], "d__Bacteria;p__Pseudomonadota;s__Escherichia coli");
    assert_eq!(v["genomes"][1]["classification_method"], "ANI");
    assert_eq!(v["version"], "gtdbtk: version 2.7.2 Copyright 2017 Pierre-Alain Chaumeil");
    assert!(v["warnings"][0].as_str().unwrap().contains("2.7.2"), "{v}");

    // a flag GTDB-Tk 2.8.0 doesn't have never reaches it
    let r = c.call("gtdbtk_classify_wf", json!({"genome_dir": "asm", "out_dir": "g2", "skip_ani_screen": true}));
    assert!(r.get("error").is_some() || r["isError"] == true, "{r}");
    c.close();

    // run record
    let run_dir = work.join("irit-runs/test-run");
    let log: Value = serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    let calls = log["calls"].as_array().unwrap();
    let by_tool = |t: &str| calls.iter().filter(|c| c["mcp_tool"] == t && c["status"] == "ok").collect::<Vec<_>>();
    let fastp = by_tool("fastp")[0];
    assert_eq!(fastp["external"], true);
    assert_eq!(fastp["version"], "fastp 1.4.0");
    assert_eq!(
        fastp["argv"],
        json!(["fastp", "--in1=reads/s1_R1.fastq", "--in2=reads/s1_R2.fastq", "--out1=trim/s1_R1.fastq", "--out2=trim/s1_R2.fastq",
               "--json=trim/s1.fastp.json", "--html=trim/s1.fastp.html", "--detect_adapter_for_pe"])
    );
    assert_eq!(outputs(fastp).len(), 4);
    let gtdb = by_tool("gtdbtk_classify_wf")[0];
    assert_eq!(gtdb["env"]["GTDBTK_DATA_PATH"], work.join("db/gtdbtk/release232").to_str().unwrap());
    assert!(gtdb["inputs"].as_array().unwrap().iter().any(|i| i["path"].as_str().unwrap().ends_with("metadata/metadata.txt")));
    let missing = calls.iter().find(|c| c["params"]["prefix"] == "nowrite").unwrap();
    assert_eq!(missing["status"], "failed");
    assert_eq!(missing["exit_status"], 0);

    let script = std::fs::read_to_string(run_dir.join("replay.sh")).unwrap();
    assert!(script.contains("want_ext_version fastp 'fastp 1.4.0'"), "{script}");
    assert!(script.contains("step 002 0 ext fastp --in1=reads/s1_R1.fastq"), "{script}");
    assert!(script.contains(&format!("GTDBTK_DATA_PATH={}; export GTDBTK_DATA_PATH", work.join("db/gtdbtk/release232").display())));
    let crate_json = std::fs::read_to_string(run_dir.join("ro-crate-metadata.json")).unwrap();
    assert!(crate_json.contains("https://github.com/oschwengers/bakta") && crate_json.contains("#gtdbtk-2.7.2"));

    // replay without the server: delete the outputs, re-run, same hashes
    let all: Vec<(PathBuf, String)> = calls.iter().filter(|c| c["status"] == "ok").flat_map(outputs).collect();
    assert!(all.len() >= 15, "{all:?}");
    for d in ["trim", "bakta", "checkm2", "gtdbtk"] {
        std::fs::remove_dir_all(work.join(d)).unwrap();
    }
    std::fs::create_dir(work.join("trim")).unwrap();
    std::fs::create_dir(work.join("bakta")).unwrap();
    let replay = || Command::new("sh").arg(run_dir.join("replay.sh")).env("PATH", &s.path).current_dir(work).output().unwrap();
    let out = replay();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "replay failed: {err}");
    assert!(err.contains("outputs match"), "{err}");
    // the run recorded 2.7.2's first --version line, and that's what the stand-in prints first
    assert!(!err.contains("warning"), "{err}");
    for (p, h) in &all {
        assert_eq!(&sha256(p), h, "{} differs after replay", p.display());
    }

    // a tool missing from PATH at replay time fails the replay instead of passing silently
    let bare = "/usr/bin:/bin";
    let out = Command::new("sh").arg(run_dir.join("replay.sh")).env("PATH", bare).current_dir(work).output().unwrap();
    assert!(!out.status.success());
}

/// A tool that isn't installed is an error with an install hint, recorded as such; nothing runs.
#[test]
fn missing_tool_gives_an_install_hint() {
    let s = setup();
    let work = &s.work;
    let empty = work.join("empty-bin");
    std::fs::create_dir(&empty).unwrap();
    let mut c = Client::spawn(work, &["--tool-path", "gtdbtk=/nonexistent/gtdbtk"], &[("PATH", empty.to_str().unwrap())]);
    let r = c.call("fastp", json!({"in1": "reads/s1_R1.fastq", "json": "s1.fastp.json"}));
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["error"], "tool_unavailable");
    let msg = r["structuredContent"]["message"].as_str().unwrap();
    assert!(msg.contains("bioconda fastp=1.4.0") && msg.contains("--tool-path"), "{msg}");
    let r = c.call("gtdbtk_classify_wf", json!({"genome_dir": "asm", "out_dir": "gtdbtk", "data_path": "db/gtdbtk/release232"}));
    assert!(r["structuredContent"]["message"].as_str().unwrap().contains("/nonexistent/gtdbtk"), "{r}");
    c.close();
    let log: Value = serde_json::from_slice(&std::fs::read(work.join("irit-runs/test-run/run.json")).unwrap()).unwrap();
    assert_eq!(log["calls"][0]["status"], "error");
    assert!(!work.join("s1.fastp.json").exists());

    // an unknown --tool-path name is refused at startup
    let out = Command::new(env!("CARGO_BIN_EXE_irit-mcp")).args(["--tool-path", "prokka=/x"]).current_dir(work).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not one of fastp, bakta, checkm2, gtdbtk"));
}
