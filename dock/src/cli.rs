//! Command line for `dock`.

use crate::env::Env;
use crate::manifest::{self, Manifest};
use crate::python::Py;
use crate::report::{self, ReportInput};
use crate::stages::*;
use crate::vina::Vina;
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "dock",
    version,
    about = "Molecular docking with AutoDock Vina: prepare, dock, profile the contacts, \
             and write a report that does not depend on anything outside itself."
)]
struct Cli {
    /// Where dock keeps its Python environment and the Vina binary.
    #[arg(long, global = true, env = "DOCK_HOME")]
    home: Option<PathBuf>,
    /// Use this AutoDock Vina binary instead of the managed one.
    #[arg(long, global = true)]
    vina: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build or check the tools dock needs. Run this once.
    Env(EnvArgs),
    /// Dock a ligand into a receptor.
    Run(RunArgs),
    /// Redock a ligand already present in a crystal structure, and report how far the
    /// best pose lands from where crystallography put it.
    Validate(ValidateArgs),
}

#[derive(Args)]
struct EnvArgs {
    /// Report what is installed without building anything.
    #[arg(long)]
    check: bool,
}

#[derive(Args)]
struct DockOpts {
    /// How hard Vina searches. Higher is slower and more reproducible. Default 16.
    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u32).range(1..=1024))]
    exhaustiveness: u32,
    /// How many poses to keep. Default 9.
    #[arg(long, default_value_t = 9, value_parser = clap::value_parser!(u32).range(1..=64))]
    num_modes: u32,
    /// Random seed. Fixed by default so a run can be repeated exactly.
    #[arg(long, default_value_t = 42)]
    seed: i64,
    /// CPU threads. Defaults to every core.
    #[arg(long)]
    cpus: Option<u32>,
    /// pH used to protonate the *receptor*. Default 7.4.
    ///
    /// It does not touch the ligand: the ligand is docked in exactly the protonation state
    /// and tautomer its SMILES describes. Vina uses hydrogens to decide donors and
    /// acceptors, so if the compound ionises near this pH, write the charged form yourself.
    #[arg(long = "receptor-ph", alias = "ph", default_value_t = 7.4)]
    ph: f64,
    /// Replace an existing output directory. Default false.
    #[arg(long)]
    overwrite: bool,
}

impl DockOpts {
    fn engine(&self) -> EngineOpts {
        EngineOpts {
            exhaustiveness: self.exhaustiveness,
            num_modes: self.num_modes,
            seed: self.seed,
            cpus: self.cpus.unwrap_or_else(|| {
                std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1)
            }),
        }
    }
}

#[derive(Args)]
struct RunArgs {
    /// Receptor structure, PDB or mmCIF. Waters and other ligands are removed.
    #[arg(long)]
    receptor: PathBuf,
    /// Ligand as SMILES. 3D coordinates are generated.
    #[arg(long, conflicts_with = "ligand_file")]
    smiles: Option<String>,
    /// Ligand from a file that already has 3D coordinates (.sdf, .mol2, .pdb).
    #[arg(long, conflicts_with = "smiles")]
    ligand_file: Option<PathBuf>,
    /// SMILES for --ligand-file, to restore bond orders. Required for a .pdb ligand.
    #[arg(long, requires = "ligand_file")]
    template_smiles: Option<String>,
    /// Name for the ligand in the report. Defaults to the file stem or "ligand".
    #[arg(long)]
    name: Option<String>,
    // `num_args = 3` would reject the documented `--center 1,2,3`: clap counts raw
    // command-line words before applying the delimiter, so the comma form is one word.
    // A range accepts both that and `--center 1 2 3`; `three()` checks the count.
    /// Box centre as x,y,z in angstroms.
    #[arg(long, value_delimiter = ',', num_args = 1..=3, conflicts_with = "reference")]
    center: Option<Vec<f64>>,
    /// Box size as x,y,z in angstroms. Default 24,24,24. Not used with --reference.
    #[arg(long, value_delimiter = ',', num_args = 1..=3, conflicts_with = "reference")]
    size: Option<Vec<f64>>,
    /// Put the box around this ligand instead (a PDB file of a bound ligand).
    #[arg(long)]
    reference: Option<PathBuf>,
    /// Padding around --reference, in angstroms. Default 5.
    #[arg(long, default_value_t = 5.0)]
    padding: f64,
    /// Where to write results.
    #[arg(long)]
    out: PathBuf,
    #[command(flatten)]
    opts: DockOpts,
}

#[derive(Args)]
struct ValidateArgs {
    /// A structure with the ligand still bound, PDB or mmCIF.
    #[arg(long)]
    structure: PathBuf,
    /// Three-letter code of the bound ligand, e.g. BAX.
    #[arg(long)]
    ligand_code: String,
    /// SMILES for that ligand, used to restore bond orders the PDB does not carry.
    #[arg(long)]
    smiles: String,
    /// Which chain to keep. Defaults to the one holding the ligand.
    #[arg(long)]
    chain: Option<String>,
    /// Where to write results.
    #[arg(long)]
    out: PathBuf,
    #[command(flatten)]
    opts: DockOpts,
}

pub fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dock: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cli: Cli) -> Result<()> {
    let root = match cli.home {
        Some(h) => h,
        None => Env::default_root()?,
    };
    match cli.command {
        Command::Env(a) => {
            let env = if a.check {
                Env::open(root, cli.vina)?
            } else {
                Env::ensure(root, cli.vina)?
            };
            let v = env.versions()?;
            println!("environment: {}", env.root.display());
            println!("vina:        {}", env.vina.display());
            if let Some(map) = v.get("versions").and_then(|x| x.as_object()) {
                for (k, val) in map {
                    match val.as_str() {
                        Some(s) => println!("  {k:<12} {s}"),
                        None => println!("  {k:<12} MISSING"),
                    }
                }
            }
            Ok(())
        }
        Command::Run(a) => run(root, cli.vina, a),
        Command::Validate(a) => validate(root, cli.vina, a),
    }
}

fn backend(env: Arc<Env>) -> Backend {
    Backend {
        ligand: Box::new(Py(env.clone())),
        receptor: Box::new(Py(env.clone())),
        engine: Box::new(Vina(env.clone())),
        profiler: Box::new(Py(env.clone())),
        rmsd: Box::new(Py(env.clone())),
        structure: Box::new(Py(env)),
    }
}

/// Decide whether `--overwrite` is allowed to delete this directory.
///
/// `--overwrite` used to call `remove_dir_all` on whatever `--out` named, before any
/// argument had been checked, so a typo'd path destroyed unrelated data and then failed
/// having done no work. It may now only delete a directory dock itself wrote — one holding
/// a `manifest.json` in our own format — or an empty one.
fn removable(out: &Path) -> Result<()> {
    let real = out
        .canonicalize()
        .with_context(|| format!("resolving {}", out.display()))?;
    if real.parent().is_none() {
        bail!("refusing to remove the filesystem root");
    }
    for var in ["HOME", "XDG_CACHE_HOME", "DOCK_HOME"] {
        if let Some(v) = std::env::var_os(var)
            && let Ok(p) = PathBuf::from(&v).canonicalize()
            && p == real
        {
            bail!("refusing to remove ${var} ({})", real.display());
        }
    }
    if let Ok(cwd) = std::env::current_dir()
        && cwd.starts_with(&real)
    {
        bail!(
            "refusing to remove {}: the current directory is inside it",
            real.display()
        );
    }
    if !real.is_dir() {
        bail!("{} is not a directory", real.display());
    }
    if std::fs::read_dir(&real)?.next().is_none() {
        return Ok(()); // empty: nothing to lose
    }
    let marker = real.join("manifest.json");
    let looks_like_ours = std::fs::read_to_string(&marker)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("format").and_then(|f| f.as_str()).map(|s| s.to_string()))
        .is_some_and(|f| f == manifest::FORMAT);
    if !looks_like_ours {
        bail!(
            "{} is not empty and was not written by dock (no manifest.json with \
             \"format\": \"{}\"). Refusing to delete it — move it aside or pick another --out.",
            real.display(),
            manifest::FORMAT
        );
    }
    Ok(())
}

fn prepare_out(out: &Path, overwrite: bool) -> Result<()> {
    if out.exists() {
        if !overwrite {
            bail!(
                "{} already exists. Pass --overwrite to replace it.",
                out.display()
            );
        }
        removable(out)?;
        std::fs::remove_dir_all(out)
            .with_context(|| format!("replacing {}", out.display()))?;
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    Ok(())
}

fn run(root: PathBuf, vina: Option<PathBuf>, a: RunArgs) -> Result<()> {
    let started = manifest::now();
    // Everything that can be checked without doing work is checked before --overwrite is
    // allowed to delete anything: a run that is going to fail on its arguments must not
    // destroy the previous run's results on the way.
    let env = Arc::new(Env::open(root, vina)?);
    if !a.receptor.is_file() {
        bail!("--receptor {} does not exist", a.receptor.display());
    }
    match (&a.smiles, &a.ligand_file) {
        (None, None) => bail!("give the ligand with --smiles or --ligand-file"),
        (_, Some(f)) if !f.is_file() => {
            bail!("--ligand-file {} does not exist", f.display())
        }
        _ => {}
    }
    if let Some(r) = &a.reference
        && !r.is_file()
    {
        bail!("--reference {} does not exist", r.display());
    }
    if a.reference.is_none() && a.center.is_none() {
        bail!("give the box with --center x,y,z or --reference <bound ligand>");
    }
    if let Some(c) = &a.center {
        three("center", c)?;
    }
    if let Some(s) = &a.size {
        three("size", s)?;
    }

    prepare_out(&a.out, a.opts.overwrite)?;
    let b = backend(env.clone());
    // (box and split now go through Backend::structure)

    eprintln!("dock: preparing receptor (pH {})", a.opts.ph);
    let receptor = b.receptor.prepare(&a.receptor, a.opts.ph, &a.out)?;

    let label = a
        .name
        .clone()
        .or_else(|| {
            a.ligand_file
                .as_ref()
                .and_then(|f| f.file_stem())
                .map(|s| s.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "ligand".to_string());

    eprintln!("dock: preparing ligand");
    let lig_out = a.out.join("ligand.pdbqt");
    let ligand = match (&a.smiles, &a.ligand_file) {
        (Some(s), _) => b.ligand.prepare_smiles(s, &lig_out)?,
        (_, Some(f)) => b.ligand.prepare_file(f, &lig_out, a.template_smiles.as_deref())?,
        _ => bail!("give the ligand with --smiles or --ligand-file"),
    };
    eprintln!(
        "dock: {} heavy atoms, {} rotatable bonds",
        ligand.heavy_atoms, ligand.rotatable_bonds
    );
    for w in &ligand.warnings {
        eprintln!("dock: warning: {w}");
    }

    let bx = resolve_box(b.structure.as_ref(), &a)?;
    eprintln!(
        "dock: box centre {:.2}, {:.2}, {:.2}  size {:.0} x {:.0} x {:.0}",
        bx.center[0], bx.center[1], bx.center[2], bx.size[0], bx.size[1], bx.size[2]
    );

    let docked = a.out.join("docked.pdbqt");
    let eopts = a.opts.engine();
    warn_box(bx, &eopts);
    eprintln!("dock: docking (exhaustiveness {}, {} threads)", eopts.exhaustiveness, eopts.cpus);
    let poses = b.engine.dock(&receptor, &ligand, bx, &eopts, &docked)?;

    finish(
        &env, &b, &a.out, &receptor, &ligand, &docked, bx, &poses, None, &label,
        &format!("{} · AutoDock Vina {}", a.receptor.display(), crate::env::VINA_VERSION),
        serde_json::json!({
            "receptor": a.receptor.display().to_string(),
            "smiles": a.smiles,
            "ligand_file": a.ligand_file.as_ref().map(|p| p.display().to_string()),
            "box": bx,
            "exhaustiveness": eopts.exhaustiveness,
            "seed": eopts.seed,
            // Vina partitions the seeded RNG across threads, so the seed alone does not
            // reproduce a run; the thread count is part of the answer.
            "cpus": eopts.cpus,
            "receptor_ph": a.opts.ph,
        }),
        started,
    )
}

/// Vina warns above 27,000 A^3; it searches the same space either way, but the search gets
/// thinner, so the top score of a run drifts. Say it before the run, not after.
fn warn_box(bx: Box3, opts: &EngineOpts) {
    let vol = bx.size[0] * bx.size[1] * bx.size[2];
    if vol > 27_000.0 {
        eprintln!(
            "dock: warning: the box is {vol:.0} A^3, over Vina's 27,000 A^3 guidance. \
             Sampling is thin at exhaustiveness {}; narrow the box or raise --exhaustiveness.",
            opts.exhaustiveness
        );
    }
}

fn three(name: &str, v: &[f64]) -> Result<[f64; 3]> {
    if v.len() != 3 {
        bail!("--{name} needs three numbers as x,y,z; got {}", v.len());
    }
    Ok([v[0], v[1], v[2]])
}

fn resolve_box(st: &dyn Structure, a: &RunArgs) -> Result<Box3> {
    if let Some(r) = &a.reference {
        return st.box_from_reference(r, a.padding);
    }
    let Some(c) = &a.center else {
        bail!("give the box with --center x,y,z or --reference <bound ligand>");
    };
    let size = a.size.clone().unwrap_or_else(|| vec![24.0, 24.0, 24.0]);
    let size = three("size", &size)?;
    if size.iter().any(|s| *s <= 0.0) {
        bail!("--size must be positive in every dimension");
    }
    Ok(Box3 { center: three("center", c)?, size })
}

fn validate(root: PathBuf, vina: Option<PathBuf>, a: ValidateArgs) -> Result<()> {
    let started = manifest::now();
    let env = Arc::new(Env::open(root, vina)?);
    if !a.structure.is_file() {
        bail!("--structure {} does not exist", a.structure.display());
    }
    if a.smiles.trim().is_empty() {
        bail!("--smiles is empty");
    }
    prepare_out(&a.out, a.opts.overwrite)?;
    let b = backend(env.clone());
    // (box and split now go through Backend::structure)

    let rec_pdb = a.out.join("receptor.pdb");
    let crystal = a.out.join("crystal_ligand.pdb");
    eprintln!("dock: splitting {} on {}", a.structure.display(), a.ligand_code);
    let split = b.structure.split(&a.structure, &a.ligand_code, &rec_pdb, &crystal)?;
    if split.ligand_copies.len() > 1 {
        eprintln!(
            "dock: {} copies of {} present ({}); keeping the one in chain {}",
            split.ligand_copies.len(),
            a.ligand_code,
            split.ligand_copies.join(", "),
            split.receptor_chain
        );
    }

    eprintln!("dock: preparing receptor (pH {})", a.opts.ph);
    let receptor = b.receptor.prepare(&rec_pdb, a.opts.ph, &a.out)?;

    eprintln!("dock: rebuilding the ligand from SMILES");
    let ligand = b.ligand.prepare_smiles(&a.smiles, &a.out.join("ligand.pdbqt"))?;

    let bx = b.structure.box_from_reference(&crystal, 5.0)?;
    let docked = a.out.join("docked.pdbqt");
    let eopts = a.opts.engine();
    warn_box(bx, &eopts);
    eprintln!("dock: docking (exhaustiveness {}, {} threads)", eopts.exhaustiveness, eopts.cpus);
    let poses = b.engine.dock(&receptor, &ligand, bx, &eopts, &docked)?;

    let rmsd = b.rmsd.rmsd_to_reference(&docked, &crystal, Some(&a.smiles))?;
    match rmsd.first() {
        Some(r) if *r <= 2.0 => eprintln!("dock: pose 1 is {r:.2} A from the crystal pose — redock succeeded"),
        Some(r) => eprintln!(
            "dock: pose 1 is {r:.2} A from the crystal pose. Over 2 A is normally counted as a \
             failed redock: check the box, the protonation, and whether the pocket needs a cofactor."
        ),
        None => {}
    }

    finish(
        &env, &b, &a.out, &receptor, &ligand, &docked, bx, &poses, Some(rmsd), &a.ligand_code,
        &format!("{} · redocking validation", a.structure.display()),
        serde_json::json!({
            "structure": a.structure.display().to_string(),
            "ligand_code": a.ligand_code,
            "smiles": a.smiles,
            "box": bx,
            "exhaustiveness": eopts.exhaustiveness,
            "seed": eopts.seed,
            "cpus": eopts.cpus,
            "receptor_ph": a.opts.ph,
        }),
        started,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish(
    env: &Arc<Env>,
    b: &Backend,
    out: &Path,
    receptor: &PreparedReceptor,
    ligand: &PreparedLigand,
    docked: &Path,
    bx: Box3,
    poses: &[Pose],
    rmsd: Option<Vec<f64>>,
    label: &str,
    subtitle: &str,
    parameters: serde_json::Value,
    started: String,
) -> Result<()> {
    eprintln!("dock: profiling contacts for pose 1");
    let contacts = b.profiler.profile(receptor, docked, 1)?;

    let spread = poses.iter().filter(|p| p.rmsd_ub > 5.0).count();
    if spread > 0 {
        eprintln!(
            "dock: {spread} of {} poses sit more than 5 A from pose 1 — a different part of \
             the pocket, or the molecule flipped end to end, not variations on one binding mode",
            poses.len()
        );
    }

    let html = report::render(&ReportInput {
        title: &format!("{label} — docking report"),
        subtitle,
        ligand_label: label,
        poses,
        contacts: &contacts,
        bx,
        rmsd: rmsd.as_deref(),
    });
    let report_path = out.join("report.html");
    std::fs::write(&report_path, html)
        .with_context(|| format!("writing {}", report_path.display()))?;

    let results = serde_json::json!({
        "poses": poses,
        "contacts": contacts,
        "rmsd_to_reference": rmsd,
        "ligand": {
            "heavy_atoms": ligand.heavy_atoms,
            "rotatable_bonds": ligand.rotatable_bonds,
            "source": ligand.source,
        },
        "receptor": { "residues": receptor.residues, "ph": receptor.ph },
    });
    std::fs::write(out.join("results.json"), serde_json::to_string_pretty(&results)?)?;

    let mut tools = BTreeMap::new();
    tools.insert("vina".to_string(), crate::env::VINA_VERSION.to_string());
    if let Ok(v) = env.versions()
        && let Some(map) = v.get("versions").and_then(|x| x.as_object())
    {
        for (k, val) in map {
            if let Some(cleaned) = val.as_str().and_then(manifest::clean_version) {
                tools.insert(k.clone(), cleaned);
            }
        }
    }

    let mut files = BTreeMap::new();
    for name in ["docked.pdbqt", "ligand.pdbqt", "report.html", "results.json"] {
        let p = out.join(name);
        if p.is_file() {
            files.insert(name.to_string(), manifest::hash_file(&p)?);
        }
    }

    Manifest {
        format: manifest::FORMAT,
        client: manifest::Client { name: "dock", version: env!("CARGO_PKG_VERSION") },
        started,
        created: manifest::now(),
        tools,
        // Recorded argv and parameters are full paths; the home directory comes out before
        // the manifest is written, because the manifest is meant to be publishable.
        commands: env
            .commands()
            .iter()
            .map(|argv| argv.iter().map(|a| manifest::redact_home(a)).collect())
            .collect(),
        parameters: manifest::redact_value(&parameters),
        results,
        files,
    }
    .write(out)?;

    println!("{}", report_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("dock").chain(args.iter().copied()))
    }

    fn run_args(args: &[&str]) -> RunArgs {
        match parse(args).expect("should parse").command {
            Command::Run(a) => a,
            _ => panic!("expected run"),
        }
    }

    /// The form the docs and the help text advertise. `num_args = 3` used to reject it,
    /// because clap counts command-line words before applying the delimiter.
    #[test]
    fn comma_separated_box_is_accepted() {
        let a = run_args(&["run", "--receptor", "r.pdb", "--smiles", "CCO",
                           "--center", "1,2,3", "--out", "o"]);
        assert_eq!(a.center.as_deref(), Some(&[1.0, 2.0, 3.0][..]));
    }

    #[test]
    fn space_separated_box_still_works() {
        let a = run_args(&["run", "--receptor", "r.pdb", "--smiles", "CCO",
                           "--center", "1", "2", "3", "--out", "o"]);
        assert_eq!(a.center.as_deref(), Some(&[1.0, 2.0, 3.0][..]));
    }

    #[test]
    fn a_box_that_is_not_three_numbers_is_rejected() {
        let a = run_args(&["run", "--receptor", "r.pdb", "--smiles", "CCO",
                           "--center", "1,2", "--out", "o"]);
        let err = three("center", a.center.as_ref().unwrap()).unwrap_err();
        assert!(format!("{err}").contains("three numbers"), "{err}");
    }

    #[test]
    fn size_cannot_be_given_with_reference() {
        // It would be silently ignored: the box comes from the reference ligand.
        assert!(parse(&["run", "--receptor", "r.pdb", "--smiles", "CCO",
                        "--reference", "l.pdb", "--size", "30,30,30", "--out", "o"]).is_err());
    }

    #[test]
    fn overwrite_refuses_a_directory_dock_did_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("precious");
        std::fs::create_dir(&victim).unwrap();
        std::fs::write(victim.join("important.txt"), b"data").unwrap();

        let err = removable(&victim).expect_err("must refuse to delete unrelated data");
        assert!(format!("{err}").contains("not empty and was not written by dock"), "{err}");
        assert!(victim.join("important.txt").exists(), "nothing may be removed");
    }

    #[test]
    fn overwrite_accepts_a_previous_dock_run() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("run1");
        std::fs::create_dir(&out).unwrap();
        std::fs::write(out.join("docked.pdbqt"), b"x").unwrap();
        std::fs::write(
            out.join("manifest.json"),
            format!("{{\"format\":\"{}\"}}", manifest::FORMAT),
        )
        .unwrap();
        removable(&out).expect("a previous dock run may be replaced");
    }

    #[test]
    fn overwrite_accepts_an_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("empty");
        std::fs::create_dir(&out).unwrap();
        removable(&out).expect("an empty directory has nothing to lose");
    }

    #[test]
    fn overwrite_refuses_the_home_directory() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("notes.txt"), b"x").unwrap();
        // SAFETY: single-threaded test, restored immediately.
        let prev = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", &home) };
        let err = removable(&home).expect_err("must refuse $HOME");
        assert!(format!("{err}").contains("$HOME"), "{err}");
        match prev {
            Some(p) => unsafe { std::env::set_var("HOME", p) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }
}
