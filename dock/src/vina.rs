//! AutoDock Vina as the docking engine.

use crate::env::Env;
use crate::stages::*;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::sync::Arc;

pub struct Vina(pub Arc<Env>);

impl Engine for Vina {
    fn dock(
        &self,
        receptor: &PreparedReceptor,
        ligand: &PreparedLigand,
        bx: Box3,
        opts: &EngineOpts,
        out: &Path,
    ) -> Result<Vec<Pose>> {
        let mut c = std::process::Command::new(&self.0.vina);
        c.arg("--receptor").arg(&receptor.pdbqt)
            .arg("--ligand").arg(&ligand.pdbqt)
            .arg("--center_x").arg(bx.center[0].to_string())
            .arg("--center_y").arg(bx.center[1].to_string())
            .arg("--center_z").arg(bx.center[2].to_string())
            .arg("--size_x").arg(bx.size[0].to_string())
            .arg("--size_y").arg(bx.size[1].to_string())
            .arg("--size_z").arg(bx.size[2].to_string())
            .arg("--exhaustiveness").arg(opts.exhaustiveness.to_string())
            .arg("--num_modes").arg(opts.num_modes.to_string())
            .arg("--seed").arg(opts.seed.to_string())
            .arg("--cpu").arg(opts.cpus.to_string())
            .arg("--out").arg(out);

        self.0.record(
            std::iter::once("vina".to_string())
                .chain(c.get_args().map(|a| a.to_string_lossy().to_string()))
                .collect(),
        );
        let res = c.output().context("starting vina")?;
        let stdout = String::from_utf8_lossy(&res.stdout);
        if !res.status.success() {
            let err = String::from_utf8_lossy(&res.stderr);
            bail!(
                "vina failed ({}):\n{}",
                res.status,
                if err.trim().is_empty() { stdout.to_string() } else { err.to_string() }
            );
        }
        let poses = parse_table(&stdout)?;
        if poses.is_empty() {
            bail!(
                "vina produced no poses. The box may not cover the binding site, or the \
                 ligand may not fit in it."
            );
        }
        Ok(poses)
    }
}

/// Read Vina's result table.
///
/// ```text
/// mode |   affinity | dist from best mode
///      | (kcal/mol) | rmsd l.b.| rmsd u.b.
/// -----+------------+----------+----------
///    1       -11.09          0          0
/// ```
fn parse_table(stdout: &str) -> Result<Vec<Pose>> {
    let mut poses = Vec::new();
    let mut in_table = false;
    for line in stdout.lines() {
        if line.trim_start().starts_with("-----") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 {
            if !poses.is_empty() {
                break; // table ended
            }
            continue;
        }
        let (Ok(rank), Ok(aff), Ok(lb), Ok(ub)) = (
            f[0].parse::<usize>(),
            f[1].parse::<f64>(),
            f[2].parse::<f64>(),
            f[3].parse::<f64>(),
        ) else {
            if !poses.is_empty() {
                break;
            }
            continue;
        };
        poses.push(Pose { rank, affinity: aff, rmsd_lb: lb, rmsd_ub: ub });
    }
    Ok(poses)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Computing Vina grid ... done.
Performing docking (random seed: 42) ...

mode |   affinity | dist from best mode
     | (kcal/mol) | rmsd l.b.| rmsd u.b.
-----+------------+----------+----------
   1       -11.09          0          0
   2       -10.39      1.777      2.252
   3       -9.901      6.047      11.56
";

    #[test]
    fn reads_the_result_table() {
        let p = parse_table(SAMPLE).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].rank, 1);
        assert!((p[0].affinity + 11.09).abs() < 1e-9);
        assert!((p[2].rmsd_ub - 11.56).abs() < 1e-9);
    }

    #[test]
    fn no_table_is_not_a_panic() {
        assert!(parse_table("vina: something went wrong").unwrap().is_empty());
    }

    #[test]
    fn stops_at_the_end_of_the_table() {
        let text = format!("{SAMPLE}\nWriting output ... done.\n");
        assert_eq!(parse_table(&text).unwrap().len(), 3);
    }
}
