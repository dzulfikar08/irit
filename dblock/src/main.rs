//! dblock: download, verify and pin bioinformatics reference databases.
//!
//! A project lists the databases it needs in `dblock.toml`; `dblock fetch` gets them into a
//! shared store (checking free space first, resuming interrupted downloads, verifying MD5s) and
//! writes `dblock.lock`, which records exactly what was used: URL, checksum, date and a
//! fingerprint of the unpacked files.

mod catalog;
mod check;
mod fetch;
mod project;

use anyhow::{Result, bail};
use catalog::{Entry, human};
use clap::{Parser, Subcommand};
use project::{Lock, Project};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(version, about = "Download, verify and pin bioinformatics reference databases")]
struct Cli {
    /// Project folder (holds dblock.toml and dblock.lock).
    #[arg(short = 'C', long, global = true, default_value = ".")]
    dir: PathBuf,

    /// Extra catalog entries (JSON, same format as the built-in catalog).
    #[arg(long, global = true)]
    catalog: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the databases dblock knows about.
    Catalog {
        /// Only this tool (e.g. gtdb-tk, checkm2, kraken).
        #[arg(long)]
        tool: Option<String>,
    },
    /// Add databases to this project's dblock.toml.
    Add {
        #[arg(required = true)]
        ids: Vec<String>,
        /// Where databases are stored (default: $DBLOCK_STORE or ~/.local/share/dblock).
        #[arg(long)]
        store: Option<String>,
    },
    /// Remove databases from this project (the files stay in the store unless --delete-data).
    Remove {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        delete_data: bool,
    },
    /// Show whether the project's databases fit on disk, and how they would be fetched.
    Plan,
    /// Download, verify and unpack the project's databases, then update dblock.lock.
    Fetch {
        /// Only these databases (default: all in dblock.toml).
        ids: Vec<String>,
        /// Unpack while downloading; the archive is never stored (needs less space, not resumable).
        #[arg(long)]
        stream: bool,
        /// Keep the downloaded archive in <store>/archives.
        #[arg(long)]
        keep_archive: bool,
    },
    /// Check that the locked databases are still on disk, unchanged.
    Verify,
    /// Check installed tool versions against the locked databases.
    Check,
    /// Print shell exports and paths for the locked databases.
    Env,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let entries = catalog::load(cli.catalog.as_deref())?;
    match cli.cmd {
        Cmd::Catalog { tool } => cmd_catalog(&entries, tool.as_deref()),
        Cmd::Add { ids, store } => cmd_add(&cli.dir, &entries, &ids, store),
        Cmd::Remove { ids, delete_data } => cmd_remove(&cli.dir, &ids, delete_data),
        Cmd::Plan => cmd_plan(&cli.dir, &entries),
        Cmd::Fetch { ids, stream, keep_archive } => {
            cmd_fetch(&cli.dir, &entries, &ids, &fetch::Opts { force_stream: stream, keep_archive })
        }
        Cmd::Verify => cmd_verify(&cli.dir),
        Cmd::Check => cmd_check(&cli.dir, &entries),
        Cmd::Env => cmd_env(&cli.dir),
    }
}

fn require_project(dir: &Path) -> Result<Project> {
    match Project::load(dir)? {
        Some(p) => Ok(p),
        None => bail!("no {} in {}; start with `dblock add <id>`", project::PROJECT_FILE, dir.display()),
    }
}

fn cmd_catalog(entries: &[Entry], tool: Option<&str>) -> Result<()> {
    let norm = |s: &str| s.to_ascii_lowercase().replace([' ', '-', '_'], "");
    let want = tool.map(norm);
    println!("{:<32} {:>9} {:>10}  DESCRIPTION", "ID", "DOWNLOAD", "ON DISK");
    for e in entries {
        if want.as_ref().is_some_and(|w| !norm(&e.tool).contains(w.as_str()) && !e.id.contains(w.as_str())) {
            continue;
        }
        let flag = if e.non_commercial { "  [non-commercial licence]" } else { "" };
        println!("{:<32} {:>9} {:>10}  {}{flag}", e.id, human(e.size), human(e.extracted), e.name);
    }
    Ok(())
}

fn cmd_add(dir: &Path, entries: &[Entry], ids: &[String], store: Option<String>) -> Result<()> {
    let mut p = Project::load(dir)?.unwrap_or_default();
    if store.is_some() {
        p.store = store;
    }
    for id in ids {
        let e = catalog::find(entries, id)?;
        if !p.databases.contains(id) {
            p.databases.push(id.clone());
        }
        println!("added {id}: {} ({} download, {} on disk)", e.name, human(e.size), human(e.extracted));
        if e.non_commercial {
            println!("  licence: {}", e.licence);
        }
    }
    p.save(dir)?;
    println!("next: `dblock plan` to check disk space, then `dblock fetch`");
    Ok(())
}

fn cmd_remove(dir: &Path, ids: &[String], delete_data: bool) -> Result<()> {
    let mut p = require_project(dir)?;
    let mut lock = Lock::load(dir)?;
    for id in ids {
        p.databases.retain(|d| d != id);
        if let Some(l) = lock.databases.iter().find(|d| &d.id == id) {
            if delete_data {
                std::fs::remove_dir_all(&l.path)?;
                println!("deleted {}", l.path);
            } else {
                println!("removed {id} from the project; files kept at {}", l.path);
            }
        }
        lock.databases.retain(|d| &d.id != id);
    }
    p.save(dir)?;
    lock.save(dir)?;
    Ok(())
}

fn cmd_plan(dir: &Path, entries: &[Entry]) -> Result<()> {
    let p = require_project(dir)?;
    let store = p.store_dir()?;
    println!("store: {}", store.display());
    let opts = fetch::Opts { force_stream: false, keep_archive: false };
    let mut total = 0;
    for id in &p.databases {
        let e = catalog::find(entries, id)?;
        if fetch::existing(&store, e).is_some() {
            println!("  {id}: already in the store");
            continue;
        }
        match fetch::plan(&store, e, &opts) {
            Ok(plan) => {
                total += plan.need;
                let how = match plan.mode {
                    fetch::Mode::DownloadThenExtract if plan.resumable > 0 => {
                        format!("download (resume from {}), verify, unpack", human(plan.resumable))
                    }
                    fetch::Mode::DownloadThenExtract => "download, verify, unpack".into(),
                    fetch::Mode::Stream => "stream: unpack while downloading (archive never stored; not resumable)".into(),
                };
                println!("  {id}: {how}; needs {} of {} free", human(plan.need), human(plan.free));
            }
            Err(err) => println!("  {id}: WON'T FIT. {err}"),
        }
    }
    if total > 0 {
        println!("together these need about {} (each check above assumes it is the only download)", human(total));
    }
    Ok(())
}

fn cmd_fetch(dir: &Path, entries: &[Entry], only: &[String], opts: &fetch::Opts) -> Result<()> {
    let p = require_project(dir)?;
    let store = p.store_dir()?;
    let mut lock = Lock::load(dir)?;
    let ids: Vec<&String> = if only.is_empty() {
        p.databases.iter().collect()
    } else {
        for id in only {
            if !p.databases.contains(id) {
                bail!("{id} is not in {}; `dblock add {id}` first", project::PROJECT_FILE);
            }
        }
        only.iter().collect()
    };
    let mut failed = Vec::new();
    for id in ids {
        let e = catalog::find(entries, id)?;
        eprintln!("{id}: {} ({})", e.name, human(e.size));
        if let Some(found) = fetch::existing(&store, e) {
            eprintln!("  already in {}, reusing it", found.path);
            lock.upsert(found);
            continue;
        }
        let result = fetch::plan(&store, e, opts).and_then(|plan| fetch::fetch(&store, e, &plan, opts));
        match result {
            Ok(l) => {
                eprintln!("  done: {} files, {} at {}", l.files, human(l.bytes), l.data_path);
                if e.non_commercial {
                    eprintln!("  licence: {}", e.licence);
                }
                lock.upsert(l);
            }
            Err(err) => {
                eprintln!("  failed: {err:#}");
                failed.push(id.clone());
            }
        }
        lock.save(dir)?;
    }
    lock.save(dir)?;
    if !failed.is_empty() {
        bail!("could not fetch: {}", failed.join(", "));
    }
    Ok(())
}

fn cmd_verify(dir: &Path) -> Result<()> {
    let lock = Lock::load(dir)?;
    if lock.databases.is_empty() {
        bail!("nothing locked yet; run `dblock fetch`");
    }
    let mut bad = 0;
    for l in &lock.databases {
        let path = Path::new(&l.path);
        if !path.is_dir() {
            println!("MISSING  {}: {} is gone", l.id, l.path);
            bad += 1;
            continue;
        }
        let (files, bytes, md5) = fetch::manifest(path)?;
        if md5 == l.manifest_md5 {
            println!("ok       {}: {} files, {}", l.id, files, human(bytes));
        } else {
            println!(
                "CHANGED  {}: now {} files / {}, locked {} files / {}",
                l.id,
                files,
                human(bytes),
                l.files,
                human(l.bytes)
            );
            bad += 1;
        }
    }
    if bad > 0 {
        bail!("{bad} database(s) differ from dblock.lock");
    }
    Ok(())
}

fn cmd_check(dir: &Path, entries: &[Entry]) -> Result<()> {
    let lock = Lock::load(dir)?;
    let mut bad = 0;
    for l in &lock.databases {
        let compat = catalog::find(entries, &l.id).ok().and_then(|e| e.compat.clone());
        let Some(c) = compat else {
            println!("--       {} ({}): no version rule published", l.id, l.tool);
            continue;
        };
        let range = match &c.max_exclusive {
            Some(max) => format!(">= {}, < {max}", c.min),
            None => format!(">= {}", c.min),
        };
        match check::check(&c) {
            check::Verdict::Ok(v) => println!("ok       {}: {} {v} (needs {range})", l.id, l.tool),
            check::Verdict::TooOld { found, min } => {
                println!("TOO OLD  {}: {} {found}, needs {min} or newer ({})", l.id, l.tool, c.source);
                bad += 1;
            }
            check::Verdict::TooNew { found, max_exclusive } => {
                println!("TOO NEW  {}: {} {found} won't read this database (needs < {max_exclusive}, {})", l.id, l.tool, c.source);
                bad += 1;
            }
            check::Verdict::NotInstalled => {
                println!("--       {}: `{}` not found here (fine if it runs in a container; needs {range})", l.id, c.command[0])
            }
            check::Verdict::Unreadable(s) => println!("??       {}: couldn't read a version from `{s}`", l.id),
        }
    }
    if bad > 0 {
        bail!("{bad} tool/database mismatch(es)");
    }
    Ok(())
}

fn cmd_env(dir: &Path) -> Result<()> {
    let lock = Lock::load(dir)?;
    for l in &lock.databases {
        match &l.env {
            Some(var) => println!("export {var}='{}'", l.data_path),
            None => println!("# {} ({}): {}", l.id, l.tool, l.data_path),
        }
    }
    Ok(())
}
