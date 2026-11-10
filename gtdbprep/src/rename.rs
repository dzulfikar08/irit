//! Find input genomes the way GTDB-Tk does, give them opaque IDs, and rewrite contig headers.
//!
//! Only header lines change: sequence lines are copied byte for byte, so gene calling sees the
//! same sequence. The mapping back to the original names stays in the local work folder.

use anyhow::{Context, Result, bail};
use flate2::read::MultiGzDecoder;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// One input genome file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputGenome {
    pub path: PathBuf,
    /// The genome ID GTDB-Tk would use for this file (`remove_extension` in GTDB-Tk's biolib).
    pub gtdbtk_id: String,
}

/// GTDB-Tk's genome ID for a file name: strip the last occurrence of `extension`, then one
/// trailing dot (`ecoli.fna.gz` with `-x gz` gives `ecoli.fna`).
pub fn gtdbtk_genome_id(file_name: &str, extension: &str) -> String {
    let mut f = if !extension.is_empty() && file_name.ends_with(extension) {
        file_name[..file_name.rfind(extension).unwrap()].to_string()
    } else {
        match file_name.rfind('.') {
            Some(i) if i > 0 => file_name[..i].to_string(),
            _ => file_name.to_string(),
        }
    };
    if f.ends_with('.') {
        f.pop();
    }
    f
}

/// Files in `dir` whose names end with `extension` (as `gtdbtk --genome_dir -x` selects them),
/// sorted by name.
pub fn list_genomes(dir: &Path, extension: &str) -> Result<Vec<InputGenome>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for entry in fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(extension) || !entry.path().is_file() {
            continue;
        }
        let id = gtdbtk_genome_id(&name, extension);
        if id.is_empty() {
            bail!("{name}: empty genome ID after removing the extension");
        }
        if !seen.insert(id.clone()) {
            bail!("two input files give the same GTDB-Tk genome ID {id}");
        }
        out.push(InputGenome {
            path: entry.path(),
            gtdbtk_id: id,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    if out.is_empty() {
        bail!("no files ending in '{extension}' in {}", dir.display());
    }
    Ok(out)
}

/// A fresh opaque genome ID: `g` + 16 hex digits from the OS random source. It carries no
/// information about the input (not even its position in the list).
pub fn random_id(taken: &HashSet<String>) -> Result<String> {
    let mut f = File::open("/dev/urandom").context("opening /dev/urandom")?;
    loop {
        let mut b = [0u8; 8];
        f.read_exact(&mut b)?;
        let id = format!("g{}", crate::hash::hex(&b));
        if !taken.contains(&id) {
            return Ok(id);
        }
    }
}

/// Opaque contig name: `<genome id>_c<n>`, numbered from 1 in file order.
pub fn contig_name(genome_id: &str, n: usize) -> String {
    format!("{genome_id}_c{n}")
}

/// Open a FASTA file, decompressing gzip if the file starts with the gzip magic bytes.
pub fn open_fasta(path: &Path) -> Result<Box<dyn BufRead>> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut magic = [0u8; 2];
    let n = f.read(&mut magic)?;
    let f = File::open(path)?;
    if n == 2 && magic == [0x1f, 0x8b] {
        Ok(Box::new(BufReader::new(MultiGzDecoder::new(f))))
    } else {
        Ok(Box::new(BufReader::new(f)))
    }
}

/// The input is not a FASTA file (empty, or sequence before the first header). `prep` leaves such
/// genomes out instead of stopping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotFasta(pub &'static str);

impl std::fmt::Display for NotFasta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for NotFasta {}

/// Copy a FASTA stream, replacing every header with an opaque contig name. Returns the original
/// header lines (without `>` and line ending) in order; contig `n` (1-based) was `headers[n-1]`.
pub fn rename_fasta<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    genome_id: &str,
) -> Result<Vec<String>> {
    let mut headers = Vec::new();
    let mut line = Vec::new();
    let mut saw_sequence_before_header = false;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if line.first() == Some(&b'>') {
            let mut end = line.len();
            while end > 1 && matches!(line[end - 1], b'\n' | b'\r') {
                end -= 1;
            }
            headers.push(String::from_utf8_lossy(&line[1..end]).into_owned());
            let newline: &[u8] = if line.ends_with(b"\r\n") {
                b"\r\n"
            } else {
                b"\n"
            };
            writer.write_all(b">")?;
            writer.write_all(contig_name(genome_id, headers.len()).as_bytes())?;
            writer.write_all(newline)?;
        } else {
            if headers.is_empty() && line.iter().any(|b| !b.is_ascii_whitespace()) {
                saw_sequence_before_header = true;
            }
            writer.write_all(&line)?;
        }
    }
    writer.flush()?;
    if saw_sequence_before_header {
        return Err(NotFasta("sequence before the first '>' header: not a FASTA file").into());
    }
    if headers.is_empty() {
        return Err(NotFasta("no FASTA headers found (empty file?)").into());
    }
    Ok(headers)
}

/// Rename one genome file into `dest` (uncompressed). Returns the original headers.
pub fn rename_file(src: &Path, dest: &Path, genome_id: &str) -> Result<Vec<String>> {
    let reader = open_fasta(src)?;
    let out = File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    rename_fasta(reader, BufWriter::new(out), genome_id)
        .with_context(|| format!("renaming {}", src.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};

    #[test]
    fn genome_ids_follow_gtdbtk() {
        assert_eq!(gtdbtk_genome_id("ecoli.fna.gz", "gz"), "ecoli.fna");
        assert_eq!(gtdbtk_genome_id("ecoli.fna", "fna"), "ecoli");
        assert_eq!(gtdbtk_genome_id("ecoli.fna", ".fna"), "ecoli");
        assert_eq!(gtdbtk_genome_id("a.fna.fna", "fna"), "a.fna");
        assert_eq!(
            gtdbtk_genome_id("test_genome_G021783475.fna.gz", "gz"),
            "test_genome_G021783475.fna"
        );
    }

    #[test]
    fn headers_replaced_sequence_untouched() {
        let input = b">NODE_1 length=8 cov=2\nACGT\nAC\n>secret plasmid\r\nGGGG\r\n";
        let mut out = Vec::new();
        let headers = rename_fasta(&input[..], &mut out, "gabc").unwrap();
        assert_eq!(headers, vec!["NODE_1 length=8 cov=2", "secret plasmid"]);
        assert_eq!(out, b">gabc_c1\nACGT\nAC\n>gabc_c2\r\nGGGG\r\n");
    }

    #[test]
    fn rejects_non_fasta() {
        let e = rename_fasta(&b"ACGT\n>x\nA\n"[..], Vec::new(), "g").unwrap_err();
        assert!(e.downcast_ref::<NotFasta>().is_some());
        let e = rename_fasta(&b""[..], Vec::new(), "g").unwrap_err();
        assert!(e.downcast_ref::<NotFasta>().is_some());
        // Still recognisable through rename_file's context.
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("e.fna"), b"").unwrap();
        let e = rename_file(&d.path().join("e.fna"), &d.path().join("o"), "g").unwrap_err();
        assert!(e.downcast_ref::<NotFasta>().is_some(), "{e:#}");
    }

    #[test]
    fn reads_gzip_and_lists_like_gtdbtk() {
        let d = tempfile::tempdir().unwrap();
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(b">orig\nACGT\n").unwrap();
        fs::write(d.path().join("b.fna.gz"), gz.finish().unwrap()).unwrap();
        fs::write(d.path().join("a.fna.gz"), b">plain\nAAAA\n").unwrap();
        fs::write(d.path().join("notes.txt"), b"x").unwrap();
        let g = list_genomes(d.path(), "gz").unwrap();
        let ids: Vec<_> = g.iter().map(|g| g.gtdbtk_id.as_str()).collect();
        assert_eq!(ids, vec!["a.fna", "b.fna"]);
        let dest = d.path().join("out.fna");
        let headers = rename_file(&g[1].path, &dest, "gx").unwrap();
        assert_eq!(headers, vec!["orig"]);
        assert_eq!(fs::read(&dest).unwrap(), b">gx_c1\nACGT\n");
    }

    #[test]
    fn random_ids_are_opaque_and_unique() {
        let mut taken = HashSet::new();
        for _ in 0..100 {
            let id = random_id(&taken).unwrap();
            assert_eq!(id.len(), 17);
            assert!(id.starts_with('g') && id[1..].chars().all(|c| c.is_ascii_hexdigit()));
            assert!(taken.insert(id));
        }
    }
}
