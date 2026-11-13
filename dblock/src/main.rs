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

use anyhow::{Context, Result, bail};
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
        /// Also delete the database folder from the shared store (needs --force).
        #[arg(long)]
        delete_data: bool,
        /// Confirm --delete-data: other projects using the same store lose the database too.
        #[arg(long, requires = "delete_data")]
        force: bool,
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
        /// Keep byte-identical files as separate copies instead of hard-linking them.
        #[arg(long)]
        no_dedupe: bool,
    },
    /// Check that the locked databases are still on disk, unchanged (file names and sizes).
    Verify {
        /// Re-hash every file and compare with the SHA-256 sums recorded at fetch time.
        #[arg(long)]
        deep: bool,
    },
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
        Cmd::Remove { ids, delete_data, force } => cmd_remove(&cli.dir, &ids, delete_data, force),
        Cmd::Plan => cmd_plan(&cli.dir, &entries),
        Cmd::Fetch { ids, stream, keep_archive, no_dedupe } => {
            cmd_fetch(&cli.dir, &entries, &ids, &fetch::Opts { force_stream: stream, keep_archive, dedupe: !no_dedupe })
        }
        Cmd::Verify { deep } => cmd_verify(&cli.dir, deep),
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
        println!("{:<32} {:>9} {:>10}  {}{flag}", e.id, human(e.size), human(e.on_disk()), e.name);
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
        println!("added {id}: {} ({} download, {} on disk)", e.name, human(e.size), human(e.on_disk()));
        if e.non_commercial {
            println!("  licence: {}", e.licence);
        }
    }
    p.save(dir)?;
    println!("next: `dblock plan` to check disk space, then `dblock fetch`");
    Ok(())
}

fn cmd_remove(dir: &Path, ids: &[String], delete_data: bool, force: bool) -> Result<()> {
    let mut p = require_project(dir)?;
    let mut lock = Lock::load(dir)?;
    let mut doomed = Vec::new();
    if delete_data {
        let store = p.store_dir()?;
        for id in ids {
            match lock.databases.iter().find(|d| &d.id == id) {
                Some(l) => doomed.push((deletable(&store, l)?, l.clone())),
                None => println!("{id} is not in {}; no files to delete", project::LOCK_FILE),
            }
        }
        if !doomed.is_empty() && !force {
            for (d, _) in &doomed {
                println!("would delete {}", d.display());
            }
            bail!(
                "the store {} is shared: every project that uses these databases loses them. \
                 Run again with --force to delete; nothing was changed",
                store.display()
            );
        }
    }
    for id in ids {
        p.databases.retain(|d| d != id);
        if !delete_data && let Some(l) = lock.databases.iter().find(|d| &d.id == id) {
            println!("removed {id} from the project; files kept at {}", l.path);
        }
        lock.databases.retain(|d| &d.id != id);
    }
    let mut kept = Vec::new();
    for (d, l) in &doomed {
        // Hold the database's lock so a fetch of the same id in another project isn't moving a
        // new copy into place while this one is deleted. The lock only serialises the two runs:
        // a project that already uses the folder is not tracked and loses it (hence --force).
        let store = d.parent().context("database folder has no parent")?;
        let _lock = fetch::lock_database(store, &l.id)?;
        if d.symlink_metadata().is_err() {
            println!("{} is already gone", d.display());
            fetch::prune_tmp(store, &l.id);
            continue;
        }
        // While we waited for the lock, another run may have replaced the folder: check again.
        if let Err(err) = deletable(store, l) {
            println!("not deleted: {err:#}");
            kept.push(d.display().to_string());
            continue;
        }
        std::fs::remove_dir_all(d).with_context(|| format!("deleting {}", d.display()))?;
        fetch::prune_tmp(store, &l.id);
        println!("deleted {} (the store is shared; other projects that used it must fetch it again)", d.display());
    }
    p.save(dir)?;
    lock.save(dir)?;
    if !kept.is_empty() {
        bail!("left in place because they changed while waiting: {}", kept.join(", "));
    }
    Ok(())
}

/// The folder `remove --delete-data` may delete: `<store>/<id>`, a real folder (not a link),
/// holding a marker for the same id. Anything else written in a lock file is refused.
fn deletable(store: &Path, l: &project::Locked) -> Result<PathBuf> {
    let path = Path::new(&l.path);
    let meta = path.symlink_metadata().with_context(|| format!("{} ({}) is not there", l.path, l.id))?;
    if !meta.is_dir() {
        bail!("refusing to delete {}: not a folder", l.path);
    }
    let store = store.canonicalize().with_context(|| format!("store {} not found", store.display()))?;
    let real = path.canonicalize()?;
    if real != store.join(&l.id) {
        bail!("refusing to delete {}: it is not {}", l.path, store.join(&l.id).display());
    }
    match project::read_marker(&real) {
        Some(m) if m.id == l.id => Ok(real),
        Some(m) => bail!("refusing to delete {}: its marker is for `{}`", l.path, m.id),
        None => bail!("refusing to delete {}: no {} marker inside", l.path, project::MARKER),
    }
}

fn cmd_plan(dir: &Path, entries: &[Entry]) -> Result<()> {
    let p = require_project(dir)?;
    let store = p.store_dir()?;
    println!("store: {}", store.display());
    let opts = fetch::Opts { force_stream: false, keep_archive: false, dedupe: true };
    let mut total = 0;
    for id in &p.databases {
        let e = catalog::find(entries, id)?;
        // Whether the tool that reads it is here: a database for a missing tool is a wasted download.
        if let Some(c) = &e.compat
            && matches!(check::check(c), check::Verdict::NotInstalled)
        {
            println!("  {id}: note: `{}` ({}) is not installed here (fine if it runs in a container)", c.command[0], e.tool);
        }
        match fetch::existing(&store, e) {
            fetch::Stored::Ready(_) => {
                println!("  {id}: already in the store");
                continue;
            }
            fetch::Stored::Bad(why) => {
                println!("  {id}: CAN'T REUSE. {why}");
                continue;
            }
            fetch::Stored::Absent => {}
        }
        if let Some(prog) = fetch::missing_prepare(e) {
            println!("  {id}: WON'T FETCH. `{prog}` prepares it after the download and is not on PATH (install {} first)", e.tool);
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
        match fetch::obtain(&store, e, opts) {
            Ok((found, true)) => {
                eprintln!(
                    "  already in {}, reusing it (file names and sizes match; `dblock verify --deep` re-hashes every file)",
                    found.path
                );
                lock.upsert(found);
            }
            Ok((l, false)) => {
                if l.linked_files > 0 {
                    eprintln!(
                        "  done: {} files, {} ({} on disk: {} hard-linked) at {}",
                        l.files,
                        human(l.bytes),
                        human(l.bytes - l.linked_bytes),
                        l.linked_files,
                        l.data_path
                    );
                } else {
                    eprintln!("  done: {} files, {} at {}", l.files, human(l.bytes), l.data_path);
                }
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

fn cmd_verify(dir: &Path, deep: bool) -> Result<()> {
    let lock = Lock::load(dir)?;
    if lock.databases.is_empty() {
        bail!("nothing locked yet; run `dblock fetch`");
    }
    let mut bad = 0;
    for l in &lock.databases {
        let mut out = String::new();
        let ok = verify_one(l, deep, &mut out);
        print!("{out}");
        if !ok {
            bad += 1;
        }
    }
    if bad > 0 {
        bail!("{bad} database(s) differ from dblock.lock or couldn't be read");
    }
    Ok(())
}

/// Check one locked database, writing its result line(s) to `out`; `false` if it differs or
/// couldn't be read. A read error (an unreadable file, say) is reported as `ERROR` for this
/// database only, so the others are still checked.
fn verify_one(l: &project::Locked, deep: bool, out: &mut String) -> bool {
    match verify_inner(l, deep, out) {
        Ok(ok) => ok,
        Err(err) => {
            out.push_str(&format!("ERROR    {}: couldn't check it: {err:#}\n", l.id));
            false
        }
    }
}

fn verify_inner(l: &project::Locked, deep: bool, out: &mut String) -> Result<bool> {
    macro_rules! say {
        ($($t:tt)*) => {{
            out.push_str(&format!($($t)*));
            out.push('\n');
        }};
    }
    let path = Path::new(&l.path);
    if !path.is_dir() {
        say!("MISSING  {}: {} is gone", l.id, l.path);
        return Ok(false);
    }
    let (files, bytes, md5) = fetch::manifest_as(path, l.manifest_format)?;
    if md5 != l.manifest_md5 {
        say!(
            "CHANGED  {}: now {} files / {}, locked {} files / {}",
            l.id,
            files,
            human(bytes),
            l.files,
            human(l.bytes)
        );
        return Ok(false);
    }
    if !deep {
        say!("ok       {}: {} files, {} (names and sizes)", l.id, files, human(bytes));
        return Ok(true);
    }
    let Some(want) = &l.contents_sha256 else {
        say!("UNCHECKED {}: no SHA-256 sums recorded (fetched by dblock 0.1); fetch it again to record them", l.id);
        return Ok(false);
    };
    eprintln!("  hashing {} ({})", l.id, human(bytes));
    // GTDB-Tk's own folder hashes, when the catalog named some: they come from the publisher,
    // so they also vouch for what was fetched, not just that it hasn't changed since.
    if !l.verify_dirs.is_empty() {
        let wrong = fetch::check_dirs(path, &l.verify_dirs)?;
        if !wrong.is_empty() {
            say!("CHANGED  {}: GTDB-Tk folder hash mismatch: {}", l.id, wrong.join("; "));
            return Ok(false);
        }
    }
    let now = fetch::sums_as(path, l.manifest_format)?;
    if &fetch::sha256_hex(&now) == want {
        let dirs = if l.verify_dirs.is_empty() {
            String::new()
        } else {
            format!(" and {} GTDB-Tk folder hash(es) match", l.verify_dirs.len())
        };
        say!("ok       {}: {} files, {}, every file's SHA-256 matches{dirs}", l.id, files, human(bytes));
        return Ok(true);
    }
    // Name the files, using the list stored with the data if it still matches the lock.
    // (A list upgraded from the v0.1.0 format also has symlink lines; leave those out.)
    let stored = std::fs::read_to_string(path.join(project::SUMS))
        .ok()
        .map(|t| if l.manifest_format.is_none() { fetch::without_links(&t) } else { t })
        .filter(|t| &fetch::sha256_hex(t) == want);
    match stored {
        Some(old) => {
            let old: std::collections::BTreeSet<&str> = old.lines().collect();
            let changed: Vec<&str> = now
                .lines()
                .filter(|line| !old.contains(line))
                .filter_map(|line| line.splitn(3, "  ").nth(2))
                .collect();
            say!("CHANGED  {}: content differs in {}", l.id, changed.join(", "));
        }
        None => say!("CHANGED  {}: content differs (and its {} list was altered too)", l.id, project::SUMS),
    }
    Ok(false)
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
            Some(var) => println!("export {var}={}", sh_quote(&l.data_path)),
            None => println!("# {} ({}): {}", l.id, l.tool, l.data_path),
        }
    }
    Ok(())
}

/// Single-quote for POSIX shells: `'` becomes `'\''`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_paths_with_single_quotes() {
        assert_eq!(sh_quote("/data/it's here"), r"'/data/it'\''s here'");
        assert_eq!(sh_quote("/plain"), "'/plain'");
    }
}

#[cfg(test)]
mod remove_tests {
    use super::*;

    fn locked(id: &str, path: &Path) -> project::Locked {
        project::Locked {
            id: id.into(),
            name: String::new(),
            version: String::new(),
            tool: String::new(),
            url: String::new(),
            archive_md5: String::new(),
            archive_bytes: 0,
            fetched: String::new(),
            path: path.display().to_string(),
            data_path: path.display().to_string(),
            files: 0,
            bytes: 0,
            manifest_md5: String::new(),
            contents_sha256: None,
            manifest_format: None,
            env: None,
            licence: String::new(),
            non_commercial: false,
            verify_dirs: Default::default(),
            linked_files: 0,
            linked_bytes: 0,
            parts: Vec::new(),
            prepared: None,
        }
    }

    /// A lock entry for `dir` as `fetch` would record it, SHA-256 list included.
    // Only the Unix-only permission test uses it.
    #[cfg(unix)]
    fn locked_with_sums(id: &str, dir: &Path) -> project::Locked {
        let mut l = locked(id, dir);
        (l.files, l.bytes, l.manifest_md5) = fetch::manifest(dir).unwrap();
        l.contents_sha256 = Some(fetch::sha256_hex(&fetch::sums(dir).unwrap()));
        l.manifest_format = Some(fetch::MANIFEST_FORMAT);
        l
    }

    #[cfg(unix)]
    #[test]
    fn deep_verify_reports_an_unreadable_database_and_goes_on() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let (a, b) = (root.path().join("a"), root.path().join("b"));
        for d in [&a, &b] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("x"), b"data").unwrap();
        }
        let (la, lb) = (locked_with_sums("a", &a), locked_with_sums("b", &b));
        std::fs::set_permissions(a.join("x"), std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(a.join("x")).is_ok() {
            return; // running as root: nothing is unreadable
        }
        let mut out = String::new();
        let results: Vec<bool> = [&la, &lb].iter().map(|l| verify_one(l, true, &mut out)).collect();
        std::fs::set_permissions(a.join("x"), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(results, [false, true], "{out}");
        assert!(out.lines().next().unwrap().starts_with("ERROR    a: couldn't check it"), "{out}");
        assert!(out.lines().nth(1).unwrap().starts_with("ok       b:"), "{out}");
    }

    #[test]
    fn only_marked_store_folders_can_be_deleted() {
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("store");
        let db = store.join("mydb");
        std::fs::create_dir_all(&db).unwrap();
        // No marker yet.
        assert!(deletable(&store, &locked("mydb", &db)).is_err());
        project::write_marker(&db, &locked("mydb", &db)).unwrap();
        assert!(deletable(&store, &locked("mydb", &db)).is_ok());
        // A lock entry pointing outside the store, even at a folder with a matching marker.
        let elsewhere = root.path().join("mydb");
        std::fs::create_dir_all(&elsewhere).unwrap();
        project::write_marker(&elsewhere, &locked("mydb", &elsewhere)).unwrap();
        assert!(deletable(&store, &locked("mydb", &elsewhere)).is_err());
        // The right folder under another id's name.
        assert!(deletable(&store, &locked("other", &db)).is_err());
    }

    #[test]
    fn delete_data_waits_for_the_database_lock() {
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("store");
        let db = store.join("mydb");
        std::fs::create_dir_all(&db).unwrap();
        project::write_marker(&db, &locked("mydb", &db)).unwrap();
        let proj = root.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        Project { store: Some(store.display().to_string()), databases: vec!["mydb".into()] }.save(&proj).unwrap();
        Lock { databases: vec![locked("mydb", &db)] }.save(&proj).unwrap();

        // Another dblock is fetching (or reusing) mydb in this store.
        let held = fetch::lock_database(&store, "mydb").unwrap();
        std::thread::scope(|s| {
            let remover = s.spawn(|| cmd_remove(&proj, &["mydb".into()], true, true));
            std::thread::sleep(std::time::Duration::from_millis(300));
            assert!(db.is_dir(), "deleted while another dblock held the lock");
            drop(held);
            remover.join().unwrap().unwrap();
        });
        assert!(!db.exists());
    }

    #[test]
    fn delete_data_checks_again_after_waiting_and_prunes_temp_folders() {
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("store");
        let db = store.join("mydb");
        std::fs::create_dir_all(&db).unwrap();
        std::fs::create_dir_all(store.join(".tmp/mydb")).unwrap();
        project::write_marker(&db, &locked("mydb", &db)).unwrap();
        let proj = root.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let save = || {
            Project { store: Some(store.display().to_string()), databases: vec!["mydb".into()] }.save(&proj).unwrap();
            Lock { databases: vec![locked("mydb", &db)] }.save(&proj).unwrap();
        };
        save();

        // While remove waits for the lock, the folder is replaced by something it must not delete.
        let held = fetch::lock_database(&store, "mydb").unwrap();
        std::thread::scope(|s| {
            let remover = s.spawn(|| cmd_remove(&proj, &["mydb".into()], true, true));
            std::thread::sleep(std::time::Duration::from_millis(300));
            std::fs::remove_file(db.join(project::MARKER)).unwrap();
            drop(held);
            let err = remover.join().unwrap().unwrap_err().to_string();
            assert!(err.contains("changed while waiting"), "{err}");
        });
        assert!(db.is_dir(), "deleted a folder that changed while waiting for the lock");

        // Normal removal also clears the empty <store>/.tmp/<id>/.
        project::write_marker(&db, &locked("mydb", &db)).unwrap();
        save();
        cmd_remove(&proj, &["mydb".into()], true, true).unwrap();
        assert!(!db.exists() && !store.join(".tmp").exists());
    }
}
