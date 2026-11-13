//! irit-mcp: the irit command-line tools as MCP tools, with replayable run records.

mod external;
mod params;
mod record;
mod results;
mod server;
mod validate;

use rmcp::ServiceExt;
use std::path::PathBuf;

const USAGE: &str = "irit-mcp: MCP server (stdio) for the irit tools

Usage: irit-mcp [--bin-dir DIR] [--tool-path NAME=PATH]... [--runs-dir DIR] [--run-id ID] [--root DIR] [--allow-outside-root]

  --bin-dir DIR          Folder with the irit binaries (env IRIT_BIN_DIR). Default: PATH.
  --tool-path NAME=PATH  Where a third-party tool is: fastp, shovill, flye, medaka_consensus, bakta,
                         checkm2, gtdbtk, mlst or amrfinder. Default: PATH.
  --runs-dir DIR         Where run records go (env IRIT_RUNS_DIR). Default: ./irit-runs
  --run-id ID            Name of this session's run folder, which must not hold a record yet.
                         Default: <UTC time>-<pid>
  --root DIR             Tools may only write inside this folder. Default: the working directory.
  --allow-outside-root   Let tools write anywhere the server's user can (not recommended).

Tools run in the server's working directory; relative paths are resolved there.";

fn main() -> anyhow::Result<()> {
    let mut bin_dir = std::env::var_os("IRIT_BIN_DIR").map(PathBuf::from);
    let mut runs_dir = std::env::var_os("IRIT_RUNS_DIR").map(PathBuf::from).unwrap_or_else(|| "irit-runs".into());
    let mut run_id = record::new_run_id();
    let mut root: Option<PathBuf> = None;
    let mut tool_paths = std::collections::HashMap::new();
    let mut allow_outside_root = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or_else(|| anyhow::anyhow!("{a} needs a value\n\n{USAGE}"));
        match a.as_str() {
            "--bin-dir" => bin_dir = Some(value()?.into()),
            "--runs-dir" => runs_dir = value()?.into(),
            "--run-id" => {
                run_id = value()?;
                anyhow::ensure!(
                    !run_id.is_empty()
                        && !run_id.starts_with('.')
                        && !run_id.contains(['/', '\\'])
                        && !run_id.chars().any(char::is_control),
                    "--run-id must be a plain folder name"
                );
            }
            "--root" => root = Some(value()?.into()),
            "--tool-path" => {
                let v = value()?;
                let (name, path) = v.split_once('=').ok_or_else(|| anyhow::anyhow!("--tool-path takes NAME=PATH, got {v}"))?;
                let names: Vec<&str> = external::PINNED.iter().map(|p| p.tool).collect();
                anyhow::ensure!(names.contains(&name), "--tool-path: {name} is not one of {}", names.join(", "));
                tool_paths.insert(name.to_string(), PathBuf::from(path));
            }
            "--allow-outside-root" => allow_outside_root = true,
            "-V" | "--version" => {
                println!("irit-mcp {}", record::IRIT_MCP_VERSION);
                return Ok(());
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => anyhow::bail!("unknown argument {a}\n\n{USAGE}"),
        }
    }
    let cwd = std::env::current_dir()?;
    let root = root.map_or_else(|| cwd.clone(), |r| cwd.join(r));
    let root = std::fs::canonicalize(&root).map_err(|e| anyhow::anyhow!("--root {}: {e}", root.display()))?;
    anyhow::ensure!(root.is_dir(), "--root {} is not a folder", root.display());
    record::check_run_dir_unused(&cwd.join(&runs_dir), &run_id).map_err(anyhow::Error::msg)?;
    let cfg = server::Config {
        bin_dir: bin_dir.map(|d| std::fs::canonicalize(cwd.join(&d)).unwrap_or_else(|_| cwd.join(d))),
        tool_paths: tool_paths.into_iter().map(|(k, p)| (k, cwd.join(p))).collect(),
        runs_root: cwd.join(&runs_dir),
        ctx: validate::Ctx::new(cwd.clone(), root, allow_outside_root, &cwd.join(&runs_dir)),
        run_id,
    };
    tokio::runtime::Runtime::new()?.block_on(async {
        let service = server::Irit::new(cfg).serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}
