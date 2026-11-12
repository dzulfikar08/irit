//! Run records: `run.json`, `replay.sh` and `ro-crate-metadata.json` in one folder per server session.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const IRIT_MCP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileRecord {
    pub path: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Set for symbolic links (e.g. `sheetgen --link-into`); size and hash are then omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symlink_to: Option<String>,
    /// An output known to differ between identical runs (a log, a file with the run's date):
    /// replay.sh checks that it exists, not its hash.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub volatile: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkRecord {
    pub path: String,
    /// What the link holds, as written (relative to the link's folder).
    pub target: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Exit status 0.
    Ok,
    /// The tool ran and exited non-zero.
    Failed,
    /// Parameters were refused before running anything.
    Rejected,
    /// The tool could not be started (binary missing, I/O error).
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRecord {
    pub id: u32,
    pub mcp_tool: String,
    pub status: Status,
    /// Parameters exactly as the client sent them.
    pub params: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    pub argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub cwd: String,
    pub started: String,
    pub ended: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<i32>,
    pub inputs: Vec<FileRecord>,
    pub outputs: Vec<FileRecord>,
    /// Folders the outputs go into: created by the server when missing, before the tool ran, and
    /// by replay.sh (`mkdir -p`), so a replay works from a clean folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dirs: Vec<String>,
    /// Symlinks the server made before the tool ran, recreated by replay.sh (`ln -sfn`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkRecord>,
    /// Paths relative to the run folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    /// The tool's stdout is its result (a sheet or table written to stdout), so replay checks it.
    #[serde(default)]
    pub stdout_is_output: bool,
    /// Environment variables that decided where the tool looked or wrote, as resolved values
    /// (e.g. the dblock store, the Nextflow home). Passed to the tool and exported by replay.sh.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// A third-party program (fastp, Bakta, CheckM2, GTDB-Tk): replay.sh takes it from PATH,
    /// not from `$IRIT_BIN_DIR`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub external: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunLog {
    pub run_id: String,
    /// Licence of the run record itself (run.json, replay.sh, the crate metadata); the data
    /// files it references keep their own terms.
    #[serde(default = "default_license")]
    pub license: String,
    pub irit_mcp_version: String,
    pub created: String,
    pub calls: Vec<CallRecord>,
}

pub struct Run {
    pub dir: PathBuf,
    pub log: RunLog,
}

pub const DEFAULT_LICENSE: &str = "https://spdx.org/licenses/CC0-1.0";

fn default_license() -> String {
    DEFAULT_LICENSE.to_string()
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn new_run_id() -> String {
    format!("{}-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"), std::process::id())
}

impl Run {
    pub fn new(runs_root: &Path, run_id: &str) -> Run {
        Run {
            dir: runs_root.join(run_id),
            log: RunLog {
                run_id: run_id.to_string(),
                license: default_license(),
                irit_mcp_version: IRIT_MCP_VERSION.to_string(),
                created: now(),
                calls: Vec::new(),
            },
        }
    }

    pub fn ensure_dir(&self) -> io::Result<()> {
        fs::create_dir_all(self.dir.join("calls"))
    }

    pub fn add(&mut self, call: CallRecord) -> io::Result<()> {
        self.log.calls.push(call);
        self.log.calls.sort_by_key(|c| c.id);
        self.write()
    }

    pub fn write(&self) -> io::Result<()> {
        self.ensure_dir()?;
        write_atomic(&self.dir.join("run.json"), &serde_json::to_vec_pretty(&self.log)?)?;
        let script = self.dir.join("replay.sh");
        write_atomic(&script, replay_script(&self.log).as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
        }
        let crate_json = serde_json::to_vec_pretty(&ro_crate(&self.log))?;
        write_atomic(&self.dir.join("ro-crate-metadata.json"), &crate_json)
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

pub fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        h.update(&buf[..n]);
    }
    Ok((size, hex(&h.finalize())))
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Every regular file under `paths` (files themselves, or folders walked recursively).
/// Symlinks to files are followed (tools read through them); symlinked folders are not
/// descended, which avoids cycles.
pub fn input_records(paths: &[PathBuf]) -> Vec<FileRecord> {
    let mut files = BTreeSet::new();
    for p in paths {
        collect_files(p, &mut files, true);
    }
    files
        .into_iter()
        .filter_map(|p| {
            let (size, sha) = hash_file(&p).ok()?;
            Some(FileRecord { path: display(&p), size, sha256: Some(sha), symlink_to: None, volatile: false })
        })
        .collect()
}

fn collect_files(p: &Path, out: &mut BTreeSet<PathBuf>, top: bool) {
    let Ok(meta) = fs::symlink_metadata(p) else { return };
    if meta.is_dir() || (top && meta.file_type().is_symlink() && p.is_dir()) {
        let Ok(rd) = fs::read_dir(p) else { return };
        for e in rd.flatten() {
            collect_files(&e.path(), out, false);
        }
    } else if p.is_file() {
        out.insert(p.to_path_buf());
    }
}

/// What a file looked like before a call. Size and mtime alone miss a rewrite within the
/// filesystem's timestamp resolution, so the inode change time and inode number are included
/// (Unix), and files under 1 MB are hashed.
#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    changed: Option<(u64, u64, i64, i64)>,
    sha256: Option<String>,
    link: Option<PathBuf>,
}

const STAMP_HASH_LIMIT: u64 = 1 << 20;

/// State of the places a tool may write, taken before it runs.
pub type Snapshot = BTreeMap<PathBuf, Stamp>;

pub fn snapshot(targets: &[PathBuf]) -> Snapshot {
    let mut snap = Snapshot::new();
    for t in targets {
        stamp_tree(t, &mut snap, true);
    }
    snap
}

/// A target that is itself a symlink (`qc -> /scratch/qc`, `sheet.csv -> real.csv`) is
/// followed, since the tool writes through it; symlinks below a target are recorded as links
/// (sheetgen's `link_into`) and never descended, which avoids cycles.
fn stamp_tree(p: &Path, snap: &mut Snapshot, top: bool) {
    let Ok(lmeta) = fs::symlink_metadata(p) else { return };
    let meta = if top && lmeta.file_type().is_symlink() {
        match fs::metadata(p) {
            Ok(m) => m,
            Err(_) => lmeta, // dangling: record the link itself
        }
    } else {
        lmeta
    };
    if meta.is_dir() {
        if let Ok(rd) = fs::read_dir(p) {
            for e in rd.flatten() {
                stamp_tree(&e.path(), snap, false);
            }
        }
        return;
    }
    let link = meta.file_type().is_symlink().then(|| fs::read_link(p).ok()).flatten();
    let sha256 = (link.is_none() && meta.len() < STAMP_HASH_LIMIT).then(|| hash_file(p).ok().map(|(_, h)| h)).flatten();
    snap.insert(p.to_path_buf(), Stamp { len: meta.len(), modified: meta.modified().ok(), changed: change_id(&meta), sha256, link });
}

#[cfg(unix)]
fn change_id(m: &fs::Metadata) -> Option<(u64, u64, i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    Some((m.dev(), m.ino(), m.ctime(), m.ctime_nsec()))
}

#[cfg(not(unix))]
fn change_id(_: &fs::Metadata) -> Option<(u64, u64, i64, i64)> {
    None
}

/// Files under `targets` that are new or changed since `before`.
pub fn output_records(before: &Snapshot, targets: &[PathBuf]) -> Vec<FileRecord> {
    snapshot(targets)
        .into_iter()
        .filter(|(p, st)| before.get(p) != Some(st))
        .filter_map(|(p, st)| match st.link {
            Some(target) => Some(FileRecord { path: display(&p), size: 0, sha256: None, symlink_to: Some(display(&target)), volatile: false }),
            None => {
                let (size, sha) = hash_file(&p).ok()?;
                Some(FileRecord { path: display(&p), size, sha256: Some(sha), symlink_to: None, volatile: false })
            }
        })
        .collect()
}

fn display(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// POSIX single-quoting: safe for any byte string except NUL (refused earlier).
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%".contains(&b)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// One line of shell comment. Everything after `# ` up to the newline is ignored by sh, so the
/// only way out is a line break: newlines, carriage returns and other control characters are
/// written as escapes. Every comment in replay.sh goes through here.
pub fn sh_comment(s: &str) -> String {
    let mut out = String::from("# ");
    for c in s.chars() {
        match c {
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => out += &format!("\\u{{{:x}}}", c as u32),
            c => out.push(c),
        }
    }
    out.push('\n');
    out
}

const REPLAY_PRELUDE: &str = r#"set -u
RUN_DIR=$(CDPATH= cd -P -- "$(dirname -- "$0")" && pwd -P)
OUT="$RUN_DIR/replay"
mkdir -p "$OUT"
failed=0
ran=0
checked=0

# irit binaries come from $IRIT_BIN_DIR when set, else from PATH; third-party tools (fastp,
# bakta, checkm2, gtdbtk) always come from PATH.
irit() { t=$1; shift; if [ -n "${IRIT_BIN_DIR:-}" ]; then "$IRIT_BIN_DIR/$t" "$@"; else "$t" "$@"; fi; }
ext() { "$@"; }
# Only the first line of --version is compared (GTDB-Tk adds one when a newer release exists).
want_version() {
  got=$(irit "$1" --version 2>/dev/null | head -n 1)
  [ "$got" = "$2" ] || echo "replay: warning: $1 reports '$got'; the run used '$2'" >&2
}
want_ext_version() {
  got=$(ext "$1" --version 2>/dev/null | head -n 1)
  [ "$got" = "$2" ] || echo "replay: warning: $1 reports '$got'; the run used '$2'" >&2
}
step() {
  n=$1; want=$2; shift 2
  ran=$((ran + 1))
  "$@" >"$OUT/$n.stdout" 2>"$OUT/$n.stderr"
  s=$?
  if [ "$s" -ne "$want" ]; then
    echo "replay: call $n exited $s, the run exited $want (see $OUT/$n.stderr)" >&2
    failed=1
  fi
}
fetch_wanted() { [ "${IRIT_REPLAY_FETCH:-}" = 1 ]; }
sha() { if command -v sha256sum >/dev/null 2>&1; then sha256sum -- "$1"; else shasum -a 256 -- "$1"; fi | cut -d' ' -f1; }
check() {
  checked=$((checked + 1))
  if [ ! -f "$2" ]; then echo "replay: missing $2" >&2; failed=1; return; fi
  [ "$(sha "$2")" = "$1" ] || { echo "replay: differs: $2" >&2; failed=1; }
}
check_exists() {
  checked=$((checked + 1))
  [ -f "$1" ] || { echo "replay: missing $1" >&2; failed=1; }
}
check_link() {
  checked=$((checked + 1))
  [ "$(readlink -- "$2" 2>/dev/null)" = "$1" ] || { echo "replay: link differs: $2" >&2; failed=1; }
}
"#;

/// A path that can't be read as an option by `cd`, `sha256sum` and the like.
fn safe_path(p: &str) -> String {
    if p.starts_with('-') { format!("./{p}") } else { p.to_string() }
}

/// `dblock fetch` downloads databases again; replay.sh only does that when asked.
fn is_fetch(c: &CallRecord) -> bool {
    c.argv.first().is_some_and(|t| t == "dblock") && c.argv.get(1).is_some_and(|s| s == "fetch")
}

fn env_name_ok(k: &str) -> bool {
    !k.is_empty() && !k.starts_with(|c: char| c.is_ascii_digit()) && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn replay_script(log: &RunLog) -> String {
    let mut s = String::new();
    s += "#!/bin/sh\n";
    s += &sh_comment(&format!(
        "Replays irit-mcp run {} without any AI, then checks every output against the SHA-256",
        log.run_id
    ));
    s += &sh_comment(&format!("recorded in run.json. Generated by irit-mcp {}.", log.irit_mcp_version));
    s += "# Usage: sh replay.sh   (IRIT_BIN_DIR=/path/to/bin sh replay.sh to pick the binaries)\n";
    s += "# dblock fetch calls are skipped unless IRIT_REPLAY_FETCH=1 (they download databases again).\n";
    s += "# Each call's recorded environment (dblock store, Nextflow home) is exported before it runs.\n";
    s += REPLAY_PRELUDE;

    let ran: Vec<&CallRecord> = log.calls.iter().filter(|c| matches!(c.status, Status::Ok)).collect();
    let mut versions = BTreeSet::new();
    let mut unknown = BTreeSet::new();
    for c in &ran {
        match (c.argv.first(), &c.version) {
            (Some(tool), Some(v)) => {
                versions.insert((tool.clone(), v.clone(), c.external));
            }
            (Some(tool), None) => {
                unknown.insert(tool.clone());
            }
            _ => {}
        }
    }
    s += "\n";
    for (tool, v, external) in &versions {
        let f = if *external { "want_ext_version" } else { "want_version" };
        s += &format!("{f} {} {}\n", sh_quote(tool), sh_quote(v));
    }
    let unknown: Vec<&String> = unknown.iter().filter(|t| !versions.iter().any(|(x, _, _)| x == *t)).collect();
    if !unknown.is_empty() {
        let names = unknown.iter().map(|t| t.as_str()).collect::<Vec<_>>().join(" ");
        s += &format!("echo {} >&2\n", sh_quote(&format!("replay: version unknown (--version failed in the run), not checked: {names}")));
    }

    // Final expected state of each output path: a later call that rewrites a file wins.
    // The flag says whether that call was a fetch, whose check only runs when fetches do.
    let mut expected: BTreeMap<String, (String, bool)> = BTreeMap::new();
    for c in &log.calls {
        let n = format!("{:03}", c.id);
        s += "\n";
        s += &sh_comment(&format!("call {n}: {} ({}, {})", c.mcp_tool, status_word(c.status), c.started));
        let cmd = c.argv.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" ");
        match c.status {
            Status::Ok => {
                let fetch = is_fetch(c);
                s += &format!("cd {} || exit 1\n", sh_quote(&safe_path(&c.cwd)));
                for (k, v) in &c.env {
                    if env_name_ok(k) {
                        s += &format!("{k}={}; export {k}\n", sh_quote(v));
                    }
                }
                if !c.dirs.is_empty() {
                    let dirs = c.dirs.iter().map(|d| sh_quote(d)).collect::<Vec<_>>().join(" ");
                    s += &format!("mkdir -p -- {dirs} || exit 1\n");
                }
                for l in &c.links {
                    s += &format!("ln -sfn -- {} {} || exit 1\n", sh_quote(&l.target), sh_quote(&safe_path(&l.path)));
                }
                let step = format!("step {n} {} {} {cmd}\n", c.exit_status.unwrap_or(0), runner(c));
                if fetch {
                    s += &sh_comment(&format!("downloads databases again: {}", step.trim_end()));
                    s += &format!(
                        "if fetch_wanted; then\n  {step}else\n  echo {} >&2\nfi\n",
                        sh_quote(&format!("replay: call {n} (dblock fetch) skipped; set IRIT_REPLAY_FETCH=1 to download again"))
                    );
                } else {
                    s += &step;
                }
                for o in &c.outputs {
                    let line = match (&o.symlink_to, &o.sha256) {
                        (Some(t), _) => format!("check_link {} {}", sh_quote(t), sh_quote(&safe_path(&o.path))),
                        (None, Some(_)) if o.volatile => format!("check_exists {}", sh_quote(&safe_path(&o.path))),
                        (None, Some(h)) => format!("check {} {}", sh_quote(h), sh_quote(&safe_path(&o.path))),
                        _ => continue,
                    };
                    expected.insert(o.path.clone(), (line, fetch));
                }
                if let (true, Some(h)) = (c.stdout_is_output, &c.stdout_sha256) {
                    expected.insert(format!("$OUT/{n}.stdout"), (format!("check {} \"$OUT/{n}.stdout\"", sh_quote(h)), fetch));
                }
            }
            Status::Failed => {
                s += &sh_comment(&format!("not replayed: exited {} in the run", c.exit_status.unwrap_or(-1)));
                if let (Some(0), Some(e)) = (c.exit_status, &c.error) {
                    s += &sh_comment(e);
                }
                if !cmd.is_empty() {
                    s += &sh_comment(&format!("{} {cmd}", runner(c)));
                }
            }
            Status::Rejected | Status::Error => {
                s += &sh_comment(&format!("not run: {}", c.error.as_deref().unwrap_or("")));
            }
        }
    }

    s += "\n# outputs\n";
    for (line, fetch) in expected.values() {
        if *fetch {
            s += &format!("if fetch_wanted; then {line}; fi\n");
        } else {
            s += line;
            s += "\n";
        }
    }
    s += "\nif [ \"$failed\" -eq 0 ]; then echo \"replay: $ran commands re-run, $checked outputs match\" >&2; fi\nexit \"$failed\"\n";
    s
}

/// The replay.sh function that finds the program: `irit` (`$IRIT_BIN_DIR` or PATH) or `ext` (PATH).
fn runner(c: &CallRecord) -> &'static str {
    if c.external { "ext" } else { "irit" }
}

fn status_word(s: Status) -> &'static str {
    match s {
        Status::Ok => "ok",
        Status::Failed => "failed",
        Status::Rejected => "rejected",
        Status::Error => "error",
    }
}

/// `file://` URI for a local path, percent-encoding everything outside RFC 3986's unreserved set.
pub fn file_uri(path: &str) -> String {
    let mut out = String::from("file://");
    if !path.starts_with('/') {
        out.push('/');
    }
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out += &format!("%{b:02X}");
        }
    }
    out
}

/// Workflow Run RO-Crate, Process Run Crate profile 0.6 (RO-Crate 1.3; its context URL
/// https://w3id.org/ro/crate/1.3/context resolves to JSON-LD, checked 2026-11-10).
/// Inputs and outputs live outside the run folder, so they are referenced by `file://` URIs
/// (absolute-URI data entities) rather than copied into the crate.
pub fn ro_crate(log: &RunLog) -> Value {
    let profile = "https://w3id.org/ro/wfrun/process/0.6";
    let mut graph = vec![
        json!({
            "@id": "ro-crate-metadata.json",
            "@type": "CreativeWork",
            "conformsTo": {"@id": "https://w3id.org/ro/crate/1.3"},
            "about": {"@id": "./"}
        }),
        json!({"@id": profile, "@type": "CreativeWork", "name": "Process Run Crate", "version": "0.6"}),
        json!({"@id": "run.json", "@type": "File", "name": "irit-mcp call log", "encodingFormat": "application/json"}),
        json!({"@id": "replay.sh", "@type": "File", "name": "Replay script (POSIX sh)", "encodingFormat": "text/x-shellscript"}),
    ];
    let mut actions = Vec::new();
    let mut tools = BTreeMap::new();
    let mut files: BTreeMap<String, Value> = BTreeMap::new();
    let mut parts = vec![json!({"@id": "run.json"}), json!({"@id": "replay.sh"})];
    for c in &log.calls {
        for (path, what) in [(&c.stdout, "stdout"), (&c.stderr, "stderr")] {
            let Some(path) = path else { continue };
            parts.push(json!({"@id": path}));
            graph.push(json!({
                "@id": path,
                "@type": "File",
                "name": format!("{what} of call {} ({})", c.id, c.mcp_tool),
                "encodingFormat": "text/plain",
            }));
        }
    }
    for c in log.calls.iter().filter(|c| matches!(c.status, Status::Ok | Status::Failed)) {
        let Some(tool) = c.argv.first() else { continue };
        let version = c.version.as_deref().and_then(crate::external::version_number).unwrap_or("unknown");
        let tool_id = format!("#{tool}-{version}");
        tools.entry(tool_id.clone()).or_insert_with(|| {
            json!({
                "@id": tool_id,
                "@type": "SoftwareApplication",
                "name": tool,
                "softwareVersion": version,
                "url": match crate::external::pinned(tool).filter(|_| c.external) {
                    Some(p) => p.url.to_string(),
                    None => format!("https://github.com/dzulfikar08/irit/blob/main/docs/tools/{tool}.md"),
                }
            })
        });
        let mut entity = |f: &FileRecord| {
            let id = file_uri(&f.path);
            files.entry(id.clone()).or_insert_with(|| {
                let mut e = json!({"@id": id, "@type": "File", "name": f.path});
                if f.symlink_to.is_none() {
                    e["contentSize"] = json!(f.size.to_string());
                }
                e
            });
            json!({"@id": id})
        };
        let object: Vec<Value> = c.inputs.iter().map(&mut entity).collect();
        let result: Vec<Value> = c.outputs.iter().map(&mut entity).collect();
        let id = format!("#call-{:03}", c.id);
        let mut action = json!({
            "@id": id,
            "@type": "CreateAction",
            "name": format!("{} (call {})", c.mcp_tool, c.id),
            "description": c.argv.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" "),
            "instrument": {"@id": tool_id},
            "object": object,
            "result": result,
            "startTime": c.started,
            "endTime": c.ended,
            "actionStatus": {"@id": if c.status == Status::Ok { "http://schema.org/CompletedActionStatus" } else { "http://schema.org/FailedActionStatus" }},
        });
        if c.status == Status::Failed {
            action["error"] = json!(format!("exit status {}", c.exit_status.unwrap_or(-1)));
        }
        actions.push(json!({"@id": id}));
        graph.push(action);
    }
    graph.insert(
        1,
        json!({
            "@id": "./",
            "@type": "Dataset",
            "name": format!("irit-mcp run {}", log.run_id),
            "description": format!(
                "Record of {} tool call(s) made through irit-mcp {}: exact commands, tool versions, input and output \
                 SHA-256 sums, and a replay script that re-runs the successful calls without an AI.",
                log.calls.len(),
                log.irit_mcp_version
            ),
            "license": {"@id": log.license},
            "datePublished": log.created,
            "conformsTo": {"@id": profile},
            "hasPart": parts,
            "mentions": actions,
        }),
    );
    graph.push(json!({"@id": log.license, "@type": "CreativeWork", "name": license_name(&log.license)}));
    graph.extend(tools.into_values());
    graph.extend(files.into_values());
    json!({
        "@context": ["https://w3id.org/ro/crate/1.3/context", "https://w3id.org/ro/terms/workflow-run/context"],
        "@graph": graph
    })
}

fn license_name(id: &str) -> &str {
    match id {
        DEFAULT_LICENSE => "CC0 1.0 Universal (applies to this run record, not to the data files it references)",
        other => other.rsplit('/').next().unwrap_or(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: u32, status: Status) -> CallRecord {
        CallRecord {
            id,
            mcp_tool: "leanqc".into(),
            status,
            params: json!({}),
            program: Some("/bin/leanqc".into()),
            argv: vec!["leanqc".into(), "--outdir=qc".into(), "--".into(), "my reads's.fq".into()],
            version: Some("leanqc 0.1.0".into()),
            cwd: "/data/x".into(),
            started: "2026-11-10T00:00:00.000Z".into(),
            ended: "2026-11-10T00:00:01.000Z".into(),
            exit_status: Some(if status == Status::Ok { 0 } else { 2 }),
            inputs: vec![FileRecord { path: "/data/x/my reads's.fq".into(), size: 3, sha256: Some("aa".into()), symlink_to: None, volatile: false }],
            outputs: vec![FileRecord { path: "/data/x/qc/r.json".into(), size: 5, sha256: Some("bb".into()), symlink_to: None, volatile: false }],
            dirs: Vec::new(),
            links: Vec::new(),
            stdout: None,
            stdout_sha256: None,
            stderr: None,
            stdout_is_output: false,
            env: BTreeMap::new(),
            external: false,
            error: None,
        }
    }

    fn log_of(calls: Vec<CallRecord>) -> RunLog {
        RunLog { run_id: "r1".into(), license: default_license(), irit_mcp_version: "0.1.0".into(), created: now(), calls }
    }

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("--outdir=qc"), "--outdir=qc");
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(sh_quote("$(rm -rf /)"), "'$(rm -rf /)'");
        assert_eq!(sh_quote(""), "''");
        assert_eq!(file_uri("/a b/é.fq"), "file:///a%20b/%C3%A9.fq");
    }

    #[test]
    fn replay_lists_only_successful_calls() {
        let mut rejected = call(3, Status::Rejected);
        rejected.argv.clear();
        rejected.error = Some("files: x.fq does not exist".into());
        let log = log_of(vec![call(1, Status::Ok), call(2, Status::Failed), rejected]);
        let s = replay_script(&log);
        assert!(s.contains("want_version leanqc 'leanqc 0.1.0'"));
        assert!(s.contains("cd /data/x || exit 1\nstep 001 0 irit leanqc --outdir=qc -- 'my reads'\\''s.fq'\n"));
        assert!(s.contains("# not replayed: exited 2 in the run\n# irit leanqc"));
        assert!(s.contains("# not run: files: x.fq does not exist"));
        assert_eq!(s.matches("\ncheck bb /data/x/qc/r.json").count(), 1);
        assert!(s.contains("$ran commands re-run, $checked outputs match"));
        assert!(!s.contains("mkdir -p --"));
    }

    #[test]
    fn replay_creates_output_folders() {
        let mut c = call(1, Status::Ok);
        c.dirs = vec!["/data/x/qc".into(), "/data/x/my run".into()];
        let s = replay_script(&log_of(vec![c]));
        assert!(s.contains("mkdir -p -- /data/x/qc '/data/x/my run' || exit 1\nstep 001 "), "{s}");
        let mut c = call(1, Status::Ok);
        c.links = vec![LinkRecord { path: "/data/x/genomes/s 1.fna".into(), target: "../asm/s1/contigs.fa".into() }];
        let s = replay_script(&log_of(vec![c]));
        assert!(s.contains("ln -sfn -- ../asm/s1/contigs.fa '/data/x/genomes/s 1.fna' || exit 1\nstep 001 "), "{s}");
    }

    #[test]
    fn records_and_snapshots() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("qc");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("old.txt"), "old").unwrap();
        let before = snapshot(std::slice::from_ref(&out));
        fs::write(out.join("new.txt"), "abc").unwrap();
        let recs = output_records(&before, std::slice::from_ref(&out));
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].size, 3);
        assert_eq!(recs[0].sha256.as_deref(), Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        let ins = input_records(&[d.path().to_path_buf()]);
        assert_eq!(ins.len(), 2);

        let mut run = Run::new(d.path(), "r1");
        run.add(call(2, Status::Ok)).unwrap();
        run.add(call(1, Status::Failed)).unwrap();
        let log: RunLog = serde_json::from_slice(&fs::read(d.path().join("r1/run.json")).unwrap()).unwrap();
        assert_eq!(log.calls.iter().map(|c| c.id).collect::<Vec<_>>(), [1, 2]);
        assert!(d.path().join("r1/replay.sh").is_file());
        let crate_json: Value = serde_json::from_slice(&fs::read(d.path().join("r1/ro-crate-metadata.json")).unwrap()).unwrap();
        let graph = crate_json["@graph"].as_array().unwrap();
        assert!(graph.iter().any(|e| e["@id"] == "#call-002" && e["instrument"]["@id"] == "#leanqc-0.1.0"));
        assert!(graph.iter().any(|e| e["@id"] == "file:///data/x/my%20reads%27s.fq"));
    }

    #[test]
    fn comments_cannot_be_escaped() {
        assert_eq!(sh_comment("a\nb\r\tc\u{1b}"), "# a\\nb\\r\\tc\\u{1b}\n");
        let mut failed = call(2, Status::Failed);
        failed.argv = vec!["sheetgen".into(), "--set=x=a\ntouch PWNED #".into()];
        let mut rejected = call(3, Status::Rejected);
        rejected.error = Some("files: a\ntouch PWNED2 does not exist".into());
        let s = replay_script(&log_of(vec![failed, rejected]));
        for line in s.lines() {
            assert!(!line.trim_start().starts_with("touch"), "{line:?} in\n{s}");
        }
    }

    fn sh_syntax_ok(script: &Path) {
        let out = std::process::Command::new("sh").arg("-n").arg(script).output().unwrap();
        assert!(out.status.success(), "sh -n: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn tree(dir: &Path) -> BTreeSet<PathBuf> {
        let mut out = BTreeSet::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if fs::symlink_metadata(&p).unwrap().is_dir() {
                    stack.push(p.clone());
                }
                out.insert(p);
            }
        }
        out
    }

    /// Hostile strings in every place replay.sh is generated from: argv, cwd, output paths,
    /// link targets, versions, errors, env values, the run id. Running the script with a stub
    /// tool must create nothing but its own replay/ folder.
    #[test]
    #[cfg(unix)]
    fn hostile_values_are_inert() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(d.path()).unwrap();
        let canary = root.join("PWNED");
        let c = canary.to_str().unwrap();
        let evil = [
            format!("a\ntouch {c} #"),
            format!("a\rtouch {c}"),
            format!("x'; touch {c}; '"),
            format!("$(touch {c})"),
            format!("`touch {c}`"),
            format!("\"; touch {c}; \""),
            "-rf".to_string(),
            "--version".to_string(),
        ];
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        // The stub ignores its arguments, so any file that appears came from the script itself.
        for t in ["leanqc", "dblock"] {
            fs::write(bin.join(t), "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(bin.join(t), fs::Permissions::from_mode(0o755)).unwrap();
        }
        // a folder name can't hold the canary's absolute path; a relative `touch` would show up
        // in the created-files check below
        let work = root.join("w $(touch PWNED) `touch PWNED` '\n");
        fs::create_dir(&work).unwrap();
        let mut calls = Vec::new();
        for (i, e) in evil.iter().enumerate() {
            let mut ok = call(i as u32 * 3 + 1, Status::Ok);
            ok.cwd = work.to_string_lossy().into_owned();
            ok.argv = vec!["leanqc".into(), format!("--outdir={e}"), "--".into(), e.clone()];
            ok.version = Some(e.clone());
            ok.env = BTreeMap::from([("DBLOCK_STORE".to_string(), e.clone()), (format!("X;touch {c}"), e.clone())]);
            ok.outputs = vec![
                FileRecord { path: e.clone(), size: 1, sha256: Some(e.clone()), symlink_to: None, volatile: false },
                FileRecord { path: format!("{e}/l"), size: 0, sha256: None, symlink_to: Some(e.clone()), volatile: false },
            ];
            ok.stdout_is_output = true;
            ok.stdout_sha256 = Some(e.clone());
            let mut failed = call(i as u32 * 3 + 2, Status::Failed);
            failed.argv = ok.argv.clone();
            let mut rejected = call(i as u32 * 3 + 3, Status::Rejected);
            rejected.error = Some(e.clone());
            rejected.mcp_tool = e.clone();
            calls.extend([ok, failed, rejected]);
        }
        let mut fetch = call(100, Status::Ok);
        fetch.argv = vec!["dblock".into(), "fetch".into(), "--".into(), evil[0].clone()];
        fetch.cwd = work.to_string_lossy().into_owned();
        calls.push(fetch);
        let mut log = log_of(calls);
        log.run_id = evil[0].clone();
        let run_dir = root.join("run");
        fs::create_dir(&run_dir).unwrap();
        let script = run_dir.join("replay.sh");
        fs::write(&script, replay_script(&log)).unwrap();
        sh_syntax_ok(&script);

        let before = tree(&root);
        for fetch in ["0", "1"] {
            let out = std::process::Command::new("sh")
                .arg(&script)
                .env("IRIT_BIN_DIR", &bin)
                .env("IRIT_REPLAY_FETCH", fetch)
                .current_dir(&work)
                .output()
                .unwrap();
            // the checks fail (the hashes are nonsense), but nothing may have been executed
            assert!(!out.status.success());
            assert!(!canary.exists(), "replay.sh ran injected code:\n{}", String::from_utf8_lossy(&out.stderr));
        }
        let created: Vec<_> = tree(&root).difference(&before).cloned().collect();
        assert!(created.iter().all(|p| p.starts_with(run_dir.join("replay"))), "{created:?}");
    }

    /// With CDPATH set, `cd run` may jump to a same-named folder elsewhere and print its path,
    /// so RUN_DIR (and the replay/ outputs) would land in the wrong place.
    #[test]
    #[cfg(unix)]
    fn replay_ignores_cdpath() {
        let d = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(d.path()).unwrap();
        fs::create_dir_all(root.join("decoy/run")).unwrap();
        fs::create_dir(root.join("run")).unwrap();
        fs::write(root.join("run/replay.sh"), replay_script(&log_of(vec![]))).unwrap();
        let out = std::process::Command::new("sh")
            .arg("run/replay.sh")
            .env("CDPATH", root.join("decoy"))
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(root.join("run/replay").is_dir());
        assert!(!root.join("decoy/run/replay").exists());
    }

    #[test]
    fn fetch_is_only_replayed_on_request() {
        let mut fetch = call(1, Status::Ok);
        fetch.mcp_tool = "dblock_fetch".into();
        fetch.argv = vec!["dblock".into(), "fetch".into()];
        fetch.env = BTreeMap::from([("DBLOCK_STORE".to_string(), "/big/my store".to_string())]);
        fetch.outputs = vec![FileRecord { path: "/p/dblock.lock".into(), size: 1, sha256: Some("cc".into()), symlink_to: None, volatile: false }];
        let s = replay_script(&log_of(vec![fetch]));
        assert!(s.contains("# downloads databases again: step 001 0 irit dblock fetch\n"));
        assert!(s.contains("if fetch_wanted; then\n  step 001 0 irit dblock fetch\nelse\n"));
        assert!(s.contains("if fetch_wanted; then check cc /p/dblock.lock; fi\n"));
        assert!(s.contains("DBLOCK_STORE='/big/my store'; export DBLOCK_STORE\n# downloads"));
        assert!(!s.lines().any(|l| l.starts_with("step ")), "{s}");
    }

    #[test]
    fn external_calls_come_from_path() {
        let mut fastp = call(1, Status::Ok);
        fastp.mcp_tool = "fastp".into();
        fastp.external = true;
        fastp.argv = vec!["fastp".into(), "--in1=r1.fq".into(), "--json=s.fastp.json".into()];
        fastp.version = Some("fastp 1.4.0".into());
        let mut missing = call(2, Status::Failed);
        missing.external = true;
        missing.mcp_tool = "bakta".into();
        missing.argv = vec!["bakta".into(), "--".into(), "a.fna".into()];
        missing.exit_status = Some(0);
        missing.error = Some("exited 0 but did not write: /w/o/a.txt".into());
        let s = replay_script(&log_of(vec![fastp, missing, call(3, Status::Ok)]));
        assert!(s.contains("want_ext_version fastp 'fastp 1.4.0'\n"), "{s}");
        assert!(s.contains("want_version leanqc 'leanqc 0.1.0'\n"));
        assert!(s.contains("step 001 0 ext fastp --in1=r1.fq --json=s.fastp.json\n"));
        let mut logged = call(4, Status::Ok);
        logged.outputs[0].volatile = true;
        assert!(replay_script(&log_of(vec![logged])).contains("\ncheck_exists /data/x/qc/r.json\n"));
        assert!(s.contains("# not replayed: exited 0 in the run\n# exited 0 but did not write: /w/o/a.txt\n# ext bakta -- a.fna\n"), "{s}");
        let mut g = call(1, Status::Ok);
        g.external = true;
        g.argv = vec!["gtdbtk".into(), "classify_wf".into()];
        g.version = Some("gtdbtk: version 2.8.0 Copyright 2017 Pierre-Alain Chaumeil".into());
        let j = ro_crate(&log_of(vec![g]));
        let tool = j["@graph"].as_array().unwrap().iter().find(|e| e["@id"] == "#gtdbtk-2.8.0").unwrap().clone();
        assert_eq!(tool["url"], "https://github.com/Ecogenomics/GTDBTk");
    }

    /// replay.sh runs a third-party tool from PATH even with IRIT_BIN_DIR set, and compares only
    /// the first line of its --version.
    #[test]
    #[cfg(unix)]
    fn replay_runs_external_tools_from_path() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(d.path()).unwrap();
        let (path_dir, irit_dir) = (root.join("path"), root.join("irit"));
        fs::create_dir(&path_dir).unwrap();
        fs::create_dir(&irit_dir).unwrap();
        let tool = path_dir.join("fastp");
        let script = "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'fastp 1.4.0'; echo 'more'; exit 0; fi\necho ran > out.txt\n";
        fs::write(&tool, script).unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let mut c = call(1, Status::Ok);
        c.external = true;
        c.argv = vec!["fastp".into(), "--in1=x".into()];
        c.version = Some("fastp 1.4.0".into());
        c.cwd = root.to_string_lossy().into_owned();
        let out = root.join("out.txt");
        c.outputs = vec![FileRecord { path: out.to_string_lossy().into_owned(), size: 4, sha256: Some(hash_bytes(b"ran\n")), symlink_to: None, volatile: false }];
        fs::create_dir(root.join("run")).unwrap();
        fs::write(root.join("run/replay.sh"), replay_script(&log_of(vec![c]))).unwrap();
        let path = format!("{}:{}", path_dir.display(), std::env::var("PATH").unwrap_or_default());
        let r = std::process::Command::new("sh")
            .arg(root.join("run/replay.sh"))
            .env("PATH", path)
            .env("IRIT_BIN_DIR", &irit_dir)
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&r.stderr);
        assert!(r.status.success(), "{err}");
        assert!(!err.contains("warning"), "{err}");
        assert_eq!(fs::read_to_string(out).unwrap(), "ran\n");
    }

    #[test]
    fn unknown_versions_are_noted_once() {
        let mut a = call(1, Status::Ok);
        a.version = None;
        let mut b = call(2, Status::Ok);
        b.version = None;
        let s = replay_script(&log_of(vec![a, b]));
        assert!(!s.contains("\nwant_version "));
        assert_eq!(s.matches("version unknown").count(), 1, "{s}");
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_output_targets_are_followed() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real");
        fs::create_dir(&real).unwrap();
        let qc = d.path().join("qc");
        std::os::unix::fs::symlink(&real, &qc).unwrap();
        let before = snapshot(std::slice::from_ref(&qc));
        fs::write(qc.join("s_leanqc.json"), "{}").unwrap();
        let recs = output_records(&before, std::slice::from_ref(&qc));
        assert_eq!(recs.len(), 1, "{recs:?}");
        assert_eq!(recs[0].path, display(&qc.join("s_leanqc.json")));
        assert!(recs[0].sha256.is_some());

        // an output file that is a symlink to a real file is hashed through the link
        fs::write(d.path().join("real.csv"), "a").unwrap();
        let sheet = d.path().join("sheet.csv");
        std::os::unix::fs::symlink(d.path().join("real.csv"), &sheet).unwrap();
        let before = snapshot(std::slice::from_ref(&sheet));
        fs::write(&sheet, "b").unwrap();
        let recs = output_records(&before, std::slice::from_ref(&sheet));
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].sha256.as_deref(), Some(hash_bytes(b"b").as_str()));
        assert!(recs[0].symlink_to.is_none());
    }

    #[test]
    fn rewrite_with_same_size_and_mtime_is_detected() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("sheet.csv");
        fs::write(&f, "aaaa").unwrap();
        let mtime = fs::metadata(&f).unwrap().modified().unwrap();
        let before = snapshot(std::slice::from_ref(&f));
        fs::write(&f, "bbbb").unwrap();
        fs::File::options().write(true).open(&f).unwrap().set_modified(mtime).unwrap();
        assert_eq!(fs::metadata(&f).unwrap().modified().unwrap(), mtime);
        let recs = output_records(&before, std::slice::from_ref(&f));
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].sha256.as_deref(), Some(hash_bytes(b"bbbb").as_str()));
        // untouched: nothing reported
        let before = snapshot(std::slice::from_ref(&f));
        assert!(output_records(&before, std::slice::from_ref(&f)).is_empty());
    }

    #[test]
    fn crate_has_licence_description_and_logs() {
        let mut c = call(1, Status::Ok);
        c.stdout = Some("calls/001.stdout".into());
        c.stderr = Some("calls/001.stderr".into());
        let j = ro_crate(&log_of(vec![c]));
        assert_eq!(j["@context"][0], "https://w3id.org/ro/crate/1.3/context");
        let graph = j["@graph"].as_array().unwrap();
        let root = graph.iter().find(|e| e["@id"] == "./").unwrap();
        assert!(root["description"].as_str().unwrap().contains("1 tool call"));
        assert_eq!(root["license"]["@id"], DEFAULT_LICENSE);
        assert!(graph.iter().any(|e| e["@id"] == DEFAULT_LICENSE));
        let parts = root["hasPart"].as_array().unwrap();
        for p in ["run.json", "replay.sh", "calls/001.stdout", "calls/001.stderr"] {
            assert!(parts.iter().any(|x| x["@id"] == p), "{p}");
            assert!(graph.iter().any(|e| e["@id"] == p && e["@type"] == "File"), "{p}");
        }
        assert_eq!(root["conformsTo"]["@id"], "https://w3id.org/ro/wfrun/process/0.6");
    }
}
