//! Core of msascope, free of any GUI code: parsing, compact storage, summaries and view geometry.

pub mod alphabet;
pub mod export;
pub mod goto;
pub mod jobs;
pub mod matrix;
pub mod motif;
pub mod parse;
pub mod pyramid;
pub mod rows;
pub mod scheme;
pub mod select;
pub mod stats;
pub mod synth;
pub mod tree;
pub mod view;

use alphabet::{Alphabet, Kind};
use anyhow::Context as _;
use matrix::Matrix;

pub struct Alignment {
    pub names: Vec<String>,
    pub matrix: Matrix,
    /// DNA, RNA, DNA/RNA (T and U kept apart) or protein.
    pub alpha: &'static Alphabet,
    /// Insert columns as a bitset (bit `c % 64` of word `c / 64`): from Stockholm's `#=GC RF`
    /// line when present, otherwise the columns where every row has lower case or `.` (the A2M and
    /// Pfam convention for insert states). Empty when there are none. Residues are still stored as
    /// upper case and `.` as a gap.
    pub inserts: Vec<u64>,
    /// Stockholm `#=GS <name> AC <accession>`, per row; empty when the file has none.
    pub accessions: Vec<Option<Box<str>>>,
    /// A Newick tree from Stockholm `#=GF NH` lines, joined; `None` when the file has none.
    pub newick: Option<String>,
}

/// The `start-end` range after the last `/` of a Pfam/HMMER-style name (`GLB2_TYLHE/32-143`).
pub fn name_range(name: &str) -> Option<(usize, usize)> {
    let (_, r) = name.rsplit_once('/')?;
    let (a, b) = r.split_once('-')?;
    let (a, b) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a >= 1).then_some((a, b))
}

impl Alignment {
    pub fn rows(&self) -> usize {
        self.matrix.rows
    }

    pub fn cols(&self) -> usize {
        self.matrix.cols
    }

    pub fn alphabet(&self) -> &'static Alphabet {
        self.alpha
    }

    pub fn kind(&self) -> Kind {
        self.alpha.kind
    }

    pub fn symbol(&self, code: u8) -> u8 {
        self.alpha.symbol(code)
    }

    pub fn is_insert(&self, c: usize) -> bool {
        self.inserts.get(c / 64).is_some_and(|w| w >> (c % 64) & 1 == 1)
    }

    pub fn insert_count(&self) -> usize {
        self.inserts.iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn accession(&self, r: usize) -> Option<&str> {
        self.accessions.get(r).and_then(|a| a.as_deref())
    }

    /// The number of residue `c` of row `r` in its sequence, 1-based, counted from the name's
    /// `/start-end` when it has one: `(position, start, end)` where start and end are the range
    /// the row covers. `None` for a gap.
    pub fn seq_position(&self, r: usize, c: usize) -> Option<(usize, usize, usize)> {
        if self.matrix.get(r, c) == alphabet::GAP {
            return None;
        }
        let before = self.matrix.residues_before(r, c);
        let len = self.matrix.residues_before(r, self.cols());
        let start = name_range(&self.names[r]).map_or(1, |(a, _)| a);
        Some((start + before, start, start + len.max(1) - 1))
    }
}

/// An alignment with everything the viewer precomputes.
pub struct Loaded {
    pub aln: Alignment,
    pub stats: stats::ColumnStats,
    pub pyramid: pyramid::Pyramid,
    pub timings: Timings,
    /// File format, and whether the file was gzipped.
    pub format: parse::Format,
    pub gzip: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub parse: std::time::Duration,
    pub stats: std::time::Duration,
    pub pyramid: std::time::Duration,
}

impl Loaded {
    pub fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        Self::open_as(path, None)
    }

    /// [`Loaded::open`] with the residue type given instead of guessed (`--type`).
    pub fn open_as(path: &std::path::Path, ty: Option<alphabet::SeqType>) -> anyhow::Result<Self> {
        Self::open_with(path, ty, None)
    }

    /// [`Loaded::open_as`] reporting to `progress`; returns [`parse::Cancelled`] when it is
    /// cancelled (checked while reading and between the steps), and what was built so far is
    /// dropped on the way out.
    pub fn open_with(path: &std::path::Path, ty: Option<alphabet::SeqType>, progress: Option<&parse::Progress>) -> anyhow::Result<Self> {
        let (format, gzip) = parse::sniff(path).with_context(|| format!("reading {}", path.display()))?;
        let check = || progress.map_or(Ok(()), |p| p.check());
        let t = std::time::Instant::now();
        let aln = parse::load_with(path, ty, progress)?;
        let parse = t.elapsed();
        check()?;
        if let Some(p) = progress {
            p.set_rows(aln.rows());
            p.set_stage(parse::Stage::Columns);
        }
        let t = std::time::Instant::now();
        let stats = stats::ColumnStats::compute(&aln);
        let stats_t = t.elapsed();
        check()?;
        if let Some(p) = progress {
            p.set_stage(parse::Stage::Overview);
        }
        let t = std::time::Instant::now();
        let pyramid = pyramid::Pyramid::build(&aln, &stats.consensus);
        let timings = Timings { parse, stats: stats_t, pyramid: t.elapsed() };
        Ok(Loaded { aln, stats, pyramid, timings, format, gzip })
    }

    /// Load time in seconds (parse + column summaries + pyramid).
    pub fn load_secs(&self) -> f64 {
        (self.timings.parse + self.timings.stats + self.timings.pyramid).as_secs_f64()
    }

    /// Memory held for the alignment: residues plus pyramid, in bytes.
    pub fn bytes(&self) -> usize {
        self.aln.matrix.bytes() + self.pyramid.bytes()
    }

    /// One-line summary for the terminal.
    pub fn summary(&self) -> String {
        let kind = self.aln.alpha.name;
        format!(
            "{} sequences × {} columns, {kind}; residues {} MiB, pyramid {} MiB; parse {:.2} s, columns {:.2} s, pyramid {:.2} s",
            self.aln.rows(),
            self.aln.cols(),
            self.aln.matrix.bytes() >> 20,
            self.pyramid.bytes() >> 20,
            self.timings.parse.as_secs_f64(),
            self.timings.stats.as_secs_f64(),
            self.timings.pyramid.as_secs_f64(),
        )
    }
}
