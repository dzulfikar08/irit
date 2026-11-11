//! Write a synthetic aligned FASTA for testing msascope:
//! `msagen ROWS COLS OUT [--protein] [--seed N] [--tree TREE.nwk]`.
//! OUT ending in .gz is gzip-compressed; `-` writes to stdout. `--tree` also writes a Newick tree
//! whose leaves are the sequences, each clade under its own subtree.

use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::{Context, Result, bail};
use msascope::alphabet::Kind;
use msascope::synth::{Spec, write_fasta};

const USAGE: &str = "usage: msagen ROWS COLS OUT [--protein] [--seed N] [--tree TREE.nwk]   (OUT: file, file.gz or -)";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut pos = Vec::new();
    let (mut kind, mut seed) = (Kind::Nucleotide, 1u64);
    let mut tree: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--protein" => kind = Kind::Protein,
            "--seed" => seed = it.next().context("--seed needs a value")?.parse()?,
            "--tree" => tree = Some(it.next().context("--tree needs a file name")?.clone()),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => pos.push(a.as_str()),
        }
    }
    let [rows, cols, out] = pos[..] else { bail!("{USAGE}") };
    let count = |s: &str, what: &str| -> Result<usize> {
        match s.parse::<usize>().with_context(|| format!("{what} must be a whole number"))? {
            0 => bail!("{what} must be at least 1"),
            n => Ok(n),
        }
    };
    let spec = Spec { rows: count(rows, "ROWS")?, cols: count(cols, "COLS")?, kind, clades: 32, seed };
    let write = |w: &mut dyn Write| -> Result<()> {
        let mut w = BufWriter::with_capacity(1 << 20, w);
        write_fasta(&spec, &mut w)?;
        w.flush()?;
        Ok(())
    };
    if let Some(p) = &tree {
        let f = File::create(p).with_context(|| format!("creating {p}"))?;
        let mut w = BufWriter::with_capacity(1 << 20, f);
        msascope::synth::write_newick(spec.rows, spec.clades, seed, &mut w)?;
        w.flush()?;
    }
    match out {
        "-" => write(&mut std::io::stdout().lock())?,
        p => {
            let f = File::create(p).with_context(|| format!("creating {p}"))?;
            if p.ends_with(".gz") {
                let mut gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
                write(&mut gz)?;
                gz.finish()?.sync_all()?;
            } else {
                let mut f = f;
                write(&mut f)?;
                f.sync_all()?;
            }
        }
    }
    Ok(())
}
