//! Does the installed tool accept this database version?

use crate::catalog::Compat;
use regex::Regex;
use std::process::Command;
use std::sync::LazyLock;

static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\.(\d+)(?:\.(\d+))?").unwrap());

pub enum Verdict {
    Ok(String),
    TooOld { found: String, min: String },
    TooNew { found: String, max_exclusive: String },
    NotInstalled,
    Unreadable(String),
}

pub fn parse(s: &str) -> Option<(u64, u64, u64)> {
    let c = VERSION.captures(s)?;
    let n = |i| c.get(i).map(|m: regex::Match| m.as_str().parse().unwrap_or(0)).unwrap_or(0);
    Some((n(1), n(2), n(3)))
}

pub fn check(compat: &Compat) -> Verdict {
    let Some((prog, args)) = compat.command.split_first() else {
        return Verdict::Unreadable("no version command in the catalog".into());
    };
    let out = match Command::new(prog).args(args).output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Verdict::NotInstalled,
        Err(e) => return Verdict::Unreadable(e.to_string()),
    };
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    judge(&text, compat)
}

pub fn judge(text: &str, compat: &Compat) -> Verdict {
    let Some(found) = parse(text) else {
        return Verdict::Unreadable(text.lines().next().unwrap_or("").to_string());
    };
    let shown = VERSION.find(text).map(|m| m.as_str().to_string()).unwrap_or_default();
    if let Some(min) = parse(&compat.min)
        && found < min {
            return Verdict::TooOld { found: shown, min: compat.min.clone() };
        }
    if let Some(max) = compat.max_exclusive.as_deref().and_then(parse)
        && found >= max {
            return Verdict::TooNew { found: shown, max_exclusive: compat.max_exclusive.clone().unwrap() };
        }
    Verdict::Ok(shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(min: &str, max: Option<&str>) -> Compat {
        Compat { command: vec![], min: min.into(), max_exclusive: max.map(String::from), source: String::new() }
    }

    #[test]
    fn versions() {
        assert_eq!(parse("gtdbtk: version 2.7.1 Copyright 2017"), Some((2, 7, 1)));
        assert_eq!(parse("q2cli version 2026.7.0"), Some((2026, 7, 0)));
        assert_eq!(parse("1.1"), Some((1, 1, 0)));
        assert!(matches!(judge("bakta 1.12.2", &c("1.11.0", Some("2.0.0"))), Verdict::Ok(_)));
        assert!(matches!(judge("bakta 1.10.4", &c("1.11.0", Some("2.0.0"))), Verdict::TooOld { .. }));
        assert!(matches!(judge("bakta 2.0.0", &c("1.11.0", Some("2.0.0"))), Verdict::TooNew { .. }));
        assert!(matches!(judge("command not understood", &c("1.0", None)), Verdict::Unreadable(_)));
    }
}
