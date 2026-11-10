//! The parts of skani 0.3.1's database format the local ANI screen needs.
//!
//! skani (Shaw & Yu, Nature Methods 2023; MIT licence, <https://github.com/bluenote-1577/skani>)
//! writes a consolidated database as three files, all bincode 1 (fixed-width little-endian
//! integers, `u64` lengths):
//!
//! - `markers.bin`: `(SketchParams, Vec<Sketch>)`, one marker-only sketch per reference;
//! - `sketches.db`: the full `(SketchParams, Sketch)` of each reference, back to back;
//! - `index.db`: `Vec<IndexEntry { file_name: String, offset: u64, length: u64 }>`.
//!
//! `skani search` screens every reference by its marker k-mers (`markers.bin`, position `j`),
//! then reads entry `j` of `index.db` from `sketches.db` for each reference that passes. This
//! module reads markers, splits records, and writes a small database of the same format that
//! holds only some references. Full sketches are copied byte for byte.

use anyhow::{Context, Result, bail, ensure};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

/// Sanity limit for one length prefix (no field in a skani sketch comes near it).
const MAX_LEN: u64 = 1 << 34;

/// A reader over bincode 1 data.
pub struct Bin<R: Read> {
    r: R,
    pos: u64,
}

impl<R: Read> Bin<R> {
    pub fn new(r: R) -> Self {
        Bin { r, pos: 0 }
    }

    pub fn pos(&self) -> u64 {
        self.pos
    }

    fn exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.r
            .read_exact(buf)
            .with_context(|| format!("unexpected end of data at byte {}", self.pos))?;
        self.pos += buf.len() as u64;
        Ok(())
    }

    pub fn u8(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.exact(&mut b)?;
        Ok(b[0])
    }

    pub fn u64(&mut self) -> Result<u64> {
        let mut b = [0u8; 8];
        self.exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    pub fn length(&mut self) -> Result<u64> {
        let n = self.u64()?;
        ensure!(
            n < MAX_LEN,
            "implausible length {n} at byte {}",
            self.pos - 8
        );
        Ok(n)
    }

    pub fn bytes(&mut self, n: u64) -> Result<Vec<u8>> {
        let mut v = vec![0u8; n as usize];
        self.exact(&mut v)?;
        Ok(v)
    }

    pub fn skip(&mut self, n: u64) -> Result<()> {
        let copied = std::io::copy(&mut (&mut self.r).take(n), &mut std::io::sink())?;
        ensure!(
            copied == n,
            "unexpected end of data at byte {}",
            self.pos + copied
        );
        self.pos += n;
        Ok(())
    }

    pub fn string(&mut self) -> Result<String> {
        let n = self.length()?;
        String::from_utf8(self.bytes(n)?).context("a name in the skani data is not UTF-8")
    }
}

/// skani's `SketchParams`, as far as the screen needs it. `raw` is the exact serialised form; it
/// starts every `sketches.db` record and every `markers.bin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Params {
    pub c: u64,
    pub k: u64,
    pub marker_c: u64,
    pub use_aa: bool,
    pub raw: Vec<u8>,
}

/// Read a `SketchParams` and keep its bytes.
pub fn read_params<R: Read>(b: &mut Bin<R>) -> Result<Params> {
    let mut raw = Vec::new();
    let mut take = |b: &mut Bin<R>, n: u64| -> Result<Vec<u8>> {
        let v = b.bytes(n)?;
        raw.extend_from_slice(&v);
        Ok(v)
    };
    let le = |v: &[u8]| u64::from_le_bytes(v.try_into().unwrap());
    let c = le(&take(b, 8)?);
    let k = le(&take(b, 8)?);
    let marker_c = le(&take(b, 8)?);
    let flags = take(b, 2)?; // use_syncs, use_aa
    let n = le(&take(b, 8)?);
    ensure!(
        n <= 1024,
        "unexpected skani parameters (encoding table of {n})"
    );
    take(b, 8 * n)?; // acgt_to_aa_encoding: Vec<u64>
    let n = le(&take(b, 8)?);
    ensure!(
        n <= 1024,
        "unexpected skani parameters (letter table of {n})"
    );
    take(b, n)?; // acgt_to_aa_letters: Vec<u8>
    take(b, 8)?; // orf_size
    ensure!(
        flags[0] <= 1 && flags[1] <= 1,
        "these are not skani 0.3 sketch parameters"
    );
    Ok(Params {
        c,
        k,
        marker_c,
        use_aa: flags[1] == 1,
        raw,
    })
}

/// One sketch's name and marker k-mers (2-bit encoded canonical 21-mers).
#[derive(Debug, Clone)]
pub struct Markers {
    pub file_name: String,
    pub markers: Vec<u64>,
}

/// Read one `Sketch` and keep its name and marker set. Works for marker-only and full sketches.
pub fn read_sketch_markers<R: Read>(b: &mut Bin<R>) -> Result<Markers> {
    let file_name = b.string()?;
    match b.u8()? {
        0 => {}
        1 => {
            // kmer_seeds_k: a map u32 -> u64
            let n = b.length()?;
            b.skip(12 * n)?;
        }
        t => bail!("bad option tag {t} in sketch {file_name}"),
    }
    let n = b.length()?; // multi_position_storage: Vec<SmallVec<[SeedPosition; _]>>
    for _ in 0..n {
        let m = b.length()?;
        b.skip(8 * m)?; // SeedPosition { pos: u32, contig_index_canonical: u32 }
    }
    let contigs = b.length()?;
    for _ in 0..contigs {
        let l = b.length()?;
        b.skip(l)?;
    }
    b.u64()?; // total_sequence_length
    let n = b.length()?;
    b.skip(4 * n)?; // contig_lengths: Vec<u32>
    b.u64()?; // repetitive_kmers
    let n = b.length()?;
    let mut markers = Vec::with_capacity(n as usize);
    for _ in 0..n {
        markers.push(b.u64()?);
    }
    b.skip(8 * 4 + 2)?; // marker_c, c, k, contig_order, individual_contig, amino_acid
    Ok(Markers { file_name, markers })
}

/// The marker-only form of a full sketch (what skani's `Sketch::get_markers_only` writes to
/// `markers.bin`): the same bytes without the seeds, the seed positions and the contig lengths.
pub fn markers_only(sketch: &[u8]) -> Result<Vec<u8>> {
    let mut b = Bin::new(sketch);
    b.string()?;
    let name_end = b.pos() as usize;
    match b.u8()? {
        0 => {}
        1 => {
            let n = b.length()?;
            b.skip(12 * n)?;
        }
        t => bail!("bad option tag {t} in a sketch"),
    }
    let n = b.length()?;
    for _ in 0..n {
        let m = b.length()?;
        b.skip(8 * m)?;
    }
    let contigs_start = b.pos() as usize;
    let contigs = b.length()?;
    for _ in 0..contigs {
        let l = b.length()?;
        b.skip(l)?;
    }
    b.u64()?; // total_sequence_length
    let contigs_end = b.pos() as usize;
    let n = b.length()?;
    b.skip(4 * n)?;
    let rest_start = b.pos() as usize;
    b.u64()?; // repetitive_kmers
    let n = b.length()?;
    b.skip(8 * n + 8 * 4 + 2)?;
    ensure!(
        b.pos() as usize == sketch.len(),
        "a sketch record has {} bytes after its last field",
        sketch.len() - b.pos() as usize
    );
    let mut out = Vec::with_capacity(sketch.len() - rest_start + contigs_end);
    out.extend_from_slice(&sketch[..name_end]);
    out.push(0); // kmer_seeds_k: None
    out.extend_from_slice(&0u64.to_le_bytes()); // multi_position_storage: empty
    out.extend_from_slice(&sketch[contigs_start..contigs_end]);
    out.extend_from_slice(&0u64.to_le_bytes()); // contig_lengths: empty
    out.extend_from_slice(&sketch[rest_start..]);
    Ok(out)
}

/// Stream through a `markers.bin`: the parameters first, then one sketch at a time.
pub struct MarkersFile<R: Read> {
    pub params: Params,
    pub count: u64,
    read: u64,
    bin: Bin<R>,
}

impl MarkersFile<BufReader<File>> {
    pub fn open(path: &Path) -> Result<Self> {
        let f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::new(BufReader::with_capacity(1 << 20, f))
    }
}

impl<R: Read> MarkersFile<R> {
    pub fn new(r: R) -> Result<Self> {
        let mut bin = Bin::new(r);
        let params = read_params(&mut bin)?;
        let count = bin.length()?;
        Ok(MarkersFile {
            params,
            count,
            read: 0,
            bin,
        })
    }

    pub fn next_sketch(&mut self) -> Result<Option<Markers>> {
        if self.read == self.count {
            return Ok(None);
        }
        self.read += 1;
        read_sketch_markers(&mut self.bin).map(Some)
    }
}

/// One `index.db` entry: where a reference's record sits in `sketches.db`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub file_name: String,
    pub offset: u64,
    pub length: u64,
}

pub fn read_index(path: &Path) -> Result<Vec<IndexEntry>> {
    let f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut b = Bin::new(BufReader::new(f));
    let n = b.length()?;
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        v.push(IndexEntry {
            file_name: b.string()?,
            offset: b.u64()?,
            length: b.u64()?,
        });
    }
    Ok(v)
}

/// The name stored in a `sketches.db` record (the sketch's `file_name`), checking that the record
/// starts with the database's parameters.
pub fn record_name(record: &[u8], params: &Params) -> Result<String> {
    ensure!(
        record.starts_with(&params.raw),
        "a sketch record does not start with the database's skani parameters"
    );
    let mut b = Bin::new(&record[params.raw.len()..]);
    b.string()
}

/// The basename skani reports for a reference, without GTDB's `_genomic.fna.gz`: the genome ID
/// GTDB-Tk shows (`GCA_021783475.1`).
pub fn genome_id(file_name: &str) -> &str {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    base.strip_suffix("_genomic.fna.gz").unwrap_or(base)
}

/// Write a skani database folder holding only `records` (full `sketches.db` records, in the
/// order given). Its `markers.bin` holds marker-only copies, as skani writes them.
pub fn write_db(dir: &Path, params: &Params, records: &[(String, Vec<u8>)]) -> Result<()> {
    fs::create_dir_all(dir)?;
    let mut markers = BufWriter::new(File::create(dir.join("markers.bin"))?);
    let mut sketches = BufWriter::new(File::create(dir.join("sketches.db"))?);
    let mut index = Vec::new();
    index.extend_from_slice(&(records.len() as u64).to_le_bytes());
    markers.write_all(&params.raw)?;
    markers.write_all(&(records.len() as u64).to_le_bytes())?;
    let mut offset = 0u64;
    for (name, rec) in records {
        ensure!(
            rec.starts_with(&params.raw),
            "sketch {name} has other skani parameters than the database"
        );
        markers.write_all(
            &markers_only(&rec[params.raw.len()..]).with_context(|| format!("sketch {name}"))?,
        )?;
        sketches.write_all(rec)?;
        index.extend_from_slice(&(name.len() as u64).to_le_bytes());
        index.extend_from_slice(name.as_bytes());
        index.extend_from_slice(&offset.to_le_bytes());
        index.extend_from_slice(&(rec.len() as u64).to_le_bytes());
        offset += rec.len() as u64;
    }
    markers.flush()?;
    sketches.flush()?;
    fs::write(dir.join("index.db"), index)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Serialise a minimal skani 0.3.1 `SketchParams` (c, k, marker_c as given).
    pub fn params_bytes(c: u64, k: u64, marker_c: u64) -> Vec<u8> {
        let mut v = Vec::new();
        for x in [c, k, marker_c] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(&64u64.to_le_bytes());
        v.extend(std::iter::repeat_n(0u8, 8 * 64));
        v.extend_from_slice(&64u64.to_le_bytes());
        v.extend(std::iter::repeat_n(0u8, 64));
        v.extend_from_slice(&30u64.to_le_bytes());
        v
    }

    fn put_str(v: &mut Vec<u8>, s: &str) {
        v.extend_from_slice(&(s.len() as u64).to_le_bytes());
        v.extend_from_slice(s.as_bytes());
    }

    /// Serialise a sketch: `full` adds seeds and positions as a real full sketch has.
    pub fn sketch_bytes(name: &str, markers: &[u64], full: bool) -> Vec<u8> {
        let mut v = Vec::new();
        put_str(&mut v, name);
        if full {
            v.push(1);
            v.extend_from_slice(&2u64.to_le_bytes());
            v.extend(std::iter::repeat_n(7u8, 24));
            v.extend_from_slice(&1u64.to_le_bytes());
            v.extend_from_slice(&2u64.to_le_bytes());
            v.extend(std::iter::repeat_n(9u8, 16));
        } else {
            v.push(0);
            v.extend_from_slice(&0u64.to_le_bytes());
        }
        v.extend_from_slice(&2u64.to_le_bytes());
        put_str(&mut v, "contig_1 some header");
        put_str(&mut v, "contig_2");
        v.extend_from_slice(&123_456u64.to_le_bytes());
        let cl: &[u32] = if full { &[100, 200] } else { &[] };
        v.extend_from_slice(&(cl.len() as u64).to_le_bytes());
        for x in cl {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&(markers.len() as u64).to_le_bytes());
        for m in markers {
            v.extend_from_slice(&m.to_le_bytes());
        }
        for x in [125u64, 125, 15, 0] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&[0, 0]);
        v
    }

    /// A consolidated skani database in `dir` with the given references (name, markers).
    pub fn fake_db(dir: &Path, refs: &[(&str, Vec<u64>)]) -> Params {
        let raw = params_bytes(125, 15, 1000);
        let params = read_params(&mut Bin::new(&raw[..])).unwrap();
        let records: Vec<(String, Vec<u8>)> = refs
            .iter()
            .map(|(n, m)| {
                let mut r = raw.clone();
                r.extend(sketch_bytes(n, m, true));
                (n.to_string(), r)
            })
            .collect();
        write_db(dir, &params, &records).unwrap();
        // write_db must write markers.bin as skani does: marker-only sketches.
        let mut mb = raw.clone();
        mb.extend_from_slice(&(refs.len() as u64).to_le_bytes());
        for (n, m) in refs {
            mb.extend(sketch_bytes(n, m, false));
        }
        assert_eq!(fs::read(dir.join("markers.bin")).unwrap(), mb);
        params
    }

    #[test]
    fn reads_back_what_it_writes() {
        let d = tempfile::tempdir().unwrap();
        let p = fake_db(
            d.path(),
            &[("a/GCA_1.1_genomic.fna.gz", vec![5, 3, 9]), ("b", vec![])],
        );
        assert_eq!((p.c, p.k, p.marker_c, p.use_aa), (125, 15, 1000, false));
        let mut m = MarkersFile::open(&d.path().join("markers.bin")).unwrap();
        assert_eq!(m.count, 2);
        let a = m.next_sketch().unwrap().unwrap();
        assert_eq!(a.file_name, "a/GCA_1.1_genomic.fna.gz");
        assert_eq!(a.markers, vec![5, 3, 9]);
        assert!(m.next_sketch().unwrap().unwrap().markers.is_empty());
        assert!(m.next_sketch().unwrap().is_none());

        let idx = read_index(&d.path().join("index.db")).unwrap();
        let db = fs::read(d.path().join("sketches.db")).unwrap();
        assert_eq!(idx.len(), 2);
        let rec = &db[idx[0].offset as usize..(idx[0].offset + idx[0].length) as usize];
        assert_eq!(record_name(rec, &p).unwrap(), "a/GCA_1.1_genomic.fna.gz");
        // A full record parses too, and gives the same markers.
        let mut b = Bin::new(&rec[p.raw.len()..]);
        assert_eq!(read_sketch_markers(&mut b).unwrap().markers, vec![5, 3, 9]);
        assert_eq!(b.pos() as usize, rec.len() - p.raw.len());
        assert_eq!(idx[1].offset, idx[0].length);
        assert_eq!(genome_id(&idx[0].file_name), "GCA_1.1");
    }

    #[test]
    fn rejects_records_with_other_parameters() {
        let raw = params_bytes(125, 15, 1000);
        let p = read_params(&mut Bin::new(&raw[..])).unwrap();
        let mut other = params_bytes(200, 15, 1000);
        other.extend(sketch_bytes("x", &[1], true));
        assert!(record_name(&other, &p).is_err());
        let d = tempfile::tempdir().unwrap();
        assert!(write_db(d.path(), &p, &[("x".into(), other)]).is_err());
    }
}
