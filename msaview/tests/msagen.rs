//! The `msagen` binary: refuses empty sizes and writes complete gzip output.

use std::process::Command;

fn msagen(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_msagen")).args(args).output().unwrap()
}

#[test]
fn refuses_zero_rows_or_columns() {
    let dir = std::env::temp_dir().join(format!("msagen-test-zero-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("zero.fa");
    for (rows, cols, target) in [("10", "0", "-"), ("0", "10", out.to_str().unwrap())] {
        let o = msagen(&[rows, cols, target]);
        assert!(!o.status.success(), "{rows} {cols}");
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(err.contains("must be at least 1") && !err.contains("panicked"), "{err}");
        assert!(o.stdout.is_empty());
    }
    assert!(!out.exists(), "no file is created for a refused size");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn gzip_output_is_complete() {
    let dir = std::env::temp_dir().join(format!("msagen-test-gz-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (plain, gz) = (dir.join("a.fa"), dir.join("a.fa.gz"));
    assert!(msagen(&["40", "300", plain.to_str().unwrap()]).status.success());
    assert!(msagen(&["40", "300", gz.to_str().unwrap()]).status.success());
    let mut text = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(std::fs::File::open(&gz).unwrap()), &mut text).unwrap();
    assert_eq!(text, std::fs::read(&plain).unwrap());
    let a = msaview::parse::load(&gz).unwrap();
    assert_eq!((a.rows(), a.cols()), (40, 300));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stdout() {
    let o = msagen(&["2", "5", "-", "--protein"]);
    assert!(o.status.success());
    assert_eq!(msaview::parse::parse_bytes(&o.stdout).unwrap().rows(), 2);
}
