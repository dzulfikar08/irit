//! End-to-end round trip against real GTDB-Tk. Ignored by default; run with
//!
//! ```sh
//! GTDBPREP_ENV=/path/to/gtdbtk-2.8-env \
//! GTDBTK_DATA_PATH=/path/to/release232_mockup \
//! GTDBPREP_TEST_GENOMES=/path/to/folder-with-test_genome_G021783475.fna.gz \
//! cargo test --release -- --ignored --nocapture
//! ```
//!
//! Optional: `GTDBPREP_SKANI=/path/to/skani-0.3.1` adds sketches to the bundle,
//! `GTDBPREP_TEST_EXT` (default `gz`) selects the input files. The test copies them into a folder
//! whose name has a space, adds an empty input that prep must leave out, and works in a folder
//! without a space (GTDB-Tk refuses spaces in genome paths).

use std::path::PathBuf;
use std::process::Command;

fn var(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {name} (see the top of tests/roundtrip.rs)"))
}

#[test]
#[ignore = "needs a GTDB-Tk 2.8 environment and reference data"]
fn bundle_classification_equals_classify_wf() {
    let env = var("GTDBPREP_ENV");
    let data = var("GTDBTK_DATA_PATH");
    let genomes = var("GTDBPREP_TEST_GENOMES");
    let ext = std::env::var("GTDBPREP_TEST_EXT").unwrap_or_else(|_| "gz".into());
    assert!(
        genomes.join("test_genome_G021783475.fna.gz").is_file() || ext != "gz",
        "GTDBPREP_TEST_GENOMES should contain GTDB's test genome test_genome_G021783475.fna.gz"
    );
    // Paths with spaces, and one empty input that prep must leave out (not stop on).
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("in put");
    std::fs::create_dir_all(&input).unwrap();
    for e in std::fs::read_dir(&genomes).unwrap() {
        let p = e.unwrap().path();
        if p.is_file() && p.to_string_lossy().ends_with(&ext) {
            std::fs::copy(&p, input.join(p.file_name().unwrap())).unwrap();
        }
    }
    let empty = format!("empty_input.{ext}");
    std::fs::write(input.join(&empty), b"").unwrap();
    // GTDB-Tk 2.8.0 refuses genome paths with a space, so the work folder (where the renamed
    // genomes are) cannot have one; prep refuses it up front. The input folder can.
    let work = tmp.path().join("work_dir");
    let tarball = tmp.path().join("out dir/bundle.tar.gz");
    std::fs::create_dir_all(tarball.parent().unwrap()).unwrap();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gtdbprep"));
    cmd.arg("verify")
        .arg("--genomes")
        .arg(&input)
        .args(["-x", &ext, "--cpus", "4", "--work"])
        .arg(&work)
        .arg("--out")
        .arg(&tarball)
        .arg("--env")
        .arg(&env)
        .arg("--data")
        .arg(&data);
    if let Some(skani) = std::env::var_os("GTDBPREP_SKANI") {
        cmd.arg("--sketch").arg("--skani").arg(skani);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        out.status.success(),
        "verify reported differences or failed"
    );
    assert!(stdout.contains("test_genome_G021783475.fna\tidentical") || ext != "gz");
    assert!(!stdout.contains("\tdifferent"));
    let empty_id = empty.strip_suffix(&ext).unwrap().trim_end_matches('.');
    assert!(
        stdout.contains(&format!("{empty_id}\texcluded")),
        "the empty input is reported as left out"
    );

    // The mapping and original names stay local: not in the tarball.
    let bytes = {
        use std::io::Read;
        let mut v = Vec::new();
        flate2::read::GzDecoder::new(std::fs::File::open(&tarball).unwrap())
            .read_to_end(&mut v)
            .unwrap();
        v
    };
    for needle in [
        "test_genome_G021783475",
        "mapping.tsv",
        "empty_input",
        "in put",
    ] {
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "{needle} leaked into the bundle"
        );
    }

    // The tarball classifies the same way as the folder it came from.
    let again = Command::new(env!("CARGO_BIN_EXE_gtdbprep"))
        .arg("classify")
        .arg("--bundle")
        .arg(&tarball)
        .arg("--work")
        .arg(&work)
        .arg("--env")
        .arg(&env)
        .arg("--data")
        .arg(&data)
        .output()
        .unwrap();
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let mut got: Vec<String> = String::from_utf8_lossy(&again.stdout)
        .lines()
        .skip(1)
        .map(str::to_string)
        .collect();
    let mut want = Vec::new();
    for domain in ["bac120", "ar53"] {
        let p = work.join(format!("baseline/classify/gtdbtk.{domain}.summary.tsv"));
        if let Ok(t) = std::fs::read_to_string(p) {
            want.extend(t.lines().skip(1).map(str::to_string));
        }
    }
    got.sort();
    want.sort();
    assert_eq!(
        got, want,
        "classify from the .tar.gz differs from classify_wf"
    );
}
