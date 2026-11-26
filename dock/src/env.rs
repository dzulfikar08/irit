//! The tools `dock` needs, kept in a cache directory it owns.
//!
//! Two things have to be present: a Python environment with the chemistry stack, and the
//! AutoDock Vina binary. Neither is packaged for Arch, and the usual advice is Bioconda,
//! which assumes a conda install this repo does not want to require. `uv` resolves the whole
//! stack from PyPI in one step, so that is what we use.
//!
//! Versions are pinned, not floated. The pins below are the set the pipeline was validated
//! against (redocking sorafenib into VEGFR-2, PDB 4ASD, 0.78 A RMSD on six cores). Floating them would
//! mean the tool silently changes its answers when a dependency releases.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The validated stack. Bump these deliberately, and re-run `dock validate` when you do.
pub const PINNED: &[&str] = &[
    "rdkit==2026.9.2",
    "meeko==0.8.0",
    "prolif==2.2.3",
    "pdb2pqr==3.8.0",
    "MDAnalysis==2.11.0",
    "gemmi==0.7.5",
    "numpy==2.5.5",
    "scipy==1.18.1",
    // openbabel-wheel only. The `openbabel` PyPI package installs a module of the same name
    // and the two together segfault the interpreter on import.
    "openbabel-wheel==3.1.1.23",
];

pub const VINA_VERSION: &str = "1.2.5";

struct VinaBuild {
    asset: &'static str,
    sha256: &'static str,
}

fn vina_build() -> Option<VinaBuild> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some(VinaBuild {
            asset: "vina_1.2.5_linux_x86_64",
            sha256: "fa0126a28a9ea9162d1b161dfa92bc76e632416db28ca246278ea4b2dc6860cb",
        }),
        // Checksums for other platforms are not recorded yet; --vina lets you supply a binary.
        _ => None,
    }
}

pub struct Env {
    pub root: PathBuf,
    pub python: PathBuf,
    pub vina: PathBuf,
    pub stages: PathBuf,
    /// Every external command this run made, argv form, in order. Goes into the manifest.
    recorded: std::sync::Mutex<Vec<Vec<String>>>,
}

/// The Python side, written next to the environment so `python -I` can find it.
const STAGES_PY: &str = include_str!("../python/stages.py");

impl Env {
    /// Where the environment lives: `$DOCK_HOME`, else `$XDG_CACHE_HOME/dock`, else `~/.cache/dock`.
    pub fn default_root() -> Result<PathBuf> {
        if let Some(p) = std::env::var_os("DOCK_HOME") {
            return Ok(PathBuf::from(p));
        }
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .context("neither DOCK_HOME, XDG_CACHE_HOME nor HOME is set")?;
        Ok(base.join("dock"))
    }

    /// Locate an already-prepared environment without building one.
    pub fn open(root: PathBuf, vina_override: Option<PathBuf>) -> Result<Self> {
        let python = root.join("venv/bin/python");
        let vina_override_was_none = vina_override.is_none();
        let vina = match vina_override {
            Some(v) => v,
            None => root.join("bin/vina"),
        };
        let stages = root.join("stages.py");
        if !python.is_file() {
            bail!(
                "no Python environment at {}. Run `dock env` once to build it \
                 (it downloads about 400 MB and takes a couple of minutes).",
                root.display()
            );
        }
        if !vina.is_file() {
            bail!(
                "AutoDock Vina not found at {}. Run `dock env`, or pass --vina /path/to/vina.",
                vina.display()
            );
        }
        // Check the managed binary every time. An interrupted `dock env` can leave a file
        // here that passes `is_file()` and is not Vina. A binary supplied with --vina is the
        // caller's business and is only checked for existence.
        if vina_override_was_none && !verify(&vina)? {
            bail!(
                "{} does not match the recorded checksum for AutoDock Vina {VINA_VERSION}. \
                 It is probably a partial download. Delete it and run `dock env` again.",
                vina.display()
            );
        }
        // Refresh every time, not only in `ensure`: the script is embedded in the binary, so
        // an upgraded dock against an existing DOCK_HOME would otherwise keep executing the
        // previous version's Python and report failures that were already fixed.
        write_stages(&stages)?;
        Ok(Env { root, python, vina, stages, recorded: Default::default() })
    }

    /// Build the environment if it is missing, and refresh the Python stage script.
    pub fn ensure(root: PathBuf, vina_override: Option<PathBuf>) -> Result<Self> {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating {}", root.display()))?;

        let venv = root.join("venv");
        if !venv.join("bin/python").is_file() {
            let uv = which("uv").context(
                "`uv` is not on PATH. dock uses it to build its Python environment; \
                 install it from https://astral.sh/uv (or your package manager) and retry.",
            )?;
            eprintln!("dock: creating Python environment in {}", venv.display());
            run(Command::new(&uv).arg("venv").arg("-p").arg("3.12").arg(&venv), "uv venv")?;
            eprintln!("dock: installing {} pinned packages (this takes a minute)", PINNED.len());
            let mut c = Command::new(&uv);
            c.arg("pip").arg("install").arg("--python").arg(venv.join("bin/python"));
            for p in PINNED {
                c.arg(p);
            }
            run(&mut c, "uv pip install")?;
        }

        // Always rewrite: the script ships inside the binary, so an upgraded dock
        // must not keep running the previous version's Python.
        let stages = root.join("stages.py");
        write_stages(&stages)?;

        let vina = match vina_override {
            Some(v) => {
                if !v.is_file() {
                    bail!("--vina {} is not a file", v.display());
                }
                v
            }
            None => ensure_vina(&root)?,
        };

        Ok(Env { root, python: venv.join("bin/python"), vina, stages, recorded: Default::default() })
    }

    /// Run one stage of the Python side and parse its single line of JSON.
    pub fn stage(&self, stage: &str, args: &[&str]) -> Result<serde_json::Value> {
        let mut c = Command::new(&self.python);
        c.arg("-I").arg(&self.stages).arg(stage).args(args);
        let mut argv = vec!["python".to_string(), "-I".to_string(), "stages.py".to_string(), stage.to_string()];
        argv.extend(args.iter().map(|a| a.to_string()));
        self.record(argv);
        let out = c
            .output()
            .with_context(|| format!("running the {stage} stage"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = if err.trim().is_empty() { String::from_utf8_lossy(&out.stdout).to_string() } else { err.to_string() };
            let tail: Vec<&str> = err.lines().rev().take(12).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            bail!("{stage} failed:\n{}", tail.join("\n"));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text
            .lines()
            .last()
            .with_context(|| format!("{stage} produced no output"))?;
        serde_json::from_str(line)
            .with_context(|| format!("{stage} did not return JSON: {line}"))
    }

    /// Note an external command for the manifest. Paths are kept as given; the manifest
    /// writer is what decides whether a value is safe to publish.
    pub fn record(&self, argv: Vec<String>) {
        if let Ok(mut r) = self.recorded.lock() {
            r.push(argv);
        }
    }

    /// Everything run so far, in order.
    pub fn commands(&self) -> Vec<Vec<String>> {
        // Recover rather than return an empty list: a manifest that claims to hold every
        // command and holds none is worse than one that is missing.
        match self.recorded.lock() {
            Ok(r) => r.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn versions(&self) -> Result<serde_json::Value> {
        self.stage("check", &[])
    }
}

fn ensure_vina(root: &Path) -> Result<PathBuf> {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin)?;
    let path = bin.join("vina");
    if path.is_file() && verify(&path).unwrap_or(false) {
        return Ok(path);
    }
    let build = vina_build().with_context(|| {
        format!(
            "no recorded AutoDock Vina build for {}-{}. Download it yourself and pass \
             --vina /path/to/vina.",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let url = format!(
        "https://github.com/ccsb-scripps/AutoDock-Vina/releases/download/v{VINA_VERSION}/{}",
        build.asset
    );
    eprintln!("dock: downloading AutoDock Vina {VINA_VERSION}");
    let curl = which("curl").context("`curl` is needed to download AutoDock Vina")?;
    // Download to a side name and rename only once the checksum matches. Writing straight to
    // `bin/vina` leaves a truncated binary at the final name when a transfer is interrupted,
    // and `Env::open` would then hand that to the user as if it were Vina.
    let partial = bin.join("vina.partial");
    let _ = std::fs::remove_file(&partial);
    let outcome = run(
        Command::new(curl).arg("-sSL").arg("--fail").arg("-o").arg(&partial).arg(&url),
        "downloading vina",
    );
    if let Err(e) = outcome {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    let got = sha256(&partial)?;
    if got != build.sha256 {
        let _ = std::fs::remove_file(&partial);
        bail!(
            "downloaded Vina does not match its recorded checksum.\n  expected {}\n  got      {}\n\
             The release asset changed, or the download was tampered with. Not using it.",
            build.sha256,
            got
        );
    }
    make_executable(&partial)?;
    std::fs::rename(&partial, &path)
        .with_context(|| format!("installing {}", path.display()))?;
    Ok(path)
}

fn verify(path: &Path) -> Result<bool> {
    match vina_build() {
        Some(b) => Ok(sha256(path)? == b.sha256),
        None => Ok(true),
    }
}

fn sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(format!("{:x}", h.finalize()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut p = std::fs::metadata(path)?.permissions();
    p.set_mode(0o755);
    std::fs::set_permissions(path, p)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Put the embedded Python where the interpreter can read it.
///
/// Unchanged content is left alone, and a change is staged next to the target and renamed:
/// a plain `write` truncates the file, and docking several ligands in parallel against one
/// DOCK_HOME would then let one process blank the script another is mid-read, surfacing as a
/// SyntaxError that looks like it came from the user's input.
fn write_stages(stages: &Path) -> Result<()> {
    if std::fs::read_to_string(stages).is_ok_and(|cur| cur == STAGES_PY) {
        return Ok(());
    }
    let tmp = stages.with_extension(format!("py.{}.tmp", std::process::id()));
    std::fs::write(&tmp, STAGES_PY)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, stages)
        .with_context(|| format!("installing {}", stages.display()))?;
    Ok(())
}

pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|c| is_executable(c))
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let out = cmd.output().with_context(|| format!("starting {what}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(10).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        bail!("{what} failed:\n{}", tail.join("\n"));
    }
    Ok(())
}
