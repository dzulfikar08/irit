//! Core of msascope, free of any GUI code: parsing, compact storage, summaries and view geometry.

pub mod alphabet;
pub mod matrix;
pub mod parse;
pub mod pyramid;
pub mod stats;
pub mod synth;
pub mod view;

use alphabet::{Alphabet, Kind};
use anyhow::Context as _;
use matrix::Matrix;

pub struct Alignment {
    pub names: Vec<String>,
    pub matrix: Matrix,
    /// DNA, RNA, DNA/RNA (T and U kept apart) or protein.
    pub alpha: &'static Alphabet,
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
        let (format, gzip) = parse::sniff(path).with_context(|| format!("reading {}", path.display()))?;
        let t = std::time::Instant::now();
        let aln = parse::load_as(path, ty)?;
        let parse = t.elapsed();
        let t = std::time::Instant::now();
        let stats = stats::ColumnStats::compute(&aln);
        let stats_t = t.elapsed();
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
