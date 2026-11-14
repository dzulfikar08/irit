//! Third-party tools (fastp, Shovill, Flye, Medaka, Bakta, CheckM2, GTDB-Tk, mlst, AMRFinderPlus,
//! MEGAHIT, CoverM, MetaBAT2, SemiBin2, galah, Kleborate, Rasusa): typed
//! parameters, checks before running, and their argument vectors.
//!
//! Every flag here exists in the pinned upstream version (see [`PINNED`]); the comment above each
//! builder names the source file it was read from. Nothing is passed that the pinned version
//! doesn't have. Options are written `--name=value` (fastp's cmdline.h, Python's argparse, Perl's
//! Getopt::Long and NCBI's ShellApplication all accept that form), so a value starting with `-` is
//! never read as a flag.

use crate::params::Invocation;
use crate::validate::{self as v, Check, Ctx, Seq};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The upstream version each wrapper's flags were checked against.
pub struct Pinned {
    pub tool: &'static str,
    pub version: &'static str,
    pub bioconda: &'static str,
    pub url: &'static str,
}

pub const PINNED: [Pinned; 16] = [
    Pinned { tool: "fastp", version: "1.4.0", bioconda: "fastp=1.4.0", url: "https://github.com/OpenGene/fastp" },
    Pinned { tool: "shovill", version: "1.4.2", bioconda: "shovill=1.4.2", url: "https://github.com/tseemann/shovill" },
    // Flye's --version prints the build too: `2.9.6-b1802` for the 2.9.6 release (and Bioconda's).
    Pinned { tool: "flye", version: "2.9.6-b1802", bioconda: "flye=2.9.6", url: "https://github.com/mikolmogorov/Flye" },
    // medaka_consensus (the polishing script) reports no version itself; the server asks `medaka`
    // beside it, which prints `medaka 2.2.2`.
    Pinned { tool: "medaka_consensus", version: "2.2.2", bioconda: "medaka=2.2.2", url: "https://github.com/nanoporetech/medaka" },
    Pinned { tool: "bakta", version: "1.12.2", bioconda: "bakta=1.12.2", url: "https://github.com/oschwengers/bakta" },
    Pinned { tool: "checkm2", version: "1.1.0", bioconda: "checkm2=1.1.0", url: "https://github.com/chklovski/CheckM2" },
    Pinned { tool: "gtdbtk", version: "2.8.0", bioconda: "gtdbtk=2.8.0", url: "https://github.com/Ecogenomics/GTDBTk" },
    Pinned { tool: "mlst", version: "2.35.0", bioconda: "mlst=2.35.0", url: "https://github.com/tseemann/mlst" },
    Pinned { tool: "amrfinder", version: "4.2.7", bioconda: "ncbi-amrfinderplus=4.2.7", url: "https://github.com/ncbi/amr" },
    Pinned { tool: "megahit", version: "1.2.9", bioconda: "megahit=1.2.9", url: "https://github.com/voutcn/megahit" },
    Pinned { tool: "coverm", version: "0.8.0", bioconda: "coverm=0.8.0", url: "https://github.com/wwood/CoverM" },
    Pinned { tool: "metabat2", version: "2.18", bioconda: "metabat2=2.18", url: "https://bitbucket.org/berkeleylab/metabat" },
    // `kleborate --version` prints `Kleborate v3.2.4`. Bioconda's package also pulls in ECTyper, whose
    // install script downloads about 900 MB; pip's (with kaptive 3.2.2, see docs/tools/mcp.md) doesn't.
    Pinned { tool: "kleborate", version: "3.2.4", bioconda: "kleborate=3.2.4", url: "https://github.com/klebgenomics/Kleborate" },
    Pinned { tool: "rasusa", version: "5.1.0", bioconda: "rasusa=5.1.0", url: "https://github.com/mbhall88/rasusa" },
    // The program is `SemiBin2`; `SemiBin2 --version` prints `2.6.0`. The package brings bedtools,
    // samtools, Prodigal and HMMER, and CPU PyTorch (about 390 MB to download).
    Pinned { tool: "SemiBin2", version: "2.6.0", bioconda: "semibin=2.6.0", url: "https://github.com/BigDataBiology/SemiBin" },
    // galah needs skani on PATH (0.4.0 in Bioconda's galah=0.6.3). The package also pulls in CheckM2
    // with TensorFlow (about 1 GB) for --run-checkm2, which irit-mcp doesn't use: `galah=0.6.3
    // --no-deps` plus `skani=0.4.0` is enough.
    Pinned { tool: "galah", version: "0.6.3", bioconda: "galah=0.6.3 skani=0.4.0", url: "https://github.com/wwood/galah" },
];

/// Arguments that make a tool print its version, and whether it prints it on stderr. MetaBAT2
/// has no --version; its --help starts `MetaBAT: ... (version 2:2.18; <build date>)` on stderr.
pub fn version_probe(tool: &str) -> (&'static str, bool) {
    match tool {
        "metabat2" => ("--help", true),
        _ => ("--version", false),
    }
}

pub fn pinned(tool: &str) -> Option<&'static Pinned> {
    PINNED.iter().find(|p| p.tool == tool)
}

/// What to tell the user when the program isn't installed.
pub fn install_hint(tool: &str) -> String {
    match pinned(tool) {
        Some(p) => format!(
            "{tool} not found on PATH. Install the version irit-mcp was checked against with Bioconda \
             (`conda install -c conda-forge -c bioconda {}`, or mamba/micromamba), then restart irit-mcp from that \
             environment, or start it with --tool-path {tool}=/path/to/{tool}",
            p.bioconda
        ),
        None => format!("{tool} not found on PATH"),
    }
}

/// The version number in a `--version` line: the first word that starts with a digit (after an
/// optional `v`) and has a dot. `gtdbtk: version 2.8.0 Copyright ...` gives `2.8.0`.
pub fn version_number(line: &str) -> Option<&str> {
    line.split_whitespace()
        .map(|w| w.trim_end_matches([',', ';', ')']))
        .map(|w| w.strip_prefix('v').unwrap_or(w))
        // MetaBAT's `2:2.18` (major series, then version)
        .map(|w| w.rsplit_once(':').filter(|(a, _)| a.chars().all(|c| c.is_ascii_digit())).map_or(w, |(_, b)| b))
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()) && w.contains('.'))
}

/// A warning when the installed version isn't the pinned one (the call still runs).
pub fn version_warning(tool: &str, reported: Option<&str>) -> Option<String> {
    let p = pinned(tool)?;
    match reported.and_then(version_number) {
        Some(got) if got == p.version => None,
        Some(got) => Some(format!(
            "installed {tool} is {got}; irit-mcp's parameters were checked against {tool} {}. Flags may differ: check the result",
            p.version
        )),
        None => Some(format!("could not read {tool}'s version; irit-mcp's parameters were checked against {tool} {}", p.version)),
    }
}

/// Warnings a tool printed on stderr that irit-mcp knows matter, for the result's `warnings` (a
/// zero exit doesn't show them otherwise).
/// - CheckM2: scikit-learn's `InconsistentVersionWarning`, when its bundled models were saved with
///   another scikit-learn than the one installed ("might lead to breaking code or invalid results").
/// - AMRFinderPlus: its own `WARNING:` lines.
/// - gtdbprep: its `gtdbprep: warning:` lines (a genome left out of the ANI screen).
pub fn stderr_warnings(tool: &str, stderr: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |w: String| {
        if !out.contains(&w) {
            out.push(w);
        }
    };
    for line in stderr.lines().map(str::trim) {
        match tool {
            "checkm2" => {
                if let Some(at) = line.find("Trying to unpickle estimator") {
                    let what = line[at..].split(". ").next().unwrap_or_default();
                    // One per version pair, not one per estimator.
                    let versions = what.split_once(" from version ").map_or(what, |(_, v)| v);
                    add(format!(
                        "CheckM2: scikit-learn InconsistentVersionWarning: its models were saved with scikit-learn {versions}, which scikit-learn says might give invalid results; install the scikit-learn version CheckM2's package pins"
                    ));
                }
            }
            "amrfinder" => {
                if let Some(w) = line.strip_prefix("WARNING:") {
                    add(format!("AMRFinderPlus: {}", w.trim()));
                }
            }
            "gtdbprep" => {
                if let Some(w) = line.strip_prefix("gtdbprep: warning: ") {
                    add(format!("gtdbprep: {w}"));
                }
            }
            _ => {}
        }
    }
    out
}

/// Warnings read from a successful call's own outputs: results that look fine by exit status but
/// aren't (issues #22, #24).
pub fn result_warnings(inv: &Invocation) -> Vec<String> {
    let given = |flag: &str| inv.args.iter().any(|a| a == flag);
    match inv.tool {
        "shovill" => inv
            .outputs
            .first()
            .and_then(|d| std::fs::read_to_string(d.join("shovill.log")).ok())
            .map(|log| shovill_log_warnings(&log, given("--gsize")))
            .unwrap_or_default(),
        "mlst" if !given("--scheme") => inv
            .outputs
            .first()
            .and_then(|o| crate::results::mlst_summary(o).ok())
            .map(|rows| mlst_scheme_warnings(&rows))
            .unwrap_or_default(),
        "kleborate" => {
            let rows: Vec<serde_json::Value> =
                kleborate_tables(&inv.outputs).iter().filter_map(|t| crate::results::kleborate_summary(t).ok()).flatten().collect();
            let strains: Vec<String> =
                inv.args.iter().skip_while(|a| *a != "--assemblies").skip(1).map(|a| kleborate_strain(a)).collect();
            kleborate_warnings(&strains, given("--preset=kpsc"), &rows)
        }
        "coverm" if inv.args.first().is_some_and(|a| a == "genome") && given("--single-genome") => {
            let reference = inv.args.iter().find_map(|a| a.strip_prefix("--reference="));
            let log = reference.and_then(|r| std::fs::read_to_string(inv.inputs[0].with_file_name("shovill.log")).ok().map(|l| (r, l)));
            match (log, inv.outputs.first().and_then(|o| crate::results::coverm_genome_summary(o).ok())) {
                (Some((r, log)), Some(rows)) => coverm_depth_warnings(r, &log, &rows),
                _ => Vec::new(),
            }
        }
        "flye" => inv.outputs.first().map(|d| flye_tandem_warnings(&d.join("assembly.fasta"), &d.join("assembly_info.txt"))).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Flye can close a small plasmid as two or more copies of itself in one circle. On the nanopore
/// run, *K. pneumoniae* AF38's 6,141 bp blaOXA-232 plasmid came out as an 18,377 bp contig holding
/// three copies, so AMRFinderPlus reported blaOXA-232 three times. AF39's 3,731 bp blaTEM-1 plasmid
/// came out inside an 11,269 bp circle with two copies of it, which isn't a clean multiple. So the
/// check is how much of a circular contig (up to 300 kb) repeats itself.
fn flye_tandem_warnings(assembly: &Path, info: &Path) -> Vec<String> {
    let circular: std::collections::HashSet<String> = std::fs::read_to_string(info)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() > 3 && f[3] == "Y").then(|| f[0].to_string())
        })
        .collect();
    let Ok(text) = std::fs::read_to_string(assembly) else { return Vec::new() };
    let mut out = Vec::new();
    for rec in text.split('>').skip(1) {
        let (head, body) = rec.split_once('\n').unwrap_or((rec, ""));
        let name = head.split_whitespace().next().unwrap_or("");
        if !circular.contains(name) {
            continue;
        }
        let seq: Vec<u8> = body.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        if seq.len() > 300_000 {
            continue;
        }
        let repeated = repeated_fraction(&seq);
        if repeated >= 0.5 {
            let shape = match tandem_copies(&seq) {
                Some(n) => format!("{n} copies of one {} bp sequence", seq.len() / n),
                None => "partly a repeat of itself".to_string(),
            };
            out.push(format!(
                "Flye's {name} ({} bp, circular) looks like {shape} ({:.0} % of it occurs twice or more): probably a small plasmid closed as several copies. Genes on it are counted more than once by AMRFinderPlus and the like, and its length is not the plasmid's; Medaka keeps the copies",
                seq.len(),
                repeated * 100.0
            ));
        }
    }
    out
}

/// The share of a sequence's 21-mer positions whose 21-mer occurs more than once in it. On the
/// nanopore runs, contigs Flye made of plasmid copies had 0.92-1.0, every other contig up to 300 kb
/// at most 0.2 (AF38, AF39, AF47 and AHB29, default and meta assemblies).
pub fn repeated_fraction(seq: &[u8]) -> f64 {
    let mut counts: std::collections::HashMap<&[u8], u32> = std::collections::HashMap::new();
    let windows: Vec<&[u8]> = seq.windows(21).collect();
    for w in &windows {
        *counts.entry(w).or_default() += 1;
    }
    if windows.is_empty() {
        return 0.0;
    }
    windows.iter().filter(|w| counts[*w] > 1).count() as f64 / windows.len() as f64
}

/// How many copies of one unit a sequence is (2 to 6), judged by 21-mers: every unit-long piece
/// shares at least 85 % of its 21-mers with the first (AF38's 3-mer: 92 %; a single copy cut in
/// pieces: about 1 %). The largest such count; None for none.
/// Set comparison tolerates the few differences between copies and Flye's arbitrary start point.
pub fn tandem_copies(seq: &[u8]) -> Option<usize> {
    const K: usize = 21;
    let kmers = |s: &[u8]| -> std::collections::HashSet<u64> {
        s.windows(K)
            .filter_map(|w| {
                w.iter().try_fold(0u64, |acc, b| {
                    let v = match b.to_ascii_uppercase() {
                        b'A' => 0,
                        b'C' => 1,
                        b'G' => 2,
                        b'T' => 3,
                        _ => return None,
                    };
                    Some(acc << 2 | v)
                })
            })
            .collect()
    };
    (2..=6).rev().find(|&n| {
        let unit = seq.len() / n;
        if unit < 1000 {
            return false;
        }
        let first = kmers(&seq[..unit]);
        (1..n).all(|i| {
            let piece = kmers(&seq[i * unit..(i + 1) * unit]);
            !piece.is_empty() && piece.iter().filter(|k| first.contains(k)).count() * 100 >= piece.len() * 85
        })
    })
}

/// Shovill's depth estimate is read bases over its own genome-size estimate unless `--gsize` was
/// given; the depth experiment (docs/research/coverage-depth.md) saw it 3x too high. When the
/// assembly CoverM mapped to has a shovill.log beside it, the two are compared.
pub fn coverm_depth_warnings(reference: &str, shovill_log: &str, rows: &[serde_json::Value]) -> Vec<String> {
    let Some(est) = isotable::parse::shovill_depth(shovill_log) else { return Vec::new() };
    rows.iter()
        .filter_map(|r| r["mapped_depth"].as_f64())
        .filter(|&d| est > 0.0 && d > 0.0 && est.max(d) / est.min(d) > 1.5)
        .map(|d| {
            format!(
                "{reference}: Shovill estimated {est:.0}x depth, mapping gives {d:.1}x; they differ by more than 1.5x. Shovill divides by its own genome-size estimate unless given gsize: report the mapped depth"
            )
        })
        .collect()
}

/// Kleborate's tables among a call's outputs that exist: Kleborate 3.2.4 writes each assembly to
/// the table of its species (KpSC, KoSC or Escherichia, whatever the preset; typing columns of a
/// species the preset doesn't cover say `Not Tested`), or to none when its species check finds
/// none of them (it prints "does not match any specified species" and skips the assembly).
pub fn kleborate_tables(outputs: &[PathBuf]) -> Vec<PathBuf> {
    outputs
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with("_output.txt") && !n.to_string_lossy().contains("hAMRonization")))
        .filter(|p| p.is_file())
        .cloned()
        .collect()
}

/// Per assembly: no row at all (a species Kleborate doesn't type), a species outside the
/// K. pneumoniae species complex when the kpsc preset was asked for, a weak species match, and
/// Kleborate's own QC warnings (too many or too small contigs, ambiguous bases).
pub fn kleborate_warnings(strains: &[String], kpsc: bool, rows: &[serde_json::Value]) -> Vec<String> {
    let mut out = Vec::new();
    for s in strains.iter().filter(|s| !rows.iter().any(|r| r["strain"] == s.as_str())) {
        out.push(format!(
            "{s}: Kleborate wrote no row: its species check found no species it types (K. pneumoniae or K. oxytoca complex, Escherichia), and it skipped the assembly. Check the species (gtdbprep_ani)"
        ));
    }
    for r in rows {
        let strain = r["strain"].as_str().unwrap_or("");
        let species = r["species"].as_str().unwrap_or("");
        if kpsc && !isotable::parse::KPSC_SPECIES.iter().any(|k| species.starts_with(k)) {
            out.push(format!(
                "{strain}: Kleborate's species call is {species:?}, outside the K. pneumoniae species complex (KpSC), so the kpsc preset's typing was not run (Not Tested); its row is in {}",
                r["table"].as_str().unwrap_or("another table")
            ));
        }
        if r["species_match"] == "weak" {
            out.push(format!("{strain}: Kleborate's species match for {species} is weak"));
        }
        if let Some(q) = r["QC_warnings"].as_str().filter(|q| !q.is_empty() && *q != "-") {
            out.push(format!("{strain}: Kleborate QC warnings: {q}"));
        }
    }
    out
}

/// Shovill 1.4.2 estimates the genome size from 21-mers seen more than 10 times. At 5-10x depth
/// that gave 0.01-0.11 Mb for a 5.5 Mb K. pneumoniae; Shovill then subsampled the reads to 150x of
/// that size and assembled 0.4-112 kb, with exit 0. At 15x the estimate was 0.5-0.7 Mb (the
/// assembly was fine, 8x bigger than the estimate). Shovill logs the size it used, its depth and
/// `Assembly is <bp>, estimated genome size was <bp> (<pct>%)`.
pub fn shovill_log_warnings(log: &str, gsize_given: bool) -> Vec<String> {
    let num_after = |prefix: &str| -> Option<f64> {
        log.lines().find_map(|l| l.trim().strip_prefix(prefix)).and_then(|r| r.split_whitespace().next()).and_then(|n| n.trim_end_matches(',').parse().ok())
    };
    let size = num_after("[shovill] Using genome size ");
    let depth = num_after("[shovill] Estimated sequencing depth: ");
    let assembly = num_after("[shovill] Assembly is ");
    let subsampled = log.contains("[shovill] Subsampling reads by factor");
    let mb = |x: f64| format!("{:.2} Mb", x / 1e6);
    let mut out = Vec::new();
    let (Some(s), Some(a)) = (size, assembly) else { return out };
    // Shovill's depth is read bases over the size it used; over the assembly length it is the real
    // depth, unless Shovill subsampled (then the reads it kept were scaled to its estimate).
    let real_depth = depth.filter(|_| !subsampled).map(|d| d * s / a);
    if gsize_given {
        if a < 0.7 * s {
            out.push(format!(
                "the assembly is {} for a given genome size of {} ({:.0}%): too little sequence for a whole genome, probably too few reads",
                mb(a),
                mb(s),
                100.0 * a / s
            ));
        }
    } else if subsampled && (a < 0.5 * s || s < 1e6) {
        out.push(format!(
            "Shovill estimated the genome size from the reads as only {} and subsampled the reads to it, then assembled {}: likely a fragment of the genome, which happens at low depth. Rerun with gsize set to the species' genome size (e.g. 5.5M)",
            mb(s),
            mb(a)
        ));
    } else if a > 2.0 * s {
        out.push(format!(
            "Shovill estimated the genome size as {} but assembled {}, so its depth figure is too high: the real depth is about {:.0}x. A low estimate is a sign of low depth; give gsize to be safe",
            mb(s),
            mb(a),
            real_depth.unwrap_or(0.0)
        ));
    }
    if let Some(d) = real_depth
        && d < 15.0
        && a >= 0.7 * s
    {
        out.push(format!(
            "about {d:.0}x depth over the assembly; in irit's depth experiment (docs/research/coverage-depth.md) resistance genes started to drop out below 15x"
        ));
    }
    out
}

/// mlst picks the scheme with the best allele matches unless `scheme` is set. On a fragmented 10x
/// K. pneumoniae assembly mlst 2.35.0 picked ecoli_achtman_4 (status OK, ST `-`), issue #24.
pub fn mlst_scheme_warnings(rows: &[serde_json::Value]) -> Vec<String> {
    rows.iter()
        .filter(|r| r["st"] == "-" && r["scheme"].as_str().is_some_and(|s| s != "-"))
        .map(|r| {
            format!(
                "{}: mlst chose scheme {} by itself but found no ST; on fragmented assemblies it can choose another species' scheme. Check the species (gtdbprep_ani) and rerun with scheme set",
                r["sample"].as_str().unwrap_or(""),
                r["scheme"].as_str().unwrap_or("")
            )
        })
        .collect()
}

/// What to try after a failure whose cause is clear from the exit and stderr: running out of
/// memory, mostly. CheckM2 reports a DIAMOND child killed by SIGKILL (the out-of-memory killer)
/// and exits 1; on the laptop its default DIAMOND block size went over an 8 GB cap on 23 MAGs,
/// and `lowmem` (block size 0.5) peaked at 5.6 GiB.
pub fn failure_hint(tool: &str, exit_status: Option<i32>, stderr: &str) -> Option<String> {
    if tool == "checkm2" && stderr.contains("died with <Signals.SIGKILL") {
        return Some("CheckM2's DIAMOND step was killed (SIGKILL), which usually means it ran out of memory; rerun with lowmem: true (DIAMOND block size 0.5) and/or fewer threads".into());
    }
    if tool == "SemiBin2" && (exit_status.is_none() || exit_status == Some(137)) && stderr.contains("Training model") {
        return Some("SemiBin2 was killed while training its own model, which holds up to 4 million contig pairs in memory (about 8 GB was not enough for one sample of 4,842 contigs); rerun with a pretrained environment (global when none matches) and depth or one BAM".into());
    }
    if exit_status.is_none() || exit_status == Some(137) {
        return Some(format!("{tool} was killed by a signal, often the out-of-memory killer under a memory cap; check the cap, give the tool less memory or fewer threads, or use its low-memory option"));
    }
    None
}

/// The conda environment a program lives in (`<prefix>/bin/<program>`, symlinks followed, with
/// `<prefix>/conda-meta`). AMRFinderPlus from Bioconda looks for `$CONDA_PREFIX` and warns when it
/// is unset (an environment used without being activated).
pub fn conda_prefix(program: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(program).ok()?;
    let prefix = real.parent()?.parent()?;
    prefix.join("conda-meta").is_dir().then(|| prefix.to_path_buf())
}

fn external(tool: &'static str) -> Invocation {
    let mut inv = Invocation::new(tool);
    inv.external = true;
    inv
}

/// A path given as a list item (CheckM2's `--input a b c`): `./` in front of one starting with `-`.
fn list_item(value: &str) -> String {
    if value.starts_with('-') { format!("./{value}") } else { value.to_string() }
}

/// Where a database is: the parameter, else the environment variable the server started with,
/// else the one database of that kind locked in the working folder's `dblock.lock` (a warning
/// then says which, since nothing in the call named it). Returns the text to pass on and the path
/// resolved against the working folder.
fn db_location(ctx: &Ctx, inv: &mut Invocation, field: &str, given: &Option<String>, var: &str) -> Check<(String, PathBuf)> {
    let text = match given {
        Some(g) => {
            v::text(field, g)?;
            g.clone()
        }
        None => match ctx.var(var) {
            Some(x) if !x.is_empty() => x.to_string_lossy().into_owned(),
            _ => match locked_db(&ctx.cwd, |db| db.get("env").and_then(toml::Value::as_str) == Some(var)) {
                Ok(Some((id, path))) => {
                    inv.warnings.push(format!("{field}: {var} is not set, so the database locked in dblock.lock was used: {id} at {path}"));
                    path
                }
                Ok(None) => {
                    return v::invalid(
                        field,
                        format!(
                            "no database given, {var} is not set and dblock.lock here locks none; pass {field} (dblock_env shows the path of a dblock-managed database)"
                        ),
                    );
                }
                Err(e) => return v::invalid(field, format!("no database given and {var} is not set; {e}; pass {field}")),
            },
        },
    };
    let path = ctx.cwd.join(&text);
    Ok((text, path))
}

/// The one database in `<cwd>/dblock.lock` that `pick` accepts: its id and data path. `None`
/// when there is no lock file or nothing matches; an error when several match or the lock is
/// unreadable.
fn locked_db(cwd: &Path, pick: impl Fn(&toml::Table) -> bool) -> Result<Option<(String, String)>, String> {
    let lock = cwd.join("dblock.lock");
    let text = match std::fs::read_to_string(&lock) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("dblock.lock: {e}")),
    };
    let doc: toml::Table = text.parse().map_err(|e| format!("dblock.lock is not valid TOML: {e}"))?;
    let found: Vec<(String, String)> = doc
        .get("database")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
        .filter(|db| pick(db))
        .filter_map(|db| {
            let id = db.get("id")?.as_str()?;
            let path = db.get("data_path").or_else(|| db.get("path"))?.as_str()?;
            Some((id.to_string(), path.to_string()))
        })
        .collect();
    match found.len() {
        0 => Ok(None),
        1 => Ok(found.into_iter().next()),
        _ => Err(format!(
            "dblock.lock locks several matching databases ({})",
            found.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn output_file_opt(
    ctx: &Ctx,
    inv: &mut Invocation,
    field: &str,
    value: &Option<String>,
    overwrite: bool,
    flag: &str,
) -> Check<Option<PathBuf>> {
    let Some(val) = value else { return Ok(None) };
    let p = v::output_file(ctx, field, val, overwrite)?;
    inv.outputs.push(p.clone());
    inv.opt(flag, val);
    Ok(Some(p))
}

// ---------- fastp ----------
// Flags: fastp v1.4.0 src/main.cpp (`cmd.add` calls), https://github.com/OpenGene/fastp/blob/v1.4.0/src/main.cpp

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FastpParams {
    /// Read 1 FASTQ (plain or gzip).
    pub in1: String,
    /// Read 2 FASTQ for paired-end data.
    #[serde(default)]
    pub in2: Option<String>,
    /// Trimmed read 1 output (`.gz` for gzip). Omit to write only the reports. Inside the root.
    #[serde(default)]
    pub out1: Option<String>,
    /// Trimmed read 2 output; needs in2 and out1. Inside the root.
    #[serde(default)]
    pub out2: Option<String>,
    /// JSON report path, inside the root. Name it `<sample>.fastp.json` so isotable can find it.
    pub json: String,
    /// HTML report path, inside the root. Default: the JSON path with `.html`.
    #[serde(default)]
    pub html: Option<String>,
    /// Replace existing output and report files. Default false: they are refused.
    #[serde(default)]
    pub overwrite: bool,
    /// Title in the HTML report.
    #[serde(default)]
    pub report_title: Option<String>,
    /// Worker threads (1-16). fastp's default is 3.
    #[serde(default)]
    pub thread: Option<u32>,
    /// gzip level for `.gz` outputs, 1-9. fastp's default is 4.
    #[serde(default)]
    pub compression: Option<u32>,
    /// Input uses phred64 qualities (output is converted to phred33).
    #[serde(default)]
    pub phred64: bool,
    /// Paired-end only: also detect the adapter by overlap analysis (slower, slightly cleaner).
    #[serde(default)]
    pub detect_adapter_for_pe: bool,
    /// Read 1 adapter (A/C/G/T/N). Default: auto-detected.
    #[serde(default)]
    pub adapter_sequence: Option<String>,
    /// Read 2 adapter, paired-end only. Default: same as adapter_sequence.
    #[serde(default)]
    pub adapter_sequence_r2: Option<String>,
    /// Turn adapter trimming off.
    #[serde(default)]
    pub disable_adapter_trimming: bool,
    /// Bases to trim from the front of read 1.
    #[serde(default)]
    pub trim_front1: Option<u32>,
    /// Bases to trim from the tail of read 1.
    #[serde(default)]
    pub trim_tail1: Option<u32>,
    /// Force polyG tail trimming (on by default for NextSeq/NovaSeq data).
    #[serde(default)]
    pub trim_poly_g: bool,
    /// Turn polyG tail trimming off.
    #[serde(default)]
    pub disable_trim_poly_g: bool,
    /// Sliding window from 5' to 3', dropping low-quality bases at the front.
    #[serde(default)]
    pub cut_front: bool,
    /// Sliding window from 3' to 5', dropping low-quality bases at the tail.
    #[serde(default)]
    pub cut_tail: bool,
    /// Sliding window from front to tail; at the first low-quality window, drop it and everything after.
    #[serde(default)]
    pub cut_right: bool,
    /// Window size for the cut options, 1-1000. Default 4.
    #[serde(default)]
    pub cut_window_size: Option<u32>,
    /// Mean quality for the cut options, 1-36. Default 20.
    #[serde(default)]
    pub cut_mean_quality: Option<u32>,
    /// A base is qualified at this phred quality or above. Default 15.
    #[serde(default)]
    pub qualified_quality_phred: Option<u32>,
    /// Percent of unqualified bases allowed, 0-100. Default 40.
    #[serde(default)]
    pub unqualified_percent_limit: Option<u32>,
    /// Discard a read with more N bases than this. Default 5.
    #[serde(default)]
    pub n_base_limit: Option<u32>,
    /// Discard a read with mean quality below this. Default 0 (off).
    #[serde(default)]
    pub average_qual: Option<u32>,
    /// Discard reads shorter than this after trimming. Default 15.
    #[serde(default)]
    pub length_required: Option<u32>,
    /// Paired-end only: correct mismatched bases in overlapping regions.
    #[serde(default)]
    pub correction: bool,
    /// Drop duplicated reads/pairs.
    #[serde(default)]
    pub dedup: bool,
    /// Report overrepresented sequences.
    #[serde(default)]
    pub overrepresentation_analysis: bool,
}

fn adapter(field: &str, s: &str) -> Check<()> {
    if s.is_empty() || !s.chars().all(|c| "ACGTNacgtn".contains(c)) {
        return v::invalid(field, format!("{s:?} is not an adapter sequence (A, C, G, T, N)"));
    }
    Ok(())
}

/// The HTML report path: as given, else the JSON path with `.html`.
pub fn fastp_html(p: &FastpParams) -> String {
    p.html.clone().unwrap_or_else(|| match p.json.strip_suffix(".json") {
        Some(stem) => format!("{stem}.html"),
        None => format!("{}.html", p.json),
    })
}

pub fn fastp(ctx: &Ctx, p: &FastpParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("fastp");
    // Seen in the real-tools workflow: the HTML report differs between identical runs; the JSON doesn't.
    inv.volatile = &[".html"];
    let in1 = v::existing_file(cwd, "in1", &p.in1)?;
    v::sequence_file(&in1, "in1", &p.in1, Seq::Fastq)?;
    inv.inputs.push(in1);
    inv.opt("in1", &p.in1);
    let paired = p.in2.is_some();
    if let Some(in2) = &p.in2 {
        let path = v::existing_file(cwd, "in2", in2)?;
        if path == inv.inputs[0] {
            return v::invalid("in2", "in2 is the same file as in1");
        }
        v::sequence_file(&path, "in2", in2, Seq::Fastq)?;
        inv.inputs.push(path);
        inv.opt("in2", in2);
    }
    if p.out2.is_some() && (!paired || p.out1.is_none()) {
        return v::invalid("out2", "out2 needs in2 and out1");
    }
    if paired && p.out1.is_some() && p.out2.is_none() {
        return v::invalid("out2", "paired-end input with out1 needs out2 as well");
    }
    for (field, on) in [("detect_adapter_for_pe", p.detect_adapter_for_pe), ("correction", p.correction)] {
        if on && !paired {
            return v::invalid(field, "only for paired-end input (give in2)");
        }
    }
    if p.adapter_sequence_r2.is_some() && !paired {
        return v::invalid("adapter_sequence_r2", "only for paired-end input (give in2)");
    }
    if p.trim_poly_g && p.disable_trim_poly_g {
        return v::invalid("trim_poly_g", "trim_poly_g and disable_trim_poly_g contradict each other");
    }
    let o1 = output_file_opt(ctx, &mut inv, "out1", &p.out1, p.overwrite, "out1")?;
    let o2 = output_file_opt(ctx, &mut inv, "out2", &p.out2, p.overwrite, "out2")?;
    let json = v::output_file(ctx, "json", &p.json, p.overwrite)?;
    inv.outputs.push(json.clone());
    inv.opt("json", &p.json);
    let html_text = fastp_html(p);
    let html = v::output_file(ctx, "html", &html_text, p.overwrite)?;
    inv.outputs.push(html.clone());
    inv.opt("html", &html_text);
    let outs: Vec<(&str, &PathBuf)> = [("out1", o1.as_ref()), ("out2", o2.as_ref()), ("json", Some(&json)), ("html", Some(&html))]
        .into_iter()
        .filter_map(|(f, p)| Some((f, p?)))
        .collect();
    for (i, (field, a)) in outs.iter().enumerate() {
        if outs[..i].iter().any(|(_, b)| b == a) || inv.inputs.contains(a) {
            return v::invalid(field, format!("{} is used twice; give every input and output its own path", a.display()));
        }
    }
    inv.expect = vec![vec![json], vec![html]];
    inv.expect.extend([o1, o2].into_iter().flatten().map(|o| vec![o]));

    if let Some(t) = &p.report_title {
        v::text("report_title", t)?;
        inv.opt("report_title", t);
    }
    let ranged = [
        ("thread", p.thread, 1, 16),
        ("compression", p.compression, 1, 9),
        ("trim_front1", p.trim_front1, 0, 10_000),
        ("trim_tail1", p.trim_tail1, 0, 10_000),
        ("cut_window_size", p.cut_window_size, 1, 1000),
        ("cut_mean_quality", p.cut_mean_quality, 1, 36),
        ("qualified_quality_phred", p.qualified_quality_phred, 0, 93),
        ("unqualified_percent_limit", p.unqualified_percent_limit, 0, 100),
        ("n_base_limit", p.n_base_limit, 0, 10_000),
        ("average_qual", p.average_qual, 0, 93),
        ("length_required", p.length_required, 0, 1_000_000),
    ];
    for (name, val, lo, hi) in ranged {
        if let Some(x) = val {
            v::range(name, x, lo, hi)?;
            inv.opt(name, x);
        }
    }
    for (name, seq) in [("adapter_sequence", &p.adapter_sequence), ("adapter_sequence_r2", &p.adapter_sequence_r2)] {
        if let Some(s) = seq {
            adapter(name, s)?;
            inv.opt(name, s);
        }
    }
    for (name, on) in [
        ("phred64", p.phred64),
        ("detect_adapter_for_pe", p.detect_adapter_for_pe),
        ("disable_adapter_trimming", p.disable_adapter_trimming),
        ("trim_poly_g", p.trim_poly_g),
        ("disable_trim_poly_g", p.disable_trim_poly_g),
        ("cut_front", p.cut_front),
        ("cut_tail", p.cut_tail),
        ("cut_right", p.cut_right),
        ("correction", p.correction),
        ("dedup", p.dedup),
        ("overrepresentation_analysis", p.overrepresentation_analysis),
    ] {
        inv.flag(name, on);
    }
    Ok(inv)
}

// ---------- Shovill ----------
// Flags: Shovill v1.4.2 bin/shovill (`setOptions`, and the checks after it),
// https://github.com/tseemann/shovill/blob/v1.4.2/bin/shovill; same list as `shovill --help` of the
// bioconda shovill=1.4.2 package. Bioconda's current Shovill is 1.4.2, not 1.1.0: 1.4.x added
// --plasmid and changed --mincov's default to 2.
// Not offered: --opts (free text pasted into the assembler's shell command), --tmpdir (Shovill makes
// its own temporary folder), --check, --version, --help.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ShovillAssembler {
    /// SPAdes (Shovill's default; needs SPAdes 3.14-3.x).
    Spades,
    /// SKESA: fast, conservative contigs.
    Skesa,
    Megahit,
    Velvet,
}

impl ShovillAssembler {
    fn name(self) -> &'static str {
        match self {
            ShovillAssembler::Spades => "spades",
            ShovillAssembler::Skesa => "skesa",
            ShovillAssembler::Megahit => "megahit",
            ShovillAssembler::Velvet => "velvet",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShovillParams {
    /// Read 1 FASTQ (plain or gzip), paired-end Illumina.
    pub r1: String,
    /// Read 2 FASTQ (plain or gzip).
    pub r2: String,
    /// Output folder, inside the root. The assembly is `<outdir>/contigs.fa`; use one folder per sample.
    pub outdir: String,
    /// Allow an outdir that already has files (Shovill replaces files of the same name). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Also make this path a link to `<outdir>/contigs.fa`, named after the sample, e.g. `genomes/s1.fna`. CheckM2, GTDB-Tk and gtdbprep_ani name genomes after the file, so give them the folder of these links. Recorded and recreated by replay.sh. An existing file there is only replaced with overwrite.
    #[serde(default)]
    pub link_as: Option<String>,
    /// Assembler. Shovill's default is spades.
    #[serde(default)]
    pub assembler: Option<ShovillAssembler>,
    /// Estimated genome size, e.g. 2.8M or 5000000. Default: estimated from the reads with KMC.
    #[serde(default)]
    pub gsize: Option<String>,
    /// Sub-sample the reads to this depth; 0 turns sub-sampling off. Default 150.
    #[serde(default)]
    pub depth: Option<u32>,
    /// Drop contigs shorter than this; 0 = Shovill picks. Default 0.
    #[serde(default)]
    pub minlen: Option<u32>,
    /// Drop contigs with lower coverage than this; 0 = Shovill picks. Default 2.
    #[serde(default)]
    pub mincov: Option<f64>,
    /// Contig name format with one counter, e.g. `s1_%05d`. Default `contig%05d`.
    #[serde(default)]
    pub namefmt: Option<String>,
    /// Keep intermediate files (reads, BAM, assembler folder).
    #[serde(default)]
    pub keepfiles: bool,
    /// CPUs. Shovill's default is up to 8.
    #[serde(default)]
    pub cpus: Option<u32>,
    /// RAM limit in GB, at least 8 (Shovill 1.4.2 refuses less) and no more than the machine has. Default: up to 16.
    #[serde(default)]
    pub ram: Option<f64>,
    /// SPAdes plasmid mode (assembler spades only).
    #[serde(default)]
    pub plasmid: bool,
    /// K-mers to use, comma-separated, e.g. 31,55,77. Default: chosen from the read length.
    #[serde(default)]
    pub kmers: Option<String>,
    /// Trim adapters with Trimmomatic first (off by default; use fastp beforehand instead).
    #[serde(default)]
    pub trim: bool,
    /// Skip read error correction (Lighter).
    #[serde(default)]
    pub noreadcorr: bool,
    /// Skip read stitching (FLASH).
    #[serde(default)]
    pub nostitch: bool,
    /// Skip post-assembly correction (Pilon). Faster.
    #[serde(default)]
    pub nocorr: bool,
}

/// Shovill 1.4.2's `$MIN_RAM_GB`.
const SHOVILL_MIN_RAM_GB: f64 = 8.0;

/// Shovill pastes the absolute output folder unquoted into a shell command (`seqkit stats
/// $outdir/contigs.fa`), so the folder's path may only hold characters a shell leaves alone.
fn shell_safe_path(field: &str, shown: &str, p: &Path) -> Check<()> {
    let s = v::resolve(p).to_string_lossy().into_owned();
    if let Some(c) = s.chars().find(|c| !(c.is_ascii_alphanumeric() || "/._-+,=@%".contains(*c))) {
        return v::invalid(
            field,
            format!("{shown} resolves to {s}, which has {c:?}; Shovill passes this path to a shell unquoted, so use letters, digits and / . _ - + , = @ % only"),
        );
    }
    Ok(())
}

/// `to` as seen from the folder `from` (both absolute and resolved): `../asm/s1/contigs.fa`. A
/// relative link keeps working when the project folder is moved or copied.
fn relative_to(from: &Path, to: &Path) -> PathBuf {
    let (a, b): (Vec<_>, Vec<_>) = (from.components().collect(), to.components().collect());
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out: PathBuf = std::iter::repeat_n("..", a.len() - common).collect();
    out.extend(&b[common..]);
    out
}

pub fn shovill(ctx: &Ctx, p: &ShovillParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("shovill");
    // Nothing Shovill writes is byte-for-byte reproducible, so every output is volatile (the
    // empty ending matches all): contigs.fa headers carry `date=YYYYMMDD`; SPAdes gave different
    // contigs in two identical runs (real-tools workflow, run 43194785664), and so did SKESA 2.5.1
    // with 8 CPUs (issue #5); shovill.log has times and the machine's CPUs and RAM. The contig
    // count, length and N50 are in the tool's result and isotable's row instead.
    inv.volatile = &[""];
    let r1 = v::existing_file(cwd, "r1", &p.r1)?;
    v::sequence_file(&r1, "r1", &p.r1, Seq::Fastq)?;
    let r2 = v::existing_file(cwd, "r2", &p.r2)?;
    if r2 == r1 {
        return v::invalid("r2", "r2 is the same file as r1");
    }
    v::sequence_file(&r2, "r2", &p.r2, Seq::Fastq)?;
    inv.inputs.extend([r1, r2]);
    inv.opt("R1", &p.r1);
    inv.opt("R2", &p.r2);
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    shell_safe_path("outdir", &p.outdir, &outdir)?;
    inv.outputs.push(outdir.clone());
    inv.opt("outdir", &p.outdir);
    // Shovill refuses any existing folder, even an empty one, without --force. irit-mcp has
    // already refused a non-empty one unless overwrite is true, and replay.sh reruns into it.
    inv.flag("force", true);
    inv.expect = vec![vec![outdir.join("contigs.fa")]];
    inv.assembly = Some(outdir.join("contigs.fa"));
    if let Some(l) = &p.link_as {
        let link = v::output_file(ctx, "link_as", l, p.overwrite)?;
        let contigs = outdir.join("contigs.fa");
        if v::resolve(&link).starts_with(v::resolve(&outdir)) {
            return v::invalid("link_as", format!("{l} is inside outdir; put the links in a folder of their own, e.g. genomes/<sample>.fna"));
        }
        let target = relative_to(&v::resolve(link.parent().unwrap_or(cwd)), &v::resolve(&contigs));
        inv.outputs.push(link.clone());
        inv.links.push((link, target));
    }
    if let Some(a) = p.assembler {
        inv.opt("assembler", a.name());
    }
    if p.plasmid && p.assembler.is_some_and(|a| a != ShovillAssembler::Spades) {
        return v::invalid("plasmid", "plasmid mode needs assembler spades (Shovill 1.4.2 refuses it otherwise)");
    }
    if let Some(g) = &p.gsize {
        // Shovill's parse_genome_size: digits and dots, then an optional G, M or K.
        let num = g.trim_end_matches(['G', 'M', 'K', 'g', 'm', 'k']);
        let ok = !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.') && num.parse::<f64>().is_ok_and(|x| x > 0.0);
        if !ok || g.len() - num.len() > 1 {
            return v::invalid("gsize", format!("{g:?} is not a genome size like 2.8M, 4500K or 5000000"));
        }
        inv.opt("gsize", g);
    }
    if let Some(d) = p.depth {
        v::range("depth", d, 0, 100_000)?;
        inv.opt("depth", d);
    }
    if let Some(m) = p.minlen {
        inv.opt("minlen", m);
    }
    if let Some(c) = p.mincov {
        if !c.is_finite() || c < 0.0 {
            return v::invalid("mincov", "must be 0 or more");
        }
        inv.opt("mincov", c);
    }
    if let Some(f) = &p.namefmt {
        // Shovill wants a `%<digits>d` counter; anything else printf would read is refused too.
        let counters = f.matches('%').count();
        let ok = counters == 1
            && f.split_once('%').is_some_and(|(_, rest)| {
                let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit());
                digits.len() < rest.len() && digits.starts_with('d')
            })
            && f.chars().all(|c| c.is_ascii_alphanumeric() || "_.-%".contains(c));
        if !ok {
            return v::invalid("namefmt", format!("{f:?} needs exactly one counter like %05d, and only letters, digits and _ . -"));
        }
        inv.opt("namefmt", f);
    }
    inv.flag("keepfiles", p.keepfiles);
    if let Some(c) = p.cpus {
        v::range("cpus", c, 1, 1024)?;
        inv.opt("cpus", c);
    }
    if let Some(r) = p.ram {
        if !r.is_finite() || r < SHOVILL_MIN_RAM_GB {
            return v::invalid("ram", format!("{r} GB is below the {SHOVILL_MIN_RAM_GB} GB Shovill 1.4.2 needs"));
        }
        inv.opt("ram", r);
    }
    inv.flag("plasmid", p.plasmid);
    if let Some(k) = &p.kmers {
        let ok = !k.is_empty() && k.split(',').all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()));
        if !ok {
            return v::invalid("kmers", format!("{k:?} is not a comma-separated list of k-mer sizes like 31,55,77"));
        }
        inv.opt("kmers", k);
    }
    for (name, on) in [("trim", p.trim), ("noreadcorr", p.noreadcorr), ("nostitch", p.nostitch), ("nocorr", p.nocorr)] {
        inv.flag(name, on);
    }
    Ok(inv)
}

/// `link_as` for an assembler: a link named after the sample to `assembly`, outside `outdir`.
fn assembly_link(ctx: &Ctx, inv: &mut Invocation, link_as: &Option<String>, outdir: &Path, assembly: &Path, overwrite: bool) -> Check<()> {
    let Some(l) = link_as else { return Ok(()) };
    let link = v::output_file(ctx, "link_as", l, overwrite)?;
    if v::resolve(&link).starts_with(v::resolve(outdir)) {
        return v::invalid("link_as", format!("{l} is inside outdir; put the links in a folder of their own, e.g. genomes/<sample>.fna"));
    }
    let target = relative_to(&v::resolve(link.parent().unwrap_or(ctx.cwd.as_path())), &v::resolve(assembly));
    inv.outputs.push(link.clone());
    inv.links.push((link, target));
    Ok(())
}

/// A genome size the way Flye reads it (`flye/main.py` `_parse_genome_size`): digits with an
/// optional k, m or g, e.g. `5m`, `5.5m`, `2.6g`, `5000000`.
fn flye_genome_size(g: &str) -> Check<()> {
    let num = g.trim_end_matches(['k', 'm', 'g', 'K', 'M', 'G']);
    let ok = !num.is_empty() && g.len() - num.len() <= 1 && num.chars().all(|c| c.is_ascii_digit() || c == '.') && num.parse::<f64>().is_ok_and(|x| x > 0.0);
    if !ok {
        return v::invalid("genome_size", format!("{g:?} is not a genome size like 5m, 5.5m or 5000000"));
    }
    Ok(())
}

// ---------- Flye ----------
// Flags: Flye 2.9.6 flye/main.py (`main`, the argparse block and `_check_input`),
// https://github.com/mikolmogorov/Flye/blob/2.9.6/flye/main.py; same list as `flye --help` of the
// bioconda flye=2.9.6 package.
// Not offered: --extra-params (free text into Flye's configuration), --resume/--resume-from/
// --stop-after (a partial run isn't a result), --polish-target (polishing only, no assembly),
// --subassemblies, --plasmids and --trestle (deprecated or unused), --hifi-error (deprecated alias
// of --read-error), --debug.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum FlyeReads {
    /// ONT R10.4.1 (or R9 Guppy 5+) reads basecalled with hac or sup: under 3 % error. The usual choice today.
    NanoHq,
    /// Older ONT reads (pre-R9 Guppy 5), 10-20 % error.
    NanoRaw,
    /// ONT reads corrected by another tool (under 3 % error).
    NanoCorr,
    /// PacBio HiFi reads (under 1 % error).
    PacbioHifi,
    /// PacBio CLR reads.
    PacbioRaw,
    /// PacBio reads corrected by another tool.
    PacbioCorr,
}

impl FlyeReads {
    fn flag(self) -> &'static str {
        match self {
            FlyeReads::NanoHq => "nano-hq",
            FlyeReads::NanoRaw => "nano-raw",
            FlyeReads::NanoCorr => "nano-corr",
            FlyeReads::PacbioHifi => "pacbio-hifi",
            FlyeReads::PacbioRaw => "pacbio-raw",
            FlyeReads::PacbioCorr => "pacbio-corr",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlyeParams {
    /// Long-read FASTQ or FASTA files (plain or gzip) of one isolate.
    pub reads: Vec<String>,
    /// Read type. Default nano-hq (ONT R10.4.1 hac/sup).
    #[serde(default)]
    pub read_type: Option<FlyeReads>,
    /// Output folder, inside the root. The assembly is `<outdir>/assembly.fasta`, with `assembly_info.txt` (length, coverage and whether each contig is circular) and `assembly_graph.gfa`; use one folder per sample.
    pub outdir: String,
    /// Allow an outdir that already has files. Flye then continues from what is there rather than starting clean, so prefer an empty folder. Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Also make this path a link to `<outdir>/assembly.fasta`, named after the sample, e.g. `genomes/s1.fna`, for CheckM2, GTDB-Tk and gtdbprep_ani. Recorded and recreated by replay.sh.
    #[serde(default)]
    pub link_as: Option<String>,
    /// Estimated genome size, e.g. 5.5m. Needed for asm_coverage. Default: Flye estimates it.
    #[serde(default)]
    pub genome_size: Option<String>,
    /// Use only the longest reads up to this coverage for the first (disjointig) stage, which caps memory on deep data; all reads are still used later. Needs genome_size; not with meta. E.g. 50.
    #[serde(default)]
    pub asm_coverage: Option<u32>,
    /// Threads (Flye's default is 1).
    #[serde(default)]
    pub threads: Option<u32>,
    /// Polishing iterations Flye runs itself (default 1).
    #[serde(default)]
    pub iterations: Option<u32>,
    /// Minimum read overlap (default: chosen from the reads).
    #[serde(default)]
    pub min_overlap: Option<u32>,
    /// Expected read error rate as a fraction, e.g. 0.03 (default: set by read_type).
    #[serde(default)]
    pub read_error: Option<f64>,
    /// Metagenome / uneven coverage mode.
    #[serde(default)]
    pub meta: bool,
    /// Don't collapse alternative haplotypes.
    #[serde(default)]
    pub keep_haplotypes: bool,
    /// Don't output contigs for alternative haplotypes.
    #[serde(default)]
    pub no_alt_contigs: bool,
    /// Scaffold contigs using the assembly graph (off by default).
    #[serde(default)]
    pub scaffold: bool,
    /// Build disjointigs single-threaded, so identical runs give identical assemblies (slower).
    #[serde(default)]
    pub deterministic: bool,
}

pub fn flye(ctx: &Ctx, p: &FlyeParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("flye");
    // Multi-threaded disjointig assembly isn't reproducible (Flye's own --deterministic exists for
    // that), and flye.log has times, so every output is volatile; the assembly's contig count,
    // length and N50 are recorded instead.
    inv.volatile = &[""];
    v::non_empty_list("reads", &p.reads)?;
    let read_type = p.read_type.unwrap_or(FlyeReads::NanoHq);
    let mut reads = Vec::new();
    for r in &p.reads {
        let path = v::existing_file(cwd, "reads", r)?;
        if reads.contains(&path) {
            return v::invalid("reads", format!("{r} is listed twice"));
        }
        if v::sequence_file(&path, "reads", r, Seq::Fastq).is_err() {
            v::sequence_file(&path, "reads", r, Seq::Fasta).map_err(|e| v::Invalid { message: format!("{r} is neither FASTQ nor FASTA ({})", e.message), ..e })?;
        }
        reads.push(path);
    }
    inv.inputs.extend(reads);
    // nargs="+": the files follow the flag as separate words; `./` keeps one starting with `-`
    // from being read as an option.
    inv.args.push(format!("--{}", read_type.flag()));
    inv.args.extend(p.reads.iter().map(|r| list_item(r)));
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    inv.outputs.push(outdir.clone());
    inv.opt("out-dir", &p.outdir);
    let assembly = outdir.join("assembly.fasta");
    inv.expect = vec![vec![assembly.clone()]];
    inv.assembly = Some(assembly.clone());
    assembly_link(ctx, &mut inv, &p.link_as, &outdir, &assembly, p.overwrite)?;
    if let Some(g) = &p.genome_size {
        flye_genome_size(g)?;
        inv.opt("genome-size", g);
    }
    if let Some(c) = p.asm_coverage {
        if p.genome_size.is_none() {
            return v::invalid("asm_coverage", "needs genome_size (Flye refuses --asm-coverage without --genome-size)");
        }
        if p.meta {
            return v::invalid("asm_coverage", "can't be combined with meta (Flye refuses --asm-coverage with --meta); leave it out");
        }
        v::range("asm_coverage", c, 1, 10_000)?;
        inv.opt("asm-coverage", c);
    }
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    if let Some(i) = p.iterations {
        v::range("iterations", i, 0, 10)?;
        inv.opt("iterations", i);
    }
    if let Some(m) = p.min_overlap {
        v::range("min_overlap", m, 1000, 100_000)?;
        inv.opt("min-overlap", m);
    }
    if let Some(e) = p.read_error {
        if !e.is_finite() || e <= 0.0 || e >= 1.0 {
            return v::invalid("read_error", "a fraction between 0 and 1, e.g. 0.03");
        }
        inv.opt("read-error", e);
    }
    if p.keep_haplotypes && p.no_alt_contigs {
        return v::invalid("no_alt_contigs", "keep_haplotypes and no_alt_contigs contradict each other");
    }
    for (name, on) in [
        ("meta", p.meta),
        ("keep-haplotypes", p.keep_haplotypes),
        ("no-alt-contigs", p.no_alt_contigs),
        ("scaffold", p.scaffold),
        ("deterministic", p.deterministic),
    ] {
        inv.flag(name, on);
    }
    Ok(inv)
}

// ---------- Medaka ----------
// Flags: medaka 2.2.2 scripts/medaka_consensus (the `getopts` string and usage text),
// https://github.com/nanoporetech/medaka/blob/v2.2.2/scripts/medaka_consensus. It is a bash script
// with short options only (getopts), so values are separate words, not `--name=value`; paths
// starting with `-` get `./` in front.
// Not offered: -f (it reuses or replaces old intermediate files; irit-mcp refuses a non-empty
// outdir instead), -x, -B (regions), -M, -p (the prefix would hide the file isotable reads), -g/-r
// (gap filling stays the default: draft sequence), -q.

/// A medaka model name or `--bacteria`: letters, digits and `_ . : -` only (no paths; models are
/// resolved by medaka itself).
fn medaka_model(m: &str) -> Check<()> {
    if m.is_empty() || m.starts_with('-') || !m.chars().all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c)) {
        return v::invalid("model", format!("{m:?} is not a medaka model name like r1041_e82_400bps_bacterial_methylation"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MedakaParams {
    /// The long reads the draft was assembled from (FASTQ or FASTA, plain or gzip).
    pub reads: String,
    /// Draft assembly FASTA to polish, e.g. Flye's `<outdir>/assembly.fasta`.
    pub draft: String,
    /// Output folder, inside the root; must be empty. The polished assembly is `<outdir>/consensus.fasta`.
    pub outdir: String,
    /// Also make this path a link to `<outdir>/consensus.fasta`, named after the sample, e.g. `genomes/s1.fna`. Recorded and recreated by replay.sh. An existing file there is only replaced with overwrite.
    #[serde(default)]
    pub link_as: Option<String>,
    /// Replace an existing link_as file. (outdir must still be empty: medaka would reuse old intermediate files.)
    #[serde(default)]
    pub overwrite: bool,
    /// Medaka model, e.g. r1041_e82_400bps_bacterial_methylation (`medaka tools list_models` lists them). Default: chosen by medaka from the read headers, which fails for reads whose headers lost the basecaller's model (e.g. many archive downloads); then name it here.
    #[serde(default)]
    pub model: Option<String>,
    /// Pick medaka's bacterial (methylation-aware) model automatically (`--bacteria`). Needs read headers that name the basecalling model; otherwise give `model`.
    #[serde(default)]
    pub bacteria: bool,
    /// Threads for feature creation (medaka's default is 1). medaka recommends no more than 2 for its inference step; this sets the alignment and feature threads.
    #[serde(default)]
    pub threads: Option<u32>,
    /// Batch size, which controls memory use (medaka's default 100: 7.4 GiB peak on a 6 Mb genome). 50 (4.8 GiB) or 25 (3.7 GiB) gave the same consensus, no slower; use one of them on 8 GB machines.
    #[serde(default)]
    pub batch_size: Option<u32>,
}

/// The model file Medaka 2.2.2 loaded, from its log line `[hh:mm:ss - Predict] Using model:
/// <path>.` A named model is downloaded at run time into medaka's package `data/` folder (or
/// `~/.medaka/data`) when it isn't there, so the record hashes the file it used (issue #21).
pub fn medaka_model_file(stderr: &str) -> Option<PathBuf> {
    stderr.lines().find_map(|l| l.split_once("Using model: ")).map(|(_, p)| PathBuf::from(p.trim().trim_end_matches('.')))
}

/// A Medaka model file's name (`<model>_model_pt.tar.gz`, or an older `<model>_model.tar.gz` or `.hdf5`).
pub fn is_medaka_model(path: &str) -> bool {
    path.ends_with("_model_pt.tar.gz") || path.ends_with("_model.tar.gz") || path.ends_with(".hdf5")
}

pub fn medaka(ctx: &Ctx, p: &MedakaParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("medaka_consensus");
    // The consensus is computed on a GPU or CPU in batches; its bytes are not promised identical
    // between runs, and the folder holds logs and a BAM, so every output is volatile.
    inv.volatile = &[""];
    let reads = v::existing_file(cwd, "reads", &p.reads)?;
    if v::sequence_file(&reads, "reads", &p.reads, Seq::Fastq).is_err() {
        v::sequence_file(&reads, "reads", &p.reads, Seq::Fasta)
            .map_err(|e| v::Invalid { message: format!("{} is neither FASTQ nor FASTA ({})", p.reads, e.message), ..e })?;
    }
    let draft = v::existing_file(cwd, "draft", &p.draft)?;
    v::sequence_file(&draft, "draft", &p.draft, Seq::Fasta)?;
    if draft == reads {
        return v::invalid("draft", "draft is the same file as reads");
    }
    inv.inputs.extend([reads, draft]);
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(false), true)?;
    inv.outputs.push(outdir.clone());
    for (flag, value) in [("-i", &p.reads), ("-d", &p.draft), ("-o", &p.outdir)] {
        inv.args.push(flag.into());
        inv.args.push(list_item(value));
    }
    let consensus = outdir.join("consensus.fasta");
    inv.expect = vec![vec![consensus.clone()]];
    inv.assembly = Some(consensus.clone());
    assembly_link(ctx, &mut inv, &p.link_as, &outdir, &consensus, p.overwrite)?;
    if let Some(m) = &p.model {
        medaka_model(m)?;
        inv.args.extend(["-m".into(), m.clone()]);
    }
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.args.extend(["-t".into(), t.to_string()]);
    }
    if let Some(b) = p.batch_size {
        v::range("batch_size", b, 1, 10_000)?;
        inv.args.extend(["-b".into(), b.to_string()]);
    }
    if p.bacteria {
        inv.args.push("--bacteria".into());
    }
    Ok(inv)
}

// ---------- Bakta ----------
// Flags: Bakta v1.12.2 bakta/utils.py `parse_arguments`; database check: bakta/db.py `check`
// (https://github.com/oschwengers/bakta/blob/v1.12.2/bakta/utils.py, .../bakta/db.py).

/// Bakta 1.12.2's `__db_schema_version__`: it refuses any other major database version.
pub const BAKTA_DB_SCHEMA: u64 = 6;

/// `bakta/db.py` FILE_NAMES at v1.12.2; plus `psc.dmnd` (full) or `pscc.dmnd` (light).
const BAKTA_DB_FILES: [&str; 26] = [
    "antifam.h3f",
    "antifam.h3i",
    "antifam.h3m",
    "antifam.h3p",
    "bakta.db",
    "expert-protein-sequences.dmnd",
    "ncRNA-genes.i1f",
    "ncRNA-genes.i1i",
    "ncRNA-genes.i1m",
    "ncRNA-genes.i1p",
    "ncRNA-regions.i1f",
    "ncRNA-regions.i1i",
    "ncRNA-regions.i1m",
    "ncRNA-regions.i1p",
    "oric.fna",
    "orit.fna",
    "pfam.h3f",
    "pfam.h3i",
    "pfam.h3m",
    "pfam.h3p",
    "rfam-go.tsv",
    "rRNA.i1f",
    "rRNA.i1i",
    "rRNA.i1m",
    "rRNA.i1p",
    "sorf.dmnd",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Gram {
    /// `--gram +`
    Positive,
    /// `--gram -`
    Negative,
    /// `--gram ?` (Bakta's default)
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BaktaSkip {
    Trna,
    Tmrna,
    Rrna,
    Ncrna,
    NcrnaRegion,
    Crispr,
    Cds,
    Pseudo,
    Sorf,
    Gap,
    Ori,
    Filter,
    Plot,
}

impl BaktaSkip {
    fn flag(self) -> &'static str {
        match self {
            BaktaSkip::Trna => "skip-trna",
            BaktaSkip::Tmrna => "skip-tmrna",
            BaktaSkip::Rrna => "skip-rrna",
            BaktaSkip::Ncrna => "skip-ncrna",
            BaktaSkip::NcrnaRegion => "skip-ncrna-region",
            BaktaSkip::Crispr => "skip-crispr",
            BaktaSkip::Cds => "skip-cds",
            BaktaSkip::Pseudo => "skip-pseudo",
            BaktaSkip::Sorf => "skip-sorf",
            BaktaSkip::Gap => "skip-gap",
            BaktaSkip::Ori => "skip-ori",
            BaktaSkip::Filter => "skip-filter",
            BaktaSkip::Plot => "skip-plot",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BaktaParams {
    /// Genome assembly, FASTA (plain or gzip).
    pub genome: String,
    /// Bakta database folder (the one holding version.json, schema v6 for Bakta 1.12). Default: $BAKTA_DB as the server started with, else the one locked in ./dblock.lock. dblock_env shows the path of a dblock-managed one (bakta-db-v6-full or -light).
    #[serde(default)]
    pub db: Option<String>,
    /// Output folder, inside the root.
    pub outdir: String,
    /// Allow an outdir that already has files (Bakta replaces files of the same name). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Prefix of the output files. Default: the genome file name without its last extension (as Bakta).
    #[serde(default)]
    pub prefix: Option<String>,
    /// Drop contigs shorter than this. Bakta's default 1 (200 with compliant).
    #[serde(default)]
    pub min_contig_length: Option<u32>,
    #[serde(default)]
    pub genus: Option<String>,
    #[serde(default)]
    pub species: Option<String>,
    #[serde(default)]
    pub strain: Option<String>,
    /// Plasmid name.
    #[serde(default)]
    pub plasmid: Option<String>,
    /// All sequences are complete replicons (chromosome/plasmids).
    #[serde(default)]
    pub complete: bool,
    /// Translation table: 11 (default), 4 or 25.
    #[serde(default)]
    pub translation_table: Option<u8>,
    /// Gram type for signal peptide prediction.
    #[serde(default)]
    pub gram: Option<Gram>,
    /// Locus (sequence) name prefix. Default `contig`.
    #[serde(default)]
    pub locus: Option<String>,
    /// Locus tag prefix. Default: generated.
    #[serde(default)]
    pub locus_tag: Option<String>,
    /// Keep the original contig headers.
    #[serde(default)]
    pub keep_contig_headers: bool,
    /// Force GenBank/ENA/DDBJ compliance.
    #[serde(default)]
    pub compliant: bool,
    /// Metagenome mode (affects CDS prediction only).
    #[serde(default)]
    pub meta: bool,
    /// Threads. Default: all available CPUs.
    #[serde(default)]
    pub threads: Option<u32>,
    /// Steps to skip, e.g. ["plot", "crispr"].
    #[serde(default)]
    pub skip: Vec<BaktaSkip>,
}

/// Checks a Bakta database folder the way Bakta 1.12.2's `db.check` does, before hours are spent.
fn bakta_db(field: &str, shown: &str, dir: &Path) -> Check<PathBuf> {
    if !dir.is_dir() {
        return v::invalid(field, format!("{shown} is not a folder"));
    }
    let version = dir.join("version.json");
    let info: serde_json::Value = match std::fs::read(&version).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
        Some(j) => j,
        None => {
            return v::invalid(
                field,
                format!(
                    "{shown} has no readable version.json, so it is not a Bakta database folder (maybe the database is in a db/ or db-light/ folder inside it)"
                ),
            );
        }
    };
    for key in ["date", "major", "minor", "type"] {
        if info.get(key).is_none() {
            return v::invalid(field, format!("{shown}/version.json has no {key:?}; not a Bakta database"));
        }
    }
    let major = info["major"].as_u64().unwrap_or(0);
    if major != BAKTA_DB_SCHEMA {
        return v::invalid(
            field,
            format!(
                "{shown} is Bakta database v{major}.{}; Bakta 1.12 needs v{BAKTA_DB_SCHEMA}.x (dblock: bakta-db-v6-full or bakta-db-v6-light)",
                info["minor"]
            ),
        );
    }
    let protein = if info["type"] == "full" { "psc.dmnd" } else { "pscc.dmnd" };
    let missing: Vec<&str> = BAKTA_DB_FILES.iter().copied().chain([protein]).filter(|f| !dir.join(f).is_file()).collect();
    if !missing.is_empty() {
        return v::invalid(field, format!("{shown} is missing {} (incomplete download?)", missing.join(", ")));
    }
    Ok(version)
}

pub fn bakta(ctx: &Ctx, p: &BaktaParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("bakta");
    // Seen in the real-tools workflow: the log, GenBank/EMBL (LOCUS date), the JSON (run start and
    // end only) and the SVG plot differ between identical runs. The .txt, GFF3, TSV and FASTA
    // outputs don't, and are hash-checked.
    inv.volatile = &[".log", ".gbff", ".embl", ".json", ".svg"];
    let genome = v::existing_file(cwd, "genome", &p.genome)?;
    v::sequence_file(&genome, "genome", &p.genome, Seq::Fasta)?;
    inv.inputs.push(genome);
    let (db_text, db) = db_location(ctx, &mut inv, "db", &p.db, "BAKTA_DB")?;
    // version.json only: the database itself is tens of GB (dblock_verify checks it).
    inv.inputs.push(bakta_db("db", &db_text, &db)?);
    inv.opt("db", &db_text);
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    inv.outputs.push(outdir.clone());
    inv.opt("output", &p.outdir);
    // Bakta refuses any existing output folder without --force; irit-mcp has already refused a
    // non-empty one unless overwrite is true, and replay.sh reruns into the same folder.
    inv.flag("force", true);
    let prefix = match &p.prefix {
        Some(x) => x.clone(),
        // Python's Path(genome).stem
        None => Path::new(&p.genome).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    v::file_stem("prefix", &prefix)?;
    inv.opt("prefix", &prefix);
    inv.expect = ["txt", "json", "tsv", "gff3", "gbff"].iter().map(|ext| vec![outdir.join(format!("{prefix}.{ext}"))]).collect();
    if let Some(m) = p.min_contig_length {
        inv.opt("min-contig-length", m);
    }
    for (name, val) in [("genus", &p.genus), ("species", &p.species), ("strain", &p.strain), ("plasmid", &p.plasmid)] {
        if let Some(x) = val {
            v::text(name, x)?;
            inv.opt(name, x);
        }
    }
    inv.flag("complete", p.complete);
    if let Some(t) = p.translation_table {
        if ![11, 4, 25].contains(&t) {
            return v::invalid("translation_table", format!("{t} is not 11, 4 or 25"));
        }
        inv.opt("translation-table", t);
    }
    if let Some(g) = p.gram {
        inv.opt(
            "gram",
            match g {
                Gram::Positive => "+",
                Gram::Negative => "-",
                Gram::Unknown => "?",
            },
        );
    }
    // Bakta's own rules (bakta/config.py): locus 1-20 of alphanumerics and _.:*#-; locus tag 1-24
    // of alphanumerics and _.-.
    if let Some(l) = &p.locus {
        let ok = (1..=20).contains(&l.len()) && l.chars().all(|c| c.is_ascii_alphanumeric() || "_.:*#-".contains(c));
        if !ok {
            return v::invalid("locus", "1-20 letters, digits or _ . : * # -");
        }
        inv.opt("locus", l);
    }
    if let Some(l) = &p.locus_tag {
        let ok = (1..=24).contains(&l.len()) && l.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
        if !ok {
            return v::invalid("locus_tag", "1-24 letters, digits or _ . -");
        }
        inv.opt("locus-tag", l);
    }
    inv.flag("keep-contig-headers", p.keep_contig_headers);
    inv.flag("compliant", p.compliant);
    inv.flag("meta", p.meta);
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    for (i, s) in p.skip.iter().enumerate() {
        inv.flag(s.flag(), !p.skip[..i].contains(s));
    }
    inv.positionals(std::slice::from_ref(&p.genome));
    Ok(inv)
}

// ---------- CheckM2 ----------
// Flags: CheckM2 1.1.0 checkm2/main.py (`predict` subparser and the shared parent parser),
// https://github.com/chklovski/CheckM2/blob/1.1.0/checkm2/main.py

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Checkm2Model {
    /// CheckM2 chooses per genome (default).
    Auto,
    /// `--general`: gradient boost model.
    General,
    /// `--specific`: neural network model.
    Specific,
    /// `--allmodels`: report both.
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Checkm2Params {
    /// Genome FASTA files (plain or gzip). Give this or genome_dir.
    #[serde(default)]
    pub genomes: Vec<String>,
    /// Folder of genome FASTA files, matched by `extension`. Give this or genomes.
    #[serde(default)]
    pub genome_dir: Option<String>,
    /// With genome_dir: file extension to match. CheckM2's default is .fna.
    #[serde(default)]
    pub extension: Option<String>,
    /// Output folder, inside the root. quality_report.tsv goes here.
    pub outdir: String,
    /// Allow a non-empty outdir. CheckM2 (--force) then DELETES everything in it first. Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// CheckM2 DIAMOND database: the .dmnd file, or a folder holding exactly one. Default: $CHECKM2DB as the server started with, else the one locked in ./dblock.lock (dblock_env shows the path for checkm2-db-v3).
    #[serde(default)]
    pub db: Option<String>,
    /// Threads.
    #[serde(default)]
    pub threads: Option<u32>,
    /// Which quality model to use. Default auto.
    #[serde(default)]
    pub model: Option<Checkm2Model>,
    /// Low memory mode: smaller DIAMOND blocks, less RAM, slower. Use on 8-16 GB laptops.
    #[serde(default)]
    pub lowmem: bool,
    /// Prodigal translation table. Default: CheckM2 picks 11 or 4.
    #[serde(default)]
    pub ttable: Option<u8>,
    /// Inputs are protein FASTA files, not nucleotide.
    #[serde(default)]
    pub genes: bool,
    /// Delete intermediate files (protein files, DIAMOND output) at the end.
    #[serde(default)]
    pub remove_intermediates: bool,
}

/// CheckM2 1.1.0's `allowable_ttables` in main.py.
const CHECKM2_TTABLES: [u8; 25] = [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14, 16, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 33];

/// The .dmnd file: given directly, or the only one under a folder (as dblock's `glob:*.dmnd`).
fn checkm2_db(field: &str, shown: &str, path: &Path) -> Check<PathBuf> {
    if path.is_file() {
        if path.extension().is_some_and(|e| e == "dmnd") {
            return Ok(path.to_path_buf());
        }
        return v::invalid(field, format!("{shown} is not a DIAMOND .dmnd file"));
    }
    if !path.is_dir() {
        return v::invalid(field, format!("{shown} does not exist"));
    }
    let mut found = Vec::new();
    let mut stack = vec![(path.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                stack.push((p, depth + 1));
            } else if p.is_file() && p.extension().is_some_and(|e| e == "dmnd") {
                found.push(p);
            }
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => v::invalid(field, format!("no .dmnd file under {shown}; not a CheckM2 database")),
        _ => v::invalid(field, format!("several .dmnd files under {shown}; give the file itself")),
    }
}

/// Genomes per CheckM2 call that stayed well under 8 GB on the laptop.
const CHECKM2_BATCH: usize = 20;

pub fn checkm2(ctx: &Ctx, p: &Checkm2Params) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("checkm2");
    inv.volatile = &["checkm2.log"];
    inv.args.push("predict".into());
    let mut input_args = Vec::new();
    match (&p.genome_dir, p.genomes.is_empty()) {
        (Some(_), false) | (None, true) => {
            return v::invalid("genomes", "give either genomes (files) or genome_dir (a folder)");
        }
        (Some(dir), true) => {
            let path = v::existing_dir(cwd, "genome_dir", dir)?;
            let ext = p.extension.as_deref().unwrap_or(".fna");
            v::extension("extension", ext)?;
            let files = v::files_with_extension(&path, ext);
            if files.is_empty() {
                return v::invalid("genome_dir", format!("no *{ext} files in {dir}; set extension (e.g. fa, fasta, fna.gz)"));
            }
            for f in &files {
                let shown = f.strip_prefix(cwd).unwrap_or(f).to_string_lossy().into_owned();
                v::sequence_file(f, "genome_dir", &shown, Seq::Fasta)?;
            }
            inv.inputs.extend(files);
            if p.extension.is_some() {
                inv.opt("extension", ext);
            }
            input_args.push(list_item(dir));
        }
        (None, false) => {
            if p.extension.is_some() {
                return v::invalid("extension", "extension applies to genome_dir only");
            }
            for g in &p.genomes {
                let path = v::existing_file(cwd, "genomes", g)?;
                v::sequence_file(&path, "genomes", g, Seq::Fasta)?;
                inv.inputs.push(path);
                input_args.push(list_item(g));
            }
        }
    }
    let (db_text, db) = db_location(ctx, &mut inv, "db", &p.db, "CHECKM2DB")?;
    let dmnd = checkm2_db("db", &db_text, &db)?;
    // Pass the file itself; the database (about 3 GB) is not hashed (dblock_verify does that).
    inv.opt("database_path", dmnd.to_string_lossy());
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    inv.outputs.push(outdir.clone());
    inv.opt("output-directory", &p.outdir);
    // CheckM2 refuses a non-empty folder without --force and empties it with --force. irit-mcp
    // has refused a non-empty folder unless overwrite is true, and replay.sh needs --force to
    // run into the folder again.
    inv.flag("force", true);
    inv.expect = vec![vec![outdir.join("quality_report.tsv")]];
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    match p.model {
        Some(Checkm2Model::General) => inv.flag("general", true),
        Some(Checkm2Model::Specific) => inv.flag("specific", true),
        Some(Checkm2Model::Both) => inv.flag("allmodels", true),
        Some(Checkm2Model::Auto) | None => {}
    }
    inv.flag("lowmem", p.lowmem);
    // Issue #25: CheckM2 1.1.0 on 48 genomes in one call peaked at 7.7 GiB under an 8 GB cap.
    if inv.inputs.len() > CHECKM2_BATCH {
        inv.warnings.push(format!(
            "{} genomes in one call: CheckM2 1.1.0 on 48 genomes peaked at 7.7 GiB on an 8 GB laptop. On 8-16 GB machines run batches of 10-{CHECKM2_BATCH}{}",
            inv.inputs.len(),
            if p.lowmem { "" } else { " and set lowmem" }
        ));
    }
    if let Some(t) = p.ttable {
        if !CHECKM2_TTABLES.contains(&t) {
            return v::invalid("ttable", format!("{t} is not a translation table CheckM2 accepts"));
        }
        inv.opt("ttable", t);
    }
    inv.flag("genes", p.genes);
    inv.flag("remove_intermediates", p.remove_intermediates);
    // `--input` takes several values (nargs='+'), so it goes last, one value per argument.
    inv.args.push("--input".into());
    inv.args.extend(input_args);
    Ok(inv)
}

// ---------- GTDB-Tk ----------
// Flags: GTDB-Tk 2.8.0 gtdbtk/cli.py (`classify_wf` subparser); data folders: gtdbtk/config/common.py;
// outputs: gtdbtk/config/output.py (https://github.com/Ecogenomics/GTDBTk/tree/2.8.0/gtdbtk).
// Not in 2.8.0, so not offered: --skip_ani_screen (only a classify_wf internal), --mash_db (gone
// since 2.5.0, when skani replaced Mash), and any memory cap (--max_memory belongs to gPlace,
// which GTDB-Tk calls as `pplacer`; see docs/research/gtdb-placement-test.md).

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GtdbtkClassifyWfParams {
    /// Folder of genome FASTA files (one genome per file), matched by `extension`.
    pub genome_dir: String,
    /// File extension of the genomes. GTDB-Tk's default is fna; use fna.gz for gzipped files.
    #[serde(default)]
    pub extension: Option<String>,
    /// Output folder, inside the root.
    pub out_dir: String,
    /// Allow an out_dir that already has files (GTDB-Tk writes over them). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// GTDB-Tk reference data folder (the release folder holding metadata/, markers/, skani/, ...). Default: $GTDBTK_DATA_PATH as the server started with, else the one locked in ./dblock.lock. dblock_env shows the path for gtdbtk-r232.
    #[serde(default)]
    pub data_path: Option<String>,
    /// Prefix of the output files. Default gtdbtk.
    #[serde(default)]
    pub prefix: Option<String>,
    /// CPUs. GTDB-Tk's default is 1.
    #[serde(default)]
    pub cpus: Option<u32>,
    /// CPUs for pplacer placement. Each pplacer thread holds its own copy of the tree in memory, so use 1 on a laptop.
    #[serde(default)]
    pub pplacer_cpus: Option<u32>,
    /// Folder for pplacer's memory-mapped file: much less RAM, slower. Must be inside the root, on a disk with tens of GB free.
    #[serde(default)]
    pub scratch_dir: Option<String>,
    /// Use the unsplit bacterial tree (GTDB-Tk before v2). Needs far more RAM than a laptop has.
    #[serde(default)]
    pub full_tree: bool,
    /// Place genomes on the tree even when skani has already classified them.
    #[serde(default)]
    pub place_species: bool,
    /// Minimum alignment fraction to assign a genome to a species cluster (0-1).
    #[serde(default)]
    pub min_af: Option<f64>,
    /// Exclude genomes with fewer than this % of amino acids in the MSA. Default 10.
    #[serde(default)]
    pub min_perc_aa: Option<f64>,
    /// Keep intermediate files.
    #[serde(default)]
    pub keep_intermediates: bool,
    /// Also write unaligned single-copy marker genes.
    #[serde(default)]
    pub write_single_copy_genes: bool,
    /// GTDB-Tk --force: continue when one genome fails (it does NOT mean overwrite).
    #[serde(default)]
    pub force: bool,
    /// Inputs are called genes (protein FASTA), not nucleotide.
    #[serde(default)]
    pub genes: bool,
}

/// Folders classify_wf reads in GTDB-Tk 2.8.0 (`gtdbtk/config/common.py`).
const GTDBTK_DIRS: [&str; 9] = ["markers", "masks", "metadata", "mrca_red", "pplacer", "radii", "split", "taxonomy", "skani"];

/// Files the split-tree placement reads (GTDB-Tk 2.8.0 `config/common.py`: BACKBONE_PPLACER_REF_PKG,
/// CLASS_LEVEL_TREE_MAPPING_FILE). Without them pplacer fails after the ANI screen and marker steps.
const GTDBTK_SPLIT_FILES: [&str; 2] = ["split/backbone/pplacer/gtdbtk_package_backbone.refpkg", "split/class_level/tree_mapping.tsv"];

fn gtdbtk_data(field: &str, shown: &str, dir: &Path, full_tree: bool) -> Check<PathBuf> {
    if !dir.is_dir() {
        return v::invalid(field, format!("{shown} is not a folder"));
    }
    let meta = dir.join("metadata/metadata.txt");
    let version = std::fs::read_to_string(&meta)
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("VERSION_DATA=").map(str::to_string)));
    if version.is_none() {
        return v::invalid(
            field,
            format!(
                "{shown} has no metadata/metadata.txt with VERSION_DATA, so it is not GTDB-Tk reference data (point at the release folder, e.g. .../release232)"
            ),
        );
    }
    let missing: Vec<&str> = GTDBTK_DIRS.iter().copied().filter(|d| !dir.join(d).is_dir()).collect();
    if missing == ["skani"] {
        return v::invalid(
            field,
            format!(
                "{shown} has no skani/ folder. GTDB-Tk 2.8.0 classify_wf always runs the skani ANI screen and has no option to skip it. \
                 With the placement data, run gtdbprep_ani first and pass its <work>/gtdbtk_data as data_path (it adds a small skani database); \
                 genomes the screen doesn't assign need tree placement, see docs/research/gtdb-placement-test.md"
            ),
        );
    }
    if !missing.is_empty() {
        return v::invalid(field, format!("{shown} is missing {} (incomplete GTDB-Tk data?)", missing.join(", ")));
    }
    if !full_tree {
        let missing: Vec<&str> = GTDBTK_SPLIT_FILES.iter().copied().filter(|f| !dir.join(f).exists()).collect();
        if !missing.is_empty() {
            return v::invalid(
                field,
                format!(
                    "{shown} has no {}, which classify_wf needs to place genomes on the split tree. \
                     GTDB's mock database has an empty split/ and needs full_tree: true",
                    missing.join(" or ")
                ),
            );
        }
    }
    Ok(meta)
}

pub fn gtdbtk_classify_wf(ctx: &Ctx, p: &GtdbtkClassifyWfParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("gtdbtk");
    // Seen in the real-tools workflow: logs, gtdbtk.json (step start/end times) and gzip files
    // (gzip headers carry a time) differ between identical runs.
    inv.volatile = &[".log", "gtdbtk.json", ".gz"];
    inv.args.push("classify_wf".into());
    let dir = v::existing_dir(cwd, "genome_dir", &p.genome_dir)?;
    let ext = p.extension.as_deref().unwrap_or("fna");
    v::extension("extension", ext)?;
    let files = v::files_with_extension(&dir, ext);
    if files.is_empty() {
        return v::invalid(
            "genome_dir",
            format!("no *.{} files in {}; set extension (e.g. fa, fasta, fna.gz)", ext.trim_start_matches('.'), p.genome_dir),
        );
    }
    for f in &files {
        let shown = f.strip_prefix(cwd).unwrap_or(f).to_string_lossy().into_owned();
        v::sequence_file(f, "genome_dir", &shown, Seq::Fasta)?;
    }
    inv.inputs.extend(files);
    inv.opt("genome_dir", &p.genome_dir);
    let (data_text, data) = db_location(ctx, &mut inv, "data_path", &p.data_path, "GTDBTK_DATA_PATH")?;
    inv.inputs.push(gtdbtk_data("data_path", &data_text, &data, p.full_tree)?);
    // A small skani database (the one gtdbprep_ani builds, a few MB) is hashed into the record;
    // the full one (85 GB in R232) is left to dblock_verify, like the other large databases.
    inv.inputs.extend(small_skani_db(&data.join("skani")));
    // GTDB-Tk has no data-path flag: it reads GTDBTK_DATA_PATH, recorded and exported by replay.sh.
    inv.env.insert("GTDBTK_DATA_PATH".into(), v::resolve(&data).to_string_lossy().into_owned());
    let out = v::output_dir(ctx, "out_dir", &p.out_dir, Some(p.overwrite), true)?;
    inv.outputs.push(out.clone());
    inv.opt("out_dir", &p.out_dir);
    if p.extension.is_some() {
        inv.opt("extension", ext);
    }
    let prefix = p.prefix.as_deref().unwrap_or("gtdbtk");
    v::file_stem("prefix", prefix)?;
    if p.prefix.is_some() {
        inv.opt("prefix", prefix);
    }
    // Either summary may be absent (no archaea, say), but not both.
    inv.expect = vec![["bac120", "ar53"].iter().map(|m| out.join("classify").join(format!("{prefix}.{m}.summary.tsv"))).collect()];
    for (name, val) in [("cpus", p.cpus), ("pplacer_cpus", p.pplacer_cpus)] {
        if let Some(x) = val {
            v::range(name, x, 1, 1024)?;
            inv.opt(name, x);
        }
    }
    if let Some(s) = &p.scratch_dir {
        v::output_dir(ctx, "scratch_dir", s, None, false)?;
        inv.opt("scratch_dir", s);
    }
    if let Some(x) = p.min_af {
        if !x.is_finite() || !(0.0..=1.0).contains(&x) {
            return v::invalid("min_af", format!("{x} is outside 0..=1"));
        }
        inv.opt("min_af", x);
    }
    if let Some(x) = p.min_perc_aa {
        v::percent("min_perc_aa", x)?;
        inv.opt("min_perc_aa", x);
    }
    for (name, on) in [
        ("full_tree", p.full_tree),
        ("place_species", p.place_species),
        ("keep_intermediates", p.keep_intermediates),
        ("write_single_copy_genes", p.write_single_copy_genes),
        ("force", p.force),
        ("genes", p.genes),
    ] {
        inv.flag(name, on);
    }
    Ok(inv)
}

// ---------- gtdbprep ani (an irit binary, wrapped here beside GTDB-Tk) ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GtdbprepAniParams {
    /// Folder of genome FASTA files (plain or gzipped), as for gtdbtk_classify_wf. The file name (without extension) is the genome's name in every result.
    pub genome_dir: String,
    /// Extension of the genome files in genome_dir. Default fna.
    #[serde(default)]
    pub extension: Option<String>,
    /// Screen index folder (meta.json, refs.bin, markers.idx): dblock's gtdbtk-r232-ani-screen (dblock_plan shows its path). Default: the ANI-screen index locked in ./dblock.lock.
    #[serde(default)]
    pub index: Option<String>,
    /// GTDB-Tk reference data with taxonomy/, radii/ and metadata/ (no skani/ needed): dblock's gtdbtk-r232-placement-lite. Default: $GTDBTK_DATA_PATH as the server started with, else the one locked in ./dblock.lock.
    #[serde(default)]
    pub data_path: Option<String>,
    /// Work folder, inside the root. Its gtdbtk_data/ is then a complete GTDB-Tk data folder: pass `<work>/gtdbtk_data` as gtdbtk_classify_wf's data_path.
    pub work: String,
    /// Allow a work folder that already has files (they are replaced). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Where reference sketches missing from the cache come from: a path or URL of an unpacked sketches.db, a URL with {part} (with part_size), or `gtdb` to range-read GTDB's own archive. Network use, and `gtdb` loads GTDB's server: ask the user first. Omit when every sketch is cached (the call then fails naming how many are missing).
    #[serde(default)]
    pub source: Option<String>,
    /// Part size in bytes, for a source with {part}.
    #[serde(default)]
    pub part_size: Option<u64>,
    /// Sketch cache shared by all runs. Default: $XDG_CACHE_HOME/irit/gtdbprep-ani, else ~/.cache/irit/gtdbprep-ani, as the server started with; the path used is in the recorded command. May be outside the root: only sketches checked against the index are added.
    #[serde(default)]
    pub cache: Option<String>,
    /// skani binary; must be the version the index was made with (0.3.1 for R232). Default: skani in the GTDB-Tk environment or on PATH.
    #[serde(default)]
    pub skani: Option<String>,
    /// Fetch only the references that could give the species call. Same species calls, far fewer downloads; GTDB-Tk's "other related references" list fewer references.
    #[serde(default)]
    pub species_only: bool,
    /// Range requests in flight at once while fetching sketches (1-16).
    #[serde(default)]
    pub fetch_jobs: Option<u8>,
    /// Conda/micromamba environment with GTDB-Tk 2.8 (its bin/ goes first on PATH). Default: $GTDBPREP_ENV as the server started with, else PATH.
    #[serde(default)]
    pub env: Option<String>,
    /// CPUs. Default 4.
    #[serde(default)]
    pub cpus: Option<u32>,
}

/// Index files hashed into the record. markers.idx (1.7 GB for R232) and gz.idx are left to
/// `dblock verify`: hashing them on every call would cost more than the screen.
const ANI_INDEX_FILES: [&str; 3] = ["meta.json", "refs.bin", "markers.idx"];
/// What `gtdbprep ani` needs from the GTDB-Tk data (`ani::synthetic_data`).
const ANI_DATA_FILES: [&str; 3] = ["metadata/metadata.txt", "radii/gtdb_radii.tsv", "taxonomy/gtdb_taxonomy.tsv"];

/// GTDB-Tk's ANI screen without the 85 GB skani database (`gtdbprep ani`, gtdbprep/src/cli.rs).
pub fn gtdbprep_ani(ctx: &Ctx, p: &GtdbprepAniParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = Invocation::new("gtdbprep");
    // Two identical runs differed only in the logs and the gzip file (its header has a time).
    inv.volatile = &[".log", ".gz"];
    inv.args.push("ani".into());
    let dir = v::existing_dir(cwd, "genome_dir", &p.genome_dir)?;
    let ext = p.extension.as_deref().unwrap_or("fna");
    v::extension("extension", ext)?;
    let files = v::files_with_extension(&dir, ext);
    if files.is_empty() {
        return v::invalid("genome_dir", format!("no *.{} files in {}; set extension (e.g. fa, fasta, fna.gz)", ext.trim_start_matches('.'), p.genome_dir));
    }
    for f in &files {
        let shown = f.strip_prefix(cwd).unwrap_or(f).to_string_lossy().into_owned();
        v::sequence_file(f, "genome_dir", &shown, Seq::Fasta)?;
    }
    inv.inputs.extend(files);
    inv.opt("genomes", &p.genome_dir);
    inv.opt("extension", ext);

    let index_text = match &p.index {
        Some(i) => i.clone(),
        None => match locked_db(cwd, |db| db.get("id").and_then(toml::Value::as_str).is_some_and(|id| id.contains("ani-screen"))) {
            Ok(Some((id, path))) => {
                inv.warnings.push(format!("index: none given, so the ANI-screen index locked in dblock.lock was used: {id} at {path}"));
                path
            }
            Ok(None) => return v::invalid("index", "no index given and dblock.lock here locks no ANI-screen index (dblock add gtdbtk-r232-ani-screen)"),
            Err(e) => return v::invalid("index", format!("no index given; {e}")),
        },
    };
    let index = v::existing_dir(cwd, "index", &index_text)?;
    let missing: Vec<&str> = ANI_INDEX_FILES.iter().copied().filter(|f| !index.join(f).is_file()).collect();
    if !missing.is_empty() {
        return v::invalid("index", format!("{index_text} has no {}; it is not a screen index (dblock add gtdbtk-r232-ani-screen)", missing.join(", ")));
    }
    inv.inputs.extend(["meta.json", "refs.bin"].map(|f| index.join(f)));
    inv.opt("index", &index_text);

    let (data_text, data) = db_location(ctx, &mut inv, "data_path", &p.data_path, "GTDBTK_DATA_PATH")?;
    let missing: Vec<&str> = ANI_DATA_FILES.iter().copied().filter(|f| !data.join(f).is_file()).collect();
    if !missing.is_empty() {
        return v::invalid(
            "data_path",
            format!("{data_text} has no {}; the ANI screen needs GTDB-Tk's taxonomy, radii and metadata (dblock add gtdbtk-r232-placement-lite)", missing.join(", ")),
        );
    }
    inv.inputs.extend(ANI_DATA_FILES.map(|f| data.join(f)));
    // Passed as a flag, not left to gtdbprep's own $GTDBTK_DATA_PATH lookup, so the command says which.
    inv.opt("data", v::resolve(&data).display());

    let work = v::output_dir(ctx, "work", &p.work, Some(p.overwrite), true)?;
    inv.outputs.push(work.clone());
    inv.opt("work", &p.work);
    inv.expect = vec![vec![work.join("assigned.json")]];

    if let Some(s) = &p.source {
        v::text("source", s)?;
        inv.opt("source", s);
    }
    match (p.part_size, &p.source) {
        (Some(n), Some(s)) if s.contains("{part}") => {
            v::range("part_size", n, 1, u64::MAX)?;
            inv.opt("part-size", n);
        }
        (Some(_), _) => return v::invalid("part_size", "part_size applies only to a source with {part}"),
        (None, Some(s)) if s.contains("{part}") => return v::invalid("part_size", "a source with {part} needs part_size"),
        _ => {}
    }

    // The cache gtdbprep would pick by itself, resolved here from the server's environment so the
    // recorded command (and replay.sh) names it.
    let cache = match &p.cache {
        Some(c) => {
            v::text("cache", c)?;
            cwd.join(c)
        }
        None => {
            let var = |k: &str| ctx.var(k).filter(|x| !x.is_empty()).map(PathBuf::from);
            match var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|h| h.join(".cache"))) {
                Some(base) => base.join("irit/gtdbprep-ani"),
                None => return v::invalid("cache", "neither XDG_CACHE_HOME nor HOME is set; pass cache"),
            }
        }
    };
    if let Some(runs) = &ctx.runs_root
        && v::resolve(&cache).starts_with(runs)
    {
        return v::invalid("cache", format!("{} is inside the run records {}; tools may not write there", cache.display(), runs.display()));
    }
    inv.opt("cache", cache.display());

    if let Some(s) = &p.skani {
        v::text("skani", s)?;
        // A bare name is looked up on PATH by gtdbprep; a path must exist.
        let path = if s.contains('/') { v::existing_file(cwd, "skani", s)? } else { PathBuf::from(s) };
        if path.is_absolute() {
            inv.inputs.push(path);
        }
        inv.opt("skani", s);
    }
    inv.flag("species-only", p.species_only);
    if let Some(j) = p.fetch_jobs {
        v::range("fetch_jobs", j, 1, 16)?;
        inv.opt("fetch-jobs", j);
    }
    match (&p.env, ctx.var("GTDBPREP_ENV").filter(|x| !x.is_empty())) {
        (Some(e), _) => {
            v::existing_dir(cwd, "env", e)?;
            inv.opt("env", e);
        }
        (None, Some(e)) => inv.opt("env", e.to_string_lossy()),
        (None, None) => {}
    }
    if let Some(c) = p.cpus {
        v::range("cpus", c, 1, 1024)?;
        inv.opt("cpus", c);
    }
    Ok(inv)
}

/// Up to this much skani database is hashed into a GTDB-Tk call's inputs.
pub const SKANI_HASH_LIMIT: u64 = 512 << 20;

/// The entries of GTDB-Tk's `skani/` folder, resolved (so a linked database is hashed under its
/// real path, where the call that built it recorded it), if they hold at most
/// [`SKANI_HASH_LIMIT`] bytes in all; nothing for a larger one.
pub fn small_skani_db(skani: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(skani).into_iter().flatten().flatten().map(|e| v::resolve(&e.path())).collect();
    entries.sort();
    let mut total = 0u64;
    let mut stack = entries.clone();
    while let Some(p) = stack.pop() {
        match std::fs::metadata(&p) {
            Ok(m) if m.is_dir() => stack.extend(std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path())),
            Ok(m) => total += m.len(),
            Err(_) => {}
        }
        if total > SKANI_HASH_LIMIT {
            return Vec::new();
        }
    }
    entries
}

/// An output whose name isotable's file search recognises: it ends in `.tsv` and holds `tag`
/// (isotable/src/parse.rs `scan`: `mlst` for mlst, `amrfinder` for AMRFinderPlus, any case for the latter).
/// Any other name is allowed (a side analysis that shouldn't join the table), with a warning that
/// isotable won't read it.
fn isotable_name(inv: &mut Invocation, value: &str, tag: &str, example: &str) {
    let name = Path::new(value).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !name.ends_with(".tsv") || !name.to_ascii_lowercase().contains(tag) {
        inv.warnings.push(format!(
            "{value}: isotable won't read this file; name it like {example} (ending in .tsv, with \"{tag}\" in the name) if it belongs in the table"
        ));
    }
}

/// The sample part of `<sample>.<tag>.tsv` (or `_<tag>`, `-<tag>`), if the name has one.
fn sample_from_output(value: &str, tag: &str) -> Option<String> {
    let name = Path::new(value).file_name()?.to_string_lossy().into_owned();
    let stem = name.strip_suffix(".tsv")?;
    let lower = stem.to_ascii_lowercase();
    [".", "_", "-"].iter().find_map(|sep| {
        let suffix = format!("{sep}{tag}");
        lower.ends_with(&suffix).then(|| stem[..stem.len() - suffix.len()].to_string())
    }).filter(|s| !s.is_empty())
}

// ---------- mlst ----------
// Flags: mlst v2.35.0 bin/mlst (`setOptions` and the main block),
// https://github.com/tseemann/mlst/blob/v2.35.0/bin/mlst; same list as `mlst --help` of bioconda
// mlst=2.35.0. Not offered: --csv and --legacy (isotable reads the tab-separated layout), --blastdb and
// --datadir (the PubMLST data comes with the installed mlst, or from $MLST_DBDIR; see `mlst_data`), --fofn,
// --list/--longlist/--info/--check (they don't type anything), --exclude, --quiet, --debug.

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MlstParams {
    /// Assemblies, FASTA (plain or gzip). Usually one: `<outdir>/contigs.fa` from shovill.
    pub contigs: Vec<String>,
    /// Result table (tab-separated), inside the root. Name it `<sample>.mlst.tsv`: isotable finds files
    /// with "mlst" in the name ending in .tsv, and the sample part becomes the default label.
    pub outfile: String,
    /// Replace existing output files. Default false: they are refused.
    #[serde(default)]
    pub overwrite: bool,
    /// Name in the FILE column instead of the file's path; one assembly only. Default, for one
    /// assembly: the sample part of outfile (`s1.mlst.tsv` gives `s1`), so isotable joins the row to
    /// that sample rather than to `contigs`.
    #[serde(default)]
    pub label: Option<String>,
    /// Also write the results as JSON here (inside the root).
    #[serde(default)]
    pub json: Option<String>,
    /// Write novel alleles to this FASTA file (inside the root).
    #[serde(default)]
    pub novel: Option<String>,
    /// Use this PubMLST scheme instead of detecting it, e.g. saureus, ecoli_achtman_4. Sets minscore to 0.
    #[serde(default)]
    pub scheme: Option<String>,
    /// More detailed table: adds a header row and STATUS and SCORE columns (isotable reads it too).
    #[serde(default)]
    pub full: bool,
    /// % DNA identity of a full allele to call it similar (~). Default 95.
    #[serde(default)]
    pub minid: Option<f64>,
    /// % DNA coverage to report a partial allele (?). Default 50.
    #[serde(default)]
    pub mincov: Option<f64>,
    /// Minimum score out of 100 to match a scheme when detecting it. Default 50.
    #[serde(default)]
    pub minscore: Option<f64>,
    /// BLAST threads. Default 1.
    #[serde(default)]
    pub threads: Option<u32>,
}

/// The data mlst 2.35.0 types with: VERSION.txt (the data's date), scheme_species_map.tab, the BLAST
/// database of every allele (`blast/mlst.fa.*`, built from the allele files, which are not listed
/// separately) and each scheme's profile table (`pubmlst/<scheme>/<scheme>.txt`, allele numbers to
/// ST). About 180 files for 2.35.0's 162 schemes.
///
/// Where it is comes from mlst itself: `mlst --help` prints the resolved `--datadir` and `--blastdb`
/// defaults (bin/mlst: `$MLST_DBDIR`, else `$FindBin::RealBin/../db`), so a wrapper script in front of
/// mlst or a set `$MLST_DBDIR` is followed. If --help gives no paths, `db/` next to the real path
/// of the program (Bioconda's layout), or `$MLST_DBDIR`, is used.
pub fn mlst_data(program: &Path, ctx: &Ctx) -> Vec<PathBuf> {
    let mut cmd = std::process::Command::new(program);
    cmd.arg("--help").current_dir(&ctx.cwd).stdin(std::process::Stdio::null());
    if let Some(d) = ctx.var("MLST_DBDIR") {
        cmd.env("MLST_DBDIR", d);
    }
    let help = cmd.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let (datadir, blastdb) = mlst_help_paths(&help);
    let (datadir, blastdb) = match (datadir, blastdb) {
        (Some(d), Some(b)) => (d, b),
        _ => {
            let db = match ctx.var("MLST_DBDIR").filter(|v| !v.is_empty()) {
                Some(d) => ctx.cwd.join(d),
                None => match std::fs::canonicalize(program).ok().and_then(|r| r.parent()?.parent().map(|p| p.join("db"))) {
                    Some(db) => db,
                    None => return Vec::new(),
                },
            };
            (db.join("pubmlst"), db.join("blast").join("mlst.fa"))
        }
    };
    let mut files = Vec::new();
    if let Some(db) = datadir.parent() {
        files.push(db.join("VERSION.txt"));
        files.push(db.join("scheme_species_map.tab"));
    }
    if let (Some(dir), Some(prefix)) = (blastdb.parent(), blastdb.file_name().and_then(|n| n.to_str()))
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        files.extend(rd.flatten().map(|e| e.path()).filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.strip_prefix(prefix).is_some_and(|r| r.starts_with('.')))
        }));
    }
    if let Ok(rd) = std::fs::read_dir(&datadir) {
        for e in rd.flatten() {
            let profile = e.path().join(format!("{}.txt", e.file_name().to_string_lossy()));
            if profile.is_file() {
                files.push(profile);
            }
        }
    }
    files.retain(|p| p.is_file());
    files.sort();
    files.dedup();
    files
}

/// `--datadir` and `--blastdb` defaults from mlst 2.35.0's --help, printed as
/// `  --datadir [X]       PubMLST data folders [...] (default '/path')`.
fn mlst_help_paths(help: &str) -> (Option<PathBuf>, Option<PathBuf>) {
    let find = |opt: &str| {
        help.lines().find(|l| l.trim_start().starts_with(opt)).and_then(|l| {
            let (_, rest) = l.rsplit_once("(default '")?;
            let path = rest.strip_suffix("')")?;
            (!path.is_empty()).then(|| PathBuf::from(path))
        })
    };
    (find("--datadir"), find("--blastdb"))
}

pub fn mlst(ctx: &Ctx, p: &MlstParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("mlst");
    inv.bundled_data = Some(mlst_data);
    // mlst reads it from the environment it inherits; recorded so replay.sh types with the same data.
    if let Some(d) = ctx.var("MLST_DBDIR").filter(|v| !v.is_empty()) {
        inv.env.insert("MLST_DBDIR".into(), d.to_string_lossy().into_owned());
    }
    // Seen in the real-tools workflow: the JSON has the same values but its keys in another order
    // each run (mlst writes it with to_json without `canonical`, and Perl randomises hash order).
    // The table isotable reads is identical, and hash-checked.
    inv.volatile = &[".json"];
    v::non_empty_list("contigs", &p.contigs)?;
    for c in &p.contigs {
        let path = v::existing_file(cwd, "contigs", c)?;
        v::sequence_file(&path, "contigs", c, Seq::Fasta)?;
        inv.inputs.push(path);
    }
    isotable_name(&mut inv, &p.outfile, "mlst", "s1.mlst.tsv");
    let out = v::output_file(ctx, "outfile", &p.outfile, p.overwrite)?;
    inv.outputs.push(out.clone());
    inv.opt("outfile", &p.outfile);
    inv.expect = vec![vec![out.clone()]];
    let mut written = vec![("outfile", out)];
    for (field, val) in [("json", &p.json), ("novel", &p.novel)] {
        if let Some(path) = output_file_opt(ctx, &mut inv, field, val, p.overwrite, field)? {
            if field == "json" {
                inv.expect.push(vec![path.clone()]);
            }
            written.push((field, path));
        }
    }
    for (i, (field, a)) in written.iter().enumerate() {
        if written[..i].iter().any(|(_, b)| b == a) || inv.inputs.contains(a) {
            return v::invalid(field, format!("{} is used twice; give every input and output its own path", a.display()));
        }
    }
    let label = match (&p.label, p.contigs.len()) {
        (Some(_), n) if n > 1 => return v::invalid("label", "label works with one assembly only (mlst refuses it for several)"),
        (Some(l), _) => Some(l.clone()),
        (None, 1) => sample_from_output(&p.outfile, "mlst"),
        (None, _) => None,
    };
    if let Some(l) = &label {
        // It becomes the first column of a tab-separated table.
        if l.is_empty() || l.contains(['\t', ',']) || l.chars().any(char::is_control) {
            return v::invalid("label", format!("{l:?} can't be a table cell (no tabs, commas or control characters)"));
        }
        inv.opt("label", l);
    }
    if let Some(s) = &p.scheme {
        if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return v::invalid("scheme", format!("{s:?} is not a PubMLST scheme name like saureus or ecoli_achtman_4"));
        }
        inv.opt("scheme", s);
    }
    inv.flag("full", p.full);
    for (name, val) in [("minid", p.minid), ("mincov", p.mincov), ("minscore", p.minscore)] {
        if let Some(x) = val {
            v::percent(name, x)?;
            inv.opt(name, x);
        }
    }
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    let files: Vec<String> = p.contigs.iter().map(|c| list_item(c)).collect();
    inv.positionals(&files);
    Ok(inv)
}

// ---------- AMRFinderPlus ----------
// Flags: AMRFinderPlus amrfinder_v4.2.7 amrfinder.cpp (`addKey`/`addFlag` calls and the checks in
// `shellBody`), https://github.com/ncbi/amr/blob/amrfinder_v4.2.7/amrfinder.cpp; same list as
// `amrfinder --help` of bioconda ncbi-amrfinderplus=4.2.7. Database files: amrfinder_update.cpp (what it
// downloads) and amrfinder_index.cpp (what it builds). Not offered: --update/--force_update (they change
// the database outside the record: fetch it with dblock instead), --blast_bin/--hmmer_bin, --pgap,
// --gpipe_org, --parm (for NCBI's testing), --debug, --log, --quiet, --list_organisms, --database_version,
// --nucleotide_flank5_output/_size.

/// amrfinder.cpp 4.2.7 `dataVer_min`: older data is refused.
const AMRFINDER_DATA_MIN: &str = "2025-09-22.2";

/// Files amrfinder_update 4.2.7 downloads besides the per-organism AMR_DNA-* files and changes.txt.
const AMRFINDER_DB_FILES: [&str; 11] = [
    "AMR.LIB",
    "AMRProt.fa",
    "AMRProt-mutation.tsv",
    "AMRProt-suppress.tsv",
    "AMRProt-susceptible.fa",
    "AMRProt-susceptible.tsv",
    "AMR_CDS.fa",
    "database_format_version.txt",
    "fam.tsv",
    "taxgroup.tsv",
    "version.txt",
];

/// Indexes amrfinder_index 4.2.7 builds (hmmpress, makeblastdb) that amrfinder needs: AMRProt.fa.phr
/// is amrfinder.cpp's own check; the other two are the HMM and nucleotide BLAST databases it searches.
const AMRFINDER_INDEX_FILES: [&str; 3] = ["AMRProt.fa.phr", "AMR.LIB.h3m", "AMR_CDS.fa.nin"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AmrAnnotationFormat {
    Bakta,
    Genbank,
    Microscope,
    Patric,
    Pgap,
    Prodigal,
    Prokka,
    Pseudomonasdb,
    Rast,
    Standard,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AmrfinderParams {
    /// Assembly (nucleotide FASTA, plain or gzip). Give this, protein, or both (both need gff).
    #[serde(default)]
    pub nucleotide: Option<String>,
    /// Proteins (FASTA), e.g. Bakta's `<prefix>.faa`.
    #[serde(default)]
    pub protein: Option<String>,
    /// GFF with the proteins' locations in `nucleotide`, e.g. Bakta's `<prefix>.gff3`; needs protein.
    #[serde(default)]
    pub gff: Option<String>,
    /// Which tool wrote the GFF. AMRFinderPlus's default is genbank; use bakta for Bakta's GFF3.
    #[serde(default)]
    pub annotation_format: Option<AmrAnnotationFormat>,
    /// AMRFinderPlus database folder (holding version.txt, indexed). Default: $AMRFINDER_DB as the server started with, else the one locked in ./dblock.lock.
    /// dblock_env shows the path of a dblock-managed one (amrfinderplus-db-4.2-2026-10-29.1).
    #[serde(default)]
    pub database: Option<String>,
    /// Taxonomy group for point mutations and organism-specific genes, e.g. Escherichia,
    /// Staphylococcus_aureus, Klebsiella_pneumoniae. Checked against the database's taxgroup.tsv.
    #[serde(default)]
    pub organism: Option<String>,
    /// Also report the "plus" genes (virulence, stress, ...).
    #[serde(default)]
    pub plus: bool,
    /// Report proteins common to the organism too; needs organism and plus.
    #[serde(default)]
    pub report_common: bool,
    /// Report all equally scoring BLAST and HMM matches.
    #[serde(default)]
    pub report_all_equal: bool,
    /// Minimum identity, 0-1, or -1 for the curated cut-offs (the default).
    #[serde(default)]
    pub ident_min: Option<f64>,
    /// Minimum coverage of the reference protein, 0-1. Default 0.5.
    #[serde(default)]
    pub coverage_min: Option<f64>,
    /// Genetic code for translated searches. Default 11.
    #[serde(default)]
    pub translation_table: Option<u8>,
    /// Text for a first "Name" column on every row, e.g. the sample name.
    #[serde(default)]
    pub name: Option<String>,
    /// Report, inside the root. Name it `<sample>.amrfinder.tsv` so isotable finds it and takes the sample name from it; another name is allowed (with a warning) for a run that shouldn't join the table.
    pub output: String,
    /// Also write every mutation position checked here (needs organism).
    #[serde(default)]
    pub mutation_all: Option<String>,
    /// Write the reported proteins as FASTA here (needs protein).
    #[serde(default)]
    pub protein_output: Option<String>,
    /// Write the reported nucleotide sequences as FASTA here (needs nucleotide).
    #[serde(default)]
    pub nucleotide_output: Option<String>,
    /// Print the hierarchy node (family) of each hit.
    #[serde(default)]
    pub print_node: bool,
    /// Threads. Default 4.
    #[serde(default)]
    pub threads: Option<u32>,
    /// Replace existing output files. Default false: they are refused.
    #[serde(default)]
    pub overwrite: bool,
}

/// `2025-09-22.2` as (date, release) for comparing.
fn data_version(s: &str) -> Option<(String, u32)> {
    let (date, n) = s.trim().split_once('.')?;
    let ok = date.len() == 10 && date.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() });
    if !ok {
        return None;
    }
    Some((date.to_string(), n.parse().ok()?))
}

/// `4.2.0` as numbers.
fn dotted(s: &str) -> Option<Vec<u64>> {
    s.trim().split('.').map(|x| x.parse().ok()).collect()
}

/// Organisms the database knows: first columns of taxgroup.tsv, AMRProt-mutation.tsv and
/// AMRProt-susceptible.tsv, as amrfinder.cpp's `db2organisms` reads them.
fn amrfinder_organisms(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = ["taxgroup.tsv", "AMRProt-mutation.tsv", "AMRProt-susceptible.tsv"]
        .iter()
        .filter_map(|f| std::fs::read_to_string(dir.join(f)).ok())
        .flat_map(|t| t.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).filter_map(|l| l.split('\t').next()).map(str::to_string).collect::<Vec<_>>())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Checks an AMRFinderPlus database folder the way amrfinder 4.2.7 does before it searches:
/// present, indexed, a format this version reads and data new enough. Returns version.txt and
/// the data version.
fn amrfinder_db(field: &str, shown: &str, dir: &Path, organism: Option<&str>, nucleotide: bool) -> Check<(PathBuf, String)> {
    if !dir.is_dir() {
        return v::invalid(field, format!("{shown} is not a folder"));
    }
    let version_file = dir.join("version.txt");
    let version = std::fs::read_to_string(&version_file).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let Some(version) = version else {
        return v::invalid(
            field,
            format!("{shown} has no version.txt, so it is not an AMRFinderPlus database folder (amrfinder_update puts it in a dated folder with a `latest` link)"),
        );
    };
    let missing: Vec<&str> = AMRFINDER_DB_FILES.iter().copied().filter(|f| !dir.join(f).is_file()).collect();
    if !missing.is_empty() {
        // Bakta's database bundles a trimmed copy (amrfinderplus-db/): the BLAST and HMMER indexes
        // without the sequence files, which serves Bakta's own protein search but not amrfinder.
        if dir.join("AMRProt.fa.phr").is_file() && !dir.join("AMRProt.fa").is_file() {
            return v::invalid(
                field,
                format!(
                    "{shown} has the indexes but not the files they were built from ({}): the trimmed copy Bakta's database ships for Bakta's own use, not a full AMRFinderPlus database. Fetch one with dblock (amrfinderplus-db-4.2-2026-10-29.1)",
                    missing.join(", ")
                ),
            );
        }
        return v::invalid(field, format!("{shown} is missing {} (incomplete download?)", missing.join(", ")));
    }
    if let Some(idx) = AMRFINDER_INDEX_FILES.iter().find(|f| !dir.join(f).is_file()) {
        return v::invalid(
            field,
            format!("{shown} is not indexed (no {idx}); run amrfinder_index on it, or fetch it with dblock (amrfinderplus-db-4.2-2026-10-29.1), which does"),
        );
    }
    let format_text = std::fs::read_to_string(dir.join("database_format_version.txt")).unwrap_or_default();
    match (dotted(&format_text), dotted(pinned("amrfinder").unwrap().version)) {
        (Some(need), Some(have)) if need > have => {
            return v::invalid(field, format!("{shown} is format {}, which needs AMRFinderPlus {} or newer; irit-mcp is checked against 4.2.7", format_text.trim(), format_text.trim()));
        }
        (Some(_), _) => {}
        (None, _) => return v::invalid(field, format!("{shown}/database_format_version.txt is not a version like 4.2.0")),
    }
    match (data_version(&version), data_version(AMRFINDER_DATA_MIN)) {
        (Some(got), Some(min)) if got < min => {
            return v::invalid(field, format!("{shown} is data version {version}; AMRFinderPlus 4.2.7 needs {AMRFINDER_DATA_MIN} or newer"));
        }
        (Some(_), _) => {}
        (None, _) => return v::invalid(field, format!("{shown}/version.txt holds {version:?}, not a version like 2026-10-29.1")),
    }
    if let Some(org) = organism {
        let known = amrfinder_organisms(dir);
        if !known.iter().any(|k| k == org) {
            return v::invalid("organism", format!("{org:?} is not an organism in {shown}; it knows {}", known.join(", ")));
        }
        // amrfinder.cpp: with a nucleotide input, an organism with its own DNA file needs that file's BLAST index.
        let dna = dir.join(format!("AMR_DNA-{org}.fa"));
        let indexed = ["ndb", "nin"].iter().any(|ext| dir.join(format!("AMR_DNA-{org}.fa.{ext}")).is_file());
        if nucleotide && dna.is_file() && !indexed {
            return v::invalid(field, format!("{shown} has AMR_DNA-{org}.fa but no BLAST index for it; run amrfinder_index on the folder"));
        }
    }
    Ok((version_file, version))
}

pub fn amrfinder(ctx: &Ctx, p: &AmrfinderParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("amrfinder");
    if p.nucleotide.is_none() && p.protein.is_none() {
        return v::invalid("nucleotide", "give nucleotide (an assembly), protein, or both");
    }
    for (field, val, flag) in [("protein", &p.protein, "protein"), ("nucleotide", &p.nucleotide, "nucleotide")] {
        if let Some(f) = val {
            let path = v::existing_file(cwd, field, f)?;
            v::sequence_file(&path, field, f, Seq::Fasta)?;
            inv.inputs.push(path);
            inv.opt(flag, f);
        }
    }
    match (&p.gff, p.protein.is_some(), p.nucleotide.is_some()) {
        (Some(_), false, _) => return v::invalid("gff", "gff gives the proteins' locations, so it needs protein"),
        (None, true, true) => return v::invalid("gff", "with both protein and nucleotide, AMRFinderPlus needs gff to link them"),
        (Some(g), true, _) => {
            inv.inputs.push(v::existing_file(cwd, "gff", g)?);
            inv.opt("gff", g);
        }
        (None, _, _) => {}
    }
    if let Some(a) = p.annotation_format {
        if p.gff.is_none() {
            return v::invalid("annotation_format", "annotation_format describes the gff; give gff too");
        }
        inv.opt("annotation_format", serde_json::to_value(a).unwrap().as_str().unwrap());
    }
    let (db_text, db) = db_location(ctx, &mut inv, "database", &p.database, "AMRFINDER_DB")?;
    if let Some(o) = &p.organism {
        v::text("organism", o)?;
    }
    // version.txt only: the database itself is checked by dblock_verify.
    let (version_file, _) = amrfinder_db("database", &db_text, &db, p.organism.as_deref(), p.nucleotide.is_some())?;
    inv.inputs.push(version_file);
    inv.opt("database", &db_text);
    if let Some(o) = &p.organism {
        inv.opt("organism", o);
    }
    if p.report_common && (p.organism.is_none() || !p.plus) {
        return v::invalid("report_common", "report_common needs organism and plus (amrfinder refuses it otherwise)");
    }
    inv.flag("plus", p.plus);
    inv.flag("report_common", p.report_common);
    inv.flag("report_all_equal", p.report_all_equal);
    if let Some(x) = p.ident_min {
        if !(x == -1.0 || (0.0..=1.0).contains(&x)) {
            return v::invalid("ident_min", format!("{x} is not -1 or between 0 and 1"));
        }
        inv.opt("ident_min", x);
    }
    if let Some(x) = p.coverage_min {
        if !x.is_finite() || !(0.0..=1.0).contains(&x) {
            return v::invalid("coverage_min", format!("{x} is not between 0 and 1"));
        }
        inv.opt("coverage_min", x);
    }
    if let Some(t) = p.translation_table {
        v::range("translation_table", t, 1, 33)?;
        inv.opt("translation_table", t);
    }
    if let Some(n) = &p.name {
        v::text("name", n)?;
        inv.opt("name", n);
    }
    isotable_name(&mut inv, &p.output, "amrfinder", "s1.amrfinder.tsv");
    let out = v::output_file(ctx, "output", &p.output, p.overwrite)?;
    inv.outputs.push(out.clone());
    inv.opt("output", &p.output);
    inv.expect = vec![vec![out.clone()]];
    let mut written = vec![("output", out)];
    // amrfinder.cpp refuses the last two without their input; mutation_all without organism only
    // warns that no point mutations are screened, so the file would say nothing.
    for (field, val, have, need) in [
        ("mutation_all", &p.mutation_all, p.organism.is_some(), "organism"),
        ("protein_output", &p.protein_output, p.protein.is_some(), "protein"),
        ("nucleotide_output", &p.nucleotide_output, p.nucleotide.is_some(), "nucleotide"),
    ] {
        if val.is_some() && !have {
            return v::invalid(field, format!("{field} needs {need}"));
        }
        if let Some(path) = output_file_opt(ctx, &mut inv, field, val, p.overwrite, field)? {
            written.push((field, path));
        }
    }
    for (i, (field, a)) in written.iter().enumerate() {
        if written[..i].iter().any(|(_, b)| b == a) || inv.inputs.contains(a) {
            return v::invalid(field, format!("{} is used twice; give every input and output its own path", a.display()));
        }
    }
    inv.flag("print_node", p.print_node);
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    Ok(inv)
}

/// Read files for a tool that takes them as comma-separated lists (MEGAHIT): each must exist,
/// look like FASTQ/FASTA, and have no comma in its name.
fn read_list(ctx: &Ctx, inv: &mut Invocation, field: &str, values: &[String]) -> Check<String> {
    let mut seen = Vec::new();
    for value in values {
        if value.contains(',') {
            return v::invalid(field, format!("{value} has a comma, which MEGAHIT reads as a list separator; rename the file"));
        }
        let p = v::existing_file(&ctx.cwd, field, value)?;
        v::sequence_file(&p, field, value, Seq::Fastq)?;
        if seen.contains(&p) || inv.inputs.contains(&p) {
            return v::invalid(field, format!("{value} is given twice"));
        }
        seen.push(p);
    }
    inv.inputs.extend(seen);
    Ok(values.iter().map(|x| list_item(x)).collect::<Vec<_>>().join(","))
}

/// Paired read lists: as many read 2 files as read 1 files.
fn read_pairs(r1: &[String], r2: &[String]) -> Check<()> {
    if r1.len() != r2.len() {
        return v::invalid("r2", format!("{} read 1 file(s) but {} read 2 file(s); they pair up in order", r1.len(), r2.len()));
    }
    Ok(())
}

// ---------- Rasusa ----------
// Flags: rasusa 5.1.0 `rasusa reads --help` (clap, src/cli.rs and src/subcmd/reads.rs),
// https://github.com/mbhall88/rasusa; the bioconda rasusa=5.1.0 package gives the same list.
// Not offered: --one-pass / --probability (an approximate count), --strict, alignment formats
// (SAM/BAM/CRAM in or out), --threads (BAM only), -Z/-O/-l (the format comes from the file name).

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RasusaParams {
    /// Reads of one sample: one FASTQ/FASTA file (long reads, or single-end), or read 1 then read 2 of a pair. Plain or gzip.
    pub reads: Vec<String>,
    /// Output file(s), one per input in the same order, inside the root. `.gz` gives gzip output.
    pub outputs: Vec<String>,
    /// Target depth, e.g. 100. Needs genome_size. Give this, bases, num or frac.
    #[serde(default)]
    pub coverage: Option<f64>,
    /// Genome size for coverage, e.g. 5.5mb or 5500000.
    #[serde(default)]
    pub genome_size: Option<String>,
    /// Target number of bases, e.g. 550mb (instead of coverage).
    #[serde(default)]
    pub bases: Option<String>,
    /// Target number of reads (of each file for a pair), e.g. 50000.
    #[serde(default)]
    pub num: Option<u64>,
    /// Target fraction of the reads, between 0 and 1.
    #[serde(default)]
    pub frac: Option<f64>,
    /// Random seed. Default 1, so the same call keeps the same reads.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Replace existing output files. Default false.
    #[serde(default)]
    pub overwrite: bool,
}

/// A size the way rasusa reads it (`4.3kb`, `7Tb`, `9000`, `4.1MB`): a number, then an optional
/// b, kb, mb, gb or tb in any case.
fn rasusa_size(field: &str, s: &str) -> Check<()> {
    let lower = s.to_ascii_lowercase();
    let num = ["tb", "gb", "mb", "kb", "b"].iter().find_map(|u| lower.strip_suffix(u)).unwrap_or(&lower);
    match num.parse::<f64>() {
        Ok(n) if n.is_finite() && n > 0.0 && !num.starts_with(['+', '-']) => Ok(()),
        _ => v::invalid(field, format!("{s:?} is not a size like 5.5mb or 5500000")),
    }
}

pub fn rasusa(ctx: &Ctx, p: &RasusaParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("rasusa");
    inv.args.push("reads".into());
    v::non_empty_list("reads", &p.reads)?;
    if p.reads.len() > 2 {
        return v::invalid("reads", "one file, or read 1 and read 2 of one pair");
    }
    if p.outputs.len() != p.reads.len() {
        return v::invalid("outputs", format!("{} input file(s) but {} output(s); give one output per input", p.reads.len(), p.outputs.len()));
    }
    let mut ins: Vec<PathBuf> = Vec::new();
    for r in &p.reads {
        let path = v::existing_file(cwd, "reads", r)?;
        if ins.contains(&path) {
            return v::invalid("reads", format!("{r} is listed twice"));
        }
        if v::sequence_file(&path, "reads", r, Seq::Fastq).is_err() {
            v::sequence_file(&path, "reads", r, Seq::Fasta).map_err(|e| v::Invalid { message: format!("{r} is neither FASTQ nor FASTA ({})", e.message), ..e })?;
        }
        ins.push(path);
    }
    let targets = [p.coverage.is_some(), p.bases.is_some(), p.num.is_some(), p.frac.is_some()].iter().filter(|t| **t).count();
    if targets != 1 {
        return v::invalid("coverage", "give exactly one target: coverage (with genome_size), bases, num or frac");
    }
    if let Some(c) = p.coverage {
        let Some(g) = &p.genome_size else {
            return v::invalid("genome_size", "coverage needs genome_size, e.g. 5.5mb");
        };
        if !c.is_finite() || c <= 0.0 {
            return v::invalid("coverage", "a depth above 0, e.g. 100");
        }
        rasusa_size("genome_size", g)?;
        inv.opt("genome-size", g);
        inv.opt("coverage", c);
    } else if p.genome_size.is_some() {
        return v::invalid("genome_size", "genome_size is only used with coverage");
    }
    if let Some(b) = &p.bases {
        rasusa_size("bases", b)?;
        inv.opt("bases", b);
    }
    if let Some(n) = p.num {
        v::range("num", n, 1, u64::MAX)?;
        inv.opt("num", n);
    }
    if let Some(f) = p.frac {
        if !f.is_finite() || f <= 0.0 || f > 1.0 {
            return v::invalid("frac", "a fraction above 0 and at most 1, e.g. 0.25");
        }
        inv.opt("frac", f);
    }
    inv.opt("seed", p.seed.unwrap_or(1));
    let mut outs = Vec::new();
    for o in &p.outputs {
        let path = v::output_file(ctx, "outputs", o, p.overwrite)?;
        if ins.iter().any(|i| v::resolve(i) == v::resolve(&path)) || outs.contains(&path) {
            return v::invalid("outputs", format!("{o} is also an input or listed twice"));
        }
        inv.opt("output", o);
        outs.push(path);
    }
    inv.expect = outs.iter().map(|o| vec![o.clone()]).collect();
    inv.outputs.extend(outs);
    inv.inputs.extend(ins);
    inv.args.extend(p.reads.iter().map(|r| list_item(r)));
    Ok(inv)
}

// ---------- MEGAHIT ----------
// Flags: MEGAHIT v1.2.9 `src/megahit` (Python getopt in `parse_option`: short -1 -2 -r -m -o -t,
// long --out-dir= --memory= --k-list= ...; `--force` is accepted there though the usage text
// doesn't list it), https://github.com/voutcn/megahit/blob/v1.2.9/src/megahit

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum MegahitPreset {
    /// `--min-count 1 --k-list 21,29,...,141`: more sensitive for low-coverage members, slower.
    MetaSensitive,
    /// `--k-min 27 --k-max 127 --k-step 10`: large, complex metagenomes such as soil.
    MetaLarge,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MegahitParams {
    /// Read 1 FASTQ files (plain or gzip), paired with r2 in order. Several runs of one sample (or a co-assembly of several samples) go in one call.
    #[serde(default)]
    pub r1: Vec<String>,
    /// Read 2 FASTQ files, as many as r1.
    #[serde(default)]
    pub r2: Vec<String>,
    /// Single-end or merged reads (FASTQ, plain or gzip).
    #[serde(default)]
    pub single: Vec<String>,
    /// Output folder, inside the root. The assembly is `<outdir>/final.contigs.fa`.
    pub outdir: String,
    /// Allow an outdir that already has files (MEGAHIT writes over them). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Memory for building the de Bruijn graph, in GB (0.5-4096). Default 4. MEGAHIT's own default is 90% of the machine's RAM, which ignores a memory cap (cgroup, container) and gets it killed under one; keep this below the cap.
    #[serde(default)]
    pub memory_gb: Option<f64>,
    /// Threads. MEGAHIT's default is every logical CPU.
    #[serde(default)]
    pub threads: Option<u32>,
    #[serde(default)]
    pub presets: Option<MegahitPreset>,
    /// k-mer sizes, e.g. 21,29,39,59,79,99,119,141: odd, 15-255, ascending, steps of at most 28. Overrides the preset's k-mers.
    #[serde(default)]
    pub k_list: Option<String>,
    /// Drop contigs shorter than this. MEGAHIT's default 200.
    #[serde(default)]
    pub min_contig_len: Option<u32>,
    /// Minimum multiplicity for keeping a (k_min+1)-mer. MEGAHIT's default 2.
    #[serde(default)]
    pub min_count: Option<u32>,
}

/// MEGAHIT's default `--memory` (0.9: a fraction of the machine's memory) replaced by this many GB.
pub const MEGAHIT_DEFAULT_MEMORY_GB: f64 = 4.0;

pub fn megahit(ctx: &Ctx, p: &MegahitParams) -> Check<Invocation> {
    let mut inv = external("megahit");
    // The log and options.json carry times and paths; the contigs depend on thread scheduling
    // (not checked to be byte-identical), so every output is volatile, as with Shovill, and the
    // assembly's contig count, length and N50 are recorded instead.
    inv.volatile = &[""];
    if p.r1.is_empty() && p.single.is_empty() {
        return v::invalid("r1", "give paired reads (r1 and r2) and/or single reads");
    }
    read_pairs(&p.r1, &p.r2)?;
    let r1 = (!p.r1.is_empty()).then(|| read_list(ctx, &mut inv, "r1", &p.r1)).transpose()?;
    let r2 = (!p.r2.is_empty()).then(|| read_list(ctx, &mut inv, "r2", &p.r2)).transpose()?;
    let se = (!p.single.is_empty()).then(|| read_list(ctx, &mut inv, "single", &p.single)).transpose()?;
    // getopt takes the argument after a short option whatever it starts with.
    for (flag, list) in [("-1", r1), ("-2", r2), ("-r", se)] {
        if let Some(l) = list {
            inv.args.push(flag.into());
            inv.args.push(l);
        }
    }
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    inv.outputs.push(outdir.clone());
    inv.opt("out-dir", &p.outdir);
    // MEGAHIT refuses any existing folder, even an empty one, without --force (which only skips
    // that check). irit-mcp has refused a non-empty one unless overwrite is true.
    inv.flag("force", true);
    inv.expect = vec![vec![outdir.join("final.contigs.fa")]];
    inv.assembly = Some(outdir.join("final.contigs.fa"));
    let gb = p.memory_gb.unwrap_or(MEGAHIT_DEFAULT_MEMORY_GB);
    if !gb.is_finite() || !(0.5..=4096.0).contains(&gb) {
        return v::invalid("memory_gb", format!("{gb} is outside 0.5..=4096 GB"));
    }
    // In bytes: a value between 0 and 1 would be read as a fraction of the machine's memory.
    inv.opt("memory", (gb * 1e9).round() as u64);
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("num-cpu-threads", t);
    }
    if let Some(pr) = p.presets {
        inv.opt("presets", match pr {
            MegahitPreset::MetaSensitive => "meta-sensitive",
            MegahitPreset::MetaLarge => "meta-large",
        });
    }
    if let Some(k) = &p.k_list {
        let ks: Option<Vec<u32>> = k.split(',').map(|x| x.parse().ok()).collect();
        let ok = ks.as_ref().is_some_and(|ks| {
            !ks.is_empty()
                && ks.iter().all(|&k| k % 2 == 1 && (15..=255).contains(&k))
                && ks.windows(2).all(|w| w[1] > w[0] && w[1] - w[0] <= 28)
        });
        if !ok {
            return v::invalid("k_list", format!("{k:?} is not ascending odd k-mer sizes in 15-255, steps of at most 28 (e.g. 21,29,39)"));
        }
        inv.opt("k-list", k);
    }
    if let Some(m) = p.min_contig_len {
        inv.opt("min-contig-len", m);
    }
    if let Some(c) = p.min_count {
        v::range("min_count", c, 1, 1_000_000)?;
        inv.opt("min-count", c);
    }
    Ok(inv)
}

// ---------- CoverM ----------
// Flags: CoverM v0.8.0 `coverm contig --full-help` (src/cli.rs), https://github.com/wwood/CoverM/tree/v0.8.0

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CovermMapper {
    /// strobealign (CoverM's default): fast, low memory.
    Strobealign,
    /// minimap2 -x sr
    #[serde(rename = "minimap2-sr")]
    Minimap2Sr,
    /// bwa mem
    BwaMem,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CovermMethod {
    /// Mean coverage (CoverM's default).
    Mean,
    TrimmedMean,
    CoveredFraction,
    CoveredBases,
    Variance,
    Length,
    Count,
    ReadsPerBase,
    Rpkm,
    Tpm,
    /// MetaBAT's depth table (contigName, contigLen, totalAvgDepth, then depth and variance per sample): the `depth` input of metabat2. Only on its own.
    Metabat,
}

impl CovermMethod {
    fn name(self) -> &'static str {
        match self {
            CovermMethod::Mean => "mean",
            CovermMethod::TrimmedMean => "trimmed_mean",
            CovermMethod::CoveredFraction => "covered_fraction",
            CovermMethod::CoveredBases => "covered_bases",
            CovermMethod::Variance => "variance",
            CovermMethod::Length => "length",
            CovermMethod::Count => "count",
            CovermMethod::ReadsPerBase => "reads_per_base",
            CovermMethod::Rpkm => "rpkm",
            CovermMethod::Tpm => "tpm",
            CovermMethod::Metabat => "metabat",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CovermContigParams {
    /// Contigs FASTA (plain or gzip), e.g. MEGAHIT's final.contigs.fa.
    pub reference: String,
    /// Read 1 FASTQ files (plain or gzip), one per sample, paired with r2 in order. Each pair is a column of coverage.
    pub r1: Vec<String>,
    /// Read 2 FASTQ files, as many as r1.
    pub r2: Vec<String>,
    /// Output table (TSV), inside the root. For binning, name it e.g. `<sample>.depth.tsv` and use methods ["metabat"].
    pub output: String,
    /// Replace an existing output. Default false.
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub mapper: Option<CovermMapper>,
    /// Coverage methods. Default: CoverM's (mean). ["metabat"] writes MetaBAT2's depth table.
    #[serde(default)]
    pub methods: Vec<CovermMethod>,
    /// Exclude reads below this percent identity (e.g. 95). Default 0.
    #[serde(default)]
    pub min_read_percent_identity: Option<f64>,
    /// Exclude reads with less than this percent of their bases aligned (e.g. 75). Default 0.
    #[serde(default)]
    pub min_read_aligned_percent: Option<f64>,
    /// Threads for mapping and sorting. CoverM's default is 1.
    #[serde(default)]
    pub threads: Option<u32>,
    /// Also keep the sorted BAM of each sample in this folder (inside the root): `<folder>/<reference file name>.<read 1 file name>.bam`. The `bams` input of semibin2 (self-supervised training). About the size of the gzipped reads.
    #[serde(default)]
    pub bam_cache_dir: Option<String>,
}

/// The BAM CoverM 0.8.0 caches for one read pair with `--bam-file-cache-directory`:
/// `<dir>/<reference file name>.<read 1 file name>.bam` (sorted by coordinate).
pub fn coverm_cached_bam(dir: &Path, reference: &str, r1: &str) -> PathBuf {
    let name = |s: &str| Path::new(s).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dir.join(format!("{}.{}.bam", name(reference), name(r1)))
}

pub fn coverm_contig(ctx: &Ctx, p: &CovermContigParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("coverm");
    inv.args.push("contig".into());
    let reference = v::existing_file(cwd, "reference", &p.reference)?;
    v::sequence_file(&reference, "reference", &p.reference, Seq::Fasta)?;
    inv.inputs.push(reference.clone());
    inv.opt("reference", &p.reference);
    v::non_empty_list("r1", &p.r1)?;
    read_pairs(&p.r1, &p.r2)?;
    let mut reads = Vec::new();
    for (a, b) in p.r1.iter().zip(&p.r2) {
        for (field, value) in [("r1", a), ("r2", b)] {
            let path = v::existing_file(cwd, field, value)?;
            v::sequence_file(&path, field, value, Seq::Fastq)?;
            if inv.inputs.contains(&path) {
                return v::invalid(field, format!("{value} is given twice"));
            }
            inv.inputs.push(path);
            reads.push(list_item(value));
        }
    }
    let out = v::output_file(ctx, "output", &p.output, p.overwrite)?;
    if inv.inputs.contains(&out) {
        return v::invalid("output", "output is one of the inputs");
    }
    inv.outputs.push(out.clone());
    inv.opt("output-file", &p.output);
    inv.expect = vec![vec![out]];
    if let Some(m) = p.mapper {
        inv.opt("mapper", match m {
            CovermMapper::Strobealign => "strobealign",
            CovermMapper::Minimap2Sr => "minimap2-sr",
            CovermMapper::BwaMem => "bwa-mem",
        });
    }
    for (field, val) in [("min_read_percent_identity", p.min_read_percent_identity), ("min_read_aligned_percent", p.min_read_aligned_percent)] {
        if let Some(x) = val {
            if !x.is_finite() || !(0.0..=100.0).contains(&x) {
                return v::invalid(field, format!("{x} is not a percentage (0-100)"));
            }
            inv.opt(&field.replace('_', "-"), x);
        }
    }
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    if p.methods.contains(&CovermMethod::Metabat) && p.methods.len() > 1 {
        return v::invalid("methods", "metabat writes a table of its own and can't be combined with other methods");
    }
    if let Some(d) = &p.bam_cache_dir {
        let dir = v::output_dir(ctx, "bam_cache_dir", d, None, true)?;
        let mut bams: Vec<PathBuf> = Vec::new();
        for r1 in &p.r1 {
            let bam = coverm_cached_bam(&dir, &p.reference, r1);
            if bams.contains(&bam) {
                return v::invalid("bam_cache_dir", format!("two read 1 files are named {:?}; CoverM would cache both samples' BAMs as one file", bam.file_name().unwrap_or_default()));
            }
            if !p.overwrite && bam.exists() {
                return v::invalid("bam_cache_dir", format!("{} already exists; pass overwrite: true to replace it", bam.display()));
            }
            bams.push(bam);
        }
        // The BAM header records CoverM's temporary sort folder (`samtools sort -T /tmp/coverm_fifo.<random>/...`),
        // so the bytes differ between identical runs; the alignments don't.
        inv.volatile = &[".bam"];
        inv.expect.extend(bams.iter().map(|b| vec![b.clone()]));
        inv.outputs.extend(bams);
        inv.opt("bam-file-cache-directory", d);
    }
    // Both take several values (clap num_args 1..), which end at the next option: methods first,
    // then the reads, last, one value per argument.
    if !p.methods.is_empty() {
        inv.args.push("--methods".into());
        inv.args.extend(p.methods.iter().map(|m| m.name().to_string()));
    }
    inv.args.push("--coupled".into());
    inv.args.extend(reads);
    Ok(inv)
}

// ---------- MetaBAT2 ----------
// Flags: MetaBAT 2.18 `metabat2 --help` (src/metabat2.cpp, boost::program_options; it has no
// --version: irit-mcp reads the version from --help), https://bitbucket.org/berkeleylab/metabat

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Metabat2Params {
    /// Contigs FASTA (plain or gzip): the metagenome assembly.
    pub contigs: String,
    /// Depth table from coverm_contig with methods ["metabat"] (or jgi_summarize_bam_contig_depths). Without it MetaBAT2 bins on tetranucleotide frequency alone, which works poorly.
    #[serde(default)]
    pub depth: Option<String>,
    /// Folder for the bins, inside the root: `<outdir>/<prefix>.<n>.fa`, one FASTA per bin, ready for checkm2 and gtdbprep_ani/gtdbtk_classify_wf (extension fa).
    pub outdir: String,
    /// Allow an outdir that already has files (bins of the same name are replaced; others stay). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Bin file name prefix (letters, digits, _ . -). Default `bin`.
    #[serde(default)]
    pub prefix: Option<String>,
    /// Shortest contig to bin, at least 1500. MetaBAT2's default 2500.
    #[serde(default)]
    pub min_contig: Option<u32>,
    /// Smallest bin written, in bp. MetaBAT2's default 200000.
    #[serde(default)]
    pub min_bin_size: Option<u32>,
    /// Random seed, 1 or more. Default here 1: MetaBAT2's own default (0) draws a random seed, so its bins would differ between runs.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Threads. MetaBAT2's default is every core.
    #[serde(default)]
    pub threads: Option<u32>,
    /// Also write `<prefix>.unbinned.fa` with the contigs left out.
    #[serde(default)]
    pub unbinned: bool,
    /// Leave the per-sample depths out of the bin FASTA headers.
    #[serde(default)]
    pub no_sample_depths: bool,
}

/// The seed irit-mcp passes when none is given (MetaBAT2's 0 means a random one).
pub const METABAT2_DEFAULT_SEED: u64 = 1;

pub fn metabat2_prefix(p: &Metabat2Params) -> &str {
    p.prefix.as_deref().unwrap_or("bin")
}

pub fn metabat2(ctx: &Ctx, p: &Metabat2Params) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("metabat2");
    let contigs = v::existing_file(cwd, "contigs", &p.contigs)?;
    v::sequence_file(&contigs, "contigs", &p.contigs, Seq::Fasta)?;
    inv.inputs.push(contigs);
    inv.opt("inFile", &p.contigs);
    if let Some(d) = &p.depth {
        let depth = v::existing_file(cwd, "depth", d)?;
        let first = std::fs::read_to_string(&depth).ok().and_then(|t| t.lines().next().map(str::to_string)).unwrap_or_default();
        if !first.starts_with("contigName\tcontigLen\ttotalAvgDepth") {
            return v::invalid("depth", format!("{d} is not a MetaBAT depth table (its header must start contigName, contigLen, totalAvgDepth); make one with coverm_contig methods [\"metabat\"]"));
        }
        inv.inputs.push(depth);
        inv.opt("abdFile", d);
    }
    let prefix = metabat2_prefix(p);
    if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) || prefix.starts_with(['-', '.']) {
        return v::invalid("prefix", format!("{prefix:?} must be letters, digits and _ . - (not starting with - or .)"));
    }
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    inv.outputs.push(outdir.clone());
    // A file in outdir among the outputs, so the server creates the folder (MetaBAT2 doesn't).
    inv.outputs.push(outdir.join(format!("{prefix}.1.fa")));
    inv.opt("outFile", format!("{}/{prefix}", p.outdir.trim_end_matches('/')));
    if let Some(m) = p.min_contig {
        v::range("min_contig", m, 1500, u32::MAX)?;
        inv.opt("minContig", m);
    }
    if let Some(s) = p.min_bin_size {
        inv.opt("minClsSize", s);
    }
    let seed = p.seed.unwrap_or(METABAT2_DEFAULT_SEED);
    if seed == 0 {
        return v::invalid("seed", "0 makes MetaBAT2 draw a random seed; give 1 or more");
    }
    inv.opt("seed", seed);
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("numThreads", t);
    }
    inv.flag("unbinned", p.unbinned);
    inv.flag("noSampleDepths", p.no_sample_depths);
    Ok(inv)
}

/// MetaBAT2's bins in `outdir`: `<prefix>.<n>.fa`, in bin-number order.
pub fn metabat2_bins(outdir: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut bins: Vec<(u64, PathBuf)> = std::fs::read_dir(outdir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix(prefix)?.strip_prefix('.')?.strip_suffix(".fa")?.parse().ok()?;
            Some((n, e.path()))
        })
        .collect();
    bins.sort();
    bins.into_iter().map(|(_, p)| p).collect()
}

// ---------- CoverM genome ----------
// Flags: CoverM v0.8.0 `coverm genome --full-help` (src/cli.rs), https://github.com/wwood/CoverM/tree/v0.8.0

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CovermGenomeParams {
    /// One isolate's assembly (FASTA, plain or gzip), e.g. Shovill's `<outdir>/contigs.fa`. All its contigs count as one genome.
    pub assembly: String,
    /// The isolate's read 1 FASTQ (plain or gzip), e.g. fastp's trimmed reads.
    pub r1: String,
    /// Read 2 FASTQ.
    pub r2: String,
    /// Output table (TSV), inside the root. Name it `<sample>.coverm.tsv` so isotable joins it to the sample.
    pub output: String,
    /// Replace an existing output. Default false.
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub mapper: Option<CovermMapper>,
    /// Exclude reads below this percent identity (e.g. 95). Default 0.
    #[serde(default)]
    pub min_read_percent_identity: Option<f64>,
    /// Exclude reads with less than this percent of their bases aligned (e.g. 75). Default 0.
    #[serde(default)]
    pub min_read_aligned_percent: Option<f64>,
    /// Threads for mapping and sorting. CoverM's default is 1.
    #[serde(default)]
    pub threads: Option<u32>,
}

/// Mean depth and covered fraction of one isolate's reads mapped to its own assembly:
/// `coverm genome --single-genome`. `--min-covered-fraction=0`, because CoverM's default (10%)
/// reports a genome covered less than that as depth 0, which would hide a wrong read/assembly pair.
pub fn coverm_genome(ctx: &Ctx, p: &CovermGenomeParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("coverm");
    inv.args.push("genome".into());
    let reference = v::existing_file(cwd, "assembly", &p.assembly)?;
    v::sequence_file(&reference, "assembly", &p.assembly, Seq::Fasta)?;
    inv.inputs.push(reference);
    inv.opt("reference", &p.assembly);
    inv.flag("single-genome", true);
    let mut reads = Vec::new();
    for (field, value) in [("r1", &p.r1), ("r2", &p.r2)] {
        let path = v::existing_file(cwd, field, value)?;
        v::sequence_file(&path, field, value, Seq::Fastq)?;
        if inv.inputs.contains(&path) {
            return v::invalid(field, format!("{value} is given twice"));
        }
        inv.inputs.push(path);
        reads.push(list_item(value));
    }
    isotable_name(&mut inv, &p.output, "coverm", "s1.coverm.tsv");
    let out = v::output_file(ctx, "output", &p.output, p.overwrite)?;
    if inv.inputs.contains(&out) {
        return v::invalid("output", "output is one of the inputs");
    }
    inv.outputs.push(out.clone());
    inv.opt("output-file", &p.output);
    inv.expect = vec![vec![out]];
    inv.opt("min-covered-fraction", 0);
    if let Some(m) = p.mapper {
        inv.opt("mapper", match m {
            CovermMapper::Strobealign => "strobealign",
            CovermMapper::Minimap2Sr => "minimap2-sr",
            CovermMapper::BwaMem => "bwa-mem",
        });
    }
    for (field, val) in [("min_read_percent_identity", p.min_read_percent_identity), ("min_read_aligned_percent", p.min_read_aligned_percent)] {
        if let Some(x) = val {
            if !x.is_finite() || !(0.0..=100.0).contains(&x) {
                return v::invalid(field, format!("{x} is not a percentage (0-100)"));
            }
            inv.opt(&field.replace('_', "-"), x);
        }
    }
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    // Both take several values (clap num_args 1..), which end at the next option.
    inv.args.extend(["--methods", "mean", "covered_fraction", "--coupled"].map(String::from));
    inv.args.extend(reads);
    Ok(inv)
}

// ---------- SemiBin2 ----------
// Flags: SemiBin 2.6.0 `SemiBin2 single_easy_bin --help` (argparse, SemiBin/main.py `parse_args`; the
// environments are `get_model_path` in SemiBin/utils.py, the models in SemiBin/models/),
// https://github.com/BigDataBiology/SemiBin/tree/v2.6.0. Not offered: multi_easy_bin (it needs the
// samples' contigs concatenated with SemiBin2 concatenate_fasta and every sample mapped to that,
// a route of its own), --semi-supervised (needs mmseqs2 and GTDB, over 100 GB), --abundance
// (strobealign-aemb files, which SemiBin2 only takes from 5 samples on), --sequencing-type
// long_read, CRAM input, --write-pre-reclustering-bins, --no-recluster, the training and graph
// knobs (--epochs, --batch-size, --max-edges, --max-node, --ml-threshold, --ratio).

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SemibinEnvironment {
    HumanGut,
    DogGut,
    CatGut,
    MouseGut,
    PigGut,
    ChickenCaecum,
    HumanOral,
    Ocean,
    Soil,
    BuiltEnvironment,
    Wastewater,
    /// Trained across environments: the one to use when none of the others matches.
    Global,
}

impl SemibinEnvironment {
    pub fn name(self) -> &'static str {
        match self {
            SemibinEnvironment::HumanGut => "human_gut",
            SemibinEnvironment::DogGut => "dog_gut",
            SemibinEnvironment::CatGut => "cat_gut",
            SemibinEnvironment::MouseGut => "mouse_gut",
            SemibinEnvironment::PigGut => "pig_gut",
            SemibinEnvironment::ChickenCaecum => "chicken_caecum",
            SemibinEnvironment::HumanOral => "human_oral",
            SemibinEnvironment::Ocean => "ocean",
            SemibinEnvironment::Soil => "soil",
            SemibinEnvironment::BuiltEnvironment => "built_environment",
            SemibinEnvironment::Wastewater => "wastewater",
            SemibinEnvironment::Global => "global",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Semibin2Params {
    /// Contigs FASTA (plain or gzip): one sample's metagenome assembly, e.g. MEGAHIT's final.contigs.fa.
    pub contigs: String,
    /// Output folder, inside the root. Bins go to `<outdir>/output_bins/<tag>_<n>.fa` (plain FASTA, ready for checkm2 with extension fa); contig_bins.tsv lists each binned contig's bin.
    pub outdir: String,
    /// Allow an outdir that already has files (SemiBin2 replaces output_bins/ and its tables). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Use SemiBin2's pretrained model for this environment instead of training one. Needed with depth; with bams, leave it out to train on this sample (self-supervised).
    #[serde(default)]
    pub environment: Option<SemibinEnvironment>,
    /// One sample's MetaBAT depth table from coverm_contig with methods ["metabat"] and one read pair. Needs environment. Give this or bams.
    #[serde(default)]
    pub depth: Option<String>,
    /// Sorted BAM files of reads mapped to these contigs, one per sample: coverm_contig's bam_cache_dir. Without environment SemiBin2 trains its own model on them (self-supervised). With environment, exactly one.
    #[serde(default)]
    pub bams: Vec<String>,
    /// Bin file name prefix (letters, digits, _ . -). Default `SemiBin`. Give each sample its own (e.g. the sample name) when bins of several samples are dereplicated together: galah and CoverM name genomes after the file.
    #[serde(default)]
    pub tag: Option<String>,
    /// Shortest contig to bin, in bp (at least 500). Default: SemiBin2 picks 1000 or 2500 from the assembly.
    #[serde(default)]
    pub min_len: Option<u32>,
    /// Smallest bin written, in kb. SemiBin2's default 200.
    #[serde(default)]
    pub minfasta_kbs: Option<u32>,
    /// Random seed. Default here 1: without one SemiBin2's results may differ between runs.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Threads. SemiBin2's default is every CPU.
    #[serde(default)]
    pub threads: Option<u32>,
}

/// The seed irit-mcp passes when none is given.
pub const SEMIBIN2_DEFAULT_SEED: u64 = 1;

/// SemiBin2 2.6.0's self-supervised training holds min(500 x contigs, 4 million) cannot-link pairs
/// of feature rows in memory each epoch (SemiBin/self_supervised_model.py, `train_self`), whatever
/// the batch size. On the laptop, one sample with 4,842 contigs of 2.5 kb or more went over an 8 GB
/// cap in the first of 15 epochs (3 min each); the pretrained global model took 1.2 GiB.
const SEMIBIN2_TRAINING_MEMORY: &str = "no environment: SemiBin2 trains its own model (self-supervised), which holds up to 4 million contig pairs in memory; on an 8 GB laptop one sample with 4,842 contigs was killed at the cap after an epoch of 3 minutes (of 15). Under 8-16 GB use a pretrained environment (global when none matches), which took 1.2 GiB and 2 minutes";

pub fn semibin2_tag(p: &Semibin2Params) -> &str {
    p.tag.as_deref().unwrap_or("SemiBin")
}

/// A BAM file sorted by coordinate: BGZF, `BAM\1`, and `SO:coordinate` in the header text.
/// SemiBin2 2.6.0 computes coverage with `bedtools genomecov`, which needs that order.
fn sorted_bam(path: &Path, field: &str, value: &str) -> Check<()> {
    use std::io::Read;
    let mut head = vec![0u8; 1 << 16];
    let n = std::fs::File::open(path)
        .map(flate2::read::MultiGzDecoder::new)
        .and_then(|mut gz| {
            let mut got = 0;
            while got < head.len() {
                match gz.read(&mut head[got..])? {
                    0 => break,
                    k => got += k,
                }
            }
            Ok(got)
        })
        .unwrap_or(0);
    let head = &head[..n];
    if head.len() < 8 || &head[..4] != b"BAM\x01" {
        return v::invalid(field, format!("{value} is not a BAM file"));
    }
    let text_len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    let text = String::from_utf8_lossy(&head[8..head.len().min(8 + text_len)]);
    if !text.lines().next().is_some_and(|l| l.starts_with("@HD") && l.contains("SO:coordinate")) {
        return v::invalid(field, format!("{value} is not sorted by coordinate (no SO:coordinate in its @HD line); coverm_contig's bam_cache_dir writes sorted BAMs"));
    }
    Ok(())
}

pub fn semibin2(ctx: &Ctx, p: &Semibin2Params) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("SemiBin2");
    // Its log carries times and the process id.
    inv.volatile = &["SemiBinRun.log"];
    inv.args.push("single_easy_bin".into());
    let contigs = v::existing_file(cwd, "contigs", &p.contigs)?;
    v::sequence_file(&contigs, "contigs", &p.contigs, Seq::Fasta)?;
    inv.inputs.push(contigs);
    inv.opt("input-fasta", &p.contigs);
    match (&p.depth, p.bams.is_empty()) {
        (Some(_), false) | (None, true) => {
            return v::invalid("depth", "give either depth (coverm_contig's metabat table, with environment) or bams (coverm_contig's bam_cache_dir)");
        }
        (Some(d), true) => {
            if p.environment.is_none() {
                return v::invalid("environment", "SemiBin2 only bins from a MetaBAT depth table with a pretrained model: set environment (global when none matches), or give bams to train one");
            }
            let depth = v::existing_file(cwd, "depth", d)?;
            let first = std::fs::read_to_string(&depth).ok().and_then(|t| t.lines().next().map(str::to_string)).unwrap_or_default();
            if !first.starts_with("contigName\tcontigLen\ttotalAvgDepth") {
                return v::invalid("depth", format!("{d} is not a MetaBAT depth table; make one with coverm_contig methods [\"metabat\"]"));
            }
            // contigName, contigLen, totalAvgDepth, then depth and variance per sample
            if first.split('\t').count() != 5 {
                return v::invalid(
                    "depth",
                    format!("{d} has {} samples; SemiBin2's pretrained models take one sample's depth (coverm_contig with one read pair)", (first.split('\t').count().saturating_sub(3)) / 2),
                );
            }
            inv.inputs.push(depth);
            inv.opt("depth-metabat2", d);
        }
        (None, false) => {
            if p.environment.is_some() && p.bams.len() > 1 {
                return v::invalid("bams", "SemiBin2's pretrained models take one sample: give one BAM, or leave environment out to train on several");
            }
            for b in &p.bams {
                let path = v::existing_file(cwd, "bams", b)?;
                sorted_bam(&path, "bams", b)?;
                if inv.inputs.contains(&path) {
                    return v::invalid("bams", format!("{b} is given twice"));
                }
                inv.inputs.push(path);
            }
        }
    }
    match p.environment {
        Some(e) => inv.opt("environment", e.name()),
        None => inv.warnings.push(SEMIBIN2_TRAINING_MEMORY.into()),
    }
    let tag = semibin2_tag(p);
    v::file_stem("tag", tag)?;
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    inv.outputs.push(outdir.clone());
    inv.opt("output", &p.outdir);
    inv.expect = vec![vec![outdir.join("contig_bins.tsv")]];
    if p.tag.is_some() {
        inv.opt("tag-output", tag);
    }
    // Plain FASTA (SemiBin2's default is gzip), as MetaBAT2 writes, for checkm2 and galah.
    inv.opt("compression", "none");
    // The CPU even where a GPU is found, so the same call gives the same bins on any machine.
    inv.opt("engine", "cpu");
    if let Some(m) = p.min_len {
        v::range("min_len", m, 500, u32::MAX)?;
        inv.opt("min-len", m);
    }
    if let Some(k) = p.minfasta_kbs {
        v::range("minfasta_kbs", k, 1, 1_000_000)?;
        inv.opt("minfasta-kbs", k);
    }
    inv.opt("random-seed", p.seed.unwrap_or(SEMIBIN2_DEFAULT_SEED));
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    // -b takes several values (nargs='*'): last, one value per argument.
    if !p.bams.is_empty() {
        inv.args.push("--input-bam".into());
        inv.args.extend(p.bams.iter().map(|b| list_item(b)));
    }
    Ok(inv)
}

/// SemiBin2's bins: `<outdir>/output_bins/<tag>_<n>.fa`, in bin-number order.
pub fn semibin2_bins(outdir: &Path, tag: &str) -> Vec<PathBuf> {
    let mut bins: Vec<(u64, PathBuf)> = std::fs::read_dir(outdir.join("output_bins"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix(tag)?.strip_prefix('_')?.strip_suffix(".fa")?.parse().ok()?;
            Some((n, e.path()))
        })
        .collect();
    bins.sort();
    bins.into_iter().map(|(_, p)| p).collect()
}

/// Genome FASTA files given as files and/or folders (matched by extension), each checked; with
/// the name each genome gets in galah's and CoverM's tables (the file name without its FASTA
/// extension), which must be unique.
fn genome_files(ctx: &Ctx, genomes: &[String], dirs: &[String], extension: &Option<String>) -> Check<Vec<(PathBuf, String, String)>> {
    let cwd = ctx.cwd.as_path();
    if genomes.is_empty() && dirs.is_empty() {
        return v::invalid("genomes", "give genome FASTA files (genomes) and/or folders of them (genome_dirs)");
    }
    if dirs.is_empty() && extension.is_some() {
        return v::invalid("extension", "extension applies to genome_dirs only");
    }
    let mut out: Vec<(PathBuf, String, String)> = Vec::new();
    for g in genomes {
        let path = v::existing_file(cwd, "genomes", g)?;
        out.push((path, g.clone(), String::new()));
    }
    let ext = extension.as_deref().unwrap_or("fa");
    v::extension("extension", ext)?;
    for d in dirs {
        let dir = v::existing_dir(cwd, "genome_dirs", d)?;
        let files = v::files_with_extension(&dir, ext);
        if files.is_empty() {
            return v::invalid("genome_dirs", format!("no *.{} files in {d}; set extension (e.g. fna, fasta)", ext.trim_start_matches('.')));
        }
        for f in files {
            let shown = format!("{}/{}", d.trim_end_matches('/'), f.file_name().unwrap_or_default().to_string_lossy());
            out.push((f, shown, String::new()));
        }
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    for (path, shown, name) in out.iter_mut() {
        v::sequence_file(path, "genomes", shown, Seq::Fasta)?;
        if seen.contains(path) {
            return v::invalid("genomes", format!("{shown} is given twice"));
        }
        seen.push(path.clone());
        *name = kleborate_strain(shown);
    }
    for (i, (_, shown, name)) in out.iter().enumerate() {
        if let Some((_, other, _)) = out[..i].iter().find(|(_, _, n)| n == name) {
            return v::invalid(
                "genomes",
                format!("{other} and {shown} would both be called {name:?} (genomes are named after the file); bins of different samples need different names: metabat2 prefix or semibin2 tag per sample"),
            );
        }
    }
    Ok(out)
}

// ---------- galah ----------
// Flags: galah 0.6.3 `galah cluster --full-help` (clap, src/cluster_argument_parsing.rs),
// https://github.com/wwood/galah/tree/v0.6.3. galah takes one --checkm2-quality-report, and panics
// when a genome is missing from it, so irit-mcp checks that first. Not offered: --run-checkm2
// (irit-mcp's checkm2 does that, with lowmem), --checkm-tab-table and --genome-info (CheckM 1, dRep),
// the dRep quality formula (it needs CheckM 1's strain heterogeneity), --cluster-method fastani and
// --precluster-method finch, contig clustering, --reference-genomes, --low-memory.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
pub enum GalahQualityFormula {
    /// galah's default: completeness - 5 x contamination - 5 x contigs/100 - 5 x ambiguous bases/100000 (Parks et al. 2020, reduced).
    #[serde(rename = "Parks2020_reduced")]
    Parks2020Reduced,
    #[serde(rename = "completeness-4contamination")]
    Completeness4Contamination,
    #[serde(rename = "completeness-5contamination")]
    Completeness5Contamination,
}

impl GalahQualityFormula {
    fn name(self) -> &'static str {
        match self {
            GalahQualityFormula::Parks2020Reduced => "Parks2020_reduced",
            GalahQualityFormula::Completeness4Contamination => "completeness-4contamination",
            GalahQualityFormula::Completeness5Contamination => "completeness-5contamination",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GalahParams {
    /// Genome FASTA files (plain or gzip), e.g. bins of several samples. Give this and/or genome_dirs.
    #[serde(default)]
    pub genomes: Vec<String>,
    /// Folders of genome FASTA files matched by extension, e.g. one metabat2 or semibin2 bin folder per sample.
    #[serde(default)]
    pub genome_dirs: Vec<String>,
    /// With genome_dirs: file extension to match. Default fa (MetaBAT2's and SemiBin2's bins).
    #[serde(default)]
    pub extension: Option<String>,
    /// Output folder, inside the root: clusters.tsv (representative, member), representatives.txt, and copies of the representative genomes in representatives/ (for coverm_abundance genome_dirs). Its representatives/ must be missing or empty.
    pub outdir: String,
    /// Allow an outdir that already has files (clusters.tsv and representatives.txt are replaced). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// ANI to dereplicate at, in percent. Default 95 (a species, as GTDB draws them).
    #[serde(default)]
    pub ani: Option<f64>,
    /// Smallest aligned fraction of two genomes, in percent, to cluster them. galah's default 15.
    #[serde(default)]
    pub min_aligned_fraction: Option<f64>,
    /// Precluster ANI, in percent, at most ani. galah's default 90.
    #[serde(default)]
    pub precluster_ani: Option<f64>,
    /// CheckM2's quality_report.tsv covering every genome (one checkm2 call over all of them): the best genome of each cluster becomes its representative, by quality_formula. Without it, the first given.
    #[serde(default)]
    pub checkm2_report: Option<String>,
    /// Leave out genomes below this completeness (percent). Needs checkm2_report.
    #[serde(default)]
    pub min_completeness: Option<f64>,
    /// Leave out genomes above this contamination (percent). Needs checkm2_report.
    #[serde(default)]
    pub max_contamination: Option<f64>,
    #[serde(default)]
    pub quality_formula: Option<GalahQualityFormula>,
    /// skani's settings for small genomes (under about 20 kb).
    #[serde(default)]
    pub small_genomes: bool,
    /// Threads. Default 1: with more, galah writes clusters.tsv and representatives.txt in a different order each run (same clusters), so replay can only check they exist.
    #[serde(default)]
    pub threads: Option<u32>,
}

/// Genome names in a CheckM2 quality_report.tsv (its Name column).
fn checkm2_report_names(path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or("").split('\t').collect();
    let need = |c: &str| header.iter().position(|h| *h == c).ok_or_else(|| format!("no {c} column; not a CheckM2 quality_report.tsv"));
    let name = need("Name")?;
    need("Completeness")?;
    need("Contamination")?;
    Ok(lines.filter_map(|l| l.split('\t').nth(name)).filter(|n| !n.is_empty()).map(str::to_string).collect())
}

pub fn galah(ctx: &Ctx, p: &GalahParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("galah");
    inv.args.push("cluster".into());
    let genomes = genome_files(ctx, &p.genomes, &p.genome_dirs, &p.extension)?;
    let ani = p.ani.unwrap_or(95.0);
    for (field, x) in [("ani", Some(ani)), ("min_aligned_fraction", p.min_aligned_fraction), ("precluster_ani", p.precluster_ani)] {
        if let Some(x) = x {
            // CoverM/galah read values up to 1 as fractions: irit-mcp takes percentages only.
            if !x.is_finite() || !(1.0..=100.0).contains(&x) {
                return v::invalid(field, format!("{x} is not a percentage between 1 and 100 (e.g. 95)"));
            }
        }
    }
    if ani < 80.0 {
        return v::invalid("ani", format!("{ani}% is below what skani estimates reliably (about 80%); species are about 95%"));
    }
    if p.precluster_ani.is_some_and(|pc| pc > ani) {
        return v::invalid("precluster_ani", "precluster_ani must not be above ani");
    }
    inv.opt("ani", ani);
    if let Some(x) = p.min_aligned_fraction {
        inv.opt("min-aligned-fraction", x);
    }
    if let Some(x) = p.precluster_ani {
        inv.opt("precluster-ani", x);
    }
    match &p.checkm2_report {
        Some(r) => {
            let path = v::existing_file(cwd, "checkm2_report", r)?;
            let names = checkm2_report_names(&path).or_else(|e| v::invalid("checkm2_report", format!("{r}: {e}")))?;
            let missing: Vec<&str> = genomes.iter().map(|(_, _, n)| n.as_str()).filter(|n| !names.iter().any(|x| x == n)).collect();
            if !missing.is_empty() {
                return v::invalid(
                    "checkm2_report",
                    format!(
                        "{r} has no row for {} of the genomes ({}{}); galah stops on that. Run checkm2 once over all of them (genomes list)",
                        missing.len(),
                        missing.iter().take(5).copied().collect::<Vec<_>>().join(", "),
                        if missing.len() > 5 { ", ..." } else { "" }
                    ),
                );
            }
            inv.inputs.push(path);
            inv.opt("checkm2-quality-report", r);
        }
        None => {
            if p.min_completeness.is_some() || p.max_contamination.is_some() || p.quality_formula.is_some() {
                return v::invalid("checkm2_report", "min_completeness, max_contamination and quality_formula need checkm2_report");
            }
            inv.warnings.push("no checkm2_report: galah picks the first genome given in each cluster as its representative, not the best one".into());
        }
    }
    for (field, flag, x) in [("min_completeness", "min-completeness", p.min_completeness), ("max_contamination", "max-contamination", p.max_contamination)] {
        if let Some(x) = x {
            v::percent(field, x)?;
            inv.opt(flag, x);
        }
    }
    if let Some(f) = p.quality_formula {
        inv.opt("quality-formula", f.name());
    }
    inv.flag("small-genomes", p.small_genomes);
    let threads = p.threads.unwrap_or(1);
    v::range("threads", threads, 1, 1024)?;
    inv.opt("threads", threads);
    if threads > 1 {
        inv.volatile = &["clusters.tsv", "representatives.txt"];
    }
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    let reps = outdir.join("representatives");
    if std::fs::read_dir(&reps).is_ok_and(|mut rd| rd.next().is_some()) {
        return v::invalid("outdir", format!("{}/representatives already has files, and galah refuses to copy into it; remove it or pick another outdir", p.outdir.trim_end_matches('/')));
    }
    let base = p.outdir.trim_end_matches('/');
    let (clusters, list) = (outdir.join("clusters.tsv"), outdir.join("representatives.txt"));
    for (out, flag, name) in
        [(&clusters, "output-cluster-definition", "clusters.tsv"), (&list, "output-representative-list", "representatives.txt")]
    {
        if !p.overwrite && out.exists() {
            return v::invalid("outdir", format!("{base}/{name} already exists; pass overwrite: true"));
        }
        inv.opt(flag, format!("{base}/{name}"));
    }
    inv.opt("output-representative-fasta-directory-copy", format!("{base}/representatives"));
    inv.outputs.extend([clusters.clone(), list.clone(), reps]);
    inv.expect = vec![vec![clusters], vec![list]];
    for (path, _, _) in &genomes {
        inv.inputs.push(path.clone());
    }
    inv.args.push("--genome-fasta-files".into());
    inv.args.extend(genomes.iter().map(|(_, shown, _)| list_item(shown)));
    Ok(inv)
}

// ---------- CoverM abundance (genome mode, many genomes and samples) ----------
// Flags: CoverM v0.8.0 `coverm genome --full-help` (src/cli.rs), https://github.com/wwood/CoverM/tree/v0.8.0.
// CoverM 0.8.0 refuses the count method unless --min-covered-fraction is 0 ("does not calculate the
// covered fraction"). Not offered: --dereplicate (galah does that as its own recorded step),
// --sharded, BAM input, sparse output (the co-occurrence step reads the dense table).

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CovermAbundanceParams {
    /// Genome FASTA files, e.g. galah's representatives. Give this and/or genome_dirs. Give all the genomes at once (a dereplicated set): reads are shared out among the genomes given, and relative abundance is a share of all reads.
    #[serde(default)]
    pub genomes: Vec<String>,
    /// Folders of genome FASTA files matched by extension, e.g. galah's `<outdir>/representatives`.
    #[serde(default)]
    pub genome_dirs: Vec<String>,
    /// With genome_dirs: file extension to match. Default fa (MetaBAT2's and SemiBin2's bins).
    #[serde(default)]
    pub extension: Option<String>,
    /// Read 1 FASTQ files (plain or gzip), one per sample, paired with r2 in order. Each sample's columns are named after its read 1 file name, so name the files after the samples.
    pub r1: Vec<String>,
    /// Read 2 FASTQ files, as many as r1.
    pub r2: Vec<String>,
    /// Output table (TSV), inside the root: one row per genome plus `unmapped`; per sample `<read 1 file> Relative Abundance (%)`, `Mean`, `Covered Fraction` and `Read Count` columns.
    pub output: String,
    /// Replace an existing output. Default false.
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub mapper: Option<CovermMapper>,
    /// Exclude reads below this percent identity (e.g. 95). Default 0.
    #[serde(default)]
    pub min_read_percent_identity: Option<f64>,
    /// Exclude reads with less than this percent of their bases aligned (e.g. 75). Default 0.
    #[serde(default)]
    pub min_read_aligned_percent: Option<f64>,
    /// Report a genome covered less than this (percent) as 0 in that sample. Default here 0 (CoverM's own is 10), because CoverM leaves out the read Count column otherwise; the Covered Fraction column lets a later step apply a threshold.
    #[serde(default)]
    pub min_covered_fraction: Option<f64>,
    /// Threads for mapping and sorting. CoverM's default is 1.
    #[serde(default)]
    pub threads: Option<u32>,
}

/// The methods coverm_abundance asks for: count only when CoverM allows it (min covered fraction 0).
pub fn coverm_abundance_methods(p: &CovermAbundanceParams) -> &'static [&'static str] {
    if p.min_covered_fraction.unwrap_or(0.0) > 0.0 {
        &["relative_abundance", "mean", "covered_fraction"]
    } else {
        &["relative_abundance", "mean", "covered_fraction", "count"]
    }
}

pub fn coverm_abundance(ctx: &Ctx, p: &CovermAbundanceParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("coverm");
    inv.args.push("genome".into());
    let genomes = genome_files(ctx, &p.genomes, &p.genome_dirs, &p.extension)?;
    for (path, _, _) in &genomes {
        inv.inputs.push(path.clone());
    }
    v::non_empty_list("r1", &p.r1)?;
    read_pairs(&p.r1, &p.r2)?;
    let mut reads = Vec::new();
    let mut samples: Vec<String> = Vec::new();
    for (a, b) in p.r1.iter().zip(&p.r2) {
        let sample = Path::new(a).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if samples.contains(&sample) {
            return v::invalid("r1", format!("two read 1 files are named {sample:?}; CoverM names each sample's columns after it, so they would clash"));
        }
        samples.push(sample);
        for (field, value) in [("r1", a), ("r2", b)] {
            let path = v::existing_file(cwd, field, value)?;
            v::sequence_file(&path, field, value, Seq::Fastq)?;
            if inv.inputs.contains(&path) {
                return v::invalid(field, format!("{value} is given twice"));
            }
            inv.inputs.push(path);
            reads.push(list_item(value));
        }
    }
    let out = v::output_file(ctx, "output", &p.output, p.overwrite)?;
    if inv.inputs.contains(&out) {
        return v::invalid("output", "output is one of the inputs");
    }
    inv.outputs.push(out.clone());
    inv.opt("output-file", &p.output);
    inv.expect = vec![vec![out]];
    let mcf = p.min_covered_fraction.unwrap_or(0.0);
    v::percent("min_covered_fraction", mcf)?;
    if mcf > 0.0 && mcf < 1.0 {
        // CoverM reads a value below 1 as a fraction (0.5 = 50%).
        return v::invalid("min_covered_fraction", format!("{mcf} would be read by CoverM as a fraction ({}%); give a percentage of at least 1, or 0", mcf * 100.0));
    }
    inv.opt("min-covered-fraction", mcf);
    if mcf > 0.0 {
        inv.warnings.push(format!(
            "min_covered_fraction {mcf}: CoverM 0.8.0 can't count reads then, so the table has no Read Count columns; the reads of genomes it reports as 0 count as unmapped"
        ));
    }
    if let Some(m) = p.mapper {
        inv.opt("mapper", match m {
            CovermMapper::Strobealign => "strobealign",
            CovermMapper::Minimap2Sr => "minimap2-sr",
            CovermMapper::BwaMem => "bwa-mem",
        });
    }
    for (field, val) in [("min_read_percent_identity", p.min_read_percent_identity), ("min_read_aligned_percent", p.min_read_aligned_percent)] {
        if let Some(x) = val {
            if !x.is_finite() || !(0.0..=100.0).contains(&x) {
                return v::invalid(field, format!("{x} is not a percentage (0-100)"));
            }
            inv.opt(&field.replace('_', "-"), x);
        }
    }
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    // Each takes several values (clap num_args 1..), which end at the next option.
    inv.args.push("--genome-fasta-files".into());
    inv.args.extend(genomes.iter().map(|(_, shown, _)| list_item(shown)));
    inv.args.push("--methods".into());
    inv.args.extend(coverm_abundance_methods(p).iter().map(|m| m.to_string()));
    inv.args.push("--coupled".into());
    inv.args.extend(reads);
    Ok(inv)
}

// ---------- Kleborate ----------
// Flags: Kleborate v3.2.4 `kleborate --help` and kleborate/__main__.py (argparse; `--outdir` must be
// a separate word: main() looks for `-o`/`--outdir` in argv before parsing, so `--outdir=x` is
// refused), https://github.com/klebgenomics/Kleborate/tree/v3.2.4. Not offered: --resume (appends
// to an existing table), --trim_headers (isotable reads either; the full headers name the module),
// the per-module options of --help_all.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KleboratePreset {
    /// K. pneumoniae species complex: species, MLST, ybt/clb/iuc/iro/rmp loci, virulence and resistance scores, AMR, Kaptive K/O loci. Writes klebsiella_pneumo_complex_output.txt.
    Kpsc,
    /// K. oxytoca species complex: species, MLST and the virulence loci. Writes klebsiella_oxytoca_complex_output.txt.
    Kosc,
    /// Escherichia: species, MLST, pathotype, ECTyper serotype (its first run downloads about 900 MB), stx, AMR. Writes escherichia_output.txt.
    Escherichia,
}

impl KleboratePreset {
    pub fn name(self) -> &'static str {
        match self {
            KleboratePreset::Kpsc => "kpsc",
            KleboratePreset::Kosc => "kosc",
            KleboratePreset::Escherichia => "escherichia",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KleborateParams {
    /// Assemblies (FASTA, plain or gzip). Kleborate names each row after the file name without its extension, which isotable joins on: give files named after the sample (shovill's link_as, e.g. genomes/s1.fna), not contigs.fa.
    pub assemblies: Vec<String>,
    /// Output folder, inside the root. Kleborate writes each assembly to the table of its species: `<outdir>/klebsiella_pneumo_complex_output.txt` for the K. pneumoniae complex.
    pub outdir: String,
    /// Allow an outdir that already has files (Kleborate replaces its tables there). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Module preset. Default kpsc, unless modules is given alone.
    #[serde(default)]
    pub preset: Option<KleboratePreset>,
    /// Kleborate modules to run as well as the preset's (or alone, without preset), e.g. klebsiella__ybst; `kleborate --list_modules` lists them. With modules, Kleborate names its table `<first module>_output.txt`, which isotable doesn't read.
    #[serde(default)]
    pub modules: Vec<String>,
}

/// File names that are the same for every sample (assemblers' fixed outputs): a Kleborate row
/// named after one can't be joined to a sample.
const GENERIC_ASSEMBLY_NAMES: [&str; 6] = ["contigs", "assembly", "consensus", "final.contigs", "scaffolds", "spades"];

/// Kleborate's row name for an assembly: the file name without `.gz` and its FASTA extension.
fn kleborate_strain(value: &str) -> String {
    let name = Path::new(value).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name = name.strip_suffix(".gz").unwrap_or(&name);
    [".fasta", ".fna", ".fas", ".fa"].iter().find_map(|e| name.strip_suffix(e)).unwrap_or(name).to_string()
}

pub fn kleborate(ctx: &Ctx, p: &KleborateParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("kleborate");
    inv.bundled_data = Some(kleborate_data);
    v::non_empty_list("assemblies", &p.assemblies)?;
    let mut strains: Vec<String> = Vec::new();
    for a in &p.assemblies {
        let path = v::existing_file(cwd, "assemblies", a)?;
        v::sequence_file(&path, "assemblies", a, Seq::Fasta)?;
        let strain = kleborate_strain(a);
        if GENERIC_ASSEMBLY_NAMES.contains(&strain.as_str()) {
            return v::invalid(
                "assemblies",
                format!("{a}: Kleborate would name the row {strain:?}, which says nothing about the sample; give the assembly a sample name (shovill's link_as, e.g. genomes/s1.fna)"),
            );
        }
        if strains.contains(&strain) {
            return v::invalid("assemblies", format!("two assemblies would both be named {strain:?} in Kleborate's table"));
        }
        strains.push(strain);
        inv.inputs.push(path);
    }
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    for m in &p.modules {
        if m.is_empty() || !m.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return v::invalid("modules", format!("{m:?} is not a Kleborate module name like klebsiella__ybst"));
        }
    }
    let preset = match (p.preset, p.modules.is_empty()) {
        (None, true) => Some(KleboratePreset::Kpsc),
        (pr, _) => pr,
    };
    // No table is certain: Kleborate writes each assembly to its species' table, or skips it
    // (see `kleborate_tables`); result_warnings names the assemblies without a row.
    match p.modules.first() {
        Some(m) => inv.outputs.push(outdir.join(format!("{m}_output.txt"))),
        None => inv.outputs.extend(isotable::parse::KLEBORATE_OUTPUTS.iter().map(|t| outdir.join(t))),
    }
    inv.outputs.push(outdir);
    inv.args.push("--outdir".into());
    inv.args.push(list_item(&p.outdir));
    if let Some(pr) = preset {
        inv.opt("preset", pr.name());
    }
    if !p.modules.is_empty() {
        inv.opt("modules", p.modules.join(","));
    }
    // nargs='+': last, one value per argument
    inv.args.push("--assemblies".into());
    inv.args.extend(p.assemblies.iter().map(|a| list_item(a)));
    Ok(inv)
}

/// A shell wrapper that runs the real program (`exec /path/to/env/bin/kleborate "$@"`, as the
/// real-tools workflow makes them): the path it runs.
fn wrapped_program(program: &Path) -> Option<PathBuf> {
    if std::fs::metadata(program).ok()?.len() > 4096 {
        return None;
    }
    let text = std::fs::read_to_string(program).ok()?;
    if !text.starts_with("#!") || !text.lines().next()?.contains("sh") {
        return None;
    }
    text.lines().filter_map(|l| l.trim().strip_prefix("exec ")).find_map(|rest| {
        let target = PathBuf::from(rest.split_whitespace().next()?.trim_matches(['"', '\'']));
        (target.is_absolute() && target.is_file()).then_some(target)
    })
}

/// The databases Kleborate 3 types with: they ship inside its Python package
/// (`kleborate/modules/*/data`: Mash sketches, MLST alleles and profiles, AMR genes) and Kaptive's
/// (`kaptive/data`: the K and O locus references), next to the program in
/// `<prefix>/lib/python3.*/site-packages`. Kaptive's METADATA is hashed too, which records its
/// version (Kleborate 3.2.4 doesn't run with Kaptive 3.3).
pub fn kleborate_data(program: &Path, _ctx: &Ctx) -> Vec<PathBuf> {
    let Ok(mut real) = std::fs::canonicalize(program) else { return Vec::new() };
    if let Some(inner) = wrapped_program(&real).and_then(|p| std::fs::canonicalize(p).ok()) {
        real = inner;
    }
    let Some(prefix) = real.parent().and_then(Path::parent) else { return Vec::new() };
    let mut files = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_none_or(|n| n != "__pycache__") {
                    walk(&p, out);
                }
            } else if !p.extension().is_some_and(|x| x == "py" || x == "pyc") {
                out.push(p);
            }
        }
    }
    for lib in std::fs::read_dir(prefix.join("lib")).into_iter().flatten().flatten() {
        if !lib.file_name().to_string_lossy().starts_with("python3") {
            continue;
        }
        let site = lib.path().join("site-packages");
        walk(&site.join("kleborate/modules"), &mut files);
        walk(&site.join("kaptive/data"), &mut files);
        for e in std::fs::read_dir(&site).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("kaptive-") && n.ends_with(".dist-info") {
                files.push(e.path().join("METADATA"));
            }
        }
    }
    files.retain(|p| p.is_file());
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn ctx(dir: &Path) -> Ctx {
        let root = std::fs::canonicalize(dir).unwrap();
        Ctx { cwd: root.clone(), runs_root: Some(root.join("irit-runs")), root, allow_outside_root: false, vars: BTreeMap::new() }
    }

    fn parse<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
        serde_json::from_value(v).unwrap()
    }

    fn write(dir: &Path, name: &str, text: &str) {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    pub fn fake_bakta_db(dir: &Path, major: u32) {
        write(dir, "version.json", &format!(r#"{{"date": "2025-02-24", "major": {major}, "minor": 0, "type": "light"}}"#));
        for f in BAKTA_DB_FILES.iter().chain(&["pscc.dmnd"]) {
            write(dir, f, "x");
        }
    }

    pub fn fake_gtdbtk_data(dir: &Path, with_skani: bool) {
        write(dir, "metadata/metadata.txt", "VERSION_DATA=r232\n");
        for d in GTDBTK_DIRS {
            if d != "skani" || with_skani {
                std::fs::create_dir_all(dir.join(d)).unwrap();
            }
        }
        std::fs::create_dir_all(dir.join(GTDBTK_SPLIT_FILES[0])).unwrap();
        write(dir, GTDBTK_SPLIT_FILES[1], "");
    }

    #[test]
    fn versions() {
        assert_eq!(version_number("gtdbtk: version 2.8.0 Copyright 2017 Pierre-Alain Chaumeil"), Some("2.8.0"));
        assert_eq!(version_number("bakta 1.12.2"), Some("1.12.2"));
        assert_eq!(version_number("1.1.0"), Some("1.1.0"));
        assert_eq!(version_number("fastp v1.4.0"), Some("1.4.0"));
        assert_eq!(version_number("no version"), None);
        assert!(version_warning("fastp", Some("fastp 1.4.0")).is_none());
        assert!(version_warning("fastp", Some("fastp 0.23.4")).unwrap().contains("0.23.4"));
        assert!(version_warning("fastp", None).is_some());
        assert!(install_hint("bakta").contains("bioconda bakta=1.12.2"));
    }

    #[test]
    fn fastp_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "-r1.fq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "r2.fq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "asm.fa", ">c1\nACGT\n");
        std::fs::create_dir(d.path().join("trim")).unwrap();
        let p: FastpParams = parse(json!({
            "in1": "-r1.fq", "in2": "r2.fq", "out1": "trim/s_1.fq.gz", "out2": "trim/s_2.fq.gz",
            "json": "trim/s.fastp.json", "thread": 4, "detect_adapter_for_pe": true, "cut_tail": true,
            "length_required": 50, "adapter_sequence": "AGATCGGAAGAGC"
        }));
        let inv = fastp(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--in1=-r1.fq",
                "--in2=r2.fq",
                "--out1=trim/s_1.fq.gz",
                "--out2=trim/s_2.fq.gz",
                "--json=trim/s.fastp.json",
                "--html=trim/s.fastp.html",
                "--thread=4",
                "--length_required=50",
                "--adapter_sequence=AGATCGGAAGAGC",
                "--detect_adapter_for_pe",
                "--cut_tail",
            ]
        );
        assert!(inv.external);
        assert_eq!(inv.expect.len(), 4);
        assert_eq!(inv.outputs.len(), 4);

        let bad = |v: serde_json::Value| fastp(&cx, &parse(v)).unwrap_err().field;
        assert_eq!(bad(json!({"in1": "missing.fq", "json": "a.json"})), "in1");
        assert_eq!(bad(json!({"in1": "asm.fa", "json": "a.json"})), "in1"); // FASTA, not FASTQ
        assert_eq!(bad(json!({"in1": "-r1.fq", "in2": "-r1.fq", "json": "a.json"})), "in2");
        assert_eq!(bad(json!({"in1": "-r1.fq", "out2": "b.fq", "json": "a.json"})), "out2");
        assert_eq!(bad(json!({"in1": "-r1.fq", "in2": "r2.fq", "out1": "a.fq", "json": "a.json"})), "out2");
        assert_eq!(bad(json!({"in1": "-r1.fq", "correction": true, "json": "a.json"})), "correction");
        assert_eq!(bad(json!({"in1": "-r1.fq", "trim_poly_g": true, "disable_trim_poly_g": true, "json": "a.json"})), "trim_poly_g");
        assert_eq!(bad(json!({"in1": "-r1.fq", "cut_mean_quality": 40, "json": "a.json"})), "cut_mean_quality");
        assert_eq!(bad(json!({"in1": "-r1.fq", "adapter_sequence": "AGT;rm", "json": "a.json"})), "adapter_sequence");
        assert_eq!(bad(json!({"in1": "-r1.fq", "json": "../a.json"})), "json");
        assert_eq!(bad(json!({"in1": "-r1.fq", "json": "a.json", "html": "a.json"})), "html");
        assert_eq!(bad(json!({"in1": "-r1.fq", "json": "-r1.fq", "overwrite": true})), "json");
        // invented flags never reach fastp
        assert!(serde_json::from_value::<FastpParams>(json!({"in1": "x", "json": "a.json", "trim_qual": 20})).is_err());
        assert!(serde_json::from_value::<FastpParams>(json!({"in1": "x", "json": "a.json", "dont_overwrite": true})).is_err());
    }

    #[test]
    fn gzip_and_empty_inputs() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(b"@r\nACGT\n+\nIIII\n").unwrap();
        std::fs::write(d.path().join("r.fq.gz"), gz.finish().unwrap()).unwrap();
        std::fs::write(d.path().join("empty.fq"), "").unwrap();
        std::fs::write(d.path().join("r.fq.bz2"), b"BZh91AY&SY").unwrap();
        let p = |n: &str| d.path().join(n);
        assert!(v::sequence_file(&p("r.fq.gz"), "in1", "r.fq.gz", Seq::Fastq).is_ok());
        assert!(v::sequence_file(&p("r.fq.gz"), "g", "r.fq.gz", Seq::Fasta).unwrap_err().message.contains("FASTQ"));
        assert!(v::sequence_file(&p("empty.fq"), "in1", "empty.fq", Seq::Fastq).unwrap_err().message.contains("empty"));
        assert!(v::sequence_file(&p("r.fq.bz2"), "in1", "r.fq.bz2", Seq::Fastq).unwrap_err().message.contains("bzip2"));
    }

    #[test]
    fn bakta_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let mut cx = ctx(d.path());
        write(d.path(), "asm/s1.fna", ">c1\nACGT\n");
        write(d.path(), "reads.fq", "@a\nACGT\n+\nIIII\n");
        fake_bakta_db(&d.path().join("db"), 6);
        let p: BaktaParams = parse(json!({
            "genome": "asm/s1.fna", "db": "db", "outdir": "ann/s1", "genus": "Escherichia",
            "gram": "negative", "translation_table": 11, "skip": ["plot", "crispr", "plot"], "threads": 4
        }));
        std::fs::create_dir(d.path().join("ann")).unwrap();
        let inv = bakta(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--db=db",
                "--output=ann/s1",
                "--force",
                "--prefix=s1",
                "--genus=Escherichia",
                "--translation-table=11",
                "--gram=-",
                "--threads=4",
                "--skip-plot",
                "--skip-crispr",
                "--",
                "asm/s1.fna",
            ]
        );
        assert!(inv.inputs.contains(&cx.root.join("db/version.json")));
        assert!(inv.expect.iter().any(|g| g[0].ends_with("ann/s1/s1.txt")));

        let bad = |cx: &Ctx, v: serde_json::Value| bakta(cx, &parse(v)).unwrap_err();
        assert_eq!(bad(&cx, json!({"genome": "reads.fq", "db": "db", "outdir": "o"})).field, "genome");
        assert_eq!(bad(&cx, json!({"genome": "asm/s1.fna", "outdir": "o"})).field, "db"); // no BAKTA_DB
        assert_eq!(
            bad(&cx, json!({"genome": "asm/s1.fna", "db": "db", "outdir": "o", "translation_table": 12})).field,
            "translation_table"
        );
        assert_eq!(bad(&cx, json!({"genome": "asm/s1.fna", "db": "db", "outdir": "o", "locus_tag": "a b"})).field, "locus_tag");
        assert_eq!(bad(&cx, json!({"genome": "asm/s1.fna", "db": "db", "outdir": "o", "prefix": "../x"})).field, "prefix");
        // the database is checked like Bakta does
        assert!(bad(&cx, json!({"genome": "asm/s1.fna", "db": "asm", "outdir": "o"})).message.contains("version.json"));
        fake_bakta_db(&d.path().join("db5"), 5);
        assert!(bad(&cx, json!({"genome": "asm/s1.fna", "db": "db5", "outdir": "o"})).message.contains("v6"));
        std::fs::remove_file(d.path().join("db/sorf.dmnd")).unwrap();
        assert!(bad(&cx, json!({"genome": "asm/s1.fna", "db": "db", "outdir": "o"})).message.contains("sorf.dmnd"));
        // $BAKTA_DB is used (and passed explicitly) when db is left out
        fake_bakta_db(&d.path().join("db6"), 6);
        cx.vars.insert("BAKTA_DB".into(), cx.root.join("db6").into());
        let inv = bakta(&cx, &parse(json!({"genome": "asm/s1.fna", "outdir": "o"}))).unwrap();
        assert_eq!(inv.args[0], format!("--db={}", cx.root.join("db6").display()));
        // a non-empty outdir needs overwrite
        write(d.path(), "o/old.txt", "x");
        assert_eq!(bad(&cx, json!({"genome": "asm/s1.fna", "outdir": "o"})).field, "outdir");
        assert!(serde_json::from_value::<BaktaParams>(json!({"genome": "g", "outdir": "o", "skip": ["everything"]})).is_err());
        assert!(serde_json::from_value::<BaktaParams>(json!({"genome": "g", "outdir": "o", "kingdom": "Bacteria"})).is_err());
    }

    #[test]
    fn checkm2_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "bins/a.fna", ">c\nACGT\n");
        write(d.path(), "bins/b.fna", ">c\nACGT\n");
        write(d.path(), "bins/notes.txt", "x");
        write(d.path(), "-odd.fa", ">c\nACGT\n");
        write(d.path(), "db/CheckM2_database/uniref100.KO.1.dmnd", "x");
        let p: Checkm2Params =
            parse(json!({"genome_dir": "bins", "outdir": "cm2", "db": "db", "lowmem": true, "threads": 2, "model": "both"}));
        let inv = checkm2(&cx, &p).unwrap();
        let dmnd = cx.root.join("db/CheckM2_database/uniref100.KO.1.dmnd");
        assert_eq!(
            inv.args,
            vec![
                "predict".to_string(),
                format!("--database_path={}", dmnd.display()),
                "--output-directory=cm2".into(),
                "--force".into(),
                "--threads=2".into(),
                "--allmodels".into(),
                "--lowmem".into(),
                "--input".into(),
                "bins".into(),
            ]
        );
        assert_eq!(inv.inputs.len(), 2);
        let p: Checkm2Params = parse(json!({"genomes": ["-odd.fa", "bins/a.fna"], "outdir": "cm2", "db": dmnd.to_str().unwrap()}));
        let inv = checkm2(&cx, &p).unwrap();
        assert_eq!(&inv.args[inv.args.len() - 3..], ["--input", "./-odd.fa", "bins/a.fna"]);

        let bad = |v: serde_json::Value| checkm2(&cx, &parse(v)).unwrap_err().field;
        assert_eq!(bad(json!({"outdir": "o", "db": "db"})), "genomes");
        assert_eq!(bad(json!({"genomes": ["bins/a.fna"], "genome_dir": "bins", "outdir": "o", "db": "db"})), "genomes");
        assert_eq!(bad(json!({"genome_dir": "bins", "extension": "fa", "outdir": "o", "db": "db"})), "genome_dir");
        assert_eq!(bad(json!({"genomes": ["bins/a.fna"], "outdir": "o", "db": "bins"})), "db");
        assert_eq!(bad(json!({"genomes": ["bins/a.fna"], "outdir": "o", "db": "db", "ttable": 7})), "ttable");
        assert_eq!(bad(json!({"genomes": ["bins/notes.txt"], "outdir": "o", "db": "db"})), "genomes");
        write(d.path(), "cm2/old", "x");
        assert_eq!(bad(json!({"genomes": ["bins/a.fna"], "outdir": "cm2", "db": "db"})), "outdir");
        assert!(serde_json::from_value::<Checkm2Params>(json!({"genomes": ["a"], "outdir": "o", "model": "fast"})).is_err());
    }

    #[test]
    fn gtdbtk_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "genomes/a.fna.gz", "");
        write(d.path(), "genomes/b.fa", ">c\nACGT\n");
        fake_gtdbtk_data(&d.path().join("gtdb/release232"), true);
        std::fs::create_dir(d.path().join("scratch")).unwrap();
        let p: GtdbtkClassifyWfParams = parse(json!({
            "genome_dir": "genomes", "extension": "fa", "out_dir": "gtdbtk", "data_path": "gtdb/release232",
            "cpus": 4, "pplacer_cpus": 1, "scratch_dir": "scratch", "force": true
        }));
        let inv = gtdbtk_classify_wf(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "classify_wf",
                "--genome_dir=genomes",
                "--out_dir=gtdbtk",
                "--extension=fa",
                "--cpus=4",
                "--pplacer_cpus=1",
                "--scratch_dir=scratch",
                "--force",
            ]
        );
        assert_eq!(inv.env["GTDBTK_DATA_PATH"], cx.root.join("gtdb/release232").to_str().unwrap());
        assert_eq!(inv.expect[0].len(), 2);
        assert!(inv.expect[0][0].ends_with("gtdbtk/classify/gtdbtk.bac120.summary.tsv"));

        let bad = |v: serde_json::Value| gtdbtk_classify_wf(&cx, &parse(v)).unwrap_err();
        let base = |extra: serde_json::Value| {
            let mut j = json!({"genome_dir": "genomes", "extension": "fa", "out_dir": "o", "data_path": "gtdb/release232"});
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            j
        };
        // the empty a.fna.gz is caught before GTDB-Tk starts
        assert_eq!(bad(base(json!({"extension": "fna.gz"}))).field, "genome_dir");
        assert_eq!(bad(base(json!({"extension": "fasta"}))).field, "genome_dir");
        assert_eq!(bad(base(json!({"min_af": 1.5}))).field, "min_af");
        assert_eq!(bad(base(json!({"scratch_dir": "/tmp"}))).field, "scratch_dir");
        assert_eq!(bad(base(json!({"data_path": "genomes"}))).field, "data_path");
        // an empty split/ (GTDB's mock database) is refused unless full_tree is set
        fake_gtdbtk_data(&d.path().join("mock/release232"), true);
        std::fs::remove_dir_all(d.path().join("mock/release232/split")).unwrap();
        std::fs::create_dir(d.path().join("mock/release232/split")).unwrap();
        let e = bad(base(json!({"data_path": "mock/release232"})));
        assert!(e.field == "data_path" && e.message.contains("full_tree"), "{}", e.message);
        assert!(gtdbtk_classify_wf(&cx, &parse(base(json!({"data_path": "mock/release232", "full_tree": true})))).is_ok());
        fake_gtdbtk_data(&d.path().join("placement/release232"), false);
        let e = bad(base(json!({"data_path": "placement/release232"})));
        assert!(e.message.contains("skani") && e.message.contains("no option to skip"), "{}", e.message);
        // flags that don't exist in GTDB-Tk 2.8.0 are refused by the schema
        for invented in ["skip_ani_screen", "max_memory", "mash_db"] {
            assert!(serde_json::from_value::<GtdbtkClassifyWfParams>(base(json!({invented: true}))).is_err(), "{invented}");
        }
    }

    #[test]
    fn megahit_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        for f in ["reads/a_1.fq.gz", "reads/a_2.fq.gz", "reads/b_1.fq", "reads/b_2.fq", "reads/x,y.fq"] {
            write(d.path(), f, "@a\nACGT\n+\nIIII\n");
        }
        let p: MegahitParams = parse(json!({
            "r1": ["reads/a_1.fq.gz", "reads/b_1.fq"], "r2": ["reads/a_2.fq.gz", "reads/b_2.fq"], "outdir": "asm/meta",
            "memory_gb": 6, "threads": 6, "presets": "meta-sensitive", "min_contig_len": 1000
        }));
        let inv = megahit(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "-1",
                "reads/a_1.fq.gz,reads/b_1.fq",
                "-2",
                "reads/a_2.fq.gz,reads/b_2.fq",
                "--out-dir=asm/meta",
                "--force",
                "--memory=6000000000",
                "--num-cpu-threads=6",
                "--presets=meta-sensitive",
                "--min-contig-len=1000",
            ]
        );
        assert_eq!(inv.inputs.len(), 4);
        assert!(inv.assembly.as_ref().unwrap().ends_with("asm/meta/final.contigs.fa"));
        // memory defaults to 4 GB in bytes, never MEGAHIT's 90%-of-the-machine fraction
        let inv = megahit(&cx, &parse(json!({"single": ["reads/b_1.fq"], "outdir": "asm/se"}))).unwrap();
        assert_eq!(inv.args, ["-r", "reads/b_1.fq", "--out-dir=asm/se", "--force", "--memory=4000000000"]);
        let bad = |j: serde_json::Value| megahit(&cx, &parse(j)).unwrap_err();
        assert_eq!(bad(json!({"outdir": "asm/x"})).field, "r1");
        assert_eq!(bad(json!({"r1": ["reads/a_1.fq.gz"], "outdir": "asm/x"})).field, "r2");
        assert_eq!(bad(json!({"single": ["reads/x,y.fq"], "outdir": "asm/x"})).field, "single");
        assert_eq!(bad(json!({"single": ["reads/b_1.fq", "reads/b_1.fq"], "outdir": "asm/x"})).field, "single");
        assert_eq!(bad(json!({"single": ["reads/b_1.fq"], "outdir": "asm/x", "memory_gb": 0.2})).field, "memory_gb");
        for k in ["21,20", "21,29,99", "29,21", "13,21", "21,x"] {
            assert_eq!(bad(json!({"single": ["reads/b_1.fq"], "outdir": "asm/x", "k_list": k})).field, "k_list", "{k}");
        }
        assert!(megahit(&cx, &parse(json!({"single": ["reads/b_1.fq"], "outdir": "asm/x", "k_list": "21,41,59"}))).is_ok());
        write(d.path(), "asm/full/final.contigs.fa", ">c\nA\n");
        assert_eq!(bad(json!({"single": ["reads/b_1.fq"], "outdir": "asm/full"})).field, "outdir");
        assert!(megahit(&cx, &parse(json!({"single": ["reads/b_1.fq"], "outdir": "asm/full", "overwrite": true}))).is_ok());
    }

    #[test]
    fn coverm_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        for f in ["reads/a_1.fq", "reads/a_2.fq", "reads/-b_1.fq", "reads/-b_2.fq"] {
            write(d.path(), f, "@a\nACGT\n+\nIIII\n");
        }
        write(d.path(), "asm/final.contigs.fa", ">k141_1\nACGT\n");
        let p: CovermContigParams = parse(json!({
            "reference": "asm/final.contigs.fa", "r1": ["reads/a_1.fq", "reads/-b_1.fq"], "r2": ["reads/a_2.fq", "reads/-b_2.fq"],
            "output": "depth/s.depth.tsv", "methods": ["metabat"], "threads": 6, "min_read_percent_identity": 95
        }));
        let inv = coverm_contig(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "contig",
                "--reference=asm/final.contigs.fa",
                "--output-file=depth/s.depth.tsv",
                "--min-read-percent-identity=95",
                "--threads=6",
                "--methods",
                "metabat",
                "--coupled",
                "reads/a_1.fq",
                "reads/a_2.fq",
                "reads/-b_1.fq",
                "reads/-b_2.fq",
            ]
        );
        let bad = |j: serde_json::Value| coverm_contig(&cx, &parse(j)).unwrap_err();
        let base = json!({"reference": "asm/final.contigs.fa", "r1": ["reads/a_1.fq"], "r2": ["reads/a_2.fq"], "output": "d.tsv"});
        let with = |k: &str, v: serde_json::Value| {
            let mut j = base.clone();
            j[k] = v;
            j
        };
        assert_eq!(bad(with("methods", json!(["metabat", "mean"]))).field, "methods");
        assert_eq!(bad(with("r2", json!([]))).field, "r2");
        assert_eq!(bad(with("r1", json!([]))).field, "r1");
        assert_eq!(bad(with("min_read_aligned_percent", json!(120))).field, "min_read_aligned_percent");
        assert_eq!(bad(with("output", json!("reads/a_1.fq"))).field, "output");
        assert_eq!(bad(with("reference", json!("reads/a_1.fq"))).field, "reference");
        // a value starting with - is never read as a flag
        assert!(coverm_contig(&cx, &parse(with("methods", json!(["mean", "variance"])))).is_ok());
        assert!(serde_json::from_value::<CovermContigParams>(with("mapper", json!("minimap2-sr"))).is_ok());
    }

    /// A coordinate-sorted BAM's start (gzip of `BAM\1`, the header length and text), as CoverM caches it.
    fn fake_bam(dir: &Path, name: &str, header: &str) {
        use std::io::Write;
        let mut raw = b"BAM\x01".to_vec();
        raw.extend((header.len() as u32).to_le_bytes());
        raw.extend(header.as_bytes());
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let mut gz = flate2::write::GzEncoder::new(std::fs::File::create(p).unwrap(), flate2::Compression::default());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap();
    }

    #[test]
    fn coverm_contig_caches_bams() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        for f in ["reads/a_1.fq", "reads/a_2.fq", "other/a_1.fq", "other/a_2.fq"] {
            write(d.path(), f, "@a\nACGT\n+\nIIII\n");
        }
        write(d.path(), "asm/final.contigs.fa", ">k141_1\nACGT\n");
        let p: CovermContigParams = parse(json!({
            "reference": "asm/final.contigs.fa", "r1": ["reads/a_1.fq"], "r2": ["reads/a_2.fq"], "output": "d.tsv", "methods": ["metabat"], "bam_cache_dir": "bams"
        }));
        let inv = coverm_contig(&cx, &p).unwrap();
        assert!(inv.args.contains(&"--bam-file-cache-directory=bams".to_string()), "{:?}", inv.args);
        let bam = cx.cwd.join("bams/final.contigs.fa.a_1.fq.bam");
        assert!(inv.expect.contains(&vec![bam.clone()]) && inv.outputs.contains(&bam));
        assert_eq!(inv.volatile, [".bam"]);
        // two samples whose read 1 files share a name would share a BAM
        let two = json!({"reference": "asm/final.contigs.fa", "r1": ["reads/a_1.fq", "other/a_1.fq"], "r2": ["reads/a_2.fq", "other/a_2.fq"],
                         "output": "d.tsv", "bam_cache_dir": "bams"});
        assert_eq!(coverm_contig(&cx, &parse(two)).unwrap_err().field, "bam_cache_dir");
        write(d.path(), "bams/final.contigs.fa.a_1.fq.bam", "x");
        assert_eq!(coverm_contig(&cx, &p).unwrap_err().field, "bam_cache_dir");
    }

    #[test]
    fn semibin2_args_checks_and_bins() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "asm/final.contigs.fa", ">k141_1\nACGT\n");
        write(d.path(), "depth/one.tsv", "contigName\tcontigLen\ttotalAvgDepth\ts.bam\ts.bam-var\nk141_1\t4\t1\t1\t0\n");
        write(d.path(), "depth/two.tsv", "contigName\tcontigLen\ttotalAvgDepth\ta.bam\ta.bam-var\tb.bam\tb.bam-var\nk141_1\t4\t1\t1\t0\t1\t0\n");
        fake_bam(d.path(), "bams/s1.bam", "@HD\tVN:1.6\tSO:coordinate\n@SQ\tSN:k141_1\tLN:4\n");
        fake_bam(d.path(), "-s2.bam", "@HD\tVN:1.6\tSO:coordinate\n");
        fake_bam(d.path(), "bams/unsorted.bam", "@HD\tVN:1.6\tSO:unsorted\n");
        let p: Semibin2Params = parse(json!({"contigs": "asm/final.contigs.fa", "depth": "depth/one.tsv", "environment": "global", "outdir": "bins/s1", "tag": "s1", "threads": 6}));
        assert_eq!(
            semibin2(&cx, &p).unwrap().args,
            [
                "single_easy_bin",
                "--input-fasta=asm/final.contigs.fa",
                "--depth-metabat2=depth/one.tsv",
                "--environment=global",
                "--output=bins/s1",
                "--tag-output=s1",
                "--compression=none",
                "--engine=cpu",
                "--random-seed=1",
                "--threads=6",
            ]
        );
        // trained on two samples' BAMs; the BAMs go last
        let p: Semibin2Params = parse(json!({"contigs": "asm/final.contigs.fa", "bams": ["bams/s1.bam", "-s2.bam"], "outdir": "b", "seed": 7, "min_len": 1000}));
        let inv = semibin2(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            ["single_easy_bin", "--input-fasta=asm/final.contigs.fa", "--output=b", "--compression=none", "--engine=cpu", "--min-len=1000", "--random-seed=7", "--input-bam", "bams/s1.bam", "./-s2.bam"]
        );
        assert_eq!(inv.expect, [vec![cx.cwd.join("b/contig_bins.tsv")]]);
        // training its own model: the memory warning comes before it runs
        assert!(inv.warnings[0].contains("self-supervised"), "{:?}", inv.warnings);
        let bad = |j: serde_json::Value| semibin2(&cx, &parse(j)).unwrap_err().field;
        let base = json!({"contigs": "asm/final.contigs.fa", "outdir": "b"});
        let with = |extra: serde_json::Value| {
            let mut j = base.clone();
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            j
        };
        assert_eq!(bad(with(json!({}))), "depth"); // neither depth nor bams
        assert_eq!(bad(with(json!({"depth": "depth/one.tsv", "bams": ["bams/s1.bam"], "environment": "soil"}))), "depth");
        assert_eq!(bad(with(json!({"depth": "depth/one.tsv"}))), "environment");
        assert_eq!(bad(with(json!({"depth": "depth/two.tsv", "environment": "soil"}))), "depth");
        assert_eq!(bad(with(json!({"bams": ["bams/s1.bam", "-s2.bam"], "environment": "ocean"}))), "bams");
        assert_eq!(bad(with(json!({"bams": ["bams/unsorted.bam"]}))), "bams");
        assert_eq!(bad(with(json!({"bams": ["depth/one.tsv"]}))), "bams");
        assert_eq!(bad(with(json!({"bams": ["bams/s1.bam", "bams/s1.bam"]}))), "bams");
        assert_eq!(bad(with(json!({"bams": ["bams/s1.bam"], "tag": "a/b"}))), "tag");
        assert_eq!(bad(with(json!({"bams": ["bams/s1.bam"], "min_len": 100}))), "min_len");
        assert!(serde_json::from_value::<Semibin2Params>(with(json!({"environment": "groundwater"}))).is_err());
        for e in ["human_gut", "chicken_caecum", "built_environment", "wastewater"] {
            let p: Semibin2Params = parse(with(json!({"environment": e})));
            assert_eq!(p.environment.unwrap().name(), e);
        }
        // bins in number order; files of other tags or kinds are not bins
        for f in ["SemiBin_10.fa", "SemiBin_2.fa", "SemiBin_0.fa", "SemiBin_1.fa.gz", "other_1.fa"] {
            write(d.path(), &format!("out/output_bins/{f}"), ">c\nA\n");
        }
        let names: Vec<String> =
            semibin2_bins(&d.path().join("out"), "SemiBin").iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["SemiBin_0.fa", "SemiBin_2.fa", "SemiBin_10.fa"]);
    }

    #[test]
    fn galah_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        for f in ["bins/s1/s1.1.fa", "bins/s1/s1.2.fa", "bins/s2/s2.1.fa", "bins/s2/notes.txt", "-odd.fna", "dup/s1.1.fa"] {
            write(d.path(), f, ">c\nACGT\n");
        }
        write(d.path(), "checkm2/quality_report.tsv", "Name\tCompleteness\tContamination\ns1.1\t90\t1\ns1.2\t60\t3\ns2.1\t95\t0.5\n-odd\t50\t0\n");
        let p: GalahParams = parse(json!({"genome_dirs": ["bins/s1", "bins/s2"], "checkm2_report": "checkm2/quality_report.tsv", "outdir": "derep",
                                          "min_completeness": 50, "max_contamination": 10, "quality_formula": "completeness-5contamination"}));
        let inv = galah(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "cluster",
                "--ani=95",
                "--checkm2-quality-report=checkm2/quality_report.tsv",
                "--min-completeness=50",
                "--max-contamination=10",
                "--quality-formula=completeness-5contamination",
                "--threads=1",
                "--output-cluster-definition=derep/clusters.tsv",
                "--output-representative-list=derep/representatives.txt",
                "--output-representative-fasta-directory-copy=derep/representatives",
                "--genome-fasta-files",
                "bins/s1/s1.1.fa",
                "bins/s1/s1.2.fa",
                "bins/s2/s2.1.fa",
            ]
        );
        assert!(inv.volatile.is_empty() && inv.warnings.is_empty());
        let inv = galah(&cx, &parse(json!({"genomes": ["-odd.fna"], "outdir": "o", "ani": 99, "precluster_ani": 95, "threads": 4, "small_genomes": true}))).unwrap();
        assert_eq!(&inv.args[..5], ["cluster", "--ani=99", "--precluster-ani=95", "--small-genomes", "--threads=4"]);
        assert_eq!(inv.args.last().unwrap(), "./-odd.fna");
        // several threads: same clusters, lines in another order
        assert_eq!(inv.volatile, ["clusters.tsv", "representatives.txt"]);
        assert!(inv.warnings[0].contains("no checkm2_report"), "{:?}", inv.warnings);
        let bad = |j: serde_json::Value| galah(&cx, &parse(j)).unwrap_err().field;
        assert_eq!(bad(json!({"outdir": "o"})), "genomes");
        assert_eq!(bad(json!({"genomes": ["bins/s1/s1.1.fa"], "extension": "fa", "outdir": "o"})), "extension");
        assert_eq!(bad(json!({"genome_dirs": ["bins/s1"], "extension": "fna", "outdir": "o"})), "genome_dirs");
        assert_eq!(bad(json!({"genome_dirs": ["bins/s1"], "genomes": ["dup/s1.1.fa"], "outdir": "o"})), "genomes");
        assert_eq!(bad(json!({"genome_dirs": ["bins/s1"], "genomes": ["bins/s1/s1.1.fa"], "outdir": "o"})), "genomes");
        assert_eq!(bad(json!({"genome_dirs": ["bins/s1"], "outdir": "o", "ani": 0.95})), "ani");
        assert_eq!(bad(json!({"genome_dirs": ["bins/s1"], "outdir": "o", "ani": 70})), "ani");
        assert_eq!(bad(json!({"genome_dirs": ["bins/s1"], "outdir": "o", "ani": 95, "precluster_ani": 97})), "precluster_ani");
        assert_eq!(bad(json!({"genome_dirs": ["bins/s1"], "outdir": "o", "min_completeness": 50})), "checkm2_report");
        write(d.path(), "short.tsv", "Name\tCompleteness\tContamination\ns1.1\t90\t1\n");
        assert_eq!(bad(json!({"genome_dirs": ["bins/s1"], "outdir": "o", "checkm2_report": "short.tsv"})), "checkm2_report");
        write(d.path(), "notcheckm.tsv", "genome\tquality\n");
        assert_eq!(bad(json!({"genome_dirs": ["bins/s1"], "outdir": "o", "checkm2_report": "notcheckm.tsv"})), "checkm2_report");
        assert!(serde_json::from_value::<GalahParams>(json!({"genome_dirs": ["bins/s1"], "outdir": "o", "quality_formula": "dRep"})).is_err());
        // representatives/ with files: galah refuses, so irit-mcp does first, even with overwrite
        write(d.path(), "derep/representatives/s1.1.fa", ">c\nA\n");
        assert_eq!(bad(json!({"genome_dirs": ["bins/s1"], "outdir": "derep", "overwrite": true})), "outdir");
    }

    #[test]
    fn coverm_abundance_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        for f in ["reads/s1_R1.fq.gz", "reads/s1_R2.fq.gz", "reads/s2_R1.fq.gz", "reads/s2_R2.fq.gz", "other/s1_R1.fq.gz"] {
            write(d.path(), f, "@a\nACGT\n+\nIIII\n");
        }
        for f in ["derep/representatives/s1.1.fa", "derep/representatives/s2.3.fa", "extra/g.fna"] {
            write(d.path(), f, ">c\nACGT\n");
        }
        let p: CovermAbundanceParams = parse(json!({
            "genome_dirs": ["derep/representatives"], "genomes": ["extra/g.fna"], "r1": ["reads/s1_R1.fq.gz", "reads/s2_R1.fq.gz"],
            "r2": ["reads/s1_R2.fq.gz", "reads/s2_R2.fq.gz"], "output": "abundance.tsv", "threads": 6
        }));
        let inv = coverm_abundance(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "genome",
                "--output-file=abundance.tsv",
                "--min-covered-fraction=0",
                "--threads=6",
                "--genome-fasta-files",
                "extra/g.fna",
                "derep/representatives/s1.1.fa",
                "derep/representatives/s2.3.fa",
                "--methods",
                "relative_abundance",
                "mean",
                "covered_fraction",
                "count",
                "--coupled",
                "reads/s1_R1.fq.gz",
                "reads/s1_R2.fq.gz",
                "reads/s2_R1.fq.gz",
                "reads/s2_R2.fq.gz",
            ]
        );
        assert!(inv.warnings.is_empty());
        // not an isolate's depth: the Shovill depth check doesn't apply
        assert!(result_warnings(&inv).is_empty());
        // CoverM's 10% threshold: no read counts then
        let p10: CovermAbundanceParams = parse(json!({"genome_dirs": ["derep/representatives"], "r1": ["reads/s1_R1.fq.gz"], "r2": ["reads/s1_R2.fq.gz"],
                                                     "output": "a.tsv", "min_covered_fraction": 10}));
        let inv = coverm_abundance(&cx, &p10).unwrap();
        assert!(inv.args.contains(&"--min-covered-fraction=10".to_string()) && !inv.args.contains(&"count".to_string()), "{:?}", inv.args);
        assert!(inv.warnings[0].contains("no Read Count"), "{:?}", inv.warnings);
        let bad = |j: serde_json::Value| coverm_abundance(&cx, &parse(j)).unwrap_err().field;
        let base = json!({"genome_dirs": ["derep/representatives"], "r1": ["reads/s1_R1.fq.gz"], "r2": ["reads/s1_R2.fq.gz"], "output": "a.tsv"});
        let with = |k: &str, v: serde_json::Value| {
            let mut j = base.clone();
            j[k] = v;
            j
        };
        assert_eq!(bad(with("min_covered_fraction", json!(0.5))), "min_covered_fraction");
        assert_eq!(bad(with("min_covered_fraction", json!(101))), "min_covered_fraction");
        assert_eq!(bad(with("r2", json!([]))), "r2");
        assert_eq!(bad(json!({"r1": ["reads/s1_R1.fq.gz"], "r2": ["reads/s1_R2.fq.gz"], "output": "a.tsv"})), "genomes");
        let same = json!({"genome_dirs": ["derep/representatives"], "r1": ["reads/s1_R1.fq.gz", "other/s1_R1.fq.gz"], "r2": ["reads/s1_R2.fq.gz", "reads/s2_R2.fq.gz"], "output": "a.tsv"});
        assert_eq!(bad(same), "r1");
        assert_eq!(bad(with("output", json!("reads/s1_R1.fq.gz"))), "output");
    }

    #[test]
    fn metabat2_args_checks_and_bins() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "asm/final.contigs.fa", ">k141_1\nACGT\n");
        write(d.path(), "depth/s.depth.tsv", "contigName\tcontigLen\ttotalAvgDepth\ts.bam\ts.bam-var\nk141_1\t4\t1\t1\t0\n");
        write(d.path(), "depth/mean.tsv", "Contig\ts Mean\nk141_1\t1\n");
        let p: Metabat2Params = parse(json!({
            "contigs": "asm/final.contigs.fa", "depth": "depth/s.depth.tsv", "outdir": "bins/s", "min_contig": 2000, "threads": 6
        }));
        let inv = metabat2(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--inFile=asm/final.contigs.fa",
                "--abdFile=depth/s.depth.tsv",
                "--outFile=bins/s/bin",
                "--minContig=2000",
                "--seed=1",
                "--numThreads=6",
            ]
        );
        // the folder is created by the server: a file in it is among the outputs
        assert!(inv.outputs.iter().any(|o| o.ends_with("bins/s/bin.1.fa")));
        let bad = |j: serde_json::Value| metabat2(&cx, &parse(j)).unwrap_err();
        assert_eq!(bad(json!({"contigs": "asm/final.contigs.fa", "depth": "depth/mean.tsv", "outdir": "b"})).field, "depth");
        assert_eq!(bad(json!({"contigs": "asm/final.contigs.fa", "outdir": "b", "min_contig": 1000})).field, "min_contig");
        assert_eq!(bad(json!({"contigs": "asm/final.contigs.fa", "outdir": "b", "seed": 0})).field, "seed");
        for prefix in ["", "-x", ".x", "a/b", "a b"] {
            assert_eq!(bad(json!({"contigs": "asm/final.contigs.fa", "outdir": "b", "prefix": prefix})).field, "prefix", "{prefix:?}");
        }
        let inv = metabat2(&cx, &parse(json!({"contigs": "asm/final.contigs.fa", "outdir": "b/", "prefix": "s1", "seed": 7, "unbinned": true}))).unwrap();
        assert_eq!(inv.args, ["--inFile=asm/final.contigs.fa", "--outFile=b/s1", "--seed=7", "--unbinned"]);
        // bins in number order; the unbinned file and others are not bins
        for f in ["bin.10.fa", "bin.2.fa", "bin.1.fa", "bin.unbinned.fa", "bin.3.fa.fai", "other.1.fa"] {
            write(d.path(), &format!("out/{f}"), ">c\nA\n");
        }
        let names: Vec<String> =
            metabat2_bins(&d.path().join("out"), "bin").iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["bin.1.fa", "bin.2.fa", "bin.10.fa"]);
    }

    #[test]
    fn out_of_memory_failures_get_a_hint() {
        // as CheckM2 1.1.0 printed it when DIAMOND hit an 8 GB cap
        let checkm2 = "[05:35:04 PM] ERROR: An error occured while running DIAMOND: Command 'diamond blastp --outfmt 6 --threads 6 --block-size 2.0 --quiet ' died with <Signals.SIGKILL: 9>.\n";
        assert!(failure_hint("checkm2", Some(1), checkm2).unwrap().contains("lowmem: true"));
        assert!(failure_hint("megahit", None, "").unwrap().contains("killed by a signal"));
        // as SemiBin2 2.6.0 logged it before the 8 GB cap killed its training
        let semibin = "2026-11-14 18:15:34 dzul SemiBin2[3661952] INFO Training model...\n  7%|| 1/15 [03:03<42:54, 183.90s/it]\n";
        assert!(failure_hint("SemiBin2", None, semibin).unwrap().contains("pretrained environment"));
        assert!(failure_hint("SemiBin2", None, "Generating features").unwrap().contains("killed by a signal"));
        assert!(failure_hint("megahit", Some(137), "").is_some());
        assert!(failure_hint("checkm2", Some(1), "ERROR: no genomes").is_none());
    }

    #[test]
    fn metabat_version_from_help() {
        assert_eq!(
            version_number("MetaBAT: Metagenome Binning based on Abundance and Tetranucleotide frequency (version 2:2.18; 20260506_185317)"),
            Some("2.18")
        );
        assert_eq!(version_number("MEGAHIT v1.2.9"), Some("1.2.9"));
        assert_eq!(version_number("coverm 0.8.0"), Some("0.8.0"));
        assert_eq!(version_probe("metabat2"), ("--help", true));
        assert_eq!(version_probe("megahit"), ("--version", false));
    }

    #[test]
    fn shovill_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "reads/s1_R1.fq.gz.txt", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "reads/s1_R1.fq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "reads/s1_R2.fq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "asm.fa", ">c\nACGT\n");
        std::fs::create_dir(d.path().join("asm")).unwrap();
        let p: ShovillParams = parse(json!({
            "r1": "reads/s1_R1.fq", "r2": "reads/s1_R2.fq", "outdir": "asm/s1", "assembler": "skesa", "gsize": "2.8M",
            "depth": 40, "cpus": 4, "ram": 8, "nocorr": true, "namefmt": "s1_%05d", "kmers": "31,55"
        }));
        let inv = shovill(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--R1=reads/s1_R1.fq",
                "--R2=reads/s1_R2.fq",
                "--outdir=asm/s1",
                "--force",
                "--assembler=skesa",
                "--gsize=2.8M",
                "--depth=40",
                "--namefmt=s1_%05d",
                "--cpus=4",
                "--ram=8",
                "--kmers=31,55",
                "--nocorr",
            ]
        );
        // link_as: a relative link outside outdir, recorded as an output
        let linked: ShovillParams = parse(json!({"r1": "reads/s1_R1.fq", "r2": "reads/s1_R2.fq", "outdir": "asm/s1", "link_as": "genomes/s1.fna"}));
        let li = shovill(&cx, &linked).unwrap();
        assert_eq!(li.links, [(cx.root.join("genomes/s1.fna"), PathBuf::from("../asm/s1/contigs.fa"))]);
        assert!(li.outputs.contains(&cx.root.join("genomes/s1.fna")));
        let inside: ShovillParams = parse(json!({"r1": "reads/s1_R1.fq", "r2": "reads/s1_R2.fq", "outdir": "asm/s1", "link_as": "asm/s1/s1.fna"}));
        assert_eq!(shovill(&cx, &inside).unwrap_err().field, "link_as");
        assert_eq!(relative_to(Path::new("/w/genomes"), Path::new("/w/asm/s1/contigs.fa")), PathBuf::from("../asm/s1/contigs.fa"));
        assert_eq!(relative_to(Path::new("/w"), Path::new("/w/asm/contigs.fa")), PathBuf::from("asm/contigs.fa"));
        // No assembler's outputs are reproducible: every output is volatile.
        assert_eq!(inv.volatile, [""]);
        let spades: ShovillParams = parse(json!({"r1": "reads/s1_R1.fq", "r2": "reads/s1_R2.fq", "outdir": "asm/s2"}));
        assert_eq!(shovill(&cx, &spades).unwrap().volatile, [""]);
        assert!(inv.expect[0][0].ends_with("asm/s1/contigs.fa"));
        assert_eq!(inv.inputs.len(), 2);

        let base = json!({"r1": "reads/s1_R1.fq", "r2": "reads/s1_R2.fq", "outdir": "asm/x"});
        let bad = |extra: serde_json::Value| {
            let mut j = base.clone();
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            shovill(&cx, &parse(j)).unwrap_err()
        };
        assert_eq!(bad(json!({"r2": "reads/s1_R1.fq"})).field, "r2");
        assert_eq!(bad(json!({"r1": "asm.fa"})).field, "r1"); // FASTA, not reads
        assert_eq!(bad(json!({"ram": 4})).field, "ram");
        assert_eq!(bad(json!({"gsize": "big"})).field, "gsize");
        assert_eq!(bad(json!({"gsize": "2.8MM"})).field, "gsize");
        assert_eq!(bad(json!({"namefmt": "contig"})).field, "namefmt");
        assert_eq!(bad(json!({"namefmt": "c%s%05d"})).field, "namefmt");
        assert_eq!(bad(json!({"kmers": "31;rm -rf"})).field, "kmers");
        assert_eq!(bad(json!({"plasmid": true, "assembler": "skesa"})).field, "plasmid");
        assert_eq!(bad(json!({"outdir": "../asm"})).field, "outdir");
        assert_eq!(bad(json!({"outdir": "asm/s 1"})).field, "outdir"); // reaches a shell unquoted
        write(d.path(), "asm/full/contigs.fa", ">c\nA\n");
        assert_eq!(bad(json!({"outdir": "asm/full"})).field, "outdir");
        assert!(shovill(&cx, &parse(json!({"r1": "reads/s1_R1.fq", "r2": "reads/s1_R2.fq", "outdir": "asm/full", "overwrite": true}))).is_ok());
        // --opts would paste free text into a shell command; it is not a parameter
        for invented in ["opts", "tmpdir", "check"] {
            let mut j = base.clone();
            j[invented] = json!("x");
            assert!(serde_json::from_value::<ShovillParams>(j).is_err(), "{invented}");
        }
    }

    #[test]
    fn flye_version_with_build() {
        assert_eq!(version_warning("flye", Some("2.9.6-b1802")), None);
        assert!(version_warning("flye", Some("2.9.5-b1801")).is_some());
    }

    #[test]
    fn rasusa_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "reads/ont.fastq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "reads/r1.fq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "-r2.fq", "@a\nACGT\n+\nIIII\n");
        let p: RasusaParams = parse(json!({"reads": ["reads/ont.fastq"], "outputs": ["sub/ont.fq.gz"], "coverage": 100, "genome_size": "5.7mb"}));
        let inv = rasusa(&cx, &p).unwrap();
        assert_eq!(inv.args, ["reads", "--genome-size=5.7mb", "--coverage=100", "--seed=1", "--output=sub/ont.fq.gz", "reads/ont.fastq"]);
        assert_eq!(inv.expect, [vec![cx.cwd.join("sub/ont.fq.gz")]]);
        let pair: RasusaParams = parse(json!({"reads": ["reads/r1.fq", "-r2.fq"], "outputs": ["s_1.fq.gz", "s_2.fq.gz"], "num": 1000, "seed": 7}));
        assert_eq!(rasusa(&cx, &pair).unwrap().args, ["reads", "--num=1000", "--seed=7", "--output=s_1.fq.gz", "--output=s_2.fq.gz", "reads/r1.fq", "./-r2.fq"]);

        let bad = |j: serde_json::Value| rasusa(&cx, &parse(j)).unwrap_err().field;
        let one = |extra: serde_json::Value| {
            let mut j = json!({"reads": ["reads/ont.fastq"], "outputs": ["o.fq"]});
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            j
        };
        assert_eq!(bad(one(json!({}))), "coverage"); // no target
        assert_eq!(bad(one(json!({"num": 5, "frac": 0.5}))), "coverage"); // two targets
        assert_eq!(bad(one(json!({"coverage": 100}))), "genome_size");
        assert_eq!(bad(one(json!({"coverage": 100, "genome_size": "big"}))), "genome_size");
        assert_eq!(bad(one(json!({"coverage": 0, "genome_size": "5mb"}))), "coverage");
        assert_eq!(bad(one(json!({"frac": 1.5}))), "frac");
        assert_eq!(bad(one(json!({"bases": "-5mb"}))), "bases");
        assert_eq!(bad(one(json!({"num": 5, "genome_size": "5mb"}))), "genome_size");
        assert_eq!(bad(json!({"reads": ["reads/ont.fastq"], "outputs": ["a.fq", "b.fq"], "num": 5})), "outputs");
        assert_eq!(bad(json!({"reads": ["reads/ont.fastq"], "outputs": ["reads/ont.fastq"], "num": 5, "overwrite": true})), "outputs");
        assert_eq!(bad(json!({"reads": ["reads/ont.fastq"], "outputs": ["reads/r1.fq"], "num": 5})), "outputs"); // exists
        assert!(rasusa_size("x", "5500000").is_ok() && rasusa_size("x", "4.1MB").is_ok() && rasusa_size("x", "7Tb").is_ok());
        assert!(rasusa_size("x", "mb").is_err() && rasusa_size("x", "5m").is_err());
    }

    #[test]
    fn flye_tandem_copies_are_flagged() {
        // a pseudo-random 6 kb unit (xorshift), so no repeats of its own
        let mut x: u32 = 2463534242;
        let unit: Vec<u8> = (0..6000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                b"ACGT"[(x % 4) as usize]
            })
            .collect();
        assert_eq!(tandem_copies(&unit), None);
        let mut three = [unit.clone(), unit.clone(), unit.clone()].concat();
        // a few differences between copies and a rotated start, as Flye gives
        for i in (7000..17000).step_by(997) {
            three[i] = if three[i] == b'A' { b'C' } else { b'A' };
        }
        three.rotate_left(1234);
        assert_eq!(tandem_copies(&three), Some(3));
        assert_eq!(tandem_copies(&[unit.clone(), unit.clone()].concat()), Some(2));

        let d = tempfile::tempdir().unwrap();
        let asm = d.path().join("assembly.fasta");
        let s = |v: &[u8]| String::from_utf8(v.to_vec()).unwrap();
        std::fs::write(&asm, format!(">contig_1\n{}\n>contig_2\n{}\n>contig_3\n{}\n", s(&three), s(&unit), s(&three))).unwrap();
        let info = d.path().join("assembly_info.txt");
        std::fs::write(&info, "#seq_name\tlength\tcov.\tcirc.\trepeat\tmult.\talt_group\tgraph_path\ncontig_1\t18000\t179\tY\tN\t1\t*\t1\ncontig_2\t6000\t90\tY\tN\t1\t*\t2\ncontig_3\t18000\t50\tN\tN\t1\t*\t3\n").unwrap();
        let w = flye_tandem_warnings(&asm, &info);
        assert_eq!(w.len(), 1, "{w:?}"); // contig_3 is linear, contig_2 a single copy
        assert!(w[0].starts_with("Flye's contig_1 (18000 bp, circular) looks like 3 copies of one 6000 bp sequence ("), "{w:?}");
        assert!(repeated_fraction(&unit) < 0.01 && repeated_fraction(&three) > 0.9);
    }

    #[test]
    fn flye_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "reads/s1.fastq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "-s1b.fasta", ">a\nACGT\n");
        write(d.path(), "reads/notes.txt", "hello\n");
        let p: FlyeParams = parse(json!({
            "reads": ["reads/s1.fastq"], "outdir": "asm/s1", "genome_size": "5.5m", "asm_coverage": 50,
            "threads": 6, "deterministic": true, "link_as": "genomes/s1.fna"
        }));
        let inv = flye(&cx, &p).unwrap();
        assert_eq!(inv.tool, "flye");
        assert_eq!(
            inv.args,
            ["--nano-hq", "reads/s1.fastq", "--out-dir=asm/s1", "--genome-size=5.5m", "--asm-coverage=50", "--threads=6", "--deterministic"]
        );
        assert!(inv.expect[0][0].ends_with("asm/s1/assembly.fasta"));
        assert_eq!(inv.assembly.as_deref(), Some(cx.root.join("asm/s1/assembly.fasta").as_path()));
        assert_eq!(inv.links, [(cx.root.join("genomes/s1.fna"), PathBuf::from("../asm/s1/assembly.fasta"))]);
        assert_eq!(inv.volatile, [""]);
        // several files, FASTA too; a name starting with `-` can't become a flag
        let two: FlyeParams = parse(json!({"reads": ["reads/s1.fastq", "-s1b.fasta"], "outdir": "asm/s2", "read_type": "pacbio-hifi"}));
        assert_eq!(flye(&cx, &two).unwrap().args, ["--pacbio-hifi", "reads/s1.fastq", "./-s1b.fasta", "--out-dir=asm/s2"]);

        let base = json!({"reads": ["reads/s1.fastq"], "outdir": "asm/x"});
        let bad = |extra: serde_json::Value| {
            let mut j = base.clone();
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            flye(&cx, &parse(j)).unwrap_err()
        };
        assert_eq!(bad(json!({"reads": []})).field, "reads");
        assert_eq!(bad(json!({"reads": ["reads/s1.fastq", "reads/s1.fastq"]})).field, "reads");
        assert_eq!(bad(json!({"reads": ["reads/notes.txt"]})).field, "reads");
        assert_eq!(bad(json!({"reads": ["reads/missing.fq"]})).field, "reads");
        assert_eq!(bad(json!({"asm_coverage": 50})).field, "asm_coverage"); // needs genome_size
        assert_eq!(bad(json!({"asm_coverage": 50, "genome_size": "5m", "meta": true})).field, "asm_coverage");
        assert_eq!(bad(json!({"genome_size": "big"})).field, "genome_size");
        assert_eq!(bad(json!({"genome_size": "5mm"})).field, "genome_size");
        assert_eq!(bad(json!({"read_error": 3.0})).field, "read_error");
        assert_eq!(bad(json!({"keep_haplotypes": true, "no_alt_contigs": true})).field, "no_alt_contigs");
        assert_eq!(bad(json!({"outdir": "../asm"})).field, "outdir");
        assert_eq!(bad(json!({"link_as": "asm/x/s1.fna"})).field, "link_as");
        write(d.path(), "asm/full/assembly.fasta", ">c\nA\n");
        assert_eq!(bad(json!({"outdir": "asm/full"})).field, "outdir");
        assert!(flye(&cx, &parse(json!({"reads": ["reads/s1.fastq"], "outdir": "asm/full", "overwrite": true}))).is_ok());
        for invented in ["extra_params", "resume", "stop_after", "polish_target", "plasmids"] {
            let mut j = base.clone();
            j[invented] = json!(true);
            assert!(serde_json::from_value::<FlyeParams>(j).is_err(), "{invented}");
        }
    }

    #[test]
    fn medaka_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "reads/s1.fastq.gz.txt", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "reads/s1.fastq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "asm/s1/assembly.fasta", ">c\nACGT\n");
        let p: MedakaParams = parse(json!({
            "reads": "reads/s1.fastq", "draft": "asm/s1/assembly.fasta", "outdir": "polished/s1",
            "model": "r1041_e82_400bps_bacterial_methylation", "threads": 2, "batch_size": 50, "link_as": "genomes/s1.fna"
        }));
        let inv = medaka(&cx, &p).unwrap();
        assert_eq!(inv.tool, "medaka_consensus");
        assert_eq!(
            inv.args,
            ["-i", "reads/s1.fastq", "-d", "asm/s1/assembly.fasta", "-o", "polished/s1", "-m", "r1041_e82_400bps_bacterial_methylation", "-t", "2", "-b", "50"]
        );
        assert!(inv.expect[0][0].ends_with("polished/s1/consensus.fasta"));
        assert_eq!(inv.links, [(cx.root.join("genomes/s1.fna"), PathBuf::from("../polished/s1/consensus.fasta"))]);
        assert_eq!(inv.inputs.len(), 2);
        let auto: MedakaParams = parse(json!({"reads": "reads/s1.fastq", "draft": "asm/s1/assembly.fasta", "outdir": "polished/s2", "bacteria": true}));
        assert_eq!(medaka(&cx, &auto).unwrap().args.last().map(String::as_str), Some("--bacteria"));

        let base = json!({"reads": "reads/s1.fastq", "draft": "asm/s1/assembly.fasta", "outdir": "polished/x"});
        let bad = |extra: serde_json::Value| {
            let mut j = base.clone();
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            medaka(&cx, &parse(j)).unwrap_err()
        };
        assert_eq!(bad(json!({"draft": "reads/s1.fastq"})).field, "draft"); // reads, not an assembly
        assert_eq!(bad(json!({"model": "-f"})).field, "model");
        assert_eq!(bad(json!({"model": "a b"})).field, "model");
        assert_eq!(bad(json!({"model": "../m.tar.gz"})).field, "model");
        assert_eq!(bad(json!({"threads": 0})).field, "threads");
        // outdir must be empty even with overwrite: medaka_consensus would reuse old intermediate files
        write(d.path(), "polished/old/calls_to_draft.bam", "x");
        assert_eq!(bad(json!({"outdir": "polished/old", "overwrite": true})).field, "outdir");
        for invented in ["force", "regions", "prefix", "qualities"] {
            let mut j = base.clone();
            j[invented] = json!(true);
            assert!(serde_json::from_value::<MedakaParams>(j).is_err(), "{invented}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn mlst_data_follows_the_program_or_mlst_dbdir() {
        let d = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(d.path()).unwrap();
        // Bioconda's layout: the real script in <prefix>/bin, data in <prefix>/db; PATH may hold a symlink.
        write(&root, "pkg/bin/mlst", "#!/bin/sh\n");
        for f in ["VERSION.txt", "scheme_species_map.tab", "blast/mlst.fa.nsq", "blast/mlst.fa.nin", "blast/other.txt",
            "pubmlst/saureus/saureus.txt", "pubmlst/saureus/arcC.tfa", "pubmlst/ecoli/ecoli.txt", "pubmlst/stray.txt"] {
            write(&root, &format!("pkg/db/{f}"), "x\n");
        }
        std::fs::create_dir(root.join("path")).unwrap();
        std::os::unix::fs::symlink(root.join("pkg/bin/mlst"), root.join("path/mlst")).unwrap();
        let mut cx = ctx(&root);
        let names = |v: Vec<PathBuf>| v.iter().map(|p| p.strip_prefix(&root).unwrap().display().to_string()).collect::<Vec<_>>();
        let want = ["pkg/db/VERSION.txt", "pkg/db/blast/mlst.fa.nin", "pkg/db/blast/mlst.fa.nsq", "pkg/db/pubmlst/ecoli/ecoli.txt",
            "pkg/db/pubmlst/saureus/saureus.txt", "pkg/db/scheme_species_map.tab"];
        assert_eq!(names(mlst_data(&root.join("path/mlst"), &cx)), want);
        // $MLST_DBDIR wins, relative to the working folder, and is passed on and recorded.
        write(&root, "mine/VERSION.txt", "2026-01-01\n");
        cx.vars.insert("MLST_DBDIR".into(), "mine".into());
        assert_eq!(names(mlst_data(&root.join("path/mlst"), &cx)), ["mine/VERSION.txt"]);
        write(&root, "asm/contigs.fa", ">c\nACGT\n");
        std::fs::create_dir(root.join("typing")).unwrap();
        let p: MlstParams = parse(json!({"contigs": ["asm/contigs.fa"], "outfile": "typing/s1.mlst.tsv"}));
        let inv = mlst(&cx, &p).unwrap();
        assert_eq!(inv.env.get("MLST_DBDIR").map(String::as_str), Some("mine"));
        assert!(inv.bundled_data.is_some());
        // A program whose data can't be found records none (the call still runs).
        assert!(mlst_data(&root.join("nowhere/mlst"), &ctx(&root)).is_empty());
        // A wrapper script in front of mlst (as in the real-tools workflow): the paths come from
        // mlst's own --help, not from where the wrapper is.
        use std::os::unix::fs::PermissionsExt;
        let help = format!(
            "#!/bin/sh\necho 'OPTIONS'\necho \"  --blastdb [X]       BLAST database prefix [default via MLST_DBDIR/blast/mlst.fa; --blastdb overrides] (default '{0}/pkg/db/blast/mlst.fa')\"\necho \"  --datadir [X]       PubMLST data folders [default via MLST_DBDIR/pubmlst; --datadir overrides] (default '{0}/pkg/db/pubmlst')\"\n",
            root.display()
        );
        write(&root, "wrap/mlst", &help);
        std::fs::set_permissions(root.join("wrap/mlst"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(names(mlst_data(&root.join("wrap/mlst"), &ctx(&root))), want);
        assert_eq!(mlst_help_paths("  --datadir [X] x (default '')\n"), (None, None));
    }

    #[test]
    fn mlst_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "asm/s1/contigs.fa", ">c\nACGT\n");
        write(d.path(), "asm/s2/contigs.fa", ">c\nACGT\n");
        write(d.path(), "reads.fq", "@a\nACGT\n+\nIIII\n");
        std::fs::create_dir(d.path().join("typing")).unwrap();
        let p: MlstParams = parse(json!({
            "contigs": ["asm/s1/contigs.fa"], "outfile": "typing/s1.mlst.tsv", "json": "typing/s1.mlst.json",
            "scheme": "saureus", "minid": 90, "threads": 2
        }));
        let inv = mlst(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--outfile=typing/s1.mlst.tsv",
                "--json=typing/s1.mlst.json",
                "--label=s1",
                "--scheme=saureus",
                "--minid=90",
                "--threads=2",
                "--",
                "asm/s1/contigs.fa",
            ]
        );
        assert_eq!(inv.expect.len(), 2);
        // several assemblies: no label, each row named by its path
        let p: MlstParams = parse(json!({"contigs": ["asm/s1/contigs.fa", "asm/s2/contigs.fa"], "outfile": "typing/all_mlst.tsv", "full": true}));
        let inv = mlst(&cx, &p).unwrap();
        assert_eq!(inv.args, ["--outfile=typing/all_mlst.tsv", "--full", "--", "asm/s1/contigs.fa", "asm/s2/contigs.fa"]);
        assert_eq!(sample_from_output("typing/S-7_mlst.tsv", "mlst").as_deref(), Some("S-7"));
        assert_eq!(sample_from_output("typing/mlst.tsv", "mlst"), None);
        assert_eq!(sample_from_output("x/s1.AMRFinder.tsv", "amrfinder").as_deref(), Some("s1"));

        let bad = |v: serde_json::Value| mlst(&cx, &parse(v)).unwrap_err().field;
        assert_eq!(bad(json!({"contigs": [], "outfile": "typing/a.mlst.tsv"})), "contigs");
        assert_eq!(bad(json!({"contigs": ["reads.fq"], "outfile": "typing/a.mlst.tsv"})), "contigs");
        // isotable wouldn't find it: allowed, with a warning
        let side = mlst(&cx, &parse(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/s1.tsv"}))).unwrap();
        assert!(side.warnings.iter().any(|w| w.contains("isotable won't read")), "{:?}", side.warnings);
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa", "asm/s2/contigs.fa"], "outfile": "typing/a.mlst.tsv", "label": "x"})), "label");
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/a.mlst.tsv", "label": "a\tb"})), "label");
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/a.mlst.tsv", "scheme": "../x"})), "scheme");
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/a.mlst.tsv", "minscore": 101})), "minscore");
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/a.mlst.tsv", "json": "typing/a.mlst.tsv"})), "json");
        assert!(serde_json::from_value::<MlstParams>(json!({"contigs": ["a"], "outfile": "a.mlst.tsv", "csv": true})).is_err());
        assert!(serde_json::from_value::<MlstParams>(json!({"contigs": ["a"], "outfile": "a.mlst.tsv", "datadir": "x"})).is_err());
    }

    /// The files amrfinder_update 4.2.7 downloads and amrfinder_index's outputs, as empty stand-ins.
    pub fn fake_amrfinder_db(dir: &Path, version: &str, format: &str, indexed: bool) {
        for f in AMRFINDER_DB_FILES {
            write(dir, f, "");
        }
        write(dir, "version.txt", &format!("{version}\n"));
        write(dir, "database_format_version.txt", &format!("{format}\n"));
        write(dir, "taxgroup.tsv", "#taxgroup\tgpipe_taxgroup\tnumber_of_nucl_ref_genes\nEscherichia\tEscherichia_coli_Shigella\t6\nStaphylococcus_aureus\tStaphylococcus_aureus\t2\nPseudomonas_aeruginosa\tPseudomonas_aeruginosa\t0\n");
        write(dir, "AMR_DNA-Escherichia.fa", ">x\nA\n");
        write(dir, "AMR_DNA-Staphylococcus_aureus.fa", ">x\nA\n");
        if indexed {
            for f in AMRFINDER_INDEX_FILES {
                write(dir, f, "");
            }
            write(dir, "AMR_DNA-Escherichia.fa.ndb", "");
        }
    }

    #[test]
    fn amrfinder_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let mut cx = ctx(d.path());
        write(d.path(), "asm/s1/contigs.fa", ">c\nACGT\n");
        write(d.path(), "ann/s1.faa", ">p\nMKV\n");
        write(d.path(), "ann/s1.gff3", "##gff-version 3\n");
        std::fs::create_dir(d.path().join("typing")).unwrap();
        fake_amrfinder_db(&d.path().join("amrdb"), "2026-10-29.1", "4.2.0", true);
        let p: AmrfinderParams = parse(json!({
            "nucleotide": "asm/s1/contigs.fa", "database": "amrdb", "organism": "Escherichia", "plus": true,
            "output": "typing/s1.amrfinder.tsv", "threads": 4, "name": "s1", "ident_min": -1, "mutation_all": "typing/s1.mut.tsv"
        }));
        let inv = amrfinder(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--nucleotide=asm/s1/contigs.fa",
                "--database=amrdb",
                "--organism=Escherichia",
                "--plus",
                "--ident_min=-1",
                "--name=s1",
                "--output=typing/s1.amrfinder.tsv",
                "--mutation_all=typing/s1.mut.tsv",
                "--threads=4",
            ]
        );
        assert!(inv.inputs.contains(&cx.root.join("amrdb/version.txt")));
        assert_eq!(inv.outputs.len(), 2);
        // proteins with their GFF (Bakta's), and the database from $AMRFINDER_DB
        cx.vars.insert("AMRFINDER_DB".into(), cx.root.join("amrdb").into());
        let p: AmrfinderParams = parse(json!({
            "protein": "ann/s1.faa", "gff": "ann/s1.gff3", "nucleotide": "asm/s1/contigs.fa", "annotation_format": "bakta",
            "output": "typing/s1_amrfinder.tsv"
        }));
        let inv = amrfinder(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            vec![
                "--protein=ann/s1.faa".to_string(),
                "--nucleotide=asm/s1/contigs.fa".into(),
                "--gff=ann/s1.gff3".into(),
                "--annotation_format=bakta".into(),
                format!("--database={}", cx.root.join("amrdb").display()),
                "--output=typing/s1_amrfinder.tsv".into(),
            ]
        );

        let base = json!({"nucleotide": "asm/s1/contigs.fa", "database": "amrdb", "output": "typing/s1.amrfinder.tsv"});
        let bad = |extra: serde_json::Value| {
            let mut j = base.clone();
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            j.as_object_mut().unwrap().retain(|_, v| !v.is_null());
            amrfinder(&cx, &parse(j)).unwrap_err()
        };
        assert_eq!(bad(json!({"nucleotide": null})).field, "nucleotide");
        assert_eq!(bad(json!({"protein": "ann/s1.faa"})).field, "gff"); // both inputs need the GFF
        assert_eq!(bad(json!({"gff": "ann/s1.gff3"})).field, "gff"); // a GFF needs proteins
        assert_eq!(bad(json!({"annotation_format": "prokka"})).field, "annotation_format");
        assert_eq!(bad(json!({"organism": "Escherichia_coli"})).field, "organism");
        assert!(bad(json!({"organism": "Escherichia_coli"})).message.contains("Staphylococcus_aureus"));
        assert_eq!(bad(json!({"report_common": true, "organism": "Escherichia"})).field, "report_common"); // needs plus
        assert_eq!(bad(json!({"ident_min": 1.5})).field, "ident_min");
        assert_eq!(bad(json!({"coverage_min": -0.1})).field, "coverage_min");
        let side = amrfinder(&cx, &parse(json!({"nucleotide": "asm/s1/contigs.fa", "database": "amrdb", "output": "side/s1.noorganism.tsv"}))).unwrap();
        assert!(side.warnings.iter().any(|w| w.contains("isotable won't read")), "{:?}", side.warnings);
        assert_eq!(bad(json!({"mutation_all": "typing/m.tsv"})).field, "mutation_all"); // needs organism
        assert_eq!(bad(json!({"protein_output": "typing/p.faa"})).field, "protein_output");
        assert_eq!(bad(json!({"nucleotide_output": "typing/s1.amrfinder.tsv"})).field, "nucleotide_output");
        assert_eq!(bad(json!({"database": "asm"})).field, "database");
        // the database is checked the way amrfinder 4.2.7 does
        fake_amrfinder_db(&d.path().join("raw"), "2026-10-29.1", "4.2.0", false);
        assert!(bad(json!({"database": "raw"})).message.contains("not indexed"));
        // Bakta's trimmed copy: indexes without the files they came from
        fake_amrfinder_db(&d.path().join("bakta-amr"), "2026-10-29.1", "4.2.0", true);
        for f in ["AMRProt.fa", "AMR.LIB", "AMR_CDS.fa"] {
            std::fs::remove_file(d.path().join("bakta-amr").join(f)).unwrap();
        }
        let e = bad(json!({"database": "bakta-amr"}));
        assert!(e.message.contains("Bakta's database ships") && e.message.contains("amrfinderplus-db-4.2"), "{}", e.message);
        fake_amrfinder_db(&d.path().join("old"), "2025-07-16.1", "4.0.0", true);
        assert!(bad(json!({"database": "old"})).message.contains("2025-09-22.2"));
        fake_amrfinder_db(&d.path().join("new"), "2027-01-01.1", "4.4.0", true);
        assert!(bad(json!({"database": "new"})).message.contains("format 4.4.0"));
        // Staphylococcus_aureus has a DNA file without its BLAST index in the fake database
        assert!(bad(json!({"organism": "Staphylococcus_aureus"})).message.contains("AMR_DNA-Staphylococcus_aureus.fa"));
        assert!(amrfinder(&cx, &parse(json!({"protein": "ann/s1.faa", "database": "amrdb", "organism": "Staphylococcus_aureus", "output": "typing/p.amrfinder.tsv"}))).is_ok());
        std::fs::remove_file(d.path().join("amrdb/fam.tsv")).unwrap();
        assert!(bad(json!({})).message.contains("fam.tsv"));
        for invented in ["update", "force_update", "blast_bin", "parm"] {
            let mut j = base.clone();
            j[invented] = json!(true);
            assert!(serde_json::from_value::<AmrfinderParams>(j).is_err(), "{invented}");
        }
    }

    #[test]
    fn gtdbprep_ani_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let mut cx = ctx(d.path());
        write(d.path(), "genomes/b168.fna", ">c1\nACGT\n");
        for f in ANI_INDEX_FILES {
            write(d.path(), &format!("ix/{f}"), "x");
        }
        for f in ANI_DATA_FILES {
            write(d.path(), &format!("lite/{f}"), "x");
        }
        cx.vars.insert("GTDBTK_DATA_PATH".into(), cx.root.join("lite").into());
        cx.vars.insert("HOME".into(), "/home/u".into());
        cx.vars.insert("GTDBPREP_ENV".into(), "/envs/gtdbtk".into());
        let p: GtdbprepAniParams = parse(json!({"genome_dir": "genomes", "index": "ix", "work": "ani/w", "species_only": true}));
        let inv = gtdbprep_ani(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "ani".to_string(),
                "--genomes=genomes".into(),
                "--extension=fna".into(),
                "--index=ix".into(),
                format!("--data={}", cx.root.join("lite").display()),
                "--work=ani/w".into(),
                "--cache=/home/u/.cache/irit/gtdbprep-ani".into(),
                "--species-only".into(),
                "--env=/envs/gtdbtk".into(),
            ]
        );
        assert!(!inv.external);
        assert_eq!(inv.outputs, [cx.root.join("ani/w")]);
        assert_eq!(inv.expect, [vec![cx.root.join("ani/w/assigned.json")]]);
        // the genome, two index files (not the 1.7 GB markers.idx) and three data files
        assert_eq!(inv.inputs.len(), 6);
        assert!(!inv.inputs.iter().any(|i| i.ends_with("markers.idx")));

        // $XDG_CACHE_HOME wins over HOME, as in gtdbprep
        cx.vars.insert("XDG_CACHE_HOME".into(), "/xdg".into());
        assert!(gtdbprep_ani(&cx, &p).unwrap().args.contains(&"--cache=/xdg/irit/gtdbprep-ani".to_string()));

        let bad = |v: serde_json::Value| {
            let mut base = json!({"genome_dir": "genomes", "index": "ix", "work": "ani/w"});
            base.as_object_mut().unwrap().extend(v.as_object().unwrap().clone());
            gtdbprep_ani(&cx, &parse(base)).unwrap_err().field
        };
        assert_eq!(bad(json!({"index": "genomes"})), "index");
        assert_eq!(bad(json!({"data_path": "ix"})), "data_path");
        assert_eq!(bad(json!({"extension": "fa"})), "genome_dir");
        assert_eq!(bad(json!({"part_size": 1000})), "part_size");
        assert_eq!(bad(json!({"source": "https://x/{part}"})), "part_size");
        assert_eq!(bad(json!({"fetch_jobs": 17})), "fetch_jobs");
        assert_eq!(bad(json!({"cache": "irit-runs/c"})), "cache");
        assert_eq!(bad(json!({"skani": "bin/skani"})), "skani");
        let ok = gtdbprep_ani(&cx, &parse(json!({"genome_dir": "genomes", "index": "ix", "work": "ani/w", "source": "gtdb", "fetch_jobs": 4}))).unwrap();
        assert!(ok.args.contains(&"--source=gtdb".to_string()) && ok.args.contains(&"--fetch-jobs=4".to_string()));
        std::fs::create_dir_all(d.path().join("ani/w")).unwrap();
        write(d.path(), "ani/w/assigned.json", "[]");
        assert_eq!(bad(json!({})), "work");
        assert!(serde_json::from_value::<GtdbprepAniParams>(json!({"genome_dir": "g", "index": "i", "work": "w", "full_db": "x"})).is_err());
    }

    #[test]
    fn databases_fall_back_to_the_projects_dblock_lock() {
        let d = tempfile::tempdir().unwrap();
        let mut cx = ctx(d.path());
        cx.vars.insert("HOME".into(), "/home/u".into());
        write(d.path(), "genomes/b168.fna", ">c1\nACGT\n");
        for f in ANI_INDEX_FILES {
            write(d.path(), &format!("store/ani/ix/{f}"), "x");
        }
        for f in ANI_DATA_FILES {
            write(d.path(), &format!("store/lite/release232/{f}"), "x");
        }
        let store = cx.root.join("store");
        let p: GtdbprepAniParams = parse(json!({"genome_dir": "genomes", "work": "ani/w"}));
        // no lock file: the same refusals as before
        let e = gtdbprep_ani(&cx, &p).unwrap_err();
        assert_eq!(e.field, "index");
        // as dblock writes it (other keys left out)
        let lock = format!(
            "[[database]]\nid = \"gtdbtk-r232-ani-screen\"\npath = \"{s}/ani\"\ndata_path = \"{s}/ani/ix\"\n\n\
             [[database]]\nid = \"gtdbtk-r232-placement-lite\"\npath = \"{s}/lite\"\ndata_path = \"{s}/lite/release232\"\nenv = \"GTDBTK_DATA_PATH\"\n",
            s = store.display()
        );
        write(d.path(), "dblock.lock", &lock);
        let inv = gtdbprep_ani(&cx, &p).unwrap();
        assert!(inv.args.contains(&format!("--index={}/ani/ix", store.display())), "{:?}", inv.args);
        assert!(inv.args.contains(&format!("--data={}/lite/release232", store.display())), "{:?}", inv.args);
        // and says so, naming the database
        assert_eq!(inv.warnings.len(), 2, "{:?}", inv.warnings);
        assert!(inv.warnings[0].contains("gtdbtk-r232-ani-screen") && inv.warnings[1].contains("GTDBTK_DATA_PATH is not set"), "{:?}", inv.warnings);
        // a parameter or the environment variable still wins, without a warning
        let mut cx2 = ctx(d.path());
        cx2.vars.insert("HOME".into(), "/home/u".into());
        cx2.vars.insert("GTDBTK_DATA_PATH".into(), store.join("lite/release232").into());
        let inv = gtdbprep_ani(&cx2, &parse(json!({"genome_dir": "genomes", "index": "store/ani/ix", "work": "ani/w"}))).unwrap();
        assert!(inv.warnings.is_empty(), "{:?}", inv.warnings);
        // two candidates: refused, naming both
        write(d.path(), "dblock.lock", &format!("{lock}\n[[database]]\nid = \"gtdbtk-r226-placement-lite\"\ndata_path = \"/x\"\nenv = \"GTDBTK_DATA_PATH\"\n"));
        let e = gtdbprep_ani(&cx, &p).unwrap_err();
        assert_eq!(e.field, "data_path");
        assert!(e.message.contains("gtdbtk-r232-placement-lite, gtdbtk-r226-placement-lite"), "{}", e.message);
    }

    fn shovill_log(size: u64, depth: u64, subsample: bool, assembly: u64) -> String {
        let sub = if subsample { "[shovill] Subsampling reads by factor 0.2412 to get from 622x to 150x" } else { "[shovill] No read depth reduction requested or necessary." };
        format!(
            "[shovill] Using genome size {size} bp\n[shovill] Estimated sequencing depth: {depth} x\n{sub}\n[shovill] Assembly is {assembly}, estimated genome size was {size} (-29.09%)\n"
        )
    }

    /// Lines from the depth experiment's logs (K. pneumoniae, 5.5 Mb), issue #22.
    #[test]
    fn shovill_low_depth_warnings() {
        // 10x, no gsize: estimate 87 kb, subsampled, 62 kb assembled.
        let w = shovill_log_warnings(&shovill_log(87239, 622, true, 61861), false);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("only 0.09 Mb") && w[0].contains("gsize"), "{w:?}");
        // 15x, no gsize: estimate 0.62 Mb, not subsampled, full assembly; real depth under 15x.
        let w = shovill_log_warnings(&shovill_log(624291, 130, false, 5496787), false);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].contains("about 15x") && w[1].contains("about 15x depth"), "{w:?}");
        // 20x, no gsize: estimate 1.8 Mb; a note with the real depth, no depth warning.
        let w = shovill_log_warnings(&shovill_log(1794857, 60, false, 5458361), false);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("about 20x"), "{w:?}");
        // 40x, no gsize: a good estimate, nothing to say.
        assert!(shovill_log_warnings(&shovill_log(5088033, 42, false, 5465887), false).is_empty());
        // 5x with gsize: 1.6 Mb of 5.5 Mb.
        let w = shovill_log_warnings(&shovill_log(5500000, 4, false, 1616790), true);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("(29%)"), "{w:?}");
        // 10x with gsize: a full assembly at low depth.
        let w = shovill_log_warnings(&shovill_log(5500000, 9, false, 5361487), true);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("about 9x depth"), "{w:?}");
        // 20x with gsize: fine.
        assert!(shovill_log_warnings(&shovill_log(5500000, 19, false, 5458422), true).is_empty());
        // A log without the lines (a failed or other version's run) gives nothing.
        assert!(shovill_log_warnings("", false).is_empty());
    }

    #[test]
    fn mlst_auto_scheme_without_st_is_flagged() {
        let rows = [
            json!({"sample": "a", "scheme": "ecoli_achtman_4", "st": "-"}),
            json!({"sample": "b", "scheme": "-", "st": "-"}),
            json!({"sample": "c", "scheme": "klebsiella", "st": "101"}),
        ];
        let w = mlst_scheme_warnings(&rows);
        assert_eq!(w.len(), 1);
        assert!(w[0].starts_with("a: mlst chose scheme ecoli_achtman_4"), "{w:?}");
    }

    #[test]
    fn known_stderr_warnings_are_surfaced() {
        let checkm2 = "[12:00] INFO: Running\n/env/lib/python3.12/site-packages/checkm2/../sklearn/base.py:525: InconsistentVersionWarning: Trying to unpickle estimator MinMaxScaler from version 1.6.1 when using version 1.9.1. This might lead to breaking code or invalid results. Use at your own risk. For more info please refer to:\n  warnings.warn(\n/env/.../base.py:525: InconsistentVersionWarning: Trying to unpickle estimator DecisionTreeRegressor from version 1.6.1 when using version 1.9.1. This might lead to breaking code\n";
        let w = stderr_warnings("checkm2", checkm2);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("1.6.1 when using version 1.9.1") && w[0].contains("invalid results"), "{w:?}");
        let amr = "Software version: 4.2.7\nWARNING: This was compiled for running under bioconda, but $CONDA_PREFIX was not found\nReverting to hard coded directory: /x\n";
        assert_eq!(stderr_warnings("amrfinder", amr), ["AMRFinderPlus: This was compiled for running under bioconda, but $CONDA_PREFIX was not found"]);
        assert!(stderr_warnings("checkm2", amr).is_empty());
        assert!(stderr_warnings("shovill", checkm2).is_empty());
        let medaka = "[11:53:12 - Predict] Processing region(s)\n[11:53:12 - Predict] Using model: /env/lib/python3.12/site-packages/medaka/data/r1041_e82_400bps_sup_v5.0.0_model_pt.tar.gz.\n";
        let m = medaka_model_file(medaka).unwrap();
        assert_eq!(m, Path::new("/env/lib/python3.12/site-packages/medaka/data/r1041_e82_400bps_sup_v5.0.0_model_pt.tar.gz"));
        assert!(is_medaka_model(&m.to_string_lossy()));
        assert!(medaka_model_file("nothing").is_none());
        let g = "gtdbprep: running skani sketch\ngtdbprep: warning: skani made no sketch of g/a.fna (genome a), usually because it is too short; it is not screened\n";
        assert_eq!(stderr_warnings("gtdbprep", g), ["gtdbprep: skani made no sketch of g/a.fna (genome a), usually because it is too short; it is not screened"]);
    }

    #[cfg(unix)]
    #[test]
    fn conda_prefix_follows_the_program() {
        let d = tempfile::tempdir().unwrap();
        let env = std::fs::canonicalize(d.path()).unwrap().join("envs/amr");
        write(&env, "bin/amrfinder", "");
        assert_eq!(conda_prefix(&env.join("bin/amrfinder")), None);
        std::fs::create_dir(env.join("conda-meta")).unwrap();
        assert_eq!(conda_prefix(&env.join("bin/amrfinder")), Some(env.clone()));
        // a symlink on PATH pointing into the environment
        std::fs::create_dir(d.path().join("bin")).unwrap();
        std::os::unix::fs::symlink(env.join("bin/amrfinder"), d.path().join("bin/amrfinder")).unwrap();
        assert_eq!(conda_prefix(&d.path().join("bin/amrfinder")), Some(env));
    }

    #[test]
    fn coverm_genome_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        for f in ["trim/s1_R1.fq.gz", "trim/s1_R2.fq.gz"] {
            write(d.path(), f, "@a\nACGT\n+\nIIII\n");
        }
        write(d.path(), "asm/s1/contigs.fa", ">c1\nACGT\n");
        let base = json!({"assembly": "asm/s1/contigs.fa", "r1": "trim/s1_R1.fq.gz", "r2": "trim/s1_R2.fq.gz", "output": "cov/s1.coverm.tsv"});
        let inv = coverm_genome(&cx, &parse(base.clone())).unwrap();
        assert_eq!(
            inv.args,
            [
                "genome",
                "--reference=asm/s1/contigs.fa",
                "--single-genome",
                "--output-file=cov/s1.coverm.tsv",
                "--min-covered-fraction=0",
                "--methods",
                "mean",
                "covered_fraction",
                "--coupled",
                "trim/s1_R1.fq.gz",
                "trim/s1_R2.fq.gz",
            ]
        );
        let with = |k: &str, v: serde_json::Value| {
            let mut j = base.clone();
            j[k] = v;
            j
        };
        let inv = coverm_genome(&cx, &parse(with("threads", json!(4)))).unwrap();
        assert!(inv.args.contains(&"--threads=4".to_string()));
        let bad = |j: serde_json::Value| coverm_genome(&cx, &parse(j)).unwrap_err();
        assert!(coverm_genome(&cx, &parse(with("output", json!("cov/s1.tsv")))).unwrap().warnings.iter().any(|w| w.contains("isotable won't read")));
        assert_eq!(bad(with("r2", json!("trim/s1_R1.fq.gz"))).field, "r2");
        assert_eq!(bad(with("assembly", json!("trim/s1_R1.fq.gz"))).field, "assembly");
        assert_eq!(bad(with("min_read_percent_identity", json!(101))).field, "min_read_percent_identity");
    }

    #[test]
    fn kleborate_args_checks_and_data() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        for f in ["genomes/s1.fna", "-s2.fasta.gz", "asm/s1/contigs.fa", "other/s1.fa"] {
            write(d.path(), f, ">c1\nACGT\n");
        }
        let inv = kleborate(&cx, &parse(json!({"assemblies": ["genomes/s1.fna", "-s2.fasta.gz"], "outdir": "kleb"}))).unwrap();
        assert_eq!(inv.args, ["--outdir", "kleb", "--preset=kpsc", "--assemblies", "genomes/s1.fna", "./-s2.fasta.gz"]);
        assert!(inv.expect.is_empty());
        assert_eq!(inv.outputs.len(), 4);
        assert!(inv.outputs[0].ends_with("kleb/klebsiella_pneumo_complex_output.txt"));
        // modules alone: no preset, and Kleborate names the table after the first module
        let inv = kleborate(&cx, &parse(json!({"assemblies": ["genomes/s1.fna"], "outdir": "-k", "modules": ["klebsiella__ybst", "klebsiella__cbst"]}))).unwrap();
        assert_eq!(inv.args, ["--outdir", "./-k", "--modules=klebsiella__ybst,klebsiella__cbst", "--assemblies", "genomes/s1.fna"]);
        assert!(inv.outputs[0].ends_with("-k/klebsiella__ybst_output.txt"));
        let inv = kleborate(&cx, &parse(json!({"assemblies": ["genomes/s1.fna"], "outdir": "k", "preset": "escherichia"}))).unwrap();
        assert!(inv.args.contains(&"--preset=escherichia".to_string()));
        let bad = |j: serde_json::Value| kleborate(&cx, &parse(j)).unwrap_err();
        assert!(bad(json!({"assemblies": ["asm/s1/contigs.fa"], "outdir": "k"})).message.contains("link_as"));
        assert!(bad(json!({"assemblies": ["genomes/s1.fna", "other/s1.fa"], "outdir": "k"})).message.contains("both be named \"s1\""));
        assert_eq!(bad(json!({"assemblies": ["genomes/s1.fna"], "outdir": "k", "modules": ["a,b"]})).field, "modules");
        assert_eq!(bad(json!({"assemblies": [], "outdir": "k"})).field, "assemblies");
        write(d.path(), "full/x.txt", "x");
        assert_eq!(bad(json!({"assemblies": ["genomes/s1.fna"], "outdir": "full"})).field, "outdir");

        // the bundled databases, found through a shell wrapper that execs the real script
        let env = d.path().join("env");
        write(&env, "bin/kleborate", "#!/usr/bin/env python3\n");
        for f in ["kleborate/modules/klebsiella__ybst/data/ybt_alleles.fasta", "kleborate/modules/klebsiella__ybst/klebsiella__ybst.py", "kaptive/data/Klebsiella_k_locus_primary_reference.gbk", "kaptive-3.2.2.dist-info/METADATA"] {
            write(&env, &format!("lib/python3.12/site-packages/{f}"), "x");
        }
        write(d.path(), "tool-bin/kleborate", &format!("#!/bin/sh\nPATH={0}/bin:$PATH\nexport PATH\nexec {0}/bin/kleborate \"$@\"\n", env.display()));
        let names: Vec<String> = kleborate_data(&d.path().join("tool-bin/kleborate"), &cx).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Klebsiella_k_locus_primary_reference.gbk", "METADATA", "ybt_alleles.fasta"]);
        assert!(kleborate_data(&d.path().join("genomes/s1.fna"), &cx).is_empty());
        assert_eq!(version_number("Kleborate v3.2.4"), Some("3.2.4"));
    }
}
