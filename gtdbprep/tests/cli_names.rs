//! Both binary names answer `--version` (the alias puts `prep` in front of everything else).

use std::process::Command;

#[test]
fn both_names_report_the_version() {
    for exe in [env!("CARGO_BIN_EXE_gtdbprep"), env!("CARGO_BIN_EXE_irit-gtdb-prep")] {
        for flag in ["--version", "-V"] {
            let out = Command::new(exe).arg(flag).output().unwrap();
            assert!(out.status.success(), "{exe} {flag}: {}", String::from_utf8_lossy(&out.stderr));
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("gtdbprep {}", env!("CARGO_PKG_VERSION")));
        }
    }
    // the alias still means `gtdbprep prep`
    let out = Command::new(env!("CARGO_BIN_EXE_irit-gtdb-prep")).arg("--help").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("irit-gtdb-prep prep"));
}
