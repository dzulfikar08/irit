//! The engine against a stand-in Vina.
//!
//! Same approach as the irit-mcp external tests: a POSIX-sh script stands in for the real
//! program, checks the flags it is given, and writes a realistically shaped result. No
//! AutoDock Vina, no Python environment, so this runs anywhere CI does.

#![cfg(unix)]

use dock::env::Env;
use dock::stages::*;
use dock::vina::Vina;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Answers like Vina, and fails loudly on a flag it was not expecting.
const FAKE_VINA: &str = r#"#!/bin/sh
die() { echo "fake-vina: $1" >&2; exit 2; }
out=""
seen_receptor=0; seen_ligand=0; seen_cx=0; seen_sx=0; seen_exh=0; seen_seed=0
while [ $# -gt 0 ]; do
  case "$1" in
    --receptor) seen_receptor=1; [ -f "$2" ] || die "receptor $2 missing"; shift 2 ;;
    --ligand)   seen_ligand=1;   [ -f "$2" ] || die "ligand $2 missing"; shift 2 ;;
    --center_x) seen_cx=1; shift 2 ;;
    --center_y|--center_z) shift 2 ;;
    --size_x) seen_sx=1; shift 2 ;;
    --size_y|--size_z) shift 2 ;;
    --exhaustiveness) seen_exh=1; echo "$2" > "$TMPDIR_EXH"; shift 2 ;;
    --num_modes) shift 2 ;;
    --seed) seen_seed=1; shift 2 ;;
    --cpu) shift 2 ;;
    --out) out="$2"; shift 2 ;;
    *) die "unknown option $1" ;;
  esac
done
[ "$seen_receptor" = 1 ] || die "no --receptor"
[ "$seen_ligand" = 1 ] || die "no --ligand"
[ "$seen_cx" = 1 ] || die "no --center_x"
[ "$seen_sx" = 1 ] || die "no --size_x"
[ "$seen_exh" = 1 ] || die "no --exhaustiveness"
[ "$seen_seed" = 1 ] || die "no --seed (runs must be repeatable)"
[ -n "$out" ] || die "no --out"
echo "MODEL 1" > "$out"
cat <<'EOF'
Computing Vina grid ... done.
Performing docking (random seed: 42) ...

mode |   affinity | dist from best mode
     | (kcal/mol) | rmsd l.b.| rmsd u.b.
-----+------------+----------+----------
   1       -11.09          0          0
   2       -10.39      1.777      2.252
   3       -9.901      6.047      11.56
EOF
"#;

/// Exits non-zero, the way Vina does when the box misses the receptor.
const FAILING_VINA: &str = r#"#!/bin/sh
echo "Error: Could not find any poses." >&2
exit 1
"#;

struct Fixture {
    _dir: tempfile::TempDir,
    env: Arc<Env>,
    receptor: PreparedReceptor,
    ligand: PreparedLigand,
}

fn write_exec(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    let mut p = std::fs::metadata(path).unwrap().permissions();
    p.set_mode(0o755);
    std::fs::set_permissions(path, p).unwrap();
}

fn fixture(vina_body: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    // Env::open insists both tools are present; stand-ins satisfy it.
    write_exec(&root.join("venv/bin/python"), "#!/bin/sh\nexit 0\n");
    let vina_path = root.join("bin/vina");
    write_exec(&vina_path, vina_body);

    let rec = root.join("rec.pdbqt");
    let lig = root.join("lig.pdbqt");
    std::fs::write(&rec, "ATOM\n").unwrap();
    std::fs::write(&lig, "ATOM\n").unwrap();

    let env = Arc::new(Env::open(root, Some(vina_path)).unwrap());
    Fixture {
        _dir: dir,
        env,
        receptor: PreparedReceptor {
            pdbqt: rec,
            protonated_pdb: PathBuf::new(),
            residues: 305,
            ph: 7.4,
        },
        ligand: PreparedLigand {
            pdbqt: lig,
            heavy_atoms: 32,
            rotatable_bonds: 5,
            source: "test".into(),
        },
    }
}

fn opts() -> EngineOpts {
    EngineOpts { exhaustiveness: 16, num_modes: 9, seed: 42, cpus: 2 }
}

fn a_box() -> Box3 {
    Box3 { center: [-24.6, -0.4, -10.9], size: [24.0, 24.0, 24.0] }
}

#[test]
fn parses_the_poses_vina_reports() {
    let f = fixture(FAKE_VINA);
    let out = f.env.root.join("docked.pdbqt");
    let poses = Vina(f.env.clone())
        .dock(&f.receptor, &f.ligand, a_box(), &opts(), &out)
        .expect("docking should succeed");

    assert_eq!(poses.len(), 3);
    assert_eq!(poses[0].rank, 1);
    assert!((poses[0].affinity + 11.09).abs() < 1e-9);
    // The decoy far from pose 1 must survive into the result: hiding it is how a docking
    // report ends up claiming more than it knows.
    assert!(poses[2].rmsd_ub > 5.0);
    assert!(out.is_file(), "the pose file should have been written");
}

#[test]
fn records_the_command_for_the_manifest() {
    let f = fixture(FAKE_VINA);
    let out = f.env.root.join("docked.pdbqt");
    Vina(f.env.clone())
        .dock(&f.receptor, &f.ligand, a_box(), &opts(), &out)
        .unwrap();

    let cmds = f.env.commands();
    assert_eq!(cmds.len(), 1, "exactly one external call was made");
    let argv = &cmds[0];
    assert_eq!(argv[0], "vina");
    assert!(argv.iter().any(|a| a == "--seed"), "the seed must be recorded");
    assert!(
        argv.iter().any(|a| a == "16"),
        "exhaustiveness must be recorded: a score without it cannot be reproduced"
    );
}

#[test]
fn a_failing_engine_is_an_error_not_an_empty_result() {
    let f = fixture(FAILING_VINA);
    let out = f.env.root.join("docked.pdbqt");
    let err = Vina(f.env.clone())
        .dock(&f.receptor, &f.ligand, a_box(), &opts(), &out)
        .expect_err("a non-zero exit must not be reported as zero poses");
    let text = format!("{err:#}");
    assert!(text.contains("vina failed"), "got: {text}");
}
