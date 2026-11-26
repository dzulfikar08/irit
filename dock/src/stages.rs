//! The pipeline, as four stages behind traits.
//!
//! Every stage is currently backed by Python (`python.rs`): RDKit, Meeko, ProLIF and pdb2pqr.
//! That path is the one validated against a crystal structure, so it is the reference.
//!
//! The reason the stages are traits and not plain functions is the migration to pure Rust.
//! `chematic` and `pdbtbx` already cover SMILES parsing, 3D embedding, Gasteiger charges,
//! SMARTS matching and PDB/mmCIF I/O, so each stage can be reimplemented one at a time —
//! but a conformer can have flawless bond lengths and still be the wrong shape, so a swap
//! is only allowed to land when it is checked against the Python implementation on the same
//! inputs. `dock validate --compare` exists for exactly that: it runs both backends and
//! diffs them. Do not retire a Python stage without that diff.

use anyhow::Result;
use std::path::{Path, PathBuf};

/// A ligand ready for docking: a PDBQT file plus what we know about it.
#[derive(Debug, Clone)]
pub struct PreparedLigand {
    pub pdbqt: PathBuf,
    pub heavy_atoms: usize,
    pub rotatable_bonds: usize,
    /// How the 3D coordinates were produced, for the manifest.
    pub source: String,
}

/// A receptor ready for docking.
#[derive(Debug, Clone)]
pub struct PreparedReceptor {
    pub pdbqt: PathBuf,
    /// Protonated structure, kept because interaction profiling needs the hydrogens.
    pub protonated_pdb: PathBuf,
    pub residues: usize,
    pub ph: f64,
}

/// One docked pose.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Pose {
    pub rank: usize,
    /// Vina score, kcal/mol. Lower is better. This is not a binding free energy.
    pub affinity: f64,
    /// RMSD to the top pose, as Vina reports it.
    pub rmsd_lb: f64,
    pub rmsd_ub: f64,
}

/// A single protein-ligand contact.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Contact {
    /// e.g. "CYS919.A"
    pub residue: String,
    /// e.g. "HBAcceptor", "Hydrophobic", "VdWContact"
    pub kind: String,
    pub distance: f64,
}

/// Turn a ligand into a dockable PDBQT.
pub trait LigandPrep {
    /// From SMILES: embed 3D coordinates, then write PDBQT.
    fn prepare_smiles(&self, smiles: &str, out: &Path) -> Result<PreparedLigand>;
    /// From a file that already carries 3D coordinates (SDF/MOL2/PDB).
    fn prepare_file(&self, input: &Path, out: &Path) -> Result<PreparedLigand>;
}

/// Protonate a receptor and write its PDBQT.
pub trait ReceptorPrep {
    fn prepare(&self, input: &Path, ph: f64, out_dir: &Path) -> Result<PreparedReceptor>;
}

/// Where the docking box sits, in angstroms.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Box3 {
    pub center: [f64; 3],
    pub size: [f64; 3],
}

/// Run the docking search.
pub trait Engine {
    fn dock(
        &self,
        receptor: &PreparedReceptor,
        ligand: &PreparedLigand,
        bx: Box3,
        opts: &EngineOpts,
        out: &Path,
    ) -> Result<Vec<Pose>>;
}

#[derive(Debug, Clone)]
pub struct EngineOpts {
    pub exhaustiveness: u32,
    pub num_modes: u32,
    pub seed: i64,
    pub cpus: u32,
}

/// Work out which residues a pose touches, and how.
pub trait Profiler {
    fn profile(&self, receptor: &PreparedReceptor, docked: &Path, pose: usize) -> Result<Vec<Contact>>;
}

/// Symmetry-aware RMSD between a docked pose and a reference ligand.
///
/// Symmetry matters: a phenyl ring flipped 180 degrees is the same structure, and an
/// implementation that compares atoms by index reports a large RMSD for an identical pose.
pub trait Rmsd {
    fn rmsd_to_reference(&self, docked: &Path, reference: &Path, smiles: &str) -> Result<Vec<f64>>;
}

/// Everything a run needs. Swapping a stage to Rust means swapping one field.
pub struct Backend {
    pub ligand: Box<dyn LigandPrep>,
    pub receptor: Box<dyn ReceptorPrep>,
    pub engine: Box<dyn Engine>,
    pub profiler: Box<dyn Profiler>,
    pub rmsd: Box<dyn Rmsd>,
}
