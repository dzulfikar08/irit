//! `sheetgen verify` end to end: exit status and the JSON report.

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

fn md5_hex(data: &[u8]) -> String {
    use md5::Digest;
    md5::Md5::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}
