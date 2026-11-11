//! Go to (Ctrl G): `column`, `row:column` or `name:position`, where the name is a sequence name or
//! accession (a unique prefix is enough) and the position is a residue number in that sequence's
//! own numbering: counted from the start of its `/start-end` range when the name has one, else
//! from 1.

use crate::Alignment;
use crate::alphabet::GAP;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Goto {
    /// Alignment column, from 0.
    Col(usize),
    /// View row and alignment column, from 0.
    RowCol(usize, usize),
    /// A sequence and a residue number in its own numbering.
    NamePos(String, usize),
    /// A sequence (no position).
    Name(String),
}

fn number(s: &str) -> Option<usize> {
    let t: String = s.trim().chars().filter(|&c| c != ',' && c != '_').collect();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}

/// Read what was typed. Numbers count from 1 and may have `,` or `_` separators.
pub fn parse(text: &str) -> Result<Goto, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("Type a column, row:column or name:position".into());
    }
    let one = |n: usize, what: &str| if n == 0 { Err(format!("{what} numbers start at 1")) } else { Ok(n - 1) };
    if let Some((l, r)) = t.rsplit_once(':') {
        let Some(n) = number(r) else { return Err(format!("Expected a number after ':', not '{}'", r.trim())) };
        if let Some(row) = number(l) {
            return Ok(Goto::RowCol(one(row, "Row")?, one(n, "Column")?));
        }
        if l.trim().is_empty() {
            return Err("Expected a name before ':'".into());
        }
        if n == 0 {
            return Err("Residue numbers start at 1".into());
        }
        return Ok(Goto::NamePos(l.trim().to_string(), n));
    }
    match number(t) {
        Some(n) => Ok(Goto::Col(one(n, "Column")?)),
        None => Ok(Goto::Name(t.to_string())),
    }
}

/// The name before the last `/` (`GLB2_TYLHE` of `GLB2_TYLHE/32-143`).
fn stem(name: &str) -> &str {
    name.rsplit_once('/').map_or(name, |(s, _)| s)
}

/// An accession without its version (`Q9XYZ1` of `Q9XYZ1.2`).
fn unversioned(a: &str) -> &str {
    a.rsplit_once('.').filter(|(_, v)| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())).map_or(a, |(s, _)| s)
}

/// The residue numbers row `r` covers: its `/start-end` range, else 1 to its length.
pub fn range(aln: &Alignment, r: usize) -> (usize, usize) {
    let len = aln.matrix.residues_before(r, aln.cols());
    let start = crate::name_range(&aln.names[r]).map_or(1, |(a, _)| a);
    (start, start + len.max(1) - 1)
}

/// The row a name refers to. Tried in turn, the first rule with any match wins: the exact name;
/// the exact accession (with or without its version); the name before `/`; a case-insensitive
/// prefix of the name or the accession. When a rule matches several rows and `pos` is given, the
/// rows whose range holds `pos` are kept (a protein with two domains in Pfam).
pub fn find_row(aln: &Alignment, query: &str, pos: Option<usize>) -> Result<usize, String> {
    let q = query.trim();
    let lower = q.to_lowercase();
    let acc = |r: usize| aln.accession(r);
    let rules: [&dyn Fn(usize) -> bool; 4] = [
        &|r| aln.names[r] == q,
        &|r| acc(r).is_some_and(|a| a == q || unversioned(a) == unversioned(q)),
        &|r| stem(&aln.names[r]) == q,
        &|r| aln.names[r].to_lowercase().starts_with(&lower) || acc(r).is_some_and(|a| a.to_lowercase().starts_with(&lower)),
    ];
    for rule in rules {
        let mut hits: Vec<usize> = (0..aln.rows()).filter(|&r| rule(r)).collect();
        if hits.is_empty() {
            continue;
        }
        if hits.len() > 1
            && let Some(p) = pos
        {
            let inside: Vec<usize> = hits.iter().copied().filter(|&r| (range(aln, r).0..=range(aln, r).1).contains(&p)).collect();
            if !inside.is_empty() {
                hits = inside;
            }
        }
        if hits.len() == 1 {
            return Ok(hits[0]);
        }
        let some: Vec<&str> = hits.iter().take(3).map(|&r| aln.names[r].as_str()).collect();
        let more = if hits.len() > 3 { format!(" and {} more", hits.len() - 3) } else { String::new() };
        return Err(format!("'{q}' matches {} sequences: {}{more}", hits.len(), some.join(", ")));
    }
    Err(format!("No sequence named '{q}'"))
}

/// The alignment column of residue `pos` (in the row's own numbering, see [`range`]) of row `r`.
pub fn column_of(aln: &Alignment, r: usize, pos: usize) -> Result<usize, String> {
    let (start, end) = range(aln, r);
    if pos < start || pos > end {
        return Err(format!("{} has residues {start}–{end}; {pos} is outside", aln.names[r]));
    }
    let mut k = pos - start;
    let mut buf = vec![0u8; aln.cols()];
    aln.matrix.decode_row(r, 0, &mut buf);
    for (c, &code) in buf.iter().enumerate() {
        if code != GAP {
            if k == 0 {
                return Ok(c);
            }
            k -= 1;
        }
    }
    Err(format!("{} has fewer residues than its range says", aln.names[r]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;

    #[test]
    fn parses_the_forms() {
        assert_eq!(parse("1,200"), Ok(Goto::Col(1199)));
        assert_eq!(parse(" 3:40 "), Ok(Goto::RowCol(2, 39)));
        assert_eq!(parse("GLB2_TYLHE/32-143:50"), Ok(Goto::NamePos("GLB2_TYLHE/32-143".into(), 50)));
        assert_eq!(parse("P02240:7"), Ok(Goto::NamePos("P02240".into(), 7)));
        assert_eq!(parse("hbb_human"), Ok(Goto::Name("hbb_human".into())));
        assert!(parse("0").is_err());
        assert!(parse("x:y").unwrap_err().contains("number after ':'"));
        assert!(parse(":5").is_err());
        assert!(parse("").is_err());
        assert!(parse("a:0").is_err());
    }

    fn aln() -> Alignment {
        parse_bytes(
            b"# STOCKHOLM 1.0\n#=GS HBB_HUMAN/3-8 AC P68871.2\nGLB2_TYLHE/32-36 --AC-DE-\nGLB2_TYLHE/90-94 ACDE-F--\nHBB_HUMAN/3-8 ACDEFG--\nHBA_HUMAN ---ACD--\n//\n",
        )
        .unwrap()
    }

    #[test]
    fn finds_rows() {
        let a = aln();
        assert_eq!(find_row(&a, "HBA_HUMAN", None), Ok(3));
        assert_eq!(find_row(&a, "P68871", None), Ok(2));
        assert_eq!(find_row(&a, "P68871.2", None), Ok(2));
        assert_eq!(find_row(&a, "hbb", None), Ok(2)); // unique prefix
        assert!(find_row(&a, "hb", None).unwrap_err().contains("matches 2 sequences"));
        assert!(find_row(&a, "GLB2_TYLHE", None).unwrap_err().contains("matches 2"));
        // The position picks the domain.
        assert_eq!(find_row(&a, "GLB2_TYLHE", Some(92)), Ok(1));
        assert_eq!(find_row(&a, "GLB2_TYLHE", Some(33)), Ok(0));
        assert_eq!(find_row(&a, "nope", None), Err("No sequence named 'nope'".into()));
    }

    #[test]
    fn residue_numbers_to_columns() {
        let a = aln();
        // GLB2_TYLHE/32-36: --AC-DE- ; residue 32 = A at column 2, 34 = D at column 5.
        assert_eq!(range(&a, 0), (32, 35));
        assert_eq!(column_of(&a, 0, 32), Ok(2));
        assert_eq!(column_of(&a, 0, 34), Ok(5));
        assert!(column_of(&a, 0, 31).unwrap_err().contains("32–35"));
        assert!(column_of(&a, 0, 36).is_err());
        // No range: counted from 1.
        assert_eq!(range(&a, 3), (1, 3));
        assert_eq!(column_of(&a, 3, 3), Ok(5));
    }
}
