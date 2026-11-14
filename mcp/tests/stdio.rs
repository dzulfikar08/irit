//! Drives irit-mcp over stdio with raw JSON-RPC, then replays the run without it.
//!
//! Needs the leanqc binary: `$IRIT_BIN_DIR`, else `../target/release` or `../target/debug`
//! (build it with `cargo build --release -p leanqc` from the workspace root).

mod common;

use common::{Client, sha256};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin_dir() -> PathBuf {
    let exe = format!("leanqc{}", std::env::consts::EXE_SUFFIX);
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target");
    let candidates: Vec<PathBuf> = match std::env::var_os("IRIT_BIN_DIR") {
        Some(d) => vec![d.into()],
        None => vec![target.join("release"), target.join("debug")],
    };
    let dir = candidates.iter().find(|d| d.join(&exe).is_file()).unwrap_or_else(|| {
        panic!("leanqc not found in {candidates:?}: run `cargo build --release -p leanqc` in the workspace root or set IRIT_BIN_DIR")
    });
    std::fs::canonicalize(dir).unwrap()
}

fn write_fastq(path: &Path, seed: u64, reads: usize) {
    let mut x = seed;
    let mut s = String::new();
    for i in 0..reads {
        let seq: String = (0..60)
            .map(|_| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                b"ACGT"[(x >> 62) as usize] as char
            })
            .collect();
        s += &format!("@read{i} 1:N:0:1\n{seq}\n+\n{}\n", "F".repeat(60));
    }
    std::fs::write(path, s).unwrap();
}


#[test]
fn leanqc_over_stdio_is_recorded_and_replayable() {
    let bin = bin_dir();
    let tmp = tempfile::tempdir().unwrap();
    let work = std::fs::canonicalize(tmp.path()).unwrap();
    write_fastq(&work.join("s1_R1.fastq"), 1, 300);
    write_fastq(&work.join("s1_R2.fastq"), 2, 300);

    let mut c = Client::start(&work, &bin);

    let tools = c.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for want in ["sheetgen", "leanqc", "isotable", "seqplan", "dblock_catalog", "dblock_plan", "dblock_fetch", "dblock_verify", "dblock_check", "dblock_env", "describe_run", "list_runs"] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }
    let leanqc_schema = &tools["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "leanqc").unwrap()["inputSchema"];
    assert_eq!(leanqc_schema["additionalProperties"], false);
    assert_eq!(leanqc_schema["required"], json!(["files"]));

    // 1: the real call
    let r = c.call("leanqc", json!({"files": ["s1_R1.fastq", "s1_R2.fastq"], "outdir": "qc"}));
    assert_ne!(r["isError"], true, "{r}");
    let s = &r["structuredContent"];
    assert_eq!(s["summary"]["files"], 2, "{s}");
    assert_eq!(s["reports"][0]["reads"], 300);
    assert!(["pass", "warn", "fail"].contains(&s["reports"][0]["verdict"].as_str().unwrap()));
    assert_eq!(s["command"], "leanqc --outdir=qc -- s1_R1.fastq s1_R2.fastq");

    // 2: missing input is refused before running, and recorded
    let r = c.call("leanqc", json!({"files": ["nope.fastq"]}));
    assert_eq!(r["isError"], true);
    assert_eq!(r["structuredContent"]["error"], "invalid_parameter");
    assert_eq!(r["structuredContent"]["field"], "files");

    // invented parameter: refused by the schema, never reaches leanqc
    let r = c.call("leanqc", json!({"files": ["s1_R1.fastq"], "kmers": 7}));
    assert!(r.get("error").is_some() || r["isError"] == true, "{r}");

    // 3: fetch without confirmation
    let r = c.call("dblock_fetch", json!({"confirm": false}));
    assert_eq!(r["structuredContent"]["field"], "confirm");

    // a summary by default: no argv, inputs or hashes
    let d = c.call("describe_run", json!({}));
    let calls = d["structuredContent"]["run"]["calls"].as_array().unwrap().clone();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0]["mcp_tool"], "leanqc");
    assert!(calls[0].get("argv").is_none() && calls[0].get("inputs").is_none(), "{}", calls[0]);
    assert!(calls[0]["outputs"].as_array().unwrap().iter().any(|o| o.as_str().unwrap().ends_with(".json")), "{}", calls[0]);
    // the full record on request, or one call
    let full = c.call("describe_run", json!({"detail": true}));
    assert!(full["structuredContent"]["run"]["calls"][0]["argv"].is_array());
    let one = c.call("describe_run", json!({"call_id": 2}));
    assert_eq!(one["structuredContent"]["call"]["id"], 2);
    assert!(one["structuredContent"]["call"]["argv"].is_array());
    let none = c.call("describe_run", json!({"call_id": 9}));
    assert_eq!(none["isError"], true);
    let runs = c.call("list_runs", json!({}));
    assert_eq!(runs["structuredContent"]["runs"][0]["run_id"], "test-run");
    c.close();

    // run record
    let run_dir = work.join("irit-runs/test-run");
    let log: Value = serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    let calls = log["calls"].as_array().unwrap();
    let first = &calls[0];
    assert_eq!(first["status"], "ok");
    assert_eq!(first["argv"], json!(["leanqc", "--outdir=qc", "--", "s1_R1.fastq", "s1_R2.fastq"]));
    assert!(first["version"].as_str().unwrap().starts_with("leanqc "));
    assert_eq!(first["cwd"], work.to_str().unwrap());
    assert_eq!(first["exit_status"], 0);
    assert!(first["started"].as_str().unwrap() <= first["ended"].as_str().unwrap());
    let r1 = work.join("s1_R1.fastq");
    assert!(first["inputs"].as_array().unwrap().iter().any(|i| i["path"] == r1.to_str().unwrap() && i["sha256"] == sha256(&r1)));
    let outputs: Vec<(PathBuf, String)> = first["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (PathBuf::from(o["path"].as_str().unwrap()), o["sha256"].as_str().unwrap().to_string()))
        .collect();
    assert!(outputs.iter().any(|(p, _)| p.ends_with("qc/s1_R1_leanqc.json")));
    assert!(outputs.iter().any(|(p, _)| p.ends_with("qc/leanqc_summary.tsv")));
    for (p, h) in &outputs {
        assert_eq!(&sha256(p), h);
    }
    assert_eq!(calls[1]["status"], "rejected");
    assert_eq!(calls[2]["mcp_tool"], "dblock_fetch");
    assert!(run_dir.join("ro-crate-metadata.json").is_file());
    assert!(run_dir.join("calls/001.stderr").is_file());

    // replay without the server: same outputs, same hashes
    std::fs::remove_dir_all(work.join("qc")).unwrap();
    let replay = |dir: &Path| {
        Command::new("sh").arg(run_dir.join("replay.sh")).env("IRIT_BIN_DIR", &bin).current_dir(dir).output().unwrap()
    };
    let out = replay(&std::env::temp_dir());
    assert!(out.status.success(), "replay failed: {}", String::from_utf8_lossy(&out.stderr));
    for (p, h) in &outputs {
        assert_eq!(&sha256(p), h, "{} differs after replay", p.display());
    }

    // and it notices when the inputs changed
    write_fastq(&r1, 99, 300);
    let out = replay(&work);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("differs"));
}

/// The reviewer's repros, through the real server: a symlinked outdir, writes outside the root,
/// overwriting, and a newline in a `set` value aimed at replay.sh.
#[test]
fn review_repros_over_stdio() {
    let bin = bin_dir();
    let tmp = tempfile::tempdir().unwrap();
    let work = std::fs::canonicalize(tmp.path()).unwrap();
    write_fastq(&work.join("s1_R1.fastq"), 1, 50);
    std::fs::create_dir(work.join("real")).unwrap();
    std::os::unix::fs::symlink(work.join("real"), work.join("qc")).unwrap();
    std::fs::create_dir(work.join("empty")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join(".bashrc");
    std::fs::write(&victim, "keep").unwrap();

    let mut c = Client::start(&work, &bin);
    // symlinked output folder: reports are found and recorded
    let r = c.call("leanqc", json!({"files": ["s1_R1.fastq"], "outdir": "qc", "no_html": true}));
    assert_ne!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["summary"]["files"], 1, "{r}");
    // the folder now has reports: replacing them must be asked for
    let r = c.call("leanqc", json!({"files": ["s1_R1.fastq"], "outdir": "qc", "no_html": true}));
    assert_eq!(r["structuredContent"]["field"], "outdir", "{r}");
    let r = c.call("leanqc", json!({"files": ["s1_R1.fastq"], "outdir": "qc", "no_html": true, "overwrite": true}));
    assert_eq!(r["structuredContent"]["summary"]["files"], 1, "a same-size rewrite must still be detected: {r}");
    // a report symlinked out of the root is not written through
    std::os::unix::fs::symlink(&victim, work.join("real/s1_R1_leanqc.html")).unwrap();
    let r = c.call("leanqc", json!({"files": ["s1_R1.fastq"], "outdir": "qc", "overwrite": true}));
    assert_eq!(r["structuredContent"]["field"], "outdir", "{r}");
    std::fs::remove_file(work.join("real/s1_R1_leanqc.html")).unwrap();
    // run records are not a target
    let r = c.call("sheetgen", json!({"dirs": ["empty"], "output": "irit-runs/test-run/replay.sh", "overwrite": true}));
    assert_eq!(r["structuredContent"]["field"], "output", "{r}");
    // outside the root
    let r = c.call("sheetgen", json!({"dirs": ["empty"], "output": victim.to_str().unwrap(), "overwrite": true}));
    assert_eq!(r["structuredContent"]["field"], "output", "{r}");
    // newline in a value
    let r = c.call("sheetgen", json!({"dirs": ["empty"], "set": {"x": "a\ntouch PWNED #"}}));
    assert_eq!(r["structuredContent"]["field"], "set", "{r}");
    c.close();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");

    let run_dir = work.join("irit-runs/test-run");
    let log: Value = serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    let outs = log["calls"][0]["outputs"].as_array().unwrap();
    assert!(outs.iter().any(|o| o["path"].as_str().unwrap().ends_with("qc/s1_R1_leanqc.json")), "{outs:?}");
    let script = run_dir.join("replay.sh");
    assert!(Command::new("sh").arg("-n").arg(&script).status().unwrap().success());
    std::fs::remove_dir_all(work.join("real")).unwrap();
    std::fs::create_dir(work.join("real")).unwrap();
    let out = Command::new("sh").arg(&script).env("IRIT_BIN_DIR", &bin).current_dir(&work).output().unwrap();
    assert!(out.status.success(), "replay failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("outputs match"));
    assert!(!work.join("PWNED").exists() && !run_dir.join("PWNED").exists());
}

/// seqplan through the server: the plan comes back as structured JSON, and the call is replayable.
#[test]
fn seqplan_over_stdio() {
    let bin = bin_dir();
    assert!(bin.join(format!("seqplan{}", std::env::consts::EXE_SUFFIX)).is_file(), "build seqplan too (cargo build -p seqplan)");
    let tmp = tempfile::tempdir().unwrap();
    let work = std::fs::canonicalize(tmp.path()).unwrap();
    let mut c = Client::start(&work, &bin);
    let r = c.call("seqplan", json!({"kit": "miseq-v3", "genome_size": "5.5M", "coverage": 50.0, "samples": 96, "run_price": 1500.0, "currency": "USD"}));
    assert_eq!(r["isError"], false, "{r}");
    let plan = &r["structuredContent"]["plan"];
    assert_eq!(plan["low"]["isolates_per_run"], 29);
    assert_eq!(plan["high"]["isolates_per_run"], 33);
    assert_eq!(plan["kit"]["source"], "https://www.illumina.com/systems/sequencing-platforms/miseq/specifications.html");
    let r = c.call("seqplan", json!({"kit": "hiseq-2000", "genome_size": "5.5M"}));
    assert_eq!(r["isError"], true);
    c.close();
    let run_dir = work.join("irit-runs/test-run");
    let out = Command::new("sh").arg(run_dir.join("replay.sh")).env("IRIT_BIN_DIR", &bin).current_dir(&work).output().unwrap();
    assert!(out.status.success(), "replay failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// sheetgen verify and cooccur over MCP with the real binaries, then replay.
#[test]
fn reads_verify_and_cooccur_over_stdio() {
    use std::io::Write;
    let bin = bin_dir();
    for b in ["sheetgen", "cooccur"] {
        assert!(bin.join(format!("{b}{}", std::env::consts::EXE_SUFFIX)).is_file(), "build {b} too (cargo build -p {b})");
    }
    let tmp = tempfile::tempdir().unwrap();
    let work = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::create_dir_all(work.join("reads")).unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(b"@r1\nACGT\n+\nIIII\n@r2\nGGCC\n+\nIIII\n").unwrap();
    let good = gz.finish().unwrap();
    std::fs::write(work.join("reads/a_1.fastq.gz"), &good).unwrap();
    let mut bad = good.clone();
    let n = bad.len();
    bad[n - 6] ^= 0xff; // the CRC
    std::fs::write(work.join("reads/b_1.fastq.gz"), &bad).unwrap();

    // eight samples, two genomes kept proportional
    let mut t = String::from("Genome");
    for s in 1..=8 {
        t.push_str(&format!("\ts{s} Relative Abundance (%)"));
    }
    t.push('\n');
    let x = [3.0, 9.0, 1.0, 7.0, 2.0, 12.0, 5.0, 4.0];
    let other = [5.0, 1.0, 8.0, 2.0, 9.0, 3.0, 4.0, 6.0];
    for (name, f) in [("g1", 1.0), ("g2", 2.0)] {
        t.push_str(name);
        for v in x {
            t.push_str(&format!("\t{}", v * f));
        }
        t.push('\n');
    }
    for (i, name) in ["g3", "g4"].iter().enumerate() {
        t.push_str(name);
        for (j, v) in other.iter().enumerate() {
            t.push_str(&format!("\t{}", v + ((i + j) % 3) as f64));
        }
        t.push('\n');
    }
    std::fs::write(work.join("abundance.tsv"), t).unwrap();

    let mut c = Client::start(&work, &bin);
    let r = c.call("reads_verify", json!({"paths": ["reads/a_1.fastq.gz"]}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["report"]["files"][0]["status"], "ok", "{r}");
    let r = c.call("reads_verify", json!({"paths": ["reads"]}));
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["error"], "reads_have_problems", "{r}");
    let statuses: Vec<&str> = r["structuredContent"]["report"]["files"].as_array().unwrap().iter().map(|f| f["status"].as_str().unwrap()).collect();
    assert!(statuses.contains(&"corrupt"), "{statuses:?}");
    let r = c.call("cooccur", json!({"table": "abundance.tsv", "output": "cooc/edges.tsv", "fdr": 0.5}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["structuredContent"]["summary"].is_object(), "{r}");
    assert!(std::fs::read_to_string(work.join("cooc/edges.tsv")).unwrap().contains("g1"), "{r}");
    c.close();
    let run_dir = work.join("irit-runs/test-run");
    let out = Command::new("sh").arg(run_dir.join("replay.sh")).env("IRIT_BIN_DIR", &bin).current_dir(&work).output().unwrap();
    // the second verify call failed on purpose; replay re-runs the ok calls
    assert!(out.status.success(), "replay failed: {}\n{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
}

