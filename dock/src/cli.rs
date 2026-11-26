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
    /// pH used to decide protonation states. Default 7.4.
    #[arg(long, default_value_t = 7.4)]
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
    /// Name for the ligand in the report. Defaults to the file stem or "ligand".
    #[arg(long)]
    name: Option<String>,
    /// Box centre as x,y,z in angstroms.
    #[arg(long, value_delimiter = ',', num_args = 3, conflicts_with = "reference")]
    center: Option<Vec<f64>>,
    /// Box size as x,y,z in angstroms. Default 24,24,24.
    #[arg(long, value_delimiter = ',', num_args = 3)]
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
        rmsd: Box::new(Py(env)),
    }
}

fn prepare_out(out: &Path, overwrite: bool) -> Result<()> {
    if out.exists() {
        if !overwrite {
            bail!(
                "{} already exists. Pass --overwrite to replace it.",
                out.display()
            );
        }
        std::fs::remove_dir_all(out)
            .with_context(|| format!("replacing {}", out.display()))?;
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    Ok(())
}

fn run(root: PathBuf, vina: Option<PathBuf>, a: RunArgs) -> Result<()> {
    let started = manifest::now();
    prepare_out(&a.out, a.opts.overwrite)?;
    let env = Arc::new(Env::open(root, vina)?);
    let b = backend(env.clone());
    let py = Py(env.clone());

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
        (_, Some(f)) => b.ligand.prepare_file(f, &lig_out)?,
        _ => bail!("give the ligand with --smiles or --ligand-file"),
    };
    eprintln!(
        "dock: {} heavy atoms, {} rotatable bonds",
        ligand.heavy_atoms, ligand.rotatable_bonds
    );

    let bx = resolve_box(&py, &a)?;
    eprintln!(
        "dock: box centre {:.2}, {:.2}, {:.2}  size {:.0} x {:.0} x {:.0}",
        bx.center[0], bx.center[1], bx.center[2], bx.size[0], bx.size[1], bx.size[2]
    );

    let docked = a.out.join("docked.pdbqt");
    let eopts = a.opts.engine();
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
            "ph": a.opts.ph,
        }),
        started,
    )
}

fn resolve_box(py: &Py, a: &RunArgs) -> Result<Box3> {
    if let Some(r) = &a.reference {
        return py.box_from_reference(r, a.padding);
    }
    let Some(c) = &a.center else {
        bail!("give the box with --center x,y,z or --reference <bound ligand>");
    };
    let size = a.size.clone().unwrap_or_else(|| vec![24.0, 24.0, 24.0]);
    Ok(Box3 {
        center: [c[0], c[1], c[2]],
        size: [size[0], size[1], size[2]],
    })
}

fn validate(root: PathBuf, vina: Option<PathBuf>, a: ValidateArgs) -> Result<()> {
    let started = manifest::now();
    prepare_out(&a.out, a.opts.overwrite)?;
    let env = Arc::new(Env::open(root, vina)?);
    let b = backend(env.clone());
    let py = Py(env.clone());

    let rec_pdb = a.out.join("receptor.pdb");
    let crystal = a.out.join("crystal_ligand.pdb");
    eprintln!("dock: splitting {} on {}", a.structure.display(), a.ligand_code);
    let split = py.split(&a.structure, &a.ligand_code, &rec_pdb, &crystal)?;
    if let Some(copies) = split.get("ligand_copies").and_then(|c| c.as_array())
        && copies.len() > 1
    {
        eprintln!(
            "dock: {} copies of {} present; using chain {}",
            copies.len(),
            a.ligand_code,
            split.get("receptor_chain").and_then(|c| c.as_str()).unwrap_or("?")
        );
    }

    eprintln!("dock: preparing receptor (pH {})", a.opts.ph);
    let receptor = b.receptor.prepare(&rec_pdb, a.opts.ph, &a.out)?;

    eprintln!("dock: rebuilding the ligand from SMILES");
    let ligand = b.ligand.prepare_smiles(&a.smiles, &a.out.join("ligand.pdbqt"))?;

    let bx = py.box_from_reference(&crystal, 5.0)?;
    let docked = a.out.join("docked.pdbqt");
    let eopts = a.opts.engine();
    eprintln!("dock: docking (exhaustiveness {}, {} threads)", eopts.exhaustiveness, eopts.cpus);
    let poses = b.engine.dock(&receptor, &ligand, bx, &eopts, &docked)?;

    let rmsd = b.rmsd.rmsd_to_reference(&docked, &crystal, &a.smiles)?;
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
            "ph": a.opts.ph,
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
            "dock: {spread} of {} poses sit more than 5 A from pose 1 — those are different \
             pockets, not variations on one binding mode",
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
        commands: env.commands(),
        parameters,
        results,
        files,
    }
    .write(out)?;

    println!("{}", report_path.display());
    Ok(())
}
