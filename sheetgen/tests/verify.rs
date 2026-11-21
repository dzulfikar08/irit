//! `sheetgen verify` end to end: exit status and the JSON report, and its failed-file list as
//! `sheetgen --exclude-from` input.

use std::io::Write;
use std::process::Command;

fn gz(text: &str) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(text.as_bytes()).unwrap();
    e.finish().unwrap()
}

fn verify(args: &[&str]) -> (i32, serde_json::Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_sheetgen")).arg("verify").args(args).arg("--json").output().unwrap();
    let report = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));
    (out.status.code().unwrap(), report)
}

#[test]
fn folder_with_md5_list() {
    let t = tempfile::tempdir().unwrap();
    let good = gz("@a\nACGT\n+\nIIII\n");
    std::fs::write(t.path().join("good_R1.fastq.gz"), &good).unwrap();
    std::fs::write(t.path().join("cut_R2.fastq.gz"), &good[..good.len() - 3]).unwrap();
    std::fs::write(t.path().join("plain.fq"), "@a\nAC\n+\nII\n").unwrap();
    let md5 = t.path().join("MD5SUMS");
    let sum = md5_hex(&good);
    std::fs::write(&md5, format!("{sum}  good_R1.fastq.gz\n{sum}  cut_R2.fastq.gz\n{sum}  gone.fastq.gz\n")).unwrap();
    let dir = t.path().to_str().unwrap();

    let (code, r) = verify(&[dir, "--md5", md5.to_str().unwrap()]);
    assert_eq!(code, 1);
    let status = |name: &str| {
        let f = r["files"].as_array().unwrap().iter().find(|f| f["path"].as_str().unwrap().ends_with(name)).unwrap();
        (f["status"].as_str().unwrap().to_string(), f.clone())
    };
    let (s, f) = status("good_R1.fastq.gz");
    assert_eq!(s, "ok");
    assert_eq!((f["reads"].as_u64(), f["bases"].as_u64(), f["md5"].as_str()), (Some(1), Some(4), Some(sum.as_str())));
    let (s, f) = status("cut_R2.fastq.gz");
    assert_eq!(s, "corrupt");
    assert!(f["problem"]["what"].as_str().unwrap().contains("truncated"), "{f}");
    assert_eq!(status("plain.fq").0, "not listed");
    assert_eq!(r["summary"]["corrupt"], 1);
    assert!(r["listed_not_checked"][0].as_str().unwrap().ends_with("gone.fastq.gz"));

    // Without a checksum source, a structurally fine file passes.
    let (code, r) = verify(&[t.path().join("plain.fq").to_str().unwrap()]);
    assert_eq!((code, r["ok"].as_bool()), (0, Some(true)));
}

#[test]
fn sample_sheet_files_are_checked() {
    let t = tempfile::tempdir().unwrap();
    let r1 = t.path().join("s_R1.fastq.gz");
    std::fs::write(&r1, gz("@a\nACGT\n+\nIIII\n")).unwrap();
    let sheet = t.path().join("samplesheet.csv");
    std::fs::write(&sheet, format!("sample,fastq_1,fastq_2\ns,{},\n", r1.display())).unwrap();
    let (code, r) = verify(&["--sheet", sheet.to_str().unwrap()]);
    assert_eq!(code, 0, "{r}");
    assert_eq!(r["files"].as_array().unwrap().len(), 1);
}

/// `verify --failed` lists the bad file; `sheetgen --exclude-from` that list leaves its sample out
/// (both lanes), and `--exclude` leaves out a sample by name.
#[test]
fn failed_files_leave_their_samples_out_of_the_sheet() {
    let t = tempfile::tempdir().unwrap();
    let reads = t.path().join("reads");
    std::fs::create_dir(&reads).unwrap();
    let good = gz("@a\nACGT\n+\nIIII\n");
    for name in ["A_S1_L001_R1_001.fastq.gz", "A_S1_L001_R2_001.fastq.gz", "B_S2_L001_R1_001.fastq.gz", "B_S2_L002_R1_001.fastq.gz"] {
        std::fs::write(reads.join(name), &good).unwrap();
    }
    std::fs::write(reads.join("B_S2_L001_R2_001.fastq.gz"), &good[..good.len() - 3]).unwrap();
    std::fs::write(reads.join("B_S2_L002_R2_001.fastq.gz"), &good).unwrap();
    std::fs::write(reads.join("C_R1.fastq.gz"), &good).unwrap();
    std::fs::write(reads.join("C_R2.fastq.gz"), &good).unwrap();
    let failed = t.path().join("failed.txt");
    let (code, _) = verify(&[reads.to_str().unwrap(), "--failed", failed.to_str().unwrap()]);
    assert_eq!(code, 1);
    let list = std::fs::read_to_string(&failed).unwrap();
    assert_eq!(list.lines().count(), 1, "{list}");
    assert!(list.trim_end().ends_with("B_S2_L001_R2_001.fastq.gz"), "{list}");

    let sheetgen = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_sheetgen")).arg(&reads).args(args).output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        assert!(o.status.success(), "{err}");
        let samples: Vec<String> =
            String::from_utf8_lossy(&o.stdout).lines().skip(1).map(|l| l.split(',').next().unwrap().to_string()).collect();
        (samples, err)
    };
    let (samples, err) = sheetgen(&["--exclude-from", failed.to_str().unwrap()]);
    assert_eq!(samples, ["A", "C"]);
    assert!(err.contains("note: left out (--exclude): B"), "{err}");
    // by name, repeatable; a name that matches nothing is a warning
    let (samples, err) = sheetgen(&["--exclude", "A", "--exclude", "C", "--exclude", "nope"]);
    assert_eq!(samples, ["B", "B"]);
    assert!(err.contains("warning: --exclude nope: no such sample or read file"), "{err}");
    // a file name alone (the list made elsewhere) works too
    std::fs::write(&failed, "# from another machine\n/elsewhere/C_R2.fastq.gz\n").unwrap();
    assert_eq!(sheetgen(&["--exclude-from", failed.to_str().unwrap()]).0, ["A", "B", "B"]);
    // leaving out every sample is an error
    let o = Command::new(env!("CARGO_BIN_EXE_sheetgen")).arg(&reads).args(["--exclude", "A", "--exclude", "B", "--exclude", "C"]).output().unwrap();
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stderr).contains("every sample was excluded"));
}

fn md5_hex(data: &[u8]) -> String {
    use md5::Digest;
    md5::Md5::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}
