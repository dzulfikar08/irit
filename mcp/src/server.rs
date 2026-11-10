//! The MCP server: one tool per irit capability, each call run as a subprocess and recorded.

use crate::external as x;
use crate::params::{self as p, Invocation};
use crate::record::{self, CallRecord, Run, RunLog, Status};
use crate::results;
use crate::validate::{self, Check, Ctx, Invalid};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const INSTRUCTIONS: &str = "Tools for sequencing-data housekeeping from the irit project: \
sheetgen (sample sheets from FASTQ folders, checked against a pipeline's schema), leanqc (read QC), \
isotable (one graded QC row per sample from tool outputs) and dblock_* (reference databases); \
plus the microbial isolate path with third-party tools: fastp (trimming), bakta (annotation), checkm2 \
(completeness/contamination) and gtdbtk_classify_wf (taxonomy). Those need the program installed \
(Bioconda) and, for the last three, a database (dblock_env shows dblock-managed paths); they can run for \
hours. Their results list `warnings` when the installed version isn't the one irit-mcp was checked against. \
Parameters are checked before anything runs; unknown parameters are refused, so read each tool's schema \
instead of guessing flags. A tool that exits non-zero returns isError with its stderr: report it, \
don't carry on as if it worked. Every call is recorded with input/output SHA-256s in a run folder whose \
replay.sh re-runs the same commands without an AI; describe_run shows it. Relative paths are resolved \
against the server's working directory. Output paths must be inside the server's root folder, and existing \
output files are only replaced when the call passes overwrite: true (ask the user first).";

/// Most of a tool's stderr is useful context for the agent, but cap it.
const STDERR_LIMIT: usize = 8000;
const ROW_LIMIT: usize = 1000;

pub struct Config {
    pub bin_dir: Option<PathBuf>,
    /// `--tool-path NAME=PATH`: where a third-party tool is, instead of searching PATH.
    pub tool_paths: HashMap<String, PathBuf>,
    pub runs_root: PathBuf,
    /// Working folder, write root and captured environment.
    pub ctx: Ctx,
    pub run_id: String,
}

struct State {
    run: Run,
    next_id: u32,
    versions: HashMap<PathBuf, Option<String>>,
}

/// Output targets of the calls running now. A call whose targets overlap one of them (same
/// path, or one inside the other) waits, so two calls never write the same files at once and
/// each call's output detection only sees its own writes.
#[derive(Default)]
struct Busy {
    active: Mutex<Vec<(u32, Vec<PathBuf>)>>,
    changed: tokio::sync::Notify,
}

struct Claim<'a> {
    busy: &'a Busy,
    id: u32,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.busy.active.lock().unwrap().retain(|(id, _)| *id != self.id);
        self.busy.changed.notify_waiters();
    }
}

fn overlaps(a: &[PathBuf], b: &[PathBuf]) -> bool {
    a.iter().any(|x| b.iter().any(|y| x.starts_with(y) || y.starts_with(x)))
}

impl Busy {
    async fn claim(&self, id: u32, targets: Vec<PathBuf>) -> Claim<'_> {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut active = self.active.lock().unwrap();
                if !active.iter().any(|(_, t)| overlaps(t, &targets)) {
                    active.push((id, targets));
                    return Claim { busy: self, id };
                }
            }
            notified.await;
        }
    }
}

#[derive(Clone)]
pub struct Irit {
    cfg: Arc<Config>,
    state: Arc<Mutex<State>>,
    busy: Arc<Busy>,
    tool_router: ToolRouter<Self>,
}

struct Done {
    record: CallRecord,
    stdout: Vec<u8>,
    stderr: String,
}

impl Done {
    fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

fn tail(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    let mut start = s.len() - limit;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("[...]{}", &s[start..])
}

/// The first non-empty line of `--version` (GTDB-Tk may add a "newer version available" line).
fn version_text(out: Option<std::process::Output>) -> Option<String> {
    let out = out.filter(|o| o.status.success())?;
    String::from_utf8_lossy(&out.stdout).lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

impl Irit {
    pub fn new(cfg: Config) -> Irit {
        let run = Run::new(&cfg.runs_root, &cfg.run_id);
        Irit {
            cfg: Arc::new(cfg),
            state: Arc::new(Mutex::new(State { run, next_id: 1, versions: HashMap::new() })),
            busy: Arc::new(Busy::default()),
            tool_router: Self::tool_router(),
        }
    }

    fn resolve(&self, tool: &str, external: bool) -> Option<PathBuf> {
        let exe = format!("{tool}{}", std::env::consts::EXE_SUFFIX);
        if external {
            return match self.cfg.tool_paths.get(tool) {
                Some(p) => Some(p.clone()).filter(|p| p.is_file()),
                None => find_on_path(&exe),
            };
        }
        match &self.cfg.bin_dir {
            Some(d) => Some(d.join(&exe)).filter(|p| p.is_file()),
            None => find_on_path(&exe),
        }
    }

    /// `program --version`, or None when it fails or prints nothing.
    async fn version(&self, program: &Path) -> Option<String> {
        if let Some(v) = self.state.lock().unwrap().versions.get(program) {
            return v.clone();
        }
        let out = tokio::process::Command::new(program).arg("--version").stdin(std::process::Stdio::null()).output().await;
        let v = version_text(out.ok());
        self.state.lock().unwrap().versions.insert(program.to_path_buf(), v.clone());
        v
    }

    fn save(&self, rec: CallRecord) {
        let mut st = self.state.lock().unwrap();
        if let Err(e) = st.run.add(rec) {
            // A record that can't be written must not go unnoticed, but the tool did run.
            eprintln!("irit-mcp: could not write run record in {}: {e}", st.run.dir.display());
        }
    }

    async fn execute<T: Serialize>(&self, mcp_tool: &str, params: &T, inv: Check<Invocation>) -> Done {
        let id = {
            let mut st = self.state.lock().unwrap();
            st.next_id += 1;
            st.next_id - 1
        };
        let mut rec = CallRecord {
            id,
            mcp_tool: mcp_tool.to_string(),
            status: Status::Rejected,
            params: serde_json::to_value(params).unwrap_or(Value::Null),
            program: None,
            argv: Vec::new(),
            version: None,
            cwd: self.cfg.ctx.cwd.to_string_lossy().into_owned(),
            started: record::now(),
            ended: String::new(),
            exit_status: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            stdout: None,
            stdout_sha256: None,
            stderr: None,
            stdout_is_output: false,
            env: Default::default(),
            external: false,
            error: None,
        };
        let finish = |mut rec: CallRecord, stdout: Vec<u8>, stderr: String| {
            rec.ended = record::now();
            self.save(rec.clone());
            Done { record: rec, stdout, stderr }
        };

        let inv = match inv {
            Ok(inv) => inv,
            Err(Invalid { field, message }) => {
                rec.error = Some(format!("{field}: {message}"));
                return finish(rec, Vec::new(), String::new());
            }
        };
        rec.argv = std::iter::once(inv.tool.to_string()).chain(inv.args.iter().cloned()).collect();
        rec.stdout_is_output = inv.stdout_is_output;
        rec.env = inv.env.clone();
        rec.external = inv.external;
        let Some(program) = self.resolve(inv.tool, inv.external) else {
            rec.status = Status::Error;
            rec.error = Some(match (&self.cfg.bin_dir, self.cfg.tool_paths.get(inv.tool)) {
                (_, Some(p)) if inv.external => format!("{} not found at {} (--tool-path)", inv.tool, p.display()),
                _ if inv.external => x::install_hint(inv.tool),
                (Some(d), _) => format!("{} not found in {}", inv.tool, d.display()),
                (None, _) => format!("{} not found on PATH; install irit or start irit-mcp with --bin-dir", inv.tool),
            });
            return finish(rec, Vec::new(), String::new());
        };
        rec.program = Some(program.to_string_lossy().into_owned());
        rec.version = self.version(&program).await;

        let targets: Vec<PathBuf> = inv.outputs.iter().map(|p| validate::resolve(p)).collect();
        let _claim = self.busy.claim(id, targets).await;
        let (inputs, outputs) = (inv.inputs.clone(), inv.outputs.clone());
        let (input_recs, before) = tokio::task::spawn_blocking(move || (record::input_records(&inputs), record::snapshot(&outputs)))
            .await
            .unwrap_or_default();
        rec.inputs = input_recs;

        let result = tokio::process::Command::new(&program)
            .args(&inv.args)
            .envs(&inv.env)
            .current_dir(&self.cfg.ctx.cwd)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output()
            .await;
        let out = match result {
            Ok(o) => o,
            Err(e) => {
                rec.status = Status::Error;
                rec.error = Some(format!("could not start {}: {e}", program.display()));
                return finish(rec, Vec::new(), String::new());
            }
        };
        let outputs = inv.outputs.clone();
        rec.outputs = tokio::task::spawn_blocking(move || record::output_records(&before, &outputs)).await.unwrap_or_default();
        for o in &mut rec.outputs {
            o.volatile = inv.volatile.iter().any(|end| o.path.ends_with(end));
        }
        rec.exit_status = out.status.code();
        rec.status = if out.status.success() { Status::Ok } else { Status::Failed };
        if !out.status.success() {
            rec.error = Some(match out.status.code() {
                Some(c) => format!("exit status {c}"),
                None => "killed by a signal".into(),
            });
        } else {
            // A zero exit without the files the tool always writes is not a result.
            let written = |p: &PathBuf| rec.outputs.iter().any(|o| Path::new(&o.path) == p);
            let missing: Vec<String> = inv
                .expect
                .iter()
                .filter(|group| !group.iter().any(written))
                .map(|g| g.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(" or "))
                .collect();
            if !missing.is_empty() {
                rec.status = Status::Failed;
                rec.error = Some(format!("exited 0 but did not write: {}", missing.join(", ")));
            }
        }

        let run_dir = {
            let st = self.state.lock().unwrap();
            let _ = st.run.ensure_dir();
            st.run.dir.clone()
        };
        let (so, se) = (format!("calls/{id:03}.stdout"), format!("calls/{id:03}.stderr"));
        if std::fs::write(run_dir.join(&so), &out.stdout).is_ok() {
            rec.stdout = Some(so);
        }
        if std::fs::write(run_dir.join(&se), &out.stderr).is_ok() {
            rec.stderr = Some(se);
        }
        rec.stdout_sha256 = Some(record::hash_bytes(&out.stdout));
        finish(rec, out.stdout, String::from_utf8_lossy(&out.stderr).into_owned())
    }

    /// Common envelope; `Err` carries an `isError` result for anything but a clean exit.
    // CallToolResult is rmcp's own error reply; these run once per tool call, so boxing it buys nothing.
    #[allow(clippy::result_large_err)]
    fn envelope(&self, d: &Done) -> Result<Value, CallToolResult> {
        let r = &d.record;
        let mut v = json!({"call_id": r.id, "run_id": self.cfg.run_id});
        if !r.argv.is_empty() {
            v["command"] = json!(r.argv.iter().map(|a| record::sh_quote(a)).collect::<Vec<_>>().join(" "));
        }
        match r.status {
            Status::Ok => {
                v["exit_status"] = json!(r.exit_status);
                v["outputs"] = json!(r.outputs);
                v["stderr"] = json!(tail(&d.stderr, STDERR_LIMIT));
                Ok(v)
            }
            Status::Rejected => {
                let (field, message) = r.error.as_deref().unwrap_or("").split_once(": ").unwrap_or(("", ""));
                v["error"] = json!("invalid_parameter");
                v["field"] = json!(field);
                v["message"] = json!(message);
                v["note"] = json!("nothing was run");
                Err(CallToolResult::structured_error(v))
            }
            Status::Error => {
                v["error"] = json!("tool_unavailable");
                v["message"] = json!(r.error);
                Err(CallToolResult::structured_error(v))
            }
            Status::Failed => {
                if r.exit_status == Some(0) {
                    v["error"] = json!("missing_outputs");
                    v["message"] = json!(r.error);
                } else {
                    v["error"] = json!("tool_failed");
                }
                v["exit_status"] = json!(r.exit_status);
                v["stderr"] = json!(tail(&d.stderr, STDERR_LIMIT));
                v["stdout"] = json!(tail(&d.stdout_text(), STDERR_LIMIT));
                v["note"] = json!("the tool refused or failed; its outputs must not be used");
                Err(CallToolResult::structured_error(v))
            }
        }
    }

    fn reply(&self, d: Done, add: impl FnOnce(&Done, &mut Value)) -> Result<CallToolResult, ErrorData> {
        Ok(match self.envelope(&d) {
            Ok(mut v) => {
                add(&d, &mut v);
                CallToolResult::structured(v)
            }
            Err(e) => e,
        })
    }

    /// The table a successful call produced: its output file, else its stdout.
    // CallToolResult is rmcp's own error reply; these run once per tool call, so boxing it buys nothing.
    #[allow(clippy::result_large_err)]
    fn table_text(&self, d: &Done, output: &Option<String>) -> Result<String, CallToolResult> {
        match output {
            Some(o) => std::fs::read_to_string(self.cfg.ctx.cwd.join(o)).map_err(|e| {
                CallToolResult::structured_error(json!({
                    "call_id": d.record.id,
                    "run_id": self.cfg.run_id,
                    "error": "output_unreadable",
                    "exit_status": d.record.exit_status,
                    "message": format!("the tool exited 0 but {o} could not be read: {e}"),
                    "note": "do not report this as an empty result",
                }))
            }),
            None => Ok(d.stdout_text()),
        }
    }

    /// Result of a third-party tool: the envelope, the version check, and `summarize`'s fields.
    /// A summary that can't be read from outputs the tool did write is an error, not an empty result.
    fn external_reply(&self, d: Done, summarize: impl FnOnce(&mut Value) -> Result<(), String>) -> Result<CallToolResult, ErrorData> {
        let mut v = match self.envelope(&d) {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let r = &d.record;
        let tool = r.argv.first().map(String::as_str).unwrap_or("");
        v["version"] = json!(r.version);
        if let Some(p) = x::pinned(tool) {
            v["checked_against"] = json!(format!("{tool} {}", p.version));
        }
        v["warnings"] = json!(x::version_warning(tool, r.version.as_deref()).into_iter().collect::<Vec<_>>());
        if let Err(e) = summarize(&mut v) {
            v["error"] = json!("output_unreadable");
            v["message"] = json!(format!("{tool} exited 0 but its output could not be read: {e}"));
            v["note"] = json!("do not report this as an empty result");
            return Ok(CallToolResult::structured_error(v));
        }
        Ok(CallToolResult::structured(v))
    }

    fn read_log(&self, run_id: Option<&str>) -> Result<RunLog, String> {
        let st = self.state.lock().unwrap();
        match run_id {
            None => Ok(st.run.log.clone()),
            Some(id) if id == st.run.log.run_id => Ok(st.run.log.clone()),
            Some(id) => {
                if id.is_empty() || id.starts_with('.') || id.contains(['/', '\\']) {
                    return Err(format!("{id:?} is not a run id"));
                }
                let path = self.cfg.runs_root.join(id).join("run.json");
                let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
            }
        }
    }
}

#[tool_router]
impl Irit {
    #[tool(description = "Build a pipeline sample sheet (CSV/TSV) from folders of FASTQ files. With `pipeline` or `schema` it uses that \
nf-core pipeline's own columns and validates every row against the pipeline's input schema, refusing values the pipeline would reject \
(the stderr then names the row, column and a hint such as `set` or `link_into`). Returns the sheet's columns and rows.")]
    async fn sheetgen(&self, Parameters(params): Parameters<p::SheetgenParams>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("sheetgen", &params, p::sheetgen(&self.cfg.ctx, &params)).await;
        let text = if d.record.status == Status::Ok {
            match self.table_text(&d, &params.output) {
                Ok(t) => t,
                Err(e) => return Ok(e),
            }
        } else {
            String::new()
        };
        self.reply(d, |_, v| {
            let (header, rows) = results::parse_table(&text);
            v["output"] = json!(params.output);
            v["columns"] = json!(header);
            v["row_count"] = json!(rows.len());
            v["rows"] = json!(results::table_objects(&header, &rows[..rows.len().min(ROW_LIMIT)]));
            v["truncated"] = json!(rows.len() > ROW_LIMIT);
        })
    }

    #[tool(description = "Read quality control (FastQC-equivalent checks, FastQC thresholds) on FASTQ/FASTA files. Writes an HTML and JSON \
report per file plus leanqc_summary.tsv into `outdir`. Returns per-file verdict (pass/warn/fail), the failed and warned checks with \
details, reads, GC and Q30, and the report paths.")]
    async fn leanqc(&self, Parameters(params): Parameters<p::LeanqcParams>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("leanqc", &params, p::leanqc(&self.cfg.ctx, &params)).await;
        if d.record.status == Status::Ok && results::leanqc_reports(&d.record.outputs).is_empty() {
            let mut v = self.envelope(&d).unwrap_or_else(|_| json!({}));
            v["error"] = json!("no_reports");
            v["message"] = json!("leanqc exited 0 but no *_leanqc.json report was found (or changed) in outdir");
            v["note"] = json!("there is no QC result to report; check outdir and stderr");
            return Ok(CallToolResult::structured_error(v));
        }
        self.reply(d, |d, v| {
            let reports = results::leanqc_reports(&d.record.outputs);
            let count = |s: &str| reports.iter().filter(|r| r["verdict"] == s).count();
            v["summary"] = json!({"files": reports.len(), "pass": count("pass"), "warn": count("warn"), "fail": count("fail")});
            v["reports"] = json!(reports);
            v.as_object_mut().unwrap().remove("outputs"); // the reports already list the paths
        })
    }

    #[tool(description = "Join QC, assembly and typing outputs (fastp, leanqc, QUAST, BUSCO, CheckM2, GTDB-Tk, Kraken 2, mlst, AMRFinderPlus) \
found under results folders into one graded row per sample. Status, worst first: conflict (files disagree), fail, incomplete (a required \
measurement is missing), warn, pass. Returns each sample's status, reasons and values, and counts per status. Thresholds are starting \
points, not a standard: say so when reporting grades.")]
    async fn isotable(&self, Parameters(params): Parameters<p::IsotableParams>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("isotable", &params, p::isotable(&self.cfg.ctx, &params)).await;
        let text = if d.record.status == Status::Ok {
            match self.table_text(&d, &params.output) {
                Ok(t) => t,
                Err(e) => return Ok(e),
            }
        } else {
            String::new()
        };
        self.reply(d, |_, v| {
            let t = results::isotable_rows(&text);
            v["counts"] = t["counts"].clone();
            v["columns"] = t["columns"].clone();
            v["samples"] = t["samples"].clone();
        })
    }

    #[tool(description = "List the reference databases dblock can fetch (GTDB-Tk, CheckM2, SILVA, Kraken 2, Bakta, CARD) with download \
size, size on disk and licence notes. Read-only.")]
    async fn dblock_catalog(&self, Parameters(params): Parameters<p::DblockCatalogParams>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("dblock_catalog", &params, p::dblock_catalog(&self.cfg.ctx, &params)).await;
        self.reply(d, |d, v| v["databases"] = json!(results::catalog_rows(&d.stdout_text())))
    }

    #[tool(description = "Add catalog databases to the project's dblock.toml (creates it). Downloads nothing.")]
    async fn dblock_add(&self, Parameters(params): Parameters<p::DblockAddParams>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("dblock_add", &params, p::dblock_add(&self.cfg.ctx, &params)).await;
        self.reply(d, |d, v| v["stdout"] = json!(d.stdout_text()))
    }

    #[tool(description = "Show whether the project's databases fit on disk and how they would be fetched. Downloads nothing and doesn't \
change the project, but creates the (empty) store folder if it doesn't exist yet, to measure its free space. Run this and show \
the user the sizes before dblock_fetch.")]
    async fn dblock_plan(&self, Parameters(params): Parameters<p::DblockProject>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("dblock_plan", &params, p::dblock_simple(&self.cfg.ctx, "plan", &params)).await;
        self.reply(d, |d, v| v["plan"] = json!(d.stdout_text()))
    }

    #[tool(description = "Download, verify and unpack the project's databases and update dblock.lock. Can download tens of GB and take \
hours. Requires confirm: true, which you may only set after showing the user dblock_plan's output and getting their agreement.")]
    async fn dblock_fetch(&self, Parameters(params): Parameters<p::DblockFetchParams>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("dblock_fetch", &params, p::dblock_fetch(&self.cfg.ctx, &params)).await;
        self.reply(d, |d, v| v["stdout"] = json!(d.stdout_text()))
    }

    #[tool(description = "Check that the locked databases are still on disk unchanged: names and sizes (seconds), or every file's \
SHA-256 with deep: true (minutes to hours). Read-only.")]
    async fn dblock_verify(&self, Parameters(params): Parameters<p::DblockVerifyParams>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("dblock_verify", &params, p::dblock_verify(&self.cfg.ctx, &params)).await;
        self.reply(d, |d, v| v["report"] = json!(d.stdout_text()))
    }

    #[tool(description = "Check installed tool versions (GTDB-Tk, CheckM2, Bakta, ...) against the locked databases. Read-only.")]
    async fn dblock_check(&self, Parameters(params): Parameters<p::DblockProject>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("dblock_check", &params, p::dblock_simple(&self.cfg.ctx, "check", &params)).await;
        self.reply(d, |d, v| v["report"] = json!(d.stdout_text()))
    }

    #[tool(description = "Environment variables (GTDBTK_DATA_PATH, CHECKM2DB, BAKTA_DB, ...) pointing at the locked databases, as a \
map and as shell export lines. Read-only.")]
    async fn dblock_env(&self, Parameters(params): Parameters<p::DblockProject>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("dblock_env", &params, p::dblock_simple(&self.cfg.ctx, "env", &params)).await;
        self.reply(d, |d, v| {
            let text = d.stdout_text();
            v["env"] = json!(results::env_exports(&text));
            v["shell"] = json!(text);
        })
    }

    #[tool(description = "fastp 1.4.0: trim and filter Illumina reads (single or paired-end) and write a JSON and HTML report. \
Adapters are auto-detected unless set. Returns the report paths and fastp's totals: reads before and after filtering, bases, \
Q30 % and duplication % (read by isotable's own fastp parser). Name `json` `<sample>.fastp.json` so isotable can join it.")]
    async fn fastp(&self, Parameters(params): Parameters<x::FastpParams>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("fastp", &params, x::fastp(&self.cfg.ctx, &params)).await;
        let json_path = self.cfg.ctx.cwd.join(&params.json);
        self.external_reply(d, |v| {
            v["report"] = json!({"json": params.json, "html": x::fastp_html(&params)});
            v["summary"] = results::fastp_summary(&json_path)?;
            Ok(())
        })
    }

    #[tool(description = "Bakta 1.12.2: annotate a bacterial genome assembly (FASTA). Needs a Bakta v6 database (`db`, or $BAKTA_DB); \
its version.json and files are checked first. Writes GFF3, GenBank, EMBL, TSV, JSON and FASTA files named `<prefix>.*` into \
`outdir`. Takes minutes to an hour. Returns the counts from Bakta's summary (<prefix>.txt): sequence stats and features (CDSs, \
tRNAs, rRNAs, hypotheticals, ...).")]
    async fn bakta(&self, Parameters(params): Parameters<x::BaktaParams>) -> Result<CallToolResult, ErrorData> {
        let inv = x::bakta(&self.cfg.ctx, &params);
        let txt = inv.as_ref().ok().and_then(|i| i.expect.first()?.first().cloned());
        let d = self.execute("bakta", &params, inv).await;
        self.external_reply(d, |v| {
            let txt = txt.ok_or("no summary path")?;
            let text = std::fs::read_to_string(&txt).map_err(|e| format!("{}: {e}", txt.display()))?;
            v["summary_file"] = json!(txt);
            v["summary"] = results::bakta_summary(&text);
            Ok(())
        })
    }

    #[tool(description = "CheckM2 1.1.0 predict: completeness and contamination of genomes or MAGs (FASTA). Needs the CheckM2 \
DIAMOND database (`db`, or $CHECKM2DB). Set lowmem on 8-16 GB machines. With overwrite: true, CheckM2 empties `outdir` first. \
Returns per genome: completeness and contamination (%), read from quality_report.tsv by isotable's parser.")]
    async fn checkm2(&self, Parameters(params): Parameters<x::Checkm2Params>) -> Result<CallToolResult, ErrorData> {
        let d = self.execute("checkm2", &params, x::checkm2(&self.cfg.ctx, &params)).await;
        let report = self.cfg.ctx.cwd.join(&params.outdir).join("quality_report.tsv");
        self.external_reply(d, |v| {
            v["genomes"] = json!(results::checkm2_summary(&report)?);
            v["report"] = json!(report);
            Ok(())
        })
    }

    #[tool(description = "GTDB-Tk 2.8.0 classify_wf: GTDB taxonomy for the genomes in a folder (skani ANI screen, then tree placement \
for genomes without a species match). Needs the full GTDB-Tk reference data (`data_path`, or $GTDBTK_DATA_PATH), including skani/. \
Placement can need over 100 GB of RAM; on a laptop set pplacer_cpus 1 and scratch_dir. GTDB-Tk 2.8.0 has no option to skip the ANI \
screen and no memory cap of its own. Returns per genome: classification, method, closest reference and ANI, and warnings, from \
classify/<prefix>.{bac120,ar53}.summary.tsv.")]
    async fn gtdbtk_classify_wf(&self, Parameters(params): Parameters<x::GtdbtkClassifyWfParams>) -> Result<CallToolResult, ErrorData> {
        let inv = x::gtdbtk_classify_wf(&self.cfg.ctx, &params);
        let summaries = inv.as_ref().map(|i| i.expect.concat()).unwrap_or_default();
        let d = self.execute("gtdbtk_classify_wf", &params, inv).await;
        // Only the summaries this call wrote: a stale ar53 file from an earlier run is not a result.
        let written: Vec<PathBuf> =
            summaries.into_iter().filter(|s| d.record.outputs.iter().any(|o| Path::new(&o.path) == s)).collect();
        self.external_reply(d, |v| {
            let mut genomes = Vec::new();
            for s in &written {
                genomes.extend(results::gtdbtk_summary(s)?);
            }
            v["summary_files"] = json!(written);
            v["genomes"] = json!(genomes);
            Ok(())
        })
    }

    #[tool(description = "Show a run record: every call with its exact command, tool version, working directory, input and output \
files with SHA-256, times and exit status, plus the paths of replay.sh (re-runs the commands without an AI) and the RO-Crate.")]
    async fn describe_run(&self, Parameters(params): Parameters<p::DescribeRunParams>) -> Result<CallToolResult, ErrorData> {
        match self.read_log(params.run_id.as_deref()) {
            Ok(log) => {
                let dir = self.cfg.runs_root.join(&log.run_id);
                Ok(CallToolResult::structured(json!({
                    "run_dir": dir,
                    "replay_script": dir.join("replay.sh"),
                    "ro_crate": dir.join("ro-crate-metadata.json"),
                    "current": log.run_id == self.cfg.run_id,
                    "run": log,
                })))
            }
            Err(e) => Ok(CallToolResult::structured_error(json!({"error": "run_not_found", "message": e}))),
        }
    }

    #[tool(description = "List recorded runs (one per server session) with their number of calls by status.")]
    async fn list_runs(&self, Parameters(_): Parameters<p::ListRunsParams>) -> Result<CallToolResult, ErrorData> {
        let mut runs = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.cfg.runs_root) {
            for e in rd.flatten() {
                let Ok(bytes) = std::fs::read(e.path().join("run.json")) else { continue };
                let Ok(log) = serde_json::from_slice::<RunLog>(&bytes) else { continue };
                let count = |s: Status| log.calls.iter().filter(|c| c.status == s).count();
                runs.push(json!({
                    "run_id": log.run_id,
                    "created": log.created,
                    "calls": log.calls.len(),
                    "ok": count(Status::Ok),
                    "failed": count(Status::Failed),
                    "rejected": count(Status::Rejected) + count(Status::Error),
                    "tools": log.calls.iter().map(|c| c.mcp_tool.as_str()).collect::<std::collections::BTreeSet<_>>(),
                }));
            }
        }
        runs.sort_by(|a, b| b["created"].as_str().cmp(&a["created"].as_str()));
        Ok(CallToolResult::structured(json!({
            "runs_dir": self.cfg.runs_root,
            "current_run": self.cfg.run_id,
            "runs": runs,
        })))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Irit {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("irit-mcp", record::IRIT_MCP_VERSION))
            .with_instructions(INSTRUCTIONS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn failed_version_is_none() {
        use std::os::unix::process::ExitStatusExt;
        let out = |code: i32, stdout: &str| std::process::Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        assert_eq!(version_text(Some(out(0, "leanqc 0.1.0\n"))), Some("leanqc 0.1.0".into()));
        assert_eq!(version_text(Some(out(0, "  \n"))), None);
        assert_eq!(version_text(Some(out(2, "leanqc 0.1.0"))), None);
        assert_eq!(version_text(None), None);
    }

    #[test]
    fn overlapping_targets() {
        let p = |s: &str| PathBuf::from(s);
        assert!(overlaps(&[p("/w/qc")], &[p("/w/qc/a.json")]));
        assert!(overlaps(&[p("/w/qc/a.json")], &[p("/w/qc")]));
        assert!(overlaps(&[p("/w/a.csv")], &[p("/w/a.csv")]));
        assert!(!overlaps(&[p("/w/qc")], &[p("/w/qc2")]));
        assert!(!overlaps(&[], &[p("/w")]));
    }

    #[tokio::test]
    async fn overlapping_calls_wait_for_each_other() {
        let busy = Arc::new(Busy::default());
        let first = busy.claim(1, vec![PathBuf::from("/w/qc")]).await;
        // a different target is not held up
        drop(busy.claim(2, vec![PathBuf::from("/w/other")]).await);
        let b = busy.clone();
        let waiter = tokio::spawn(async move {
            let _c = b.claim(3, vec![PathBuf::from("/w/qc/x.json")]).await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "overlapping call ran concurrently");
        drop(first);
        tokio::time::timeout(Duration::from_secs(5), waiter).await.unwrap().unwrap();
        assert!(busy.active.lock().unwrap().is_empty());
    }
}
