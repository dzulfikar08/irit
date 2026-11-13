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
out= ; inputs= ; ext=fna
while [ $# -gt 0 ]; do
  case "$1" in
    --output-directory=*) out=${1#--output-directory=} ;;
    --database_path=*) [ -f "${1#--database_path=}" ] || die "no database" ;;
    --force|--lowmem|--general|--specific|--allmodels|--genes|--remove_intermediates) ;;
    --extension=*) ext=${1#--extension=}; ext=${ext#.} ;;
    --threads=*|--ttable=*) ;;
    --input) shift; inputs="$*"; break ;;
    *) die "unknown option $1" ;;
  esac
  shift
done
mkdir -p "$out"
printf 'Name\tCompleteness\tContamination\tCompleteness_Model_Used\n' > "$out/quality_report.tsv"
for i in $inputs; do
  if [ -d "$i" ]; then set -- "$i"/*."$ext"; else set -- "$i"; fi
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

const SHOVILL: &str = r#"
[ "${1:-}" = --version ] && { echo "shovill 1.4.2"; exit 0; }
out= ; r1= ; force=0
for a in "$@"; do
  case "$a" in
    --R1=*) r1=${a#--R1=} ;;
    --R2=*) ;;
    --outdir=*) out=${a#--outdir=} ;;
    --force) force=1 ;;
    --assembler=*|--gsize=*|--depth=*|--minlen=*|--mincov=*|--namefmt=*|--cpus=*|--ram=*|--kmers=*) ;;
    --keepfiles|--plasmid|--trim|--noreadcorr|--nostitch|--nocorr) ;;
    *) die "unknown option $a" ;;
  esac
done
[ -d "$out" ] && [ $force = 0 ] && die "Folder '$out' already exists. Try using --force"
mkdir -p "$out"
echo "[shovill] started $(date +%s%N)" > "$out/shovill.log"
{ echo '>contig00001 len=12000 cov=40.0'; printf '%12000s\n' '' | tr ' ' A; echo '>contig00002 len=8000 cov=38.0'; printf '%8000s\n' '' | tr ' ' C; } > "$out/contigs.fa"
"#;

const MLST: &str = r#"
[ "${1:-}" = --version ] && { echo "mlst 2.35.0"; exit 0; }
out= ; json= ; label= ; full=0
while [ $# -gt 0 ]; do
  case "$1" in
    --outfile=*) out=${1#--outfile=} ;;
    --json=*) json=${1#--json=} ;;
    --novel=*) ;;
    --label=*) label=${1#--label=} ;;
    --full) full=1 ;;
    --scheme=*|--minid=*|--mincov=*|--minscore=*|--threads=*) ;;
    --) shift; break ;;
    *) die "unknown option $1" ;;
  esac
  shift
done
[ $full = 1 ] && printf 'FILE\tSCHEME\tST\tSTATUS\tSCORE\tALLELES\n' > "$out" || : > "$out"
for f in "$@"; do
  [ -f "$f" ] || die "Unable to read from '$f'"
  printf '%s\tsaureus\t22\tarcC(7)\taroE(6)\tglpF(1)\tgmk(5)\tpta(8)\ttpi(8)\tyqiL(6)\n' "${label:-$f}" >> "$out"
done
[ -n "$json" ] && printf '[{"id": "%s", "scheme": "saureus", "sequence_type": "22"}]\n' "${label:-$1}" > "$json"
exit 0
"#;

const AMRFINDER: &str = r#"
[ "${1:-}" = --version ] && { echo "4.2.7"; exit 0; }
out= ; db= ; name= ; plus=0
for a in "$@"; do
  case "$a" in
    --output=*) out=${a#--output=} ;;
    --database=*) db=${a#--database=} ;;
    --name=*) name=${a#--name=} ;;
    --plus) plus=1 ;;
    --nucleotide=*|--protein=*|--gff=*|--annotation_format=*|--organism=*|--ident_min=*|--coverage_min=*) ;;
    --translation_table=*|--threads=*|--mutation_all=*|--protein_output=*|--nucleotide_output=*) ;;
    --report_common|--report_all_equal|--print_node) ;;
    *) die "unknown option $a" ;;
  esac
done
[ -f "$db/AMRProt.fa.phr" ] || die "The BLAST database for AMRProt.fa ($db/AMRProt.fa.phr) was not found."
h='Protein id\tContig id\tStart\tStop\tStrand\tElement symbol\tElement name\tScope\tType\tSubtype\tClass\tSubclass\tMethod\tTarget length\tReference sequence length\t%% Coverage of reference\t%% Identity to reference\tAlignment length\tClosest reference accession\tClosest reference name\tHMM accession\tHMM description\n'
r1='NA\tcontig00001\t1\t2007\t+\tmecA\tPBP2a family beta-lactam-resistant peptidoglycan transpeptidase MecA\tcore\tAMR\tAMR\tBETA-LACTAM\tMETHICILLIN\tEXACTX\t668\t668\t100.00\t100.00\t668\tWP_000721309.1\tMecA\tNA\tNA\n'
r2='NA\tcontig00002\t5\t850\t-\tblaZ\tpenicillin-hydrolyzing class A beta-lactamase BlaZ\tcore\tAMR\tAMR\tBETA-LACTAM\tBETA-LACTAM\tALLELEX\t281\t281\t100.00\t100.00\t281\tWP_000733283.1\tBlaZ\tNA\tNA\n'
r3='NA\tcontig00001\t90\t900\t+\tsea\tenterotoxin type A\tplus\tVIRULENCE\tVIRULENCE\tNA\tNA\tEXACTX\t257\t257\t100.00\t100.00\t257\tWP_000750426.1\tSEA\tNA\tNA\n'
[ $plus = 1 ] || r3=
if [ -n "$name" ]; then
  printf "Name\t$h$name\t$r1$name\t$r2" > "$out"
  if [ -n "$r3" ]; then printf "$name\t$r3" >> "$out"; fi
else
  printf "$h$r1$r2$r3" > "$out"
fi
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

/// An indexed AMRFinderPlus 2026-10-29.1 database as empty stand-ins: what amrfinder_update 4.2.7
/// downloads and the index files irit-mcp checks for.
fn fake_amrfinder_db(dir: &Path) {
    for f in [
        "AMR.LIB", "AMRProt.fa", "AMRProt-mutation.tsv", "AMRProt-suppress.tsv", "AMRProt-susceptible.fa", "AMRProt-susceptible.tsv",
        "AMR_CDS.fa", "fam.tsv", "AMRProt.fa.phr", "AMR.LIB.h3m", "AMR_CDS.fa.nin", "AMR_DNA-Staphylococcus_aureus.fa",
        "AMR_DNA-Staphylococcus_aureus.fa.ndb",
    ] {
        write(dir, f, "");
    }
    write(dir, "version.txt", "2026-10-29.1\n");
    write(dir, "database_format_version.txt", "4.2.0\n");
    write(dir, "taxgroup.tsv", "#taxgroup\tgpipe_taxgroup\tnumber_of_nucl_ref_genes\nStaphylococcus_aureus\tStaphylococcus_aureus\t2\n");
}

const MEGAHIT: &str = r#"
[ "${1:-}" = --version ] && { echo "MEGAHIT v1.2.9"; exit 0; }
out= ; reads=
while [ $# -gt 0 ]; do
  case "$1" in
    -1|-2|-r) shift; reads="$reads,$1" ;;
    --out-dir=*) out=${1#--out-dir=} ;;
    --memory=*) [ "${1#--memory=}" -gt 1 ] || die "memory given as a fraction of the machine" ;;
    --force|--num-cpu-threads=*|--presets=*|--k-list=*|--min-contig-len=*|--min-count=*) ;;
    *) die "unknown option $1" ;;
  esac
  shift
done
[ -n "$reads" ] || die "no reads"
for f in $(echo "$reads" | tr ',' ' '); do [ -f "$f" ] || die "cannot open $f"; done
mkdir -p "$out"
echo "started $(date +%s%N)" > "$out/log"
printf '>k141_1 flag=1 multi=5.0000 len=12\nACGTACGTACGT\n>k141_2 flag=1 multi=3.0000 len=8\nGGGGCCCC\n' > "$out/final.contigs.fa"
"#;

const COVERM: &str = r#"
[ "${1:-}" = --version ] && { echo "coverm 0.8.0"; exit 0; }
[ "${1:-}" = contig ] || die "expected contig"
shift
out= ; ref= ; methods= ; mode=
while [ $# -gt 0 ]; do
  case "$1" in
    --reference=*) ref=${1#--reference=} ;;
    --output-file=*) out=${1#--output-file=} ;;
    --mapper=*|--threads=*|--min-read-percent-identity=*|--min-read-aligned-percent=*) ;;
    --methods) mode=m ;;
    --coupled) mode=c ;;
    -*) die "unknown option $1" ;;
    *) case "$mode" in m) methods="$methods $1" ;; c) [ -f "$1" ] || die "cannot open $1" ;; *) die "stray $1" ;; esac ;;
  esac
  shift
done
[ -f "$ref" ] || die "no reference"
case "$methods" in
  *metabat*) printf 'contigName\tcontigLen\ttotalAvgDepth\ts1.bam\ts1.bam-var\nk141_1\t12\t5\t5\t0.5\nk141_2\t8\t3\t3\t0.2\n' > "$out" ;;
  *) printf 'Contig\ts1 Mean\nk141_1\t5\nk141_2\t3\n' > "$out" ;;
esac
echo "[2026-11-13T05:10:00Z INFO  coverm] In sample 'final.contigs.fa/s1_R1.fastq', found 90 reads mapped out of 100 total (90.00%)" >&2
"#;

const METABAT2: &str = r#"
case "${1:-}" in
  --help) { echo; echo "MetaBAT: Metagenome Binning based on Abundance and Tetranucleotide frequency (version 2:2.18; 20260506_185317)"; } >&2; exit 0 ;;
  --version) echo "unrecognised option '--version'" >&2; exit 1 ;;
esac
in= ; out=
for a in "$@"; do
  case "$a" in
    --inFile=*) in=${a#--inFile=} ;;
    --outFile=*) out=${a#--outFile=} ;;
    --abdFile=*) [ -f "${a#--abdFile=}" ] || die "no depth file" ;;
    --minContig=*|--minClsSize=*|--seed=*|--numThreads=*|--unbinned|--noSampleDepths) ;;
    *) die "unknown option $a" ;;
  esac
done
[ -f "$in" ] || die "no contigs"
[ -d "$(dirname "$out")" ] || die "output folder missing"
printf '>k141_1\nACGTACGTACGT\n' > "$out.1.fa"
printf '>k141_2\nGGGGCCCC\n' > "$out.2.fa"
"#;

/// The irit binaries (isotable): `$IRIT_BIN_DIR`, else the workspace's target/release or target/debug.
fn irit_bin_dir() -> PathBuf {
    let exe = format!("isotable{}", std::env::consts::EXE_SUFFIX);
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target");
    let candidates: Vec<PathBuf> = match std::env::var_os("IRIT_BIN_DIR") {
        Some(d) => vec![d.into()],
        None => vec![target.join("release"), target.join("debug")],
    };
    let dir = candidates.iter().find(|d| d.join(&exe).is_file()).unwrap_or_else(|| {
        panic!("isotable not found in {candidates:?}: run `cargo build --release -p isotable` in the workspace root or set IRIT_BIN_DIR")
    });
    std::fs::canonicalize(dir).unwrap()
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
    for (n, b) in [
        ("fastp", FASTP),
        ("bakta", BAKTA),
        ("checkm2", CHECKM2),
        ("gtdbtk", GTDBTK),
        ("shovill", SHOVILL),
        ("mlst", MLST),
        ("amrfinder", AMRFINDER),
        ("megahit", MEGAHIT),
        ("coverm", COVERM),
        ("metabat2", METABAT2),
    ] {
        script(&tools, n, b);
    }
    fake_amrfinder_db(&work.join("db/amrfinderplus/2026-10-29.1"));
    // mlst's bundled data, where the stand-in's `../db` is (as in Bioconda's package).
    for (f, text) in [("VERSION.txt", "2025-12-29\n"), ("scheme_species_map.tab", "#SCHEME\tGENUS\tSPECIES\n"),
        ("blast/mlst.fa.nsq", "x"), ("pubmlst/saureus/saureus.txt", "ST\tarcC\n22\t7\n")] {
        write(&work, &format!("db/{f}"), text);
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
    std::fs::create_dir_all(work.join("db/gtdbtk/release232/split/backbone/pplacer/gtdbtk_package_backbone.refpkg")).unwrap();
    write(&work, "db/gtdbtk/release232/split/class_level/tree_mapping.tsv", "");
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
        json!({"in1": "reads/s1_R1.fastq", "in2": "reads/s1_R2.fastq", "out1": "reads/s1_R1.fastq/trim.fq", "out2": "trim/s1_R2.fastq",
               "json": "trim/s1.fastp.json", "detect_adapter_for_pe": true}),
    );
    assert_eq!(r["isError"], true, "a file is in the way: {r}");
    assert_eq!(r["structuredContent"]["field"], "out1");
    assert!(!work.join("trim").exists(), "nothing is created for a refused call");
    // trim/ doesn't exist yet: the server creates it
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

    // 3: Bakta with the database from $BAKTA_DB (bakta/ is created by the server)
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
    // the data is in no dblock store and no call of this run wrote it (#8); then the version
    let w = v["warnings"].as_array().unwrap();
    assert!(w[0].as_str().unwrap().contains("is not in a dblock store and no earlier call of this run wrote it"), "{v}");
    assert!(w[1].as_str().unwrap().contains("2.7.2"), "{v}");

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
    assert_eq!(fastp["dirs"], json!([work.join("trim")]));
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
    // replay.sh creates the output folders itself
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

/// Metagenome to MAGs with stand-ins: MEGAHIT, CoverM (MetaBAT depth), MetaBAT2, CheckM2 on the
/// bins and isotable in MAG mode, then replayed.
#[test]
fn mag_path_with_stand_in_tools_is_recorded_and_replayable() {
    let s = setup();
    let work = &s.work;
    let bin = irit_bin_dir();
    let mut c = Client::spawn(work, &["--bin-dir", bin.to_str().unwrap()], &[("PATH", &s.path)]);
    let tools = c.request("tools/list", json!({}));
    let list = tools["result"]["tools"].as_array().unwrap();
    for want in ["megahit", "coverm_contig", "metabat2"] {
        let t = list.iter().find(|t| t["name"] == want).unwrap_or_else(|| panic!("{want} missing"));
        assert_eq!(t["inputSchema"]["additionalProperties"], false, "{want}");
    }
    // 1: MEGAHIT, memory in bytes (4 GB by default, not MEGAHIT's 90% of the machine)
    let r = c.call("megahit", json!({"r1": ["reads/s1_R1.fastq"], "r2": ["reads/s1_R2.fastq"], "outdir": "asm/meta", "threads": 2}));
    assert_ne!(r["isError"], true, "{r}");
    let v = &r["structuredContent"];
    assert_eq!(v["assembly"]["contigs"], 2, "{v}");
    assert_eq!(v["assembly"]["total_length"], 20);
    assert_eq!(v["version"], "MEGAHIT v1.2.9");
    assert_eq!(v["warnings"], json!([]));
    assert!(v["command"].as_str().unwrap().contains("--memory=4000000000"), "{v}");
    // 2: CoverM's MetaBAT depth table
    let r = c.call(
        "coverm_contig",
        json!({"reference": "asm/meta/final.contigs.fa", "r1": ["reads/s1_R1.fastq"], "r2": ["reads/s1_R2.fastq"],
               "output": "depth/s1.depth.tsv", "methods": ["metabat"], "threads": 2}),
    );
    assert_ne!(r["isError"], true, "{r}");
    let v = &r["structuredContent"];
    assert_eq!(v["contigs_with_coverage"], 2, "{v}");
    assert_eq!(v["mapped"][0]["percent"], 90.0);
    assert_eq!(v["version"], "coverm 0.8.0");
    // 3: MetaBAT2; its version comes from --help on stderr
    let r = c.call("metabat2", json!({"contigs": "asm/meta/final.contigs.fa", "depth": "depth/s1.depth.tsv", "outdir": "bins/s1"}));
    assert_ne!(r["isError"], true, "{r}");
    let v = &r["structuredContent"];
    assert_eq!(v["bin_count"], 2, "{v}");
    assert_eq!(v["bins"][0]["bin"], "bin.1");
    assert_eq!(v["binned_length"], 20);
    assert!(v["version"].as_str().unwrap().contains("version 2:2.18"), "{v}");
    assert_eq!(v["warnings"], json!([]), "{v}");
    assert!(v["command"].as_str().unwrap().contains("--seed=1"), "{v}");
    // a plain coverage table is not a depth table
    let r = c.call(
        "coverm_contig",
        json!({"reference": "asm/meta/final.contigs.fa", "r1": ["reads/s1_R1.fastq"], "r2": ["reads/s1_R2.fastq"], "output": "depth/mean.tsv"}),
    );
    assert_ne!(r["isError"], true, "{r}");
    let r = c.call("metabat2", json!({"contigs": "asm/meta/final.contigs.fa", "depth": "depth/mean.tsv", "outdir": "bins/x"}));
    assert_eq!(r["structuredContent"]["field"], "depth", "{r}");
    // 4: CheckM2 on the bins, then isotable in MAG mode
    let r = c.call("checkm2", json!({"genome_dir": "bins/s1", "extension": "fa", "outdir": "checkm2", "db": "db/checkm2"}));
    assert_ne!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["genomes"][1]["genome"], "bin.2", "{r}");
    let r = c.call("isotable", json!({"dirs": ["checkm2"], "mode": "mag", "output": "mags.tsv"}));
    assert_ne!(r["isError"], true, "{r}");
    let samples = r["structuredContent"]["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 2, "{r}");
    assert_eq!(samples[0]["values"]["mimag_tier"], "high*", "{r}");
    c.close();

    // run record and replay
    let run_dir = work.join("irit-runs/test-run");
    let log: Value = serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    let ok: Vec<&Value> = log["calls"].as_array().unwrap().iter().filter(|c| c["status"] == "ok").collect();
    assert_eq!(
        ok.iter().map(|c| c["mcp_tool"].as_str().unwrap()).collect::<Vec<_>>(),
        ["megahit", "coverm_contig", "metabat2", "coverm_contig", "checkm2", "isotable"]
    );
    // MEGAHIT's outputs are volatile (as Shovill's); the bins and depth table are not
    assert!(ok[0]["outputs"].as_array().unwrap().iter().all(|o| o["volatile"] == true), "{}", ok[0]["outputs"]);
    assert!(ok[2]["outputs"].as_array().unwrap().iter().all(|o| o["volatile"] != true), "{}", ok[2]["outputs"]);
    let script = std::fs::read_to_string(run_dir.join("replay.sh")).unwrap();
    assert!(script.contains("want_ext_version metabat2 "), "{script}");
    let all: Vec<(PathBuf, String)> = ok.iter().flat_map(|c| outputs(c)).collect();
    for d in ["asm", "depth", "bins", "checkm2"] {
        std::fs::remove_dir_all(work.join(d)).unwrap();
    }
    std::fs::remove_file(work.join("mags.tsv")).unwrap();
    let path = format!("{}:{}", bin.display(), s.path);
    let out = Command::new("sh").arg(run_dir.join("replay.sh")).env("PATH", &path).current_dir(work).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "replay failed: {err}");
    assert!(err.contains("outputs match"), "{err}");
    // MetaBAT2's version is read from --help on replay too, so there is no mismatch warning
    assert!(!err.contains("warning: metabat2"), "{err}");
    for (p, h) in &all {
        if p.ends_with("log") {
            assert!(p.is_file());
            continue;
        }
        assert_eq!(&sha256(p), h, "{} differs after replay", p.display());
    }
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
    assert!(String::from_utf8_lossy(&out.stderr).contains("not one of fastp, shovill, flye, medaka_consensus, bakta, checkm2, gtdbtk, mlst, amrfinder"));
}

/// Reads to assembly (Shovill), typing (mlst) and resistance genes (AMRFinderPlus), then isotable
/// over the results: the sample names line up although Shovill always writes `contigs.fa`.
#[test]
fn assembly_typing_and_amr_with_stand_ins_join_in_isotable_and_replay() {
    let s = setup();
    let work = &s.work;
    let amr_db = work.join("db/amrfinderplus/2026-10-29.1");
    let bin = irit_bin_dir();
    let mut c = Client::spawn(
        work,
        &["--bin-dir", bin.to_str().unwrap()],
        &[("PATH", &s.path), ("AMRFINDER_DB", amr_db.to_str().unwrap())],
    );
    let tools = c.request("tools/list", json!({}));
    let list = tools["result"]["tools"].as_array().unwrap();
    for want in ["shovill", "mlst", "amrfinder"] {
        let t = list.iter().find(|t| t["name"] == want).unwrap_or_else(|| panic!("{want} missing"));
        assert_eq!(t["inputSchema"]["additionalProperties"], false, "{want}");
    }
    // 1: Shovill
    let r = c.call(
        "shovill",
        json!({"r1": "reads/s1_R1.fastq", "r2": "reads/s1_R2.fastq", "outdir": "asm/s1", "assembler": "skesa", "nocorr": true, "ram": 8,
               "link_as": "genomes/s1.fna"}),
    );
    assert_ne!(r["isError"], true, "{r}");
    // genomes/s1.fna names the assembly for CheckM2 and GTDB-Tk; a relative link, so the project can move
    assert_eq!(std::fs::read_link(work.join("genomes/s1.fna")).unwrap(), Path::new("../asm/s1/contigs.fa"));
    assert_eq!(std::fs::read(work.join("genomes/s1.fna")).unwrap(), std::fs::read(work.join("asm/s1/contigs.fa")).unwrap());
    let v = &r["structuredContent"];
    assert_eq!(v["assembly"], json!({"contigs": 2, "total_length": 20000, "n50": 12000, "longest": 12000, "gc_percent": 40.0}), "{v}");
    assert_eq!(v["version"], "shovill 1.4.2");
    assert_eq!(v["warnings"], json!([]));
    // a second call into the same folder needs overwrite
    let r = c.call("shovill", json!({"r1": "reads/s1_R1.fastq", "r2": "reads/s1_R2.fastq", "outdir": "asm/s1"}));
    assert_eq!(r["structuredContent"]["field"], "outdir", "{r}");

    // 2: mlst; the label comes from the output name, not from contigs.fa
    let r = c.call("mlst", json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/s1.mlst.tsv", "json": "typing/s1.mlst.json"}));
    assert_ne!(r["isError"], true, "{r}");
    let res = &r["structuredContent"]["results"][0];
    assert_eq!((res["sample"].as_str(), res["scheme"].as_str(), res["st"].as_str()), (Some("s1"), Some("saureus"), Some("22")), "{r}");
    assert!(r["structuredContent"]["command"].as_str().unwrap().contains("--label=s1"));
    assert_eq!(r["structuredContent"]["database_version"], "2025-12-29", "{r}");
    let r = c.call("mlst", json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/s1.tsv"}));
    assert_eq!(r["structuredContent"]["field"], "outfile", "{r}");

    // 3: AMRFinderPlus with the database from $AMRFINDER_DB
    let r = c.call(
        "amrfinder",
        json!({"nucleotide": "asm/s1/contigs.fa", "organism": "Staphylococcus_aureus", "plus": true, "output": "typing/s1.amrfinder.tsv"}),
    );
    assert_ne!(r["isError"], true, "{r}");
    let v = &r["structuredContent"];
    assert_eq!(v["database_version"], "2026-10-29.1", "{v}");
    assert_eq!(v["summary"]["amr_genes"], "blaZ,mecA");
    assert_eq!(v["summary"]["counts_by_type"], json!({"AMR": 2, "VIRULENCE": 1}));
    assert_eq!(v["summary"]["hits"][0]["element_symbol"], "mecA");
    assert_eq!(v["summary"]["hits"][0]["identity_to_reference"], "100.00");
    assert!(v["command"].as_str().unwrap().contains(&format!("--database={}", amr_db.display())));
    // an organism the database doesn't have is refused before amrfinder runs
    let r = c.call("amrfinder", json!({"nucleotide": "asm/s1/contigs.fa", "organism": "Escherichia", "output": "typing/x.amrfinder.tsv"}));
    assert_eq!(r["structuredContent"]["field"], "organism", "{r}");

    // 4: isotable joins mlst and AMRFinderPlus to sample s1
    // contigs and N50 come from Shovill's contigs.fa (no QUAST in irit-mcp)
    let r = c.call("isotable", json!({"dirs": ["asm", "typing"], "require": ["contigs", "n50", "mlst", "amr"], "output": "typing/isotable.tsv"}));
    assert_ne!(r["isError"], true, "{r}");
    let samples = r["structuredContent"]["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 1, "{r}");
    let s1 = &samples[0];
    assert_eq!(s1["sample"], "s1");
    assert_eq!(s1["values"]["mlst_st"], "22", "{s1}");
    assert_eq!(s1["values"]["amr_genes"], "blaZ,mecA");
    assert_eq!((&s1["values"]["contigs"], &s1["values"]["n50"]), (&json!("2"), &json!("12000")), "{s1}");
    assert_eq!(s1["status"], "pass", "{s1}");
    c.close();

    // run record and replay
    let run_dir = work.join("irit-runs/test-run");
    let log: Value = serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    let calls = log["calls"].as_array().unwrap();
    let ok: Vec<&Value> = calls.iter().filter(|c| c["status"] == "ok").collect();
    assert_eq!(ok.iter().map(|c| c["mcp_tool"].as_str().unwrap()).collect::<Vec<_>>(), ["shovill", "mlst", "amrfinder", "isotable"]);
    let mlst_inputs: Vec<&str> = ok[1]["inputs"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
    for want in ["db/VERSION.txt", "db/scheme_species_map.tab", "db/blast/mlst.fa.nsq", "db/pubmlst/saureus/saureus.txt"] {
        assert!(mlst_inputs.iter().any(|p| p.ends_with(want)), "{want} not in {mlst_inputs:?}");
    }
    let amr = ok[2];
    assert_eq!(amr["version"], "4.2.7");
    assert!(amr["inputs"].as_array().unwrap().iter().any(|i| i["path"].as_str().unwrap().ends_with("2026-10-29.1/version.txt")));
    let shovill_log = outputs(ok[0]).into_iter().find(|(p, _)| p.ends_with("shovill.log")).unwrap();
    assert!(ok[0]["outputs"].as_array().unwrap().iter().any(|o| o["path"] == shovill_log.0.to_str().unwrap() && o["volatile"] == true));
    // contigs.fa too (dated headers; SKESA and SPAdes vary between identical runs): only its existence is replayed
    assert!(ok[0]["outputs"].as_array().unwrap().iter().all(|o| o["volatile"] == true), "{}", ok[0]["outputs"]);
    let script = std::fs::read_to_string(run_dir.join("replay.sh")).unwrap();
    assert!(script.contains("want_ext_version amrfinder 4.2.7"), "{script}");
    assert!(script.contains("want_ext_version shovill 'shovill 1.4.2'"), "{script}");

    let all: Vec<(PathBuf, String)> = ok.iter().flat_map(|c| outputs(c)).collect();
    assert!(script.contains("ln -sfn -- ../asm/s1/contigs.fa "), "{script}");
    for d in ["asm", "typing", "genomes"] {
        std::fs::remove_dir_all(work.join(d)).unwrap();
    }
    let path = format!("{}:{}", bin.display(), s.path);
    let out = Command::new("sh").arg(run_dir.join("replay.sh")).env("PATH", &path).current_dir(work).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "replay failed: {err}");
    assert!(err.contains("outputs match"), "{err}");
    assert_eq!(std::fs::read_link(work.join("genomes/s1.fna")).unwrap(), Path::new("../asm/s1/contigs.fa"));
    for (p, h) in &all {
        if p.ends_with("shovill.log") {
            assert!(p.is_file());
            continue;
        }
        assert_eq!(&sha256(p), h, "{} differs after replay", p.display());
    }
}

/// Shovill whose assembly changes every run (as SKESA's did on the laptop): one base more each time.
const SHOVILL_UNSTABLE: &str = r#"
[ "${1:-}" = --version ] && { echo "shovill 1.4.2"; exit 0; }
out=
for a in "$@"; do case "$a" in --outdir=*) out=${a#--outdir=} ;; esac; done
mkdir -p "$out"
runs="$(dirname "$0")/.shovill-runs"
n=$(( $(cat "$runs" 2>/dev/null || echo 0) + 1 )); echo $n > "$runs"
echo "[shovill] run $n" > "$out/shovill.log"
{ printf '>contig00001 date=%s\n' "$(date +%Y%m%d)"; printf "%$((12000 + n))s\n" '' | tr ' ' A; printf '>contig00002\n'; printf '%8000s\n' '' | tr ' ' C; } > "$out/contigs.fa"
"#;

/// CheckM2 reporting each genome's size, so its report follows the assembly.
const CHECKM2_SIZED: &str = r#"
[ "${1:-}" = --version ] && { echo "1.1.0"; exit 0; }
out= ; dir=
for a in "$@"; do case "$a" in --output-directory=*) out=${a#--output-directory=} ;; --*) ;; *) dir=$a ;; esac; done
mkdir -p "$out"
printf 'Name\tCompleteness\tContamination\tGenome_Size\n' > "$out/quality_report.tsv"
for f in "$dir"/*.fna; do b=$(basename "$f" .fna); printf '%s\t100.0\t0.03\t%s\n' "$b" "$(wc -c < "$f" | tr -d ' ')" >> "$out/quality_report.tsv"; done
"#;

/// The B. subtilis laptop run's shape with an assembler that is not reproducible: from a folder
/// with every output deleted, irit-mcp's replay.sh reruns everything; the assembly differs and is
/// reported with its contigs, length and N50 beside the run's; CheckM2's report and isotable's row,
/// made from it, are reported as following it and don't fail the replay; the trimmed reads still
/// match. An output that changes without such a cause still fails.
#[test]
fn outputs_made_from_a_volatile_assembly_follow_it_on_replay() {
    let s = setup();
    let work = &s.work;
    script(&work.join("fake-bin"), "shovill", SHOVILL_UNSTABLE);
    script(&work.join("fake-bin"), "checkm2", CHECKM2_SIZED);
    let bin = irit_bin_dir();
    let mut c = Client::spawn(work, &["--bin-dir", bin.to_str().unwrap()], &[("PATH", &s.path)]);
    let mut ok = |tool: &str, args: Value| {
        let r = c.call(tool, args);
        assert_ne!(r["isError"], true, "{r}");
        r["structuredContent"].clone()
    };
    ok("fastp", json!({"in1": "reads/s1_R1.fastq", "in2": "reads/s1_R2.fastq", "out1": "trim/b168_R1.fastq", "out2": "trim/b168_R2.fastq", "json": "trim/b168.fastp.json"}));
    let v = ok("shovill", json!({"r1": "trim/b168_R1.fastq", "r2": "trim/b168_R2.fastq", "outdir": "asm/b168", "link_as": "genomes/b168.fna", "ram": 8}));
    assert_eq!(v["assembly"]["total_length"], 20001, "{v}");
    ok("checkm2", json!({"genome_dir": "genomes", "outdir": "checkm2", "db": "db/checkm2"}));
    let v = ok("isotable", json!({"dirs": ["trim", "asm", "checkm2"], "output": "b168.isotable.tsv"}));
    assert_eq!(v["samples"][0]["values"]["assembly_length"], "20001", "{v}");
    c.close();

    let run_dir = work.join("irit-runs/test-run");
    let log: Value = serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    let shovill = log["calls"].as_array().unwrap().iter().find(|c| c["mcp_tool"] == "shovill").unwrap();
    assert_eq!(shovill["assembly"]["n50"], 12001, "{shovill}");
    let fastp_json = work.join("trim/b168.fastp.json");
    let fastp_hash = sha256(&fastp_json);
    for d in ["trim", "asm", "genomes", "checkm2"] {
        std::fs::remove_dir_all(work.join(d)).unwrap();
    }
    std::fs::remove_file(work.join("b168.isotable.tsv")).unwrap();
    let path = format!("{}:{}", bin.display(), s.path);
    let replay = |script: &str| Command::new("sh").arg(run_dir.join(script)).env("PATH", &path).current_dir(work).output().unwrap();
    let out = replay("replay.sh");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "replay failed: {err}");
    let asm = work.join("asm/b168/contigs.fa");
    assert!(err.contains(&format!("assembly {} is now 2 contigs, 20002 bp, N50 12002; the run had 2 contigs, 20001 bp, N50 12001", asm.display())), "{err}");
    assert!(err.contains(&format!("reads {}, which differs from the run", work.join("genomes/b168.fna").display())), "{err}");
    for f in ["checkm2/quality_report.tsv", "b168.isotable.tsv"] {
        assert!(err.contains(&format!("from an input that differs: {}", work.join(f).display())), "{f}: {err}");
    }
    assert!(err.contains("differ only because a file they were made from differs"), "{err}");
    assert!(!err.lines().any(|l| l.starts_with("replay: differs: ")), "{err}");
    assert_eq!(sha256(&fastp_json), fastp_hash);

    // fastp's JSON is not made from the assembly: if it changes, the replay fails
    let broken = std::fs::read_to_string(run_dir.join("replay.sh")).unwrap().replace("step 001 0 ext fastp", "step 001 0 ext true");
    std::fs::write(run_dir.join("replay-broken.sh"), broken).unwrap();
    std::fs::write(&fastp_json, "{}").unwrap();
    let out = replay("replay-broken.sh");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && err.contains(&format!("replay: differs: {}", fastp_json.display())), "{err}");
}

/// gtdbprep ani: writes `<work>/gtdbtk_data` (links to the data's folders, plus `skani/database`
/// linking to `<work>/db`), `assigned.json`, and prints its per-genome table, like the real one.
const GTDBPREP: &str = r#"
[ "${1:-}" = --version ] && { echo "gtdbprep 0.3.4"; exit 0; }
[ "${1:-}" = ani ] || die "expected ani"
shift
genomes= ; ext=fna ; work= ; data=
for a in "$@"; do
  case "$a" in
    --genomes=*) genomes=${a#--genomes=} ;;
    --extension=*) ext=${a#--extension=} ;;
    --data=*) data=${a#--data=} ;;
    --work=*) work=${a#--work=} ;;
    --index=*|--cache=*|--source=*|--part-size=*|--fetch-jobs=*|--env=*|--skani=*|--cpus=*|--species-only) ;;
    *) die "unknown option $a" ;;
  esac
done
mkdir -p "$work/db"
echo "sketches of 21 references" > "$work/db/sketches.db"
rm -rf "$work/gtdbtk_data"; mkdir -p "$work/gtdbtk_data/skani"
for d in "$data"/*; do [ "$(basename "$d")" = skani ] || ln -s "$(cd "$d" && pwd -P)" "$work/gtdbtk_data/$(basename "$d")"; done
ln -s "$(cd "$work/db" && pwd -P)" "$work/gtdbtk_data/skani/database"
printf 'genome\tmarkers\tcandidates\tassigned_by_ani\n'
for f in "$genomes"/*."$ext"; do printf '%s\t120\t21\ttrue\n' "$(basename "$f" ".$ext")"; done
echo '["s1"]' > "$work/assigned.json"
"#;

/// GTDB-Tk's data folder (#8): in a dblock store, or written by an earlier call of the run
/// (gtdbprep_ani's `<work>/gtdbtk_data`, whose small skani database is then hashed into the
/// call's inputs under its real path), it is accounted for; anywhere else the call warns.
#[test]
fn gtdbtk_data_from_the_run_or_a_store_is_vouched_for_else_warned_about() {
    let s = setup();
    let work = &s.work;
    script(&work.join("fake-bin"), "gtdbprep", GTDBPREP);
    for f in ["meta.json", "refs.bin", "markers.idx"] {
        write(work, &format!("db/ani-screen-r232/{f}"), "x");
    }
    write(work, "db/gtdbtk/release232/taxonomy/gtdb_taxonomy.tsv", "x\n");
    write(work, "db/gtdbtk/release232/radii/gtdb_radii.tsv", "x\n");
    let cache = work.join("cache");
    let store = work.join("store");
    // a second session in the same folder needs its own run id
    let spawn = |store: &Path, run_id: &str| {
        Client::spawn(work, &["--run-id", run_id], &[("PATH", &s.path), ("XDG_CACHE_HOME", cache.to_str().unwrap()), ("DBLOCK_STORE", store.to_str().unwrap())])
    };
    let mut c = spawn(&store, "test-run");
    let warned = |r: &Value| {
        assert_ne!(r["isError"], true, "{r}");
        r["structuredContent"]["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("not in a dblock store"))
    };
    let r = c.call("gtdbtk_classify_wf", json!({"genome_dir": "asm", "out_dir": "g1", "data_path": "db/gtdbtk/release232"}));
    assert!(warned(&r), "{r}");
    let r = c.call("gtdbprep_ani", json!({"genome_dir": "asm", "index": "db/ani-screen-r232", "data_path": "db/gtdbtk/release232", "work": "ani"}));
    assert_ne!(r["isError"], true, "{r}");
    let r = c.call("gtdbtk_classify_wf", json!({"genome_dir": "asm", "out_dir": "g2", "data_path": "ani/gtdbtk_data"}));
    assert!(!warned(&r), "{r}");
    c.close();
    let log: Value = serde_json::from_slice(&std::fs::read(work.join("irit-runs/test-run/run.json")).unwrap()).unwrap();
    let calls = log["calls"].as_array().unwrap();
    assert!(calls[0]["warnings"][0].as_str().unwrap().contains("not in a dblock store"), "{}", calls[0]);
    let sketches = work.join("ani/db/sketches.db");
    assert!(calls[2]["inputs"].as_array().unwrap().iter().any(|i| i["path"] == sketches.to_str().unwrap()), "{}", calls[2]);
    assert!(calls[2].get("warnings").is_none(), "{}", calls[2]);
    // the same data inside the dblock store: no warning
    let mut c = spawn(&work.join("db"), "test-run-2");
    let r = c.call("gtdbtk_classify_wf", json!({"genome_dir": "asm", "out_dir": "g3", "data_path": "db/gtdbtk/release232"}));
    assert!(!warned(&r), "{r}");
    c.close();
}
