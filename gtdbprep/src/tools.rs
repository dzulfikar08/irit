//! Running GTDB-Tk and friends as subprocesses (argument vectors, never a shell).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where the tools live and which reference data they read.
#[derive(Debug, Clone)]
pub struct Runner {
    /// A conda/micromamba environment prefix; its `bin/` goes first on `PATH`.
    pub env_prefix: Option<PathBuf>,
    /// `GTDBTK_DATA_PATH` for GTDB-Tk.
    pub data: PathBuf,
    pub cpus: usize,
    /// Folder for one log file per command.
    pub log_dir: PathBuf,
}

impl Runner {
    fn path_var(&self) -> Option<OsString> {
        let prefix = self.env_prefix.as_ref()?;
        let mut paths = vec![prefix.join("bin")];
        if let Some(p) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&p));
        }
        std::env::join_paths(paths).ok()
    }

    pub fn command(&self, program: &str) -> Command {
        let program_path = self
            .env_prefix
            .as_ref()
            .map(|p| p.join("bin").join(program))
            .filter(|p| p.is_file())
            .map(PathBuf::into_os_string)
            .unwrap_or_else(|| program.into());
        self.command_at(program_path)
    }

    /// Like `command`, for an exact program path.
    pub fn command_at(&self, program_path: impl Into<OsString>) -> Command {
        let mut c = Command::new(program_path.into());
        if let Some(path) = self.path_var() {
            c.env("PATH", path);
        }
        c.env("GTDBTK_DATA_PATH", &self.data);
        // pplacer from Bioconda aborts in glibc's locale loader (`_nl_intern_locale_data`
        // assertion) under some host locales; the C locale avoids it and changes no output.
        c.env("LC_ALL", "C");
        c.stdin(Stdio::null());
        c
    }

    /// Run a command, sending stdout and stderr to `<log_dir>/<name>.log`. On failure, the error
    /// includes the end of the log.
    pub fn run(&self, name: &str, mut cmd: Command) -> Result<()> {
        fs::create_dir_all(&self.log_dir)?;
        let log_path = self.log_dir.join(format!("{name}.log"));
        let log = File::create(&log_path)?;
        cmd.stdout(log.try_clone()?).stderr(log);
        let shown = describe(&cmd);
        eprintln!("gtdbprep: running {shown}");
        let status = cmd.status().with_context(|| format!("starting {shown}"))?;
        if !status.success() {
            let text = fs::read_to_string(&log_path).unwrap_or_default();
            let tail: Vec<&str> = text.lines().rev().take(15).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            bail!(
                "{shown} failed ({status}); log {}:\n{}",
                log_path.display(),
                tail.join("\n")
            );
        }
        // GTDB-Tk logs errors but can still exit 0 in a few paths; catch its error marker.
        let text = fs::read_to_string(&log_path).unwrap_or_default();
        if text.contains("ERROR: Controlled exit") {
            bail!("{shown} reported an error; see {}", log_path.display());
        }
        Ok(())
    }

    /// First line of a tool's version output, from stdout or stderr. `None` if it won't run.
    pub fn version_line(&self, program: &str, args: &[&str]) -> Option<String> {
        let out = self.command(program).args(args).output().ok()?;
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        text.lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && first_version(l).is_some())
            .map(str::to_string)
    }
}

/// The command line as text, for logs and error messages.
pub fn describe(cmd: &Command) -> String {
    let mut parts = vec![cmd.get_program().to_string_lossy().into_owned()];
    parts.extend(cmd.get_args().map(|a| {
        let a = a.to_string_lossy();
        if a.contains('\n') {
            "<inline script>".to_string()
        } else {
            a.into_owned()
        }
    }));
    parts.join(" ")
}

/// The first dotted version number in a line: `v1.1.alpha19-0-g807f6f3` gives `1.1.alpha19`,
/// `# HMMER 3.4 (Aug 2023)` gives `3.4`, `Prodigal V2.6.3: February, 2016` gives `2.6.3`.
pub fn first_version(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() || (start > 0 && bytes[start - 1].is_ascii_digit()) {
            continue;
        }
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'.') {
            end += 1;
        }
        let v = line[start..end].trim_end_matches('.');
        if v.contains('.') {
            return Some(v.to_string());
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ToolVersion {
    pub version: Option<String>,
    /// The line the version was read from.
    pub reported: Option<String>,
}

impl ToolVersion {
    /// The line goes into the bundle's manifest, so a line holding a path (a Python traceback
    /// naming `$HOME`, say) is not kept: `reported` falls back to the parsed version.
    pub fn from_line(line: Option<String>) -> Self {
        let version = line.as_deref().and_then(first_version);
        let reported = match line {
            Some(l) if !l.contains('/') && !l.contains('\\') => Some(l),
            _ => version.clone(),
        };
        ToolVersion { version, reported }
    }
}

/// GTDB-Tk releases whose internal ANI-screen API (`ANIScreener.run_aniscreen`) `classify` is
/// tested against.
pub const GTDBTK_SUPPORTED: [&str; 1] = ["2.8.0"];

/// Search a `PATH`-style list for a file called `name`.
fn which(name: &str, path_var: Option<OsString>) -> Option<PathBuf> {
    let path_var = path_var.or_else(|| std::env::var_os("PATH"))?;
    std::env::split_paths(&path_var)
        .map(|d| d.join(name))
        .find(|c| c.is_file())
}

/// The Python a script runs under, from its first lines: `#!/x/bin/python3`,
/// `#!/usr/bin/env python3` (looked up on `path_var`), or conda's long-prefix form
/// `#!/bin/sh` followed by `'''exec' "/x/bin/python3" "$0" "$@"`.
pub fn shebang_interpreter(script: &str, path_var: Option<OsString>) -> Option<PathBuf> {
    let mut lines = script.lines();
    let first = lines.next()?.strip_prefix("#!")?.trim();
    let mut words = first.split_whitespace();
    let prog = words.next()?;
    let base = Path::new(prog).file_name()?.to_string_lossy().into_owned();
    if base == "env" {
        let name = words.find(|w| !w.starts_with('-') && !w.contains('='))?;
        return which(name, path_var);
    }
    if base == "sh" || base == "bash" {
        return lines.take(5).find_map(|l| {
            l.trim()
                .strip_prefix("'''exec' \"")
                .and_then(|rest| rest.split('"').next())
                .map(PathBuf::from)
        });
    }
    Some(PathBuf::from(prog))
}

impl Runner {
    /// The Python interpreter GTDB-Tk runs under: the environment's `python3` with `--env`,
    /// otherwise the interpreter named by the shebang of the `gtdbtk` script found on `PATH`.
    pub fn gtdbtk_python(&self) -> Result<PathBuf> {
        if let Some(prefix) = &self.env_prefix {
            for name in ["python3", "python"] {
                let p = prefix.join("bin").join(name);
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
        let script = self
            .env_prefix
            .as_ref()
            .map(|p| p.join("bin/gtdbtk"))
            .filter(|p| p.is_file())
            .or_else(|| which("gtdbtk", self.path_var()))
            .context("no gtdbtk on PATH (use --env)")?;
        let mut head = vec![0u8; 4096];
        let n = std::io::Read::read(&mut File::open(&script)?, &mut head)?;
        let text = String::from_utf8_lossy(&head[..n]);
        shebang_interpreter(&text, self.path_var())
            .filter(|p| p.is_file())
            .with_context(|| format!("cannot tell which Python runs {}", script.display()))
    }
}

/// Versions of the tools the laptop side runs, as they report themselves.
pub fn laptop_versions(r: &Runner) -> std::collections::BTreeMap<String, ToolVersion> {
    let mut m = std::collections::BTreeMap::new();
    m.insert(
        "gtdbtk".into(),
        ToolVersion::from_line(r.version_line("gtdbtk", &["--version"])),
    );
    m.insert(
        "prodigal".into(),
        ToolVersion::from_line(r.version_line("prodigal", &["-v"])),
    );
    m.insert(
        "hmmer".into(),
        ToolVersion::from_line(r.version_line("hmmsearch", &["-h"])),
    );
    m
}

/// `skani --version` for a given binary path.
pub fn skani_version(skani: &Path) -> Option<String> {
    let out = Command::new(skani)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines().find_map(first_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_from_tool_banners() {
        assert_eq!(
            first_version("gtdbtk: version 2.8.0 Copyright 2017").as_deref(),
            Some("2.8.0")
        );
        assert_eq!(
            first_version("Prodigal V2.6.3: February, 2016").as_deref(),
            Some("2.6.3")
        );
        assert_eq!(
            first_version("# HMMER 3.4 (Aug 2023); http://hmmer.org/").as_deref(),
            Some("3.4")
        );
        assert_eq!(
            first_version("v1.1.alpha19-0-g807f6f3").as_deref(),
            Some("1.1.alpha19")
        );
        assert_eq!(first_version("skani 0.3.1").as_deref(), Some("0.3.1"));
        assert_eq!(first_version("no version here 2023"), None);
    }

    #[test]
    fn version_lines_with_paths_are_not_kept() {
        let v = ToolVersion::from_line(Some("gtdbtk: version 2.8.0".into()));
        assert_eq!(v.reported.as_deref(), Some("gtdbtk: version 2.8.0"));
        let v = ToolVersion::from_line(Some(
            r#"File "/home/someone/env/lib/python3.13/site-packages/x.py", line 3"#.into(),
        ));
        assert!(!v.reported.as_deref().unwrap_or_default().contains('/'));
        assert_eq!(v.reported, v.version);
    }

    #[test]
    fn interpreter_from_shebang() {
        assert_eq!(
            shebang_interpreter("#!/opt/env/bin/python3.13\nimport re\n", None),
            Some(PathBuf::from("/opt/env/bin/python3.13"))
        );
        let conda = "#!/bin/sh\n'''exec' \"/very long/env/bin/python3.13\" \"$0\" \"$@\"\n' '''\n";
        assert_eq!(
            shebang_interpreter(conda, None),
            Some(PathBuf::from("/very long/env/bin/python3.13"))
        );
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("python3"), b"").unwrap();
        let pv = std::env::join_paths([d.path()]).unwrap();
        assert_eq!(
            shebang_interpreter("#!/usr/bin/env python3\n", Some(pv)),
            Some(d.path().join("python3"))
        );
        assert_eq!(shebang_interpreter("import re\n", None), None);
    }

    #[test]
    fn interpreter_comes_from_env_or_gtdbtk_script() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("bin")).unwrap();
        let py = d.path().join("bin/python3.13");
        fs::write(&py, b"").unwrap();
        fs::write(
            d.path().join("bin/gtdbtk"),
            format!("#!{}\nimport re\n", py.display()),
        )
        .unwrap();
        let mut r = Runner {
            env_prefix: Some(d.path().into()),
            data: "/data".into(),
            cpus: 1,
            log_dir: d.path().into(),
        };
        // No bin/python3 in the env: the gtdbtk script's shebang decides.
        assert_eq!(r.gtdbtk_python().unwrap(), py);
        fs::write(d.path().join("bin/python3"), b"").unwrap();
        assert_eq!(r.gtdbtk_python().unwrap(), d.path().join("bin/python3"));
        r.env_prefix = Some(d.path().join("missing"));
        let _ = r.gtdbtk_python(); // falls back to PATH; must not panic
    }

    #[test]
    fn env_prefix_goes_first_on_path() {
        let d = tempfile::tempdir().unwrap();
        let r = Runner {
            env_prefix: Some(d.path().into()),
            data: "/data".into(),
            cpus: 1,
            log_dir: d.path().into(),
        };
        let c = r.command("gtdbtk");
        let envs: Vec<_> = c.get_envs().collect();
        let path = envs.iter().find(|(k, _)| *k == "PATH").unwrap().1.unwrap();
        let first = std::env::split_paths(path).next().unwrap();
        assert_eq!(first, d.path().join("bin"));
        assert!(
            envs.iter()
                .any(|(k, v)| *k == "GTDBTK_DATA_PATH" && v.unwrap() == "/data")
        );
    }
}
