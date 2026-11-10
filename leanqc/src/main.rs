//! leanqc: quality control for FASTQ/FASTA files in one pass, small memory, one binary.
//!
//! Covers FastQC's main modules (per-position quality and base content, per-read quality,
//! GC, length, N content, adapters, duplication, overrepresented sequences) and writes a
//! self-contained HTML report, JSON, a summary TSV and a MultiQC general-stats table.

mod report;
mod stats;

use anyhow::{Context, Result, bail};
use clap::Parser;
use rayon::prelude::*;
use stats::{Report, Status};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(version, about = "Fast, low-memory quality control for sequencing reads")]
struct Cli {
    /// FASTQ or FASTA files (plain, gzip, bzip2, xz or zstd).
    #[arg(required = true)]
    files: Vec<PathBuf>,

    /// Folder for reports.
    #[arg(short, long, default_value = "leanqc")]
    outdir: PathBuf,

    /// Files processed at once (each file uses one thread).
    #[arg(short, long, default_value_t = default_threads())]
    threads: usize,

    /// Distinct sequences tracked for duplication and overrepresented sequences (FastQC: 100,000).
    /// Tracked sequences are counted through the whole file.
    #[arg(long, default_value_t = 100_000)]
    dup_distinct: usize,

    /// Quality encoding, instead of detecting it: 33 (Sanger, Illumina 1.8+) or 64 (Illumina 1.3–1.7).
    #[arg(long, value_parser = ["33", "64"])]
    phred: Option<String>,

    /// Skip the HTML reports (JSON and TSV only).
    #[arg(long)]
    no_html: bool,
}

fn default_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(4)
}

fn stem(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let lower = name.to_ascii_lowercase();
    for ext in [".fastq.gz", ".fq.gz", ".fastq.bz2", ".fq.bz2", ".fastq.xz", ".fastq.zst", ".fastq", ".fq", ".fasta.gz", ".fa.gz", ".fasta", ".fa", ".fna", ".gz"] {
        if lower.ends_with(ext) {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    name.into_owned()
}

fn analyse(path: &Path, dup_distinct: usize, phred: Option<u8>) -> Result<Report> {
    let mut reader = needletail::parse_fastx_file(path).with_context(|| format!("opening {}", path.display()))?;
    let mut acc = stats::Acc::new(dup_distinct);
    while let Some(rec) = reader.next() {
        let rec = rec.with_context(|| format!("reading {}", path.display()))?;
        let seq = rec.seq();
        acc.add(rec.id(), &seq, rec.qual());
    }
    Ok(acc.finish(path.file_name().unwrap_or_default().to_string_lossy().into_owned(), phred))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.outdir).with_context(|| format!("creating {}", cli.outdir.display()))?;
    let mut stems = std::collections::BTreeSet::new();
    for f in &cli.files {
        if !stems.insert(stem(f)) {
            bail!("two inputs share the name `{}`; reports would overwrite each other", stem(f));
        }
    }
    rayon::ThreadPoolBuilder::new().num_threads(cli.threads.max(1)).build_global()?;
    let version = env!("CARGO_PKG_VERSION");
    let phred: Option<u8> = cli.phred.as_deref().map(|p| p.parse()).transpose()?;

    let results: Vec<(PathBuf, Result<(Report, f64)>)> = cli
        .files
        .par_iter()
        .map(|f| {
            let t = Instant::now();
            let r = analyse(f, cli.dup_distinct, phred).and_then(|rep| {
                let s = stem(f);
                let json = serde_json::to_vec(&rep)?;
                std::fs::write(cli.outdir.join(format!("{s}_leanqc.json")), json)?;
                if !cli.no_html {
                    std::fs::write(cli.outdir.join(format!("{s}_leanqc.html")), report::html(&rep, version))?;
                }
                Ok((rep, t.elapsed().as_secs_f64()))
            });
            (f.clone(), r)
        })
        .collect();

    let mut tsv = String::from(
        "file\treads\tbases\tmean_len\tgc_percent\tq30_percent\tdedup_remaining_percent\tmax_adapter_percent\tfails\twarns\n",
    );
    let mut mqc = String::from(
        "# id: 'leanqc'\n# section_name: 'leanqc'\n# plot_type: 'generalstats'\nSample\treads\tgc_percent\tq30_percent\tdedup_remaining_percent\tmax_adapter_percent\n",
    );
    let mut failed = 0;
    for (path, res) in &results {
        match res {
            Ok((r, secs)) => {
                let fails: Vec<_> = r.checks.iter().filter(|c| c.status == Status::Fail).map(|c| c.name).collect();
                let warns: Vec<_> = r.checks.iter().filter(|c| c.status == Status::Warn).map(|c| c.name).collect();
                let amax = r.adapters.iter().map(|a| a.max_percent).fold(0.0, f64::max);
                let q30 = r.q30_percent.map(|v| format!("{v:.2}")).unwrap_or_default();
                let dedup = r.duplication.remaining_if_deduplicated_percent;
                tsv.push_str(&format!(
                    "{}\t{}\t{}\t{:.1}\t{:.2}\t{q30}\t{dedup:.1}\t{amax:.2}\t{}\t{}\n",
                    r.file,
                    r.reads,
                    r.bases,
                    r.mean_len,
                    r.gc_percent,
                    fails.join(","),
                    warns.join(",")
                ));
                mqc.push_str(&format!("{}\t{}\t{:.2}\t{q30}\t{dedup:.1}\t{amax:.2}\n", stem(path), r.reads, r.gc_percent));
                eprintln!(
                    "{}: {} reads, GC {:.1}%, Q30 {}%, {} fail / {} warn ({secs:.1}s)",
                    r.file,
                    r.reads,
                    r.gc_percent,
                    if q30.is_empty() { "n/a" } else { q30.as_str() },
                    fails.len(),
                    warns.len()
                );
                for w in &r.warnings {
                    eprintln!("  warning: {w}");
                }
            }
            Err(e) => {
                failed += 1;
                eprintln!("{}: error: {e:#}", path.display());
            }
        }
    }
    std::fs::File::create(cli.outdir.join("leanqc_summary.tsv"))?.write_all(tsv.as_bytes())?;
    std::fs::File::create(cli.outdir.join("leanqc_mqc.tsv"))?.write_all(mqc.as_bytes())?;
    if failed > 0 {
        bail!("{failed} of {} files could not be read", results.len());
    }
    Ok(())
}
