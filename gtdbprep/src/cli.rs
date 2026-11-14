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
    /// GTDB-Tk's ANI species screen without the 85 GB skani database: screen locally, fetch only
    /// the reference sketches needed, run GTDB-Tk's screen on them.
    Ani(AniArgs),
    /// Tree placement on placement-only reference data: `identify`, `align --skip_gtdb_refs`, then
    /// `classify` without the ANI screen, with gPlace as the placer under a memory cap.
    Place(PlaceArgs),
    /// Make the screen index `ani` needs, from a full skani database (maintainers).
    #[command(subcommand)]
    AniIndex(AniIndexCmd),
}

#[derive(Args)]
struct AniArgs {
    /// Folder of genome FASTA files (plain or gzipped), as GTDB-Tk's --genome_dir.
    #[arg(long)]
    genomes: PathBuf,
    /// File name extension, as GTDB-Tk's -x.
    #[arg(short = 'x', long, default_value = "fna")]
    extension: String,
    /// Screen index folder (meta.json, refs.bin, markers.idx).
    #[arg(long)]
    index: PathBuf,
    /// Where sketches the cache doesn't have come from: a path or URL of an unpacked sketches.db,
    /// a URL with {part} and --part-size, or `gtdb` to range-read GTDB's own archive (opt-in: it
    /// loads GTDB's server). Not needed when every sketch is already cached.
    #[arg(long)]
    source: Option<String>,
    /// Part size in bytes, for a source with {part}.
    #[arg(long)]
    part_size: Option<u64>,
    /// Another URL or local path for GTDB's archive (for the `gtdb` source).
    #[arg(long)]
    gtdb_url: Option<String>,
    /// Work folder (query sketches, the small database, GTDB-Tk's output, logs).
    #[arg(long)]
    work: PathBuf,
    /// Keep fetched sketches here, to reuse in later runs (default: $XDG_CACHE_HOME/irit/gtdbprep-ani,
    /// or ~/.cache/irit/gtdbprep-ani).
    #[arg(long)]
    cache: Option<PathBuf>,
    /// skani binary; must be the version the database was made with (0.3.1 for R232).
    #[arg(long, default_value = "skani")]
    skani: PathBuf,
    /// Go on with another skani version.
    #[arg(long)]
    allow_skani_version: bool,
    /// Testing: skip the local screen and run GTDB-Tk's screen on this full skani database.
    #[arg(long)]
    full_db: Option<PathBuf>,
    /// Range requests in flight at once while fetching sketches (1 to 16).
    #[arg(long, default_value_t = crate::ani::source::DEFAULT_JOBS as u8,
          value_parser = clap::value_parser!(u8).range(1..=crate::ani::source::MAX_JOBS as i64))]
    fetch_jobs: u8,
    /// Fetch only the references that could give the species call (ANI within reach of the
    /// smallest species radius). Same species calls as a full run, far fewer downloads, but
    /// GTDB-Tk's "other related references" and unassigned-hits files list fewer references.
    #[arg(long)]
    species_only: bool,
    /// GTDB-Tk reference data with taxonomy/, radii/ and metadata/ (the placement mirror has
    /// them; no skani/ needed).
    #[arg(long, env = "GTDBTK_DATA_PATH")]
    data: PathBuf,
    /// Conda/micromamba environment with GTDB-Tk 2.8.
    #[arg(long, env = "GTDBPREP_ENV")]
    env: Option<PathBuf>,
    #[arg(long, default_value_t = 4)]
    cpus: usize,
}

#[derive(Args)]
struct PlaceArgs {
    /// Folder of genome FASTA files, as GTDB-Tk's --genome_dir.
    #[arg(long)]
    genomes: PathBuf,
    /// File name extension, as GTDB-Tk's -x.
    #[arg(short = 'x', long, default_value = "fna")]
    extension: String,
    /// Work folder (GTDB-Tk's identify/, align/ and classify/ output, place.tsv, place.json, logs).
    /// Must be new, empty, or an earlier `place` run's (its files are replaced).
    #[arg(long)]
    work: PathBuf,
    /// gPlace's --max_memory: 6G kept every GTDB R232 tree at about 5 GiB.
    #[arg(long, default_value = crate::place::DEFAULT_MAX_MEMORY)]
    max_memory: String,
    /// CPUs for each gPlace call (GTDB-Tk's --pplacer_cpus).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..))]
    pplacer_cpus: u16,
    /// Work folder of `gtdbprep ani`: genomes its screen assigned to a species are not placed and
    /// get the screen's call in place.tsv.
    #[arg(long)]
    skip_assigned: Option<PathBuf>,
    /// The gplace program (default: the GTDB-Tk environment's, else the one on PATH).
    #[arg(long)]
    gplace: Option<PathBuf>,
    /// Pass --full_tree to GTDB-Tk (default: only when the reference data has no split trees).
    #[arg(long)]
    full_tree: Option<bool>,
    /// GTDB-Tk reference data with the placement trees (`gtdbtk-r232-placement-lite` has them;
    /// no skani/ or msa/ needed).
    #[arg(long, env = "GTDBTK_DATA_PATH")]
    data: PathBuf,
    /// Conda/micromamba environment with GTDB-Tk 2.8.0 and gPlace.
    #[arg(long, env = "GTDBPREP_ENV")]
    env: Option<PathBuf>,
    #[arg(long, default_value_t = 4)]
    cpus: usize,
}

#[derive(Subcommand)]
enum AniIndexCmd {
    /// Build meta.json, refs.bin and markers.idx from a skani database folder.
    Build {
        /// The skani database folder (markers.bin, index.db, sketches.db).
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// GTDB release label, e.g. r232.
        #[arg(long)]
        release: Option<String>,
    },
    /// Read a GTDB-Tk .tar.gz once and write a seek index for its sketches.db.
    GzScan {
        /// The archive, or - for standard input.
        #[arg(long)]
        input: PathBuf,
        /// Output folder (gz-full.idx, gz.json).
        #[arg(long)]
        out: PathBuf,
        /// Uncompressed bytes between checkpoints.
        #[arg(long, default_value_t = 1 << 20)]
        spacing: u64,
        /// Tar member to index.
        #[arg(long, default_value = "skani/database/sketches.db")]
        member: String,
        /// The archive's URL, recorded for clients.
        #[arg(long)]
        url: String,
    },
    /// Add a gz-scan result to an index, keeping every N-th checkpoint.
    GzAttach {
        #[arg(long)]
        index: PathBuf,
        /// The gz-scan output folder.
        #[arg(long)]
        scan: PathBuf,
        #[arg(long, default_value_t = 1)]
        every: usize,
    },
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

fn place_options(a: &PlaceArgs, full_tree: bool) -> crate::place::PlaceOptions {
    crate::place::PlaceOptions {
        genomes: a.genomes.clone(),
        extension: a.extension.clone(),
        work: a.work.clone(),
        max_memory: a.max_memory.clone(),
        pplacer_cpus: usize::from(a.pplacer_cpus),
        skip_assigned: a.skip_assigned.clone(),
        gplace: a.gplace.clone(),
        full_tree,
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
        Command::Ani(a) => {
            let opts = crate::ani::AniOptions {
                genomes: a.genomes,
                extension: a.extension,
                index: a.index,
                source: a.source,
                part_size: a.part_size,
                gtdb_url: a.gtdb_url,
                work: a.work,
                cache: a.cache,
                skani: a.skani,
                allow_skani_version: a.allow_skani_version,
                full_db: a.full_db,
                fetch_jobs: usize::from(a.fetch_jobs),
                species_only: a.species_only,
            };
            let t = Tools {
                env_prefix: a.env,
                data: a.data,
                cpus: a.cpus.max(1),
            };
            let r = crate::ani::run(&opts, &t)?;
            print!("{}", r.to_tsv());
            if r.species_only {
                println!(
                    "{} reference(s) screened in, {} could give the species call (--species-only)",
                    r.candidates, r.used
                );
            } else {
                println!("{} reference(s) screened in", r.candidates);
            }
            println!(
                "{} already cached; fetched {} bytes in {} request(s)",
                r.cached, r.fetched_bytes, r.requests
            );
            println!(
                "times: screen {:.1} s, fetch {:.1} s, GTDB-Tk {:.1} s",
                r.seconds_screen, r.seconds_fetch, r.seconds_gtdbtk
            );
            println!(
                "GTDB-Tk's ANI screen output: {}",
                r.out_dir.join("classify/ani_screen").display()
            );
            Ok(true)
        }
        Command::Place(a) => {
            let ft = full_tree(a.full_tree, &a.data);
            let opts = place_options(&a, ft);
            let t = Tools {
                env_prefix: a.env,
                data: a.data,
                cpus: a.cpus.max(1),
            };
            let r = crate::place::run(&opts, &t)?;
            print!("{}", crate::place::rows_to_tsv(&r.rows));
            println!(
                "{} genome(s) placed with gPlace ({} gplace call(s), --max_memory {}), {} skipped \
                 (assigned by the ANI screen)",
                r.placed, r.gplace_calls, opts.max_memory, r.skipped
            );
            println!(
                "times: identify {:.1} s, align {:.1} s, classify {:.1} s",
                r.seconds_identify, r.seconds_align, r.seconds_classify
            );
            println!("table: {}", r.table.display());
            if r.placed > 0 {
                println!(
                    "GTDB-Tk's summary: {}",
                    r.gtdbtk_out.join("classify").display()
                );
            }
            Ok(true)
        }
        Command::AniIndex(AniIndexCmd::Build { db, out, release }) => {
            let s = crate::ani::screen::build(&db, &out, release.as_deref())?;
            println!(
                "{} references, {} marker k-mers (largest set {}); markers.bin {} bytes -> \
                 markers.idx {} bytes + refs.bin {} bytes; largest sketch {} bytes",
                s.refs,
                s.markers_total,
                s.markers_max,
                s.markers_bin_bytes,
                s.markers_idx_bytes,
                s.refs_bin_bytes,
                s.sketch_bytes_max
            );
            Ok(true)
        }
        Command::AniIndex(AniIndexCmd::GzScan {
            input,
            out,
            spacing,
            member,
            url,
        }) => {
            let info = crate::ani::gz_scan(&input, &out, spacing, &member, &url)?;
            println!("{}", serde_json::to_string_pretty(&info)?);
            Ok(true)
        }
        Command::AniIndex(AniIndexCmd::GzAttach { index, scan, every }) => {
            let n = crate::ani::gz_attach(&index, &scan, every)?;
            println!(
                "{n} checkpoints written to {}",
                index.join("gz.idx").display()
            );
            Ok(true)
        }
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
    // `--version` belongs to the program, not the subcommand (`gtdbprep prep --version` is refused).
    let asks_version = args.get(1).is_some_and(|a| a == "--version" || a == "-V");
    if let Some(sub) = fixed.filter(|_| !asks_version) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn place(args: &[&str]) -> Result<PlaceArgs, clap::Error> {
        let mut v = vec!["gtdbprep", "place"];
        v.extend_from_slice(args);
        match Cli::try_parse_from(v)?.command {
            Command::Place(a) => Ok(a),
            _ => unreachable!(),
        }
    }

    #[test]
    fn place_arguments() {
        let base = ["--genomes", "g", "--work", "w", "--data", "d"];
        let a = place(&base).unwrap();
        assert_eq!(
            (
                a.extension.as_str(),
                a.max_memory.as_str(),
                a.pplacer_cpus,
                a.cpus
            ),
            ("fna", "6G", 1, 4)
        );
        assert!(a.skip_assigned.is_none() && a.gplace.is_none() && a.full_tree.is_none());
        let mut v = base.to_vec();
        v.extend([
            "-x",
            "fa",
            "--max-memory",
            "8.5G",
            "--pplacer-cpus",
            "2",
            "--skip-assigned",
            "ani",
            "--gplace",
            "/x/gplace",
            "--env",
            "/e",
            "--cpus",
            "6",
            "--full-tree",
            "false",
        ]);
        let a = place(&v).unwrap();
        let o = place_options(&a, false);
        assert_eq!(o.extension, "fa");
        assert_eq!(o.max_memory, "8.5G");
        assert_eq!(o.pplacer_cpus, 2);
        assert_eq!(o.skip_assigned.as_deref(), Some(Path::new("ani")));
        assert_eq!(o.gplace.as_deref(), Some(Path::new("/x/gplace")));
        assert_eq!(a.env.as_deref(), Some(Path::new("/e")));
        assert_eq!(a.cpus, 6);
        assert_eq!(a.full_tree, Some(false));
        // --work and --genomes are required; --pplacer-cpus 0 is refused.
        assert!(place(&["--genomes", "g", "--data", "d"]).is_err());
        assert!(place(&["--work", "w", "--data", "d"]).is_err());
        let mut v = base.to_vec();
        v.extend(["--pplacer-cpus", "0"]);
        assert!(place(&v).is_err());
    }
}
