//! Write a synthetic aligned FASTA for testing msascope: `msagen ROWS COLS OUT [--protein] [--seed N]`.
//! OUT ending in .gz is gzip-compressed; `-` writes to stdout.

use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::{Context, Result, bail};
use msascope::alphabet::Kind;
use msascope::synth::{Spec, write_fasta};

const USAGE: &str = "usage: msagen ROWS COLS OUT [--protein] [--seed N]   (OUT: file, file.gz or -)";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut pos = Vec::new();
    let (mut kind, mut seed) = (Kind::Nucleotide, 1u64);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--protein" => kind = Kind::Protein,
            "--seed" => seed = it.next().context("--seed needs a value")?.parse()?,
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
