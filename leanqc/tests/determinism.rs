//! The JSON report must be byte-for-byte the same for the same input, whatever `--threads` is
//! and however often it runs (irit-mcp's `replay.sh` checks it by SHA-256).

use std::io::Write;
use std::path::Path;
use std::process::Command;

/// Reads with many duplication levels, and more distinct sequences than `--dup-distinct`, so the
/// duplication estimate sums many corrected per-level counts (the float that used to change).
fn write_reads(path: &Path, seed: u64) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut x = seed;
    let mut next = || {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        x >> 33
    };
    for i in 0..20_000u32 {
        // Small pools give high duplication levels, the large one many singletons.
        let pool = match next() % 10 {
            0..=3 => 50,
            4..=6 => 2_000,
            _ => 1_000_000,
        };
        let id = next() % pool;
        let mut s = id;
        let seq: String = (0..60)
            .map(|_| {
                s = s.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
                b"ACGT"[(s >> 60) as usize % 4] as char
            })
            .collect();
        writeln!(f, "@r{i}\n{seq}\n+\n{}", "I".repeat(60)).unwrap();
    }
}

fn run(files: &[&Path], out: &Path, threads: u32) {
    let status = Command::new(env!("CARGO_BIN_EXE_leanqc"))
        .args(["--no-html", "--dup-distinct", "3000", "--threads", &threads.to_string(), "--outdir"])
        .arg(out)
        .args(files)
        .output()
        .unwrap();
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
}

#[test]
fn json_is_identical_across_threads_and_runs() {
    let d = tempfile::tempdir().unwrap();
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let (a, b) = (d.path().join("a.fastq"), d.path().join("b.fastq"));
    write_reads(&a, 1);
    write_reads(&b, 2);
    let dup = corpus.join("duplicates.fastq.gz");
    let files = [a.as_path(), b.as_path(), dup.as_path()];
    let mut first: Option<Vec<Vec<u8>>> = None;
    for (n, threads) in [1, 2, 3, 1, 2, 3, 4, 4].into_iter().enumerate() {
        let out = d.path().join(format!("out{n}"));
        run(&files, &out, threads);
        let jsons: Vec<Vec<u8>> = ["a_leanqc.json", "b_leanqc.json", "duplicates_leanqc.json"]
            .iter()
            .map(|j| std::fs::read(out.join(j)).unwrap())
            .collect();
        match &first {
            None => first = Some(jsons),
            Some(f) => {
                for (i, (x, y)) in f.iter().zip(&jsons).enumerate() {
                    assert!(x == y, "file {i} differs with --threads {threads} (run {n})");
                }
            }
        }
    }
    // The estimate is really exercised: distinct sequences hit the tracking limit.
    let j: serde_json::Value = serde_json::from_slice(&first.unwrap()[0]).unwrap();
    let dup = &j["duplication"];
    assert!(dup["reads_at_limit"].as_u64().unwrap() < dup["reads"].as_u64().unwrap(), "{dup}");
}
