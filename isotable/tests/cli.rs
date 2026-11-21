//! Runs the isotable binary on the aggregator outputs in `testdata/`: hAMRonization 1.4.1 tables
//! (with a native AMRFinderPlus file for one genome) and nf-core/mag bin summaries in the 5.5.0 and
//! 5.0.0 layouts. See the README in each folder for how they were made.

use std::path::{Path, PathBuf};
use std::process::Command;

fn testdata(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(rel)
}

/// (stdout, stderr); panics if isotable fails.
fn isotable(args: &[&str]) -> (String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_isotable")).args(args).output().expect("run isotable");
    let (out, err) = (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned());
    assert!(o.status.success(), "{err}");
    (out, err)
}

/// Rows of the output table by sample, each a column → value map.
fn rows(tsv: &str) -> Vec<std::collections::BTreeMap<String, String>> {
    let mut lines = tsv.lines();
    let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
    lines.map(|l| header.iter().map(|h| h.to_string()).zip(l.split('\t').map(String::from)).collect()).collect()
}

#[test]
fn hamronization_tables_fill_the_amr_columns() {
    let dir = testdata("hamronization");
    let (out, err) = isotable(&[dir.to_str().unwrap(), "--require", "amr"]);
    let rows = rows(&out);
    assert_eq!(rows.len(), 2, "{out}");
    let g = &rows[0];
    assert_eq!(g["sample"], "GCF_010120755.1_ASM1012075v1_genomic");
    assert_eq!(g["status"], "pass");
    assert_eq!(g["amr_genes"], "NDM-1,aacA-ENT1,cirA_Y253CfsTer5,eat(A),gyrA_S83I,mgrB_Q30Ter,msr(C),nalC,nalC p.G71E");
    assert_eq!(
        g["amr_by_tool"],
        "abricate: aacA-ENT1,eat(A),msr(C); amrfinderplus: cirA_Y253CfsTer5,gyrA_S83I,mgrB_Q30Ter; rgi: NDM-1,nalC,nalC p.G71E"
    );
    assert_eq!((rows[1]["sample"].as_str(), rows[1]["amr_by_tool"].as_str()), ("SAMPLE_B", "amrfinderplus: blaTEM-156,pmrB p.C84R,sul2"));
    assert!(err.contains("from 1 AMRFinderPlus, 2 hAMRonization"), "{err}");
    assert!(err.contains("note: AMRFinderPlus rows of hAMRonization table") && err.contains("own AMRFinderPlus output is used"), "{err}");
}

#[test]
fn nf_core_mag_bin_summary_in_mag_mode() {
    for version in ["5.5.0", "5.0.0"] {
        let dir = testdata(&format!("nfcore-mag/{version}"));
        let (out, err) = isotable(&[dir.to_str().unwrap(), "--mode", "mag"]);
        let rows = rows(&out);
        let got: Vec<(&str, &str, &str)> =
            rows.iter().map(|r| (r["sample"].as_str(), r["status"].as_str(), r["mimag_tier"].as_str())).collect();
        assert_eq!(
            got,
            [
                ("MEGAHIT-MetaBAT2-sample1.1", "pass", "high*"),
                ("MEGAHIT-MetaBAT2-sample1.2", "warn", "medium"),
                ("MEGAHIT-MetaBAT2-sample1.3", "warn", "medium"),
                ("MEGAHIT-MetaBAT2-sample1.4", "fail", "low"),
                ("MEGAHIT-MetaBAT2-sample1.5", "fail", "low"),
            ],
            "{version}: {out}"
        );
        assert_eq!(rows[2]["checkm2_completeness"], "64");
        assert!(rows[2]["reasons"].contains("note: Patescibacteria have reduced genomes"), "{version}: {out}");
        assert!(rows[3]["reasons"].contains("note: Patescibacteria"), "{version}: {out}");
        assert!(!rows[0]["reasons"].contains("Patescibacteria"), "{version}: {out}");
        assert!(err.contains("from 1 nf-core/mag"), "{version}: {err}");
    }
    // 5.5.0 has BUSCO and CheckM beside CheckM2: shown, and CheckM2 sets the tier
    let (out, _) = isotable(&[testdata("nfcore-mag/5.5.0").to_str().unwrap(), "--mode", "mag"]);
    let r = &rows(&out)[2];
    assert_eq!((r["checkm_completeness"].as_str(), r["busco_complete"].as_str()), ("41", "55.3"));
    assert_eq!(r["mag_depths"], "sample1=5.5,sample2=0.4");
    // high* is explained: a column on the row, a note at the end, the MultiQC table's description
    assert_eq!((rows(&out)[0]["mimag_rrna_trna"].as_str(), rows(&out)[1]["mimag_rrna_trna"].as_str()), ("not checked", ""));
    let t = tempfile::tempdir().unwrap();
    let mqc = t.path().join("mqc.tsv");
    let (_, err) = isotable(&[testdata("nfcore-mag/5.5.0").to_str().unwrap(), "--mode", "mag", "--mqc", mqc.to_str().unwrap()]);
    assert!(err.contains("note: high* = MIMAG high quality by completeness (> 90%) and contamination (< 5%) only"), "{err}");
    assert!(std::fs::read_to_string(&mqc).unwrap().contains("# description: 'high* = MIMAG high quality"));
}

/// A sheet in MAG mode: bins keep their rows and get the sample their name contains; samples with
/// no bins get no rows, and names that match no sample give one warning, not one each.
#[test]
fn mag_mode_with_a_sheet() {
    let t = tempfile::tempdir().unwrap();
    let mut report = String::from("Name\tCompleteness\tContamination\n");
    for (bin, c) in [("D01-S1.1", 97.0), ("D01-S1.2", 60.0), ("D01-S2_SemiBin_4", 95.0), ("bin.7", 70.0), ("bin.8", 70.0)] {
        report.push_str(&format!("{bin}\t{c}\t1.0\n"));
    }
    std::fs::write(t.path().join("quality_report.tsv"), report).unwrap();
    // D01-S3 has read QC but no bins: it still counts as a sample with no bins
    std::fs::write(
        t.path().join("D01-S3.fastp.json"),
        r#"{"summary": {"before_filtering": {"total_reads": 100, "total_bases": 15000}, "after_filtering": {"total_reads": 90, "total_bases": 13000, "q30_rate": 0.9}}}"#,
    )
    .unwrap();
    let sheet = t.path().join("sheet.csv");
    std::fs::write(&sheet, "sample,fastq_1,fastq_2\nD01-S1,a_1.fq.gz,a_2.fq.gz\nD01-S2,b_1.fq.gz,b_2.fq.gz\nD01-S3,c_1.fq.gz,c_2.fq.gz\n").unwrap();
    let (out, err) = isotable(&[t.path().to_str().unwrap(), "--mode", "mag", "--sheet", sheet.to_str().unwrap()]);
    let got: Vec<(String, String, String)> =
        rows(&out).iter().filter(|r| r["sample"] != "D01-S3").map(|r| (r["sample"].clone(), r["bin_sample"].clone(), r["status"].clone())).collect();
    let want = [("D01-S1.1", "D01-S1", "pass"), ("D01-S1.2", "D01-S1", "warn"), ("D01-S2_SemiBin_4", "D01-S2", "pass"), ("bin.7", "", "warn"), ("bin.8", "", "warn")];
    assert_eq!(got, want.map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())), "{out}");
    assert_eq!(err.matches("warning").count(), 1, "{err}");
    assert!(err.contains("warning: 2 names matched no sheet sample; they got rows of their own, with no bin_sample: bin.7, bin.8"), "{err}");
    assert!(rows(&out).iter().any(|r| r["sample"] == "D01-S3" && r["reads_raw"] == "100"), "{out}");
    assert!(err.contains("note: 1 of 3 sheet samples have no bins here: D01-S3"), "{err}");
    assert!(!err.contains("not an exact name match"), "{err}");
}

#[test]
fn nf_core_mag_bin_summary_is_not_read_in_isolate_mode() {
    let o = Command::new(env!("CARGO_BIN_EXE_isotable")).arg(testdata("nfcore-mag/5.5.0")).output().unwrap();
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("no recognised tool outputs") && err.contains("read with --mode mag"), "{err}");
    // beside other outputs it is listed as skipped
    let (_, err) = isotable(&[testdata("nfcore-mag/5.5.0").to_str().unwrap(), testdata("hamronization").to_str().unwrap()]);
    assert!(err.contains("bin_summary.tsv (nf-core/mag bin summary; read with --mode mag)"), "{err}");
}
