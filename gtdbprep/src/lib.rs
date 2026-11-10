//! gtdbprep: the laptop half of a split GTDB-Tk.
//!
//! Runs GTDB-Tk's own `identify` and `align --skip_gtdb_refs` on renamed genomes and packs the
//! results into a small bundle that `gtdbtk classify --align_dir` (local, or a future shared
//! placement server) reads unchanged. See `docs/tools/gtdbprep.md` and
//! `docs/research/split-gtdb.md`.

pub mod cli;
pub mod hash;
pub mod manifest;
pub mod outputs;
pub mod pipeline;
pub mod reference;
pub mod rename;
pub mod tools;
