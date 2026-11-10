//! Command line.

use crate::outputs::Outcome;
use crate::pipeline::{self, ClassifyOptions, PrepOptions, Tools, VerifyOptions};
use crate::reference;
use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "gtdbprep",
    version,
    about = "Classify genomes with GTDB-Tk without the 100 GB download: run the light stages here and \
             write a small, renamed bundle for placement"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Rename genomes, run `gtdbtk identify` + `align --skip_gtdb_refs`, write the bundle.
    Prep(PrepArgs),
    /// Classify a bundle with a local `gtdbtk classify`; results get the original names back.
    Classify(ClassifyArgs),
    /// Round trip: prep, classify from the bundle, run plain `classify_wf`, compare per genome.
    Verify(VerifyArgs),
}

#[derive(Args, Clone)]
struct ToolArgs {
    /// GTDB-Tk reference data (prep needs only markers/, masks/, metadata/ and taxonomy/).
    #[arg(long, env = "GTDBTK_DATA_PATH")]
    data: PathBuf,
    /// Conda/micromamba environment with GTDB-Tk 2.8 (its bin/ is put first on PATH).
    #[arg(long, env = "GTDBPREP_ENV")]
    env: Option<PathBuf>,
    #[arg(long, default_value_t = 4)]
    cpus: usize,
}

#[derive(Args, Clone)]
struct PrepArgs {
    /// Folder of genome FASTA files (plain or gzipped).
    #[arg(long)]
    genomes: PathBuf,
    /// File name extension to pick up, as GTDB-Tk's -x (`gz` for `*.fna.gz`).
    #[arg(short = 'x', long, default_value = "fna")]
    extension: String,
    /// Work folder: renamed genomes, the local name mapping, GTDB-Tk output, logs, bundle/.
    #[arg(long)]
    work: PathBuf,
    /// Also copy the bundle here: a folder, or a file ending in .tar.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Replace gtdbprep's own files in a non-empty work folder.
    #[arg(long)]
    force: bool,
    /// Add skani 0.3.1 sketches (contain DNA fragments; see the privacy notes).
    #[arg(long)]
    sketch: bool,
    /// skani 0.3.1 binary for --sketch.
    #[arg(long, default_value = "skani")]
    skani: PathBuf,
    /// Go on when the reference folders don't match GTDB-Tk's REF_HASHES.
    #[arg(long)]
    allow_ref_mismatch: bool,
    #[command(flatten)]
    tools: ToolArgs,
}

#[derive(Args)]
struct ClassifyArgs {
    /// Bundle folder, .tar or .tar.gz.
    #[arg(long)]
    bundle: PathBuf,
    /// The work folder `prep` made (renamed genomes and mapping).
    #[arg(long)]
    work: PathBuf,
    /// Pass --full_tree to GTDB-Tk (default: only when the reference data has no split trees,
    /// as in GTDB's mockup database).
    #[arg(long)]
    full_tree: Option<bool>,
    #[command(flatten)]
    tools: ToolArgs,
}

#[derive(Args)]
struct VerifyArgs {
    #[command(flatten)]
    prep: PrepArgs,
    #[arg(long)]
    full_tree: Option<bool>,
}

fn tools(a: &ToolArgs) -> Tools {
    Tools {
        env_prefix: a.env.clone(),
        data: a.data.clone(),
        cpus: a.cpus.max(1),
    }
}

fn prep_options(a: &PrepArgs) -> PrepOptions {
    PrepOptions {
        genomes: a.genomes.clone(),
        extension: a.extension.clone(),
        work: a.work.clone(),
        out: a.out.clone(),
        force: a.force,
        sketch: a.sketch.then(|| a.skani.clone()),
        allow_ref_mismatch: a.allow_ref_mismatch,
    }
}

fn full_tree(flag: Option<bool>, data: &Path) -> bool {
    flag.unwrap_or_else(|| {
        let none = !reference::has_split_trees(data);
        if none {
            eprintln!(
                "gtdbprep: {}/split is empty (mockup data?); using --full_tree",
                data.display()
            );
        }
        none
    })
}

fn run(cli: Cli) -> Result<bool> {
    match cli.command {
        Command::Prep(a) => {
            let r = pipeline::prep(&prep_options(&a), &tools(&a.tools))?;
            println!(
                "bundle written to {}",
                a.out.as_deref().unwrap_or(&r.bundle).display()
            );
            println!("{}", pipeline::size_report(&r));
            println!(
                "local name mapping (do not send): {}",
                r.bundle.with_file_name(pipeline::MAPPING).display()
            );
            for (g, why) in &r.excluded {
                println!("left out (not in the bundle): {g}: {why}");
            }
            if r.manifest.sketches.is_some() {
                println!("WARNING: {}", pipeline::SKETCH_WARNING);
            }
            Ok(true)
        }
        Command::Classify(a) => {
            let ft = full_tree(a.full_tree, &a.tools.data);
            let s = pipeline::classify(
                &ClassifyOptions {
                    bundle: a.bundle,
                    work: a.work.clone(),
                    full_tree: ft,
                },
                &tools(&a.tools),
            )?;
            print!("{}", s.to_tsv());
            Ok(true)
        }
        Command::Verify(a) => {
            let ft = full_tree(a.full_tree, &a.prep.tools.data);
            let r = pipeline::verify(
                &VerifyOptions {
                    prep: prep_options(&a.prep),
                    full_tree: ft,
                },
                &tools(&a.prep.tools),
            )
            .context("verify")?;
            println!("{}", pipeline::size_report(&r.prep));
            for (g, o) in &r.outcomes {
                println!("{g}\t{}", pipeline::outcome_text(o).trim_end());
            }
            let same = r
                .outcomes
                .iter()
                .filter(|(_, o)| *o == Outcome::Identical)
                .count();
            let left_out = r
                .outcomes
                .iter()
                .filter(|(_, o)| matches!(o, Outcome::ExcludedByPrep(_)))
                .count();
            println!(
                "{same}/{} genome(s) identical to classify_wf; {left_out} input(s) left out by prep",
                r.outcomes.len() - left_out
            );
            if same == 0 {
                println!("nothing was compared");
            }
            Ok(r.all_identical())
        }
    }
}

/// Entry point for both binaries. `fixed` puts a subcommand in front of the arguments
/// (`irit-gtdb-prep` is `gtdbprep prep`).
pub fn main(fixed: Option<&str>) -> ExitCode {
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let Some(sub) = fixed {
        args.insert(1, sub.into());
    }
    let cli = Cli::parse_from(args);
    match run(cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("gtdbprep: error: {e:#}");
            ExitCode::from(2)
        }
    }
}
