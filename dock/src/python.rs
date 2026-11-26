//! Python-backed stages: RDKit, Meeko, ProLIF, pdb2pqr.
//!
//! This is the reference implementation. When a stage moves to Rust, the replacement is
//! checked against the one here on the same input before it is allowed to become the default.

use crate::env::Env;
use crate::stages::*;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Py(pub Arc<Env>);

fn s(v: &serde_json::Value, k: &str) -> Result<String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .map(|x| x.to_string())
        .with_context(|| format!("stage result has no string field `{k}`"))
}

fn u(v: &serde_json::Value, k: &str) -> Result<usize> {
    v.get(k)
        .and_then(|x| x.as_u64())
        .map(|x| x as usize)
        .with_context(|| format!("stage result has no integer field `{k}`"))
}

impl LigandPrep for Py {
    fn prepare_smiles(&self, smiles: &str, out: &Path) -> Result<PreparedLigand> {
        let o = out.display().to_string();
        let v = self.0.stage("prep-ligand", &["--smiles", smiles, "--out", &o])?;
        Ok(PreparedLigand {
            pdbqt: PathBuf::from(s(&v, "pdbqt")?),
            heavy_atoms: u(&v, "heavy_atoms")?,
            rotatable_bonds: u(&v, "rotatable_bonds")?,
            source: s(&v, "source")?,
        })
    }

    fn prepare_file(&self, input: &Path, out: &Path) -> Result<PreparedLigand> {
        let (i, o) = (input.display().to_string(), out.display().to_string());
        let v = self.0.stage("prep-ligand", &["--input", &i, "--out", &o])?;
        Ok(PreparedLigand {
            pdbqt: PathBuf::from(s(&v, "pdbqt")?),
            heavy_atoms: u(&v, "heavy_atoms")?,
            rotatable_bonds: u(&v, "rotatable_bonds")?,
            source: s(&v, "source")?,
        })
    }
}

impl ReceptorPrep for Py {
    fn prepare(&self, input: &Path, ph: f64, out_dir: &Path) -> Result<PreparedReceptor> {
        let (i, d, p) = (input.display().to_string(), out_dir.display().to_string(), ph.to_string());
        let v = self.0.stage("prep-receptor", &["--input", &i, "--out-dir", &d, "--ph", &p])?;
        Ok(PreparedReceptor {
            pdbqt: PathBuf::from(s(&v, "pdbqt")?),
            protonated_pdb: PathBuf::from(s(&v, "protonated_pdb")?),
            residues: u(&v, "residues")?,
            ph,
        })
    }
}

impl Profiler for Py {
    fn profile(&self, receptor: &PreparedReceptor, docked: &Path, pose: usize) -> Result<Vec<Contact>> {
        let r = receptor.protonated_pdb.display().to_string();
        let d = docked.display().to_string();
        let p = pose.to_string();
        let v = self.0.stage("profile", &["--receptor", &r, "--docked", &d, "--pose", &p])?;
        let arr = v.get("contacts").and_then(|c| c.as_array()).context("no contacts returned")?;
        arr.iter()
            .map(|c| {
                Ok(Contact {
                    residue: s(c, "residue")?,
                    kind: s(c, "kind")?,
                    distance: c.get("distance").and_then(|d| d.as_f64()).unwrap_or(0.0),
                })
            })
            .collect()
    }
}

impl Rmsd for Py {
    fn rmsd_to_reference(&self, docked: &Path, reference: &Path, smiles: &str) -> Result<Vec<f64>> {
        let (d, r) = (docked.display().to_string(), reference.display().to_string());
        let mut args = vec!["--docked", d.as_str(), "--reference", r.as_str()];
        if !smiles.is_empty() {
            args.push("--smiles");
            args.push(smiles);
        }
        let v = self.0.stage("rmsd", &args)?;
        Ok(v.get("rmsd")
            .and_then(|x| x.as_array())
            .context("no rmsd array returned")?
            .iter()
            .filter_map(|x| x.as_f64())
            .collect())
    }
}

impl Py {
    /// Docking box centred on a reference ligand.
    pub fn box_from_reference(&self, reference: &Path, padding: f64) -> Result<Box3> {
        let r = reference.display().to_string();
        let p = padding.to_string();
        let v = self.0.stage("box", &["--reference", &r, "--padding", &p])?;
        let get = |k: &str| -> Result<[f64; 3]> {
            let a = v.get(k).and_then(|x| x.as_array()).with_context(|| format!("no `{k}`"))?;
            if a.len() != 3 {
                anyhow::bail!("`{k}` is not three numbers");
            }
            Ok([
                a[0].as_f64().unwrap_or_default(),
                a[1].as_f64().unwrap_or_default(),
                a[2].as_f64().unwrap_or_default(),
            ])
        };
        Ok(Box3 { center: get("center")?, size: get("size")? })
    }

    /// Split a structure into protein and a named ligand.
    pub fn split(&self, input: &Path, ligand: &str, out_receptor: &Path, out_ligand: &Path)
        -> Result<serde_json::Value>
    {
        let i = input.display().to_string();
        let r = out_receptor.display().to_string();
        let l = out_ligand.display().to_string();
        self.0.stage(
            "split",
            &["--input", &i, "--ligand", ligand, "--out-receptor", &r, "--out-ligand", &l],
        )
    }
}
