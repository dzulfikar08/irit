//! Reading an nf-core `schema_input.json` (JSON Schema for the sample sheet) and filling it.

use crate::{Unit, clean_name};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a column holds, guessed from its name and `meta` tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Sample,
    R1,
    R2,
    Long,
    SingleEnd,
    Other,
}

#[derive(Debug)]
struct Column {
    name: String,
    role: Role,
    required: bool,
    /// Patterns from the property (any one passing is enough, as in `anyOf`).
    patterns: Vec<Regex>,
    /// The schema allows an empty string.
    allows_empty: bool,
    /// The schema's patterns accept the literal `NA` (bacass-style blanks).
    allows_na: bool,
    choices: Option<Vec<String>>,
    message: Option<String>,
}

pub struct Schema {
    columns: Vec<Column>,
}

impl Schema {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let json: Value = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let items = json.get("items").unwrap_or(&json);
        let props = items
            .get("properties")
            .and_then(Value::as_object)
            .with_context(|| format!("{} has no items.properties", path.display()))?;
        let required: Vec<&str> = items
            .get("required")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        let mut columns = Vec::new();
        for (name, prop) in props {
            let branches: Vec<&Value> = match prop.get("anyOf").and_then(Value::as_array) {
                Some(b) => b.iter().collect(),
                None => vec![prop],
            };
            let mut patterns = Vec::new();
            let mut allows_empty = false;
            for b in &branches {
                if let Some(p) = b.get("pattern").and_then(Value::as_str) {
                    match Regex::new(p) {
                        Ok(r) => patterns.push(r),
                        Err(_) => eprintln!("  note: column {name}: pattern uses features sheetgen can't check, skipped"),
                    }
                }
                if b.get("maxLength").and_then(Value::as_u64) == Some(0)
                    || b.get("minLength").and_then(Value::as_u64) == Some(0)
                {
                    allows_empty = true;
                }
            }
            if patterns.is_empty() && !required.contains(&name.as_str()) {
                allows_empty = true;
            }
            let allows_na = patterns.iter().any(|r| r.is_match("NA"));
            let choices = prop.get("enum").and_then(Value::as_array).map(|a| {
                a.iter().filter_map(|v| v.as_str().map(String::from)).collect()
            });
            let meta: Vec<&str> = prop
                .get("meta")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            columns.push(Column {
                role: role_of(name, &meta),
                required: required.contains(&name.as_str()),
                name: name.clone(),
                patterns,
                allows_empty,
                allows_na,
                choices,
                message: prop.get("errorMessage").and_then(Value::as_str).map(String::from),
            });
        }
        if !columns.iter().any(|c| c.role == Role::Sample) {
            bail!("couldn't tell which column is the sample name in {}", path.display());
        }
        Ok(Schema { columns })
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
        let has_long_col = self.columns.iter().any(|c| c.role == Role::Long);
        if units.iter().any(|u| u.long.is_some()) && !has_long_col {
            warnings.push("this pipeline has no long-read column; long reads went into the R1 column".into());
        }

        let mut problems = Vec::new();
        let mut rows = Vec::new();
        for (i, u) in units.iter().enumerate() {
            let mut row = Vec::new();
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
                        Role::Other => None,
                    }
                };
                let value = match value {
                    Some(v) => v,
                    // Pipelines whose patterns accept `NA` use it for blanks in their own examples.
                    None if c.allows_na => "NA".into(),
                    None => String::new(),
                };
                if let Some(p) = self.check(c, &value) {
                    problems.push(format!("row {} ({}), column {}: {p}", i + 1, u.sample, c.name));
                }
                row.push(value);
            }
            rows.push(row);
        }
        if !problems.is_empty() {
            let missing: Vec<_> = self
                .columns
                .iter()
                .filter(|c| c.role == Role::Other && c.required && !fixed.contains_key(&c.name))
                .map(|c| match &c.choices {
                    Some(ch) => format!("--set {}=<{}>", c.name, ch.join("|")),
                    None => format!("--set {}=<value>", c.name),
                })
                .collect();
            let mut hint = if missing.is_empty() {
                String::new()
            } else {
                format!("\nhint: this pipeline needs {}", missing.join(" "))
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

    /// Why `value` breaks the column's rules, if it does.
    fn check(&self, c: &Column, value: &str) -> Option<String> {
        if value.is_empty() {
            return if c.required {
                Some("required but empty".into())
            } else if !c.allows_empty && !c.patterns.is_empty() {
                Some("empty, but the schema wants a value".into())
            } else {
                None
            };
        }
        if let Some(ch) = &c.choices
            && !ch.iter().any(|x| x == value) {
                return Some(format!("`{value}` is not one of {}", ch.join(", ")));
            }
        if !c.patterns.is_empty() && !c.patterns.iter().any(|r| r.is_match(value)) {
            return Some(match &c.message {
                Some(m) => format!("`{value}`: {m}"),
                None => format!("`{value}` doesn't match the schema pattern"),
            });
        }
        None
    }
}

fn role_of(name: &str, meta: &[&str]) -> Role {
    let n = name.to_ascii_lowercase().replace(['-', ' '], "_");
    if meta.iter().any(|m| matches!(*m, "id" | "sample")) || matches!(n.as_str(), "sample" | "id" | "sample_id" | "sampleid")
    {
        return Role::Sample;
    }
    match n.as_str() {
        "fastq_1" | "fastq1" | "r1" | "read1" | "reads_1" | "forward" | "fq1" => Role::R1,
        "fastq_2" | "fastq2" | "r2" | "read2" | "reads_2" | "reverse" | "fq2" => Role::R2,
        "single_end" => Role::SingleEnd,
        _ if n.contains("long") && (n.contains("fastq") || n.contains("read")) => Role::Long,
        "nanopore" | "ont" | "longreads" => Role::Long,
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
    }
}
