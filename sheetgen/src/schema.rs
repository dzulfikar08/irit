//! Reading an nf-core `schema_input.json` (JSON Schema for the sample sheet), filling it, and
//! checking the result the way nf-schema will.
//!
//! Rows are checked with the `jsonschema` crate (draft 2020-12, ECMA-style patterns with
//! look-around via fancy-regex). Like nf-schema, blank cells are left out of the row object and
//! values are cast to the column's declared type (`"1"` becomes `1` for an integer column) before
//! validation. nf-schema's own keywords (`uniqueEntries`, `unique`, `exists`) are checked here.
//! A keyword that neither does is listed, and stops the run unless `--allow-unchecked`.

use crate::{Unit, clean_name};
use anyhow::{Context, Result, bail};
use jsonschema::PatternOptions;
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// What a column holds, guessed from its name and `meta` tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Sample,
    R1,
    R2,
    Long,
    SingleEnd,
    /// Run/lane number within a sample (mag `run`, taxprofiler `run_accession`).
    Run,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
enum Unique {
    No,
    /// Value appears in one row only.
    All,
    /// Value appears once among rows that share these columns (mag: `run` within `sample`).
    Within(Vec<String>),
}

#[derive(Debug)]
struct Column {
    name: String,
    role: Role,
    required: bool,
    /// One of the column's patterns lists `NA` as an explicit alternative, as bacass's
    /// `^(...|NA)$` does: blanks are written as `NA`.
    na_blank: bool,
    /// JSON types the column declares (in `type` or its `anyOf`/`oneOf` branches).
    types: BTreeSet<String>,
    choices: Option<Vec<String>>,
    message: Option<String>,
    /// nf-schema `exists: true`, with the expected kind from `format`.
    exists: Option<PathKind>,
    unique: Unique,
}

#[derive(Debug, Clone, Copy)]
enum PathKind {
    File,
    Dir,
    Any,
}

pub struct Schema {
    columns: Vec<Column>,
    /// `None` when the schema couldn't be compiled and the user accepted that.
    validator: Option<jsonschema::Validator>,
    /// nf-schema `uniqueEntries` lists (top level, or in a top-level `allOf`).
    unique_entries: Vec<Vec<String>>,
    /// Columns that other filled columns make required (`dependentRequired` targets).
    dependent: BTreeSet<String>,
    /// Constraints nobody evaluates (only kept with --allow-unchecked).
    pub unchecked: Vec<String>,
}

/// Keywords the `jsonschema` crate evaluates, or that only annotate.
const KNOWN: &[&str] = &[
    // validation (jsonschema crate)
    "type", "enum", "const", "pattern", "minLength", "maxLength", "minimum", "maximum",
    "exclusiveMinimum", "exclusiveMaximum", "multipleOf", "required", "dependentRequired",
    "minItems", "maxItems", "uniqueItems", "minContains", "maxContains", "minProperties",
    "maxProperties", "$ref",
    // annotations
    "$schema", "$id", "$comment", "$anchor", "title", "description", "default", "examples",
    "deprecated", "readOnly", "writeOnly", "contentMediaType", "contentEncoding",
    // nf-schema annotations (shown to users, not validated by nf-schema either)
    "meta", "errorMessage", "help_text", "fa_icon", "hidden", "mimetype",
    // nf-schema keywords sheetgen checks itself
    "uniqueEntries", "unique", "exists",
];

/// `format` values that are checked (by `jsonschema` or, for paths, with `exists`).
const KNOWN_FORMATS: &[&str] = &[
    "file-path", "directory-path", "path", "date", "date-time", "time", "duration", "email",
    "uri", "uri-reference", "uuid", "ipv4", "ipv6", "hostname", "regex",
];

impl Schema {
    pub fn load(path: &Path, allow_unchecked: bool) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let json: Value = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Self::from_json(json, &path.display().to_string(), allow_unchecked)
    }

    fn from_json(json: Value, name: &str, allow_unchecked: bool) -> Result<Self> {
        let items = json.get("items").unwrap_or(&json);
        let Some(props) = items.get("properties").and_then(Value::as_object) else {
            bail!(
                "{name} does not describe a table with columns (no items.properties); \
                 pipelines such as nf-core/fetchngs take a plain list of accessions, which sheetgen doesn't write"
            );
        };
        let required: Vec<&str> = str_list(items.get("required"));

        let mut unchecked = Vec::new();
        audit(&json, "", &mut unchecked);
        // uniqueEntries is only understood at the array level (top level or a top-level allOf).
        let mut unique_entries = Vec::new();
        let top_level = std::iter::once(&json).chain(json.get("allOf").and_then(Value::as_array).into_iter().flatten());
        for s in top_level {
            if let Some(cols) = s.get("uniqueEntries") {
                unique_entries.push(str_list(Some(cols)).into_iter().map(String::from).collect());
            }
        }
        if items.get("uniqueEntries").is_some() {
            unchecked.push("items: `uniqueEntries` inside a row (nf-schema expects it on the array)".into());
        }

        let mut columns = Vec::new();
        for (col, prop) in props {
            let branches: Vec<&Value> = std::iter::once(prop)
                .chain(["anyOf", "oneOf", "allOf"].iter().filter_map(|k| prop.get(*k)).filter_map(Value::as_array).flatten())
                .collect();
            let mut types = BTreeSet::new();
            let mut na_blank = false;
            let mut exists = None;
            let mut format = None;
            for b in &branches {
                match b.get("type") {
                    Some(Value::String(t)) => {
                        types.insert(t.clone());
                    }
                    Some(Value::Array(ts)) => types.extend(ts.iter().filter_map(Value::as_str).map(String::from)),
                    _ => {}
                }
                if let Some(p) = b.get("pattern").and_then(Value::as_str) {
                    na_blank |= NA_ALTERNATIVE.is_match(p);
                }
                if b.get("exists") == Some(&Value::Bool(true)) {
                    exists = Some(PathKind::Any);
                }
                format = format.or(b.get("format").and_then(Value::as_str));
            }
            if exists.is_some() {
                exists = Some(match format {
                    Some("file-path") => PathKind::File,
                    Some("directory-path") => PathKind::Dir,
                    _ => PathKind::Any,
                });
            }
            let unique = match prop.get("unique") {
                Some(Value::Bool(true)) => Unique::All,
                Some(Value::Array(a)) => Unique::Within(a.iter().filter_map(Value::as_str).map(String::from).collect()),
                _ => Unique::No,
            };
            let meta: Vec<&str> = match prop.get("meta") {
                Some(Value::String(s)) => vec![s.as_str()],
                m => str_list(m),
            };
            columns.push(Column {
                role: role_of(col, &meta),
                required: required.contains(&col.as_str()),
                name: col.clone(),
                na_blank,
                types,
                choices: prop.get("enum").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()),
                message: prop.get("errorMessage").and_then(Value::as_str).map(String::from),
                exists,
                unique,
            });
        }
        if !columns.iter().any(|c| c.role == Role::Sample) {
            bail!("couldn't tell which column is the sample name in {name}");
        }
        let dependent = items
            .get("dependentRequired")
            .and_then(Value::as_object)
            .map(|m| m.values().flat_map(|v| str_list(Some(v))).map(String::from).collect())
            .unwrap_or_default();

        let validator = match jsonschema::options()
            .with_pattern_options(PatternOptions::fancy_regex())
            .should_validate_formats(true)
            .should_ignore_unknown_formats(true)
            .build(&json)
        {
            Ok(v) => Some(v),
            Err(e) => {
                unchecked.push(format!("the whole schema: it doesn't compile ({e})"));
                None
            }
        };
        if !unchecked.is_empty() && !allow_unchecked {
            bail!(
                "{name} has rules sheetgen can't check, so a sheet that passes here could still fail in the pipeline:\n  {}\n\
                 rerun with --allow-unchecked to write the sheet anyway (those rules are then left to the pipeline)",
                unchecked.join("\n  ")
            );
        }
        Ok(Schema { columns, validator, unique_entries, dependent, unchecked })
    }

    pub fn has_long_column(&self) -> bool {
        self.columns.iter().any(|c| c.role == Role::Long)
    }

    /// A run/lane column tells several rows of one sample apart (mag `run`).
    pub fn has_run_column(&self) -> bool {
        self.columns.iter().any(|c| c.role == Role::Run)
    }

    pub fn build(
        &self,
        units: &[Unit],
        fixed: &BTreeMap<String, String>,
        fmt: &dyn Fn(&Path) -> String,
        warnings: &mut Vec<String>,
    ) -> Result<(Vec<String>, Vec<Vec<String>>)> {
        for key in fixed.keys() {
            if !self.columns.iter().any(|c| &c.name == key) {
                let names: Vec<_> = self.columns.iter().map(|c| c.name.as_str()).collect();
                bail!("--set {key}: the schema has no such column (columns: {})", names.join(", "));
            }
        }
        let has_long_col = self.has_long_column();
        if !has_long_col && let Some(u) = units.iter().find(|u| u.long.is_some() && u.r1.is_some()) {
            bail!("sample `{}` has short and long reads, but this schema has no long-read column", u.sample);
        }
        for u in &self.unchecked {
            warnings.push(format!("not checked: {u}"));
        }

        let mut rows = Vec::new();
        let mut objects = Vec::new();
        let mut run_no: HashMap<&str, usize> = HashMap::new();
        for u in units {
            let run = run_no.entry(&u.sample).or_default();
            *run += 1;
            let mut row = Vec::new();
            let mut obj = Map::new();
            for c in &self.columns {
                let path = |p: &Option<PathBuf>| p.as_ref().map(|p| fmt(p));
                let value = if let Some(v) = fixed.get(&c.name) {
                    Some(v.clone())
                } else {
                    match c.role {
                        Role::Sample => Some(clean_name(&u.sample)),
                        Role::R1 if !has_long_col => path(&u.r1).or_else(|| path(&u.long)),
                        Role::R1 => path(&u.r1),
                        Role::R2 => path(&u.r2),
                        Role::Long => path(&u.long),
                        Role::SingleEnd => Some((u.r2.is_none()).to_string()),
                        Role::Run => Some(run.to_string()),
                        Role::Other => None,
                    }
                };
                let value = match value.filter(|v| !v.is_empty()) {
                    Some(v) => v,
                    None if c.na_blank => "NA".into(),
                    None => String::new(),
                };
                if !value.is_empty() {
                    obj.insert(c.name.clone(), cast(&value, &c.types));
                }
                row.push(value);
            }
            rows.push(row);
            objects.push(obj);
        }

        let problems = self.problems(units, &objects);
        if !problems.is_empty() {
            let missing: Vec<_> = self
                .columns
                .iter()
                .filter(|c| c.role == Role::Other && (c.required || self.dependent.contains(&c.name)) && !fixed.contains_key(&c.name))
                .map(|c| match &c.choices {
                    Some(ch) => format!("--set {}=<{}>", c.name, ch.join("|")),
                    None => format!("--set {}=<value>", c.name),
                })
                .collect();
            let mut hint = if missing.is_empty() {
                String::new()
            } else {
                format!("\nhint: this pipeline may need {}", missing.join(" "))
            };
            if problems.iter().any(|p| p.contains('/') && p.contains("space")) {
                hint.push_str("\nhint: --link-into <dir> makes space-free symlinks and lists those instead");
            }
            let shown: Vec<_> = problems.iter().take(10).cloned().collect();
            let more = if problems.len() > 10 { format!("\n... and {} more", problems.len() - 10) } else { String::new() };
            bail!("sample sheet would fail the pipeline's own checks:\n{}{more}{hint}", shown.join("\n"));
        }
        let header = self.columns.iter().map(|c| c.name.clone()).collect();
        Ok((header, rows))
    }

    /// Every rule the rows break, as `row N (sample), column C: why`.
    fn problems(&self, units: &[Unit], objects: &[Map<String, Value>]) -> Vec<String> {
        let at = |i: usize| format!("row {} ({})", i + 1, units[i].sample);
        let mut out = Vec::new();
        let instance = Value::Array(objects.iter().cloned().map(Value::Object).collect());
        if let Some(v) = &self.validator {
            let mut seen = BTreeSet::new();
            for e in v.iter_errors(&instance) {
                let mut parts = e.instance_path().as_str().split('/').skip(1).map(|s| s.replace("~1", "/").replace("~0", "~"));
                let row: Option<usize> = parts.next().and_then(|s| s.parse().ok());
                let col = parts.next();
                let msg = match (row, &col) {
                    (Some(i), Some(col)) => {
                        let value = objects[i].get(col).map(show).unwrap_or_default();
                        let why = match self.columns.iter().find(|c| &c.name == col).and_then(|c| c.message.as_ref()) {
                            Some(m) => m.clone(),
                            None => short(&e.to_string()),
                        };
                        format!("{}, column {col}: `{value}`: {why}", at(i))
                    }
                    (Some(i), None) => format!("{}: {}", at(i), row_message(&e.to_string(), &objects[i])),
                    _ => format!("sheet: {}", short(&e.to_string())),
                };
                // One schema rule can fail through several anyOf branches; report it once.
                if seen.insert(msg.clone()) {
                    out.push(msg);
                }
            }
        }
        for cols in &self.unique_entries {
            let mut first: HashMap<Vec<Option<&Value>>, usize> = HashMap::new();
            for (i, o) in objects.iter().enumerate() {
                let key: Vec<Option<&Value>> = cols.iter().map(|c| o.get(c)).collect();
                if key.iter().all(Option::is_none) {
                    continue;
                }
                if let Some(j) = first.insert(key, i) {
                    out.push(format!(
                        "{}: same {} as row {} (uniqueEntries: each combination may appear once)",
                        at(i),
                        cols.join(" + "),
                        j + 1
                    ));
                }
            }
        }
        for c in &self.columns {
            let group = match &c.unique {
                Unique::No => continue,
                Unique::All => Vec::new(),
                Unique::Within(g) => g.clone(),
            };
            let mut first: HashMap<(Vec<Option<&Value>>, &Value), usize> = HashMap::new();
            for (i, o) in objects.iter().enumerate() {
                let Some(v) = o.get(&c.name) else { continue };
                let key = (group.iter().map(|g| o.get(g)).collect(), v);
                if let Some(j) = first.insert(key, i) {
                    let scope = if group.is_empty() { String::new() } else { format!(" for the same {}", group.join(" + ")) };
                    out.push(format!("{}, column {}: `{}` already used in row {}{scope}", at(i), c.name, show(v), j + 1));
                }
            }
        }
        for c in &self.columns {
            let Some(kind) = c.exists else { continue };
            for (i, o) in objects.iter().enumerate() {
                let Some(Value::String(p)) = o.get(&c.name) else { continue };
                if p == "NA" && c.na_blank {
                    continue;
                }
                let path = Path::new(p);
                let ok = match kind {
                    PathKind::File => path.is_file(),
                    PathKind::Dir => path.is_dir(),
                    PathKind::Any => path.exists(),
                };
                if !ok {
                    out.push(format!("{}, column {}: `{p}` does not exist (relative paths are taken from the current folder)", at(i), c.name));
                }
            }
        }
        out
    }
}

/// `NA` written as one alternative of a pattern: `(...|NA)`, `(NA|...)`, `^NA$`.
static NA_ALTERNATIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^\^?|[(|])NA([|)]|\$?$)").unwrap());

/// A cell's value as nf-schema would hand it to the validator.
fn cast(v: &str, types: &BTreeSet<String>) -> Value {
    if types.contains("integer")
        && let Ok(n) = v.parse::<i64>()
    {
        return Value::from(n);
    }
    if types.contains("number")
        && let Ok(x) = v.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(x)
    {
        return Value::Number(n);
    }
    if types.contains("boolean") && (v == "true" || v == "false") {
        return Value::Bool(v == "true");
    }
    Value::String(v.to_string())
}

fn show(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn short(msg: &str) -> String {
    if msg.chars().count() > 200 { format!("{}...", msg.chars().take(200).collect::<String>()) } else { msg.to_string() }
}

/// Row-level messages quote the whole row object; say which columns instead.
fn row_message(msg: &str, row: &Map<String, Value>) -> String {
    let filled = Value::Object(row.clone()).to_string();
    let msg = msg.replace(&filled, "this row");
    short(&msg)
}

fn str_list(v: Option<&Value>) -> Vec<&str> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

/// Collect keywords (and `format`s) that neither the `jsonschema` crate nor sheetgen evaluates.
fn audit(v: &Value, at: &str, out: &mut Vec<String>) {
    let Some(obj) = v.as_object() else { return };
    let here = if at.is_empty() { "schema" } else { at };
    for (k, val) in obj {
        match k.as_str() {
            "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => {
                for (name, sub) in val.as_object().into_iter().flatten() {
                    audit(sub, &format!("{at}/{name}"), out);
                }
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                for sub in val.as_array().into_iter().flatten() {
                    audit(sub, at, out);
                }
            }
            "items" | "not" | "if" | "then" | "else" | "additionalProperties" | "contains" | "propertyNames"
            | "unevaluatedItems" | "unevaluatedProperties" | "additionalItems" => audit(val, at, out),
            "format" => {
                let f = val.as_str().unwrap_or_default();
                if !KNOWN_FORMATS.contains(&f) {
                    out.push(format!("{here}: format `{f}`"));
                }
            }
            k if KNOWN.contains(&k) => {}
            other => out.push(format!("{here}: keyword `{other}`")),
        }
    }
}

fn role_of(name: &str, meta: &[&str]) -> Role {
    let n = name.to_ascii_lowercase().replace(['-', ' '], "_");
    if meta.iter().any(|m| matches!(*m, "id" | "sample")) || matches!(n.as_str(), "sample" | "id" | "sample_id" | "sampleid")
    {
        return Role::Sample;
    }
    if meta.contains(&"run") || matches!(n.as_str(), "run" | "run_accession") {
        return Role::Run;
    }
    let reads = n.contains("fastq") || n.contains("read") || n.starts_with("fq");
    match n.as_str() {
        "fastq_1" | "fastq1" | "r1" | "read1" | "reads_1" | "forward" | "fq1" => Role::R1,
        "fastq_2" | "fastq2" | "r2" | "read2" | "reads_2" | "reverse" | "fq2" => Role::R2,
        "single_end" => Role::SingleEnd,
        _ if n.contains("platform") => Role::Other,
        _ if n.contains("long") && reads => Role::Long,
        "nanopore" | "ont" | "longreads" => Role::Long,
        _ if reads && n.ends_with("_1") => Role::R1,
        _ if reads && n.ends_with("_2") => Role::R2,
        _ => Role::Other,
    }
}

/// Find `schema_input.json` for a pipeline Nextflow has already pulled.
pub fn find_for_pipeline(name: &str) -> Result<PathBuf> {
    let home = std::env::var_os("NXF_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".nextflow")))
        .context("neither NXF_HOME nor HOME is set")?;
    let assets = home.join("assets");
    let direct = assets.join(name).join("assets/schema_input.json");
    if direct.is_file() {
        return Ok(direct);
    }
    // Newer Nextflow keeps checkouts under assets/.repos/<name>/clones/<commit>/
    let clones = assets.join(".repos").join(name).join("clones");
    if let Ok(entries) = std::fs::read_dir(&clones) {
        let mut found: Vec<(std::time::SystemTime, PathBuf)> = entries
            .flatten()
            .map(|e| e.path().join("assets/schema_input.json"))
            .filter(|p| p.is_file())
            .map(|p| (p.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
            .collect();
        found.sort();
        if let Some((_, p)) = found.pop() {
            return Ok(p);
        }
    }
    bail!(
        "no schema for {name} in {}; run `nextflow pull {name}` first, or pass --schema",
        assets.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles() {
        assert_eq!(role_of("ID", &["sample"]), Role::Sample);
        assert_eq!(role_of("sample", &[]), Role::Sample);
        assert_eq!(role_of("fastq_1", &[]), Role::R1);
        assert_eq!(role_of("R2", &[]), Role::R2);
        assert_eq!(role_of("LongFastQ", &[]), Role::Long);
        assert_eq!(role_of("strandedness", &[]), Role::Other);
        assert_eq!(role_of("short_reads_1", &[]), Role::R1);
        assert_eq!(role_of("short_reads_2", &[]), Role::R2);
        assert_eq!(role_of("long_reads", &[]), Role::Long);
        assert_eq!(role_of("long_reads_platform", &["lr_platform"]), Role::Other);
        assert_eq!(role_of("run", &["run"]), Role::Run);
        assert_eq!(role_of("run_accession", &[]), Role::Run);
    }

    #[test]
    fn na_only_when_listed_as_an_alternative() {
        for p in [r"^(\S+\.f(ast)?q\.gz|NA)$", r"^(NA|\d+)$", "^NA$"] {
            assert!(NA_ALTERNATIVE.is_match(p), "{p}");
        }
        for p in [r"^\S+$", r"^[A-Z]+$", r"^NAME\d+$", r"^(\S+|NAN)$"] {
            assert!(!NA_ALTERNATIVE.is_match(p), "{p}");
        }
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    /// Paired units for the given (sample, lane) list, with real (empty) files under `dir`.
    fn units(dir: &Path, samples: &[(&str, u32)]) -> Vec<Unit> {
        samples
            .iter()
            .map(|(s, lane)| {
                let mk = |m: &str| {
                    let p = dir.join(format!("{s}_L00{lane}_{m}.fastq.gz"));
                    std::fs::write(&p, "").unwrap();
                    Some(p)
                };
                Unit { sample: s.to_string(), r1: mk("R1"), r2: mk("R2"), long: None, numeric_pair: false }
            })
            .collect()
    }

    fn sets(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn run(schema: &Schema, units: &[Unit], fixed: &[(&str, &str)]) -> Result<(Vec<String>, Vec<Vec<String>>)> {
        let fmt = |p: &Path| p.display().to_string();
        schema.build(units, &sets(fixed), &fmt, &mut Vec::new())
    }

    #[test]
    fn bacass_columns_and_na_blanks() {
        let t = tempfile::tempdir().unwrap();
        let s = Schema::load(&fixture("bacass-2.7.0.schema_input.json"), false).unwrap();
        let (header, rows) = run(&s, &units(t.path(), &[("A", 1), ("A", 2)]), &[]).unwrap();
        assert_eq!(header, ["ID", "R1", "R2", "LongFastQ", "Fast5", "GenomeSize"]);
        assert_eq!(&rows[0][3..], ["NA", "NA", "NA"]);
        // A space in a path breaks bacass's pattern; its own message is shown.
        let spaced = tempfile::Builder::new().prefix("with space").tempdir().unwrap();
        let err = run(&s, &units(spaced.path(), &[("A", 1)]), &[]).unwrap_err().to_string();
        assert!(err.contains("column R1") && err.contains("cannot contain spaces"), "{err}");
    }

    #[test]
    fn rnaseq_needs_strandedness_from_its_enum() {
        let t = tempfile::tempdir().unwrap();
        let s = Schema::load(&fixture("rnaseq-3.27.0.schema_input.json"), false).unwrap();
        let u = units(t.path(), &[("A", 1)]);
        let err = run(&s, &u, &[]).unwrap_err().to_string();
        assert!(err.contains("strandedness") && err.contains("--set strandedness=<forward|reverse|unstranded|auto>"), "{err}");
        assert!(run(&s, &u, &[("strandedness", "sideways")]).unwrap_err().to_string().contains("column strandedness"));
        assert!(run(&s, &u, &[("strandedness", "auto"), ("percent_mapped", "101")]).is_err());
        let (header, rows) = run(&s, &u, &[("strandedness", "auto")]).unwrap();
        assert_eq!(header[..4], ["sample", "fastq_1", "fastq_2", "strandedness"]);
        assert_eq!(rows[0][8], ""); // percent_mapped: blank, not NA
    }

    #[test]
    fn mag_dependent_required_and_unique_entries() {
        let t = tempfile::tempdir().unwrap();
        let s = Schema::load(&fixture("mag-5.5.0.schema_input.json"), false).unwrap();
        let u = units(t.path(), &[("A", 1), ("A", 2)]);
        // short_reads_1 filled → short_reads_platform required (dependentRequired).
        let err = run(&s, &u, &[("group", "0")]).unwrap_err().to_string();
        assert!(err.contains("short_reads_platform"), "{err}");
        // Lanes get run 1, 2 so (sample, run) stays unique.
        let (header, rows) = run(&s, &u, &[("group", "0"), ("short_reads_platform", "ILLUMINA")]).unwrap();
        let run_col = header.iter().position(|h| h == "run").unwrap();
        assert_eq!((rows[0][run_col].as_str(), rows[1][run_col].as_str()), ("1", "2"));
        let err = run(&s, &u, &[("group", "0"), ("short_reads_platform", "ILLUMINA"), ("run", "1")]).unwrap_err().to_string();
        assert!(err.contains("uniqueEntries"), "{err}");
        assert!(run(&s, &u, &[("short_reads_platform", "ILLUMINA")]).unwrap_err().to_string().contains("group"));
    }

    #[test]
    fn taxprofiler_unique_files_and_platform() {
        let t = tempfile::tempdir().unwrap();
        let s = Schema::load(&fixture("taxprofiler-2.0.1.schema_input.json"), false).unwrap();
        let u = units(t.path(), &[("A", 1), ("B", 1)]);
        let err = run(&s, &u, &[]).unwrap_err().to_string();
        assert!(err.contains("instrument_platform"), "{err}");
        run(&s, &u, &[("instrument_platform", "ILLUMINA")]).unwrap();
        // The same file listed twice breaks uniqueEntries [fastq_1].
        let mut twice = u.clone();
        twice[1].r1 = twice[0].r1.clone();
        let err = run(&s, &twice, &[("instrument_platform", "ILLUMINA")]).unwrap_err().to_string();
        assert!(err.contains("same fastq_1"), "{err}");
    }

    #[test]
    fn fetchngs_is_not_a_fastq_sheet() {
        let err = Schema::load(&fixture("fetchngs-1.13.0.schema_input.json"), false).err().unwrap().to_string();
        assert!(err.contains("plain list of accessions"), "{err}");
    }

    #[test]
    fn lookaround_type_and_top_level_unique_entries() {
        // The reviewer's schema: a look-ahead pattern, an integer column, uniqueEntries on the array.
        let json = serde_json::json!({"items":{"type":"object","properties":{
            "sample":{"type":"string","pattern":"^(?!ctrl)\\S+$"},
            "fastq_1":{"type":"string","pattern":"^\\S+\\.f(ast)?q\\.gz$"},
            "fastq_2":{"type":"string","pattern":"^\\S+\\.f(ast)?q\\.gz$"},
            "replicate":{"type":"integer"}},
            "required":["sample","fastq_1","replicate"]},"uniqueEntries":["sample"]});
        let s = Schema::from_json(json, "test", false).unwrap();
        let t = tempfile::tempdir().unwrap();
        let ok = units(t.path(), &[("s1", 1)]);
        run(&s, &ok, &[("replicate", "1")]).unwrap();
        assert!(run(&s, &ok, &[("replicate", "one")]).unwrap_err().to_string().contains("column replicate"));
        assert!(run(&s, &units(t.path(), &[("ctrlx", 1)]), &[("replicate", "1")]).unwrap_err().to_string().contains("column sample"));
        let err = run(&s, &units(t.path(), &[("s1", 1), ("s1", 2)]), &[("replicate", "1")]).unwrap_err().to_string();
        assert!(err.contains("row 2 (s1): same sample as row 1"), "{err}");
    }

    #[test]
    fn unknown_keywords_stop_the_run_unless_allowed() {
        let json = serde_json::json!({"items":{"type":"object","properties":{
            "sample":{"type":"string"},
            "fastq_1":{"type":"string","schema":"assets/other.json","format":"fancy-path"}}}});
        let err = Schema::from_json(json.clone(), "test", false).err().unwrap().to_string();
        assert!(err.contains("keyword `schema`") && err.contains("format `fancy-path`"), "{err}");
        let s = Schema::from_json(json, "test", true).unwrap();
        assert_eq!(s.unchecked.len(), 2);
    }

    #[test]
    fn missing_files_fail_exists() {
        let s = Schema::load(&fixture("rnaseq-3.27.0.schema_input.json"), false).unwrap();
        let u = vec![Unit { sample: "A".into(), r1: Some("/nonexistent/A_R1.fastq.gz".into()), r2: None, long: None, numeric_pair: false }];
        let err = run(&s, &u, &[("strandedness", "auto")]).unwrap_err().to_string();
        assert!(err.contains("does not exist"), "{err}");
    }

    #[test]
    fn casts_to_declared_types() {
        let t = |ts: &[&str]| ts.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(cast("3", &t(&["string", "integer"])), Value::from(3));
        assert_eq!(cast("x3", &t(&["integer"])), Value::from("x3"));
        assert_eq!(cast("true", &t(&["boolean"])), Value::Bool(true));
        assert_eq!(cast("2.5", &t(&["number"])), serde_json::json!(2.5));
        assert_eq!(cast("2", &t(&["string"])), Value::from("2"));
    }
}
