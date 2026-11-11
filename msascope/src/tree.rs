//! Newick trees: a parser that never recurses (trees with hundreds of thousands of leaves, or one
//! leaf deeper than the last), leaf-to-sequence matching, and the row order and collapsed clades
//! the viewer shows.
//!
//! Nodes live in one flat `Vec` (parent, first child, next sibling, branch length, label), 24
//! bytes each; labels share one string.

use std::collections::HashMap;
use std::ops::Range;

use anyhow::{Result, bail};
use rayon::prelude::*;

use crate::Alignment;
use crate::alphabet::{GAP, NCODES};
use crate::view::RowView;

/// No node.
pub const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub parent: u32,
    pub first_child: u32,
    pub next_sibling: u32,
    /// Branch length to the parent; 0 when the file gives none.
    pub len: f32,
    label: (u32, u32),
}

#[derive(Debug, Default)]
pub struct Tree {
    /// Node 0 is the root.
    pub nodes: Vec<Node>,
    labels: String,
    /// Whether any branch has a length (`:0.1`). Without lengths every branch counts as 1.
    pub has_lengths: bool,
}

impl Tree {
    /// Parse the first tree of a Newick text: quoted labels (`'a b'`, `''` for a quote), labels on
    /// internal nodes (names or support values), branch lengths, `[...]` comments (NHX
    /// annotations included, ignored), whitespace and line breaks anywhere between tokens. The
    /// closing `;` may be missing at the end of the text.
    pub fn parse(text: &str) -> Result<Tree> {
        let b = text.as_bytes();
        let mut t = Tree::default();
        let mut last: Vec<u32> = Vec::new(); // last child of each node, to append siblings
        let new_node = |t: &mut Tree, last: &mut Vec<u32>, parent: u32| -> u32 {
            let id = t.nodes.len() as u32;
            t.nodes.push(Node { parent, first_child: NONE, next_sibling: NONE, len: 0.0, label: (0, 0) });
            last.push(NONE);
            if parent != NONE {
                let p = parent as usize;
                match last[p] {
                    NONE => t.nodes[p].first_child = id,
                    l => t.nodes[l as usize].next_sibling = id,
                }
                last[p] = id;
            }
            id
        };
        let mut i = 0;
        // Skip a byte order mark and leading space.
        if b.starts_with(b"\xEF\xBB\xBF") {
            i = 3;
        }
        let mut cur = NONE;
        let mut labelled = false;
        let mut ended = false;
        while i < b.len() {
            let ch = b[i];
            match ch {
                b' ' | b'\t' | b'\r' | b'\n' => i += 1,
                b'[' => {
                    let mut depth = 0;
                    let start = i;
                    loop {
                        if i >= b.len() {
                            bail!("unclosed [ comment starting at byte {start}");
                        }
                        match b[i] {
                            b'[' => depth += 1,
                            b']' => {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                }
                b'(' => {
                    if cur == NONE {
                        if !t.nodes.is_empty() {
                            bail!("byte {i}: text after the end of the tree");
                        }
                        cur = new_node(&mut t, &mut last, NONE);
                    } else if labelled || t.nodes[cur as usize].first_child != NONE {
                        bail!("byte {i}: '(' after a label or a closed group (missing ','?)");
                    }
                    cur = new_node(&mut t, &mut last, cur);
                    labelled = false;
                    i += 1;
                }
                b',' => {
                    let parent = if cur == NONE { NONE } else { t.nodes[cur as usize].parent };
                    if parent == NONE {
                        bail!("byte {i}: ',' outside parentheses");
                    }
                    cur = new_node(&mut t, &mut last, parent);
                    labelled = false;
                    i += 1;
                }
                b')' => {
                    let parent = if cur == NONE { NONE } else { t.nodes[cur as usize].parent };
                    if parent == NONE {
                        bail!("byte {i}: ')' without a matching '('");
                    }
                    cur = parent;
                    labelled = false;
                    i += 1;
                }
                b':' => {
                    if cur == NONE {
                        cur = new_node(&mut t, &mut last, NONE);
                    }
                    i += 1;
                    while i < b.len() && b[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    let s = i;
                    while i < b.len() && matches!(b[i], b'0'..=b'9' | b'.' | b'-' | b'+' | b'e' | b'E') {
                        i += 1;
                    }
                    let v: f32 = std::str::from_utf8(&b[s..i]).ok().and_then(|x| x.parse().ok()).ok_or_else(|| anyhow::anyhow!("byte {s}: bad branch length"))?;
                    t.nodes[cur as usize].len = if v.is_finite() { v.max(0.0) } else { 0.0 };
                    t.has_lengths = true;
                }
                b';' => {
                    ended = true;
                    break;
                }
                _ => {
                    if cur == NONE {
                        cur = new_node(&mut t, &mut last, NONE);
                    }
                    if labelled {
                        bail!("byte {i}: two labels on one node (missing ',' or ':'?)");
                    }
                    let start = t.labels.len() as u32;
                    if ch == b'\'' || ch == b'"' {
                        i += 1;
                        loop {
                            if i >= b.len() {
                                bail!("unclosed quoted label");
                            }
                            if b[i] == ch {
                                if b.get(i + 1) == Some(&ch) {
                                    t.labels.push(ch as char);
                                    i += 2;
                                    continue;
                                }
                                i += 1;
                                break;
                            }
                            // Copy one UTF-8 character.
                            let n = utf8_len(b[i]);
                            t.labels.push_str(std::str::from_utf8(&b[i..(i + n).min(b.len())]).unwrap_or("?"));
                            i += n;
                        }
                    } else {
                        let s = i;
                        while i < b.len() && !matches!(b[i], b'(' | b')' | b',' | b':' | b';' | b'[' | b' ' | b'\t' | b'\r' | b'\n') {
                            i += 1;
                        }
                        t.labels.push_str(&text[s..i]);
                    }
                    t.nodes[cur as usize].label = (start, t.labels.len() as u32 - start);
                    labelled = true;
                }
            }
        }
        if t.nodes.is_empty() {
            bail!("no tree found");
        }
        if !ended && cur != 0 {
            bail!("unbalanced parentheses: the tree ends inside a group");
        }
        if ended && cur != 0 && cur != NONE {
            bail!("unbalanced parentheses: ';' inside a group");
        }
        Ok(t)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn label(&self, n: u32) -> &str {
        let (s, l) = self.nodes[n as usize].label;
        &self.labels[s as usize..(s + l) as usize]
    }

    pub fn is_leaf(&self, n: u32) -> bool {
        self.nodes[n as usize].first_child == NONE
    }

    pub fn children(&self, n: u32) -> impl Iterator<Item = u32> + '_ {
        let mut c = self.nodes[n as usize].first_child;
        std::iter::from_fn(move || {
            (c != NONE).then(|| {
                let x = c;
                c = self.nodes[c as usize].next_sibling;
                x
            })
        })
    }

    pub fn leaf_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.first_child == NONE).count()
    }

    /// Nodes in pre-order (a parent before its children, children in file order).
    pub fn preorder(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack = vec![0u32];
        let mut kids = Vec::new();
        while let Some(n) = stack.pop() {
            out.push(n);
            kids.clear();
            kids.extend(self.children(n));
            stack.extend(kids.iter().rev());
        }
        out
    }

    /// Bytes held: nodes and labels.
    pub fn bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Node>() + self.labels.capacity()
    }
}

fn utf8_len(b: u8) -> usize {
    match b {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

/// Whether a file looks like a Newick tree: by extension, or by its first non-space character
/// being `(` (an alignment never starts with one).
pub fn looks_like_tree(path: &std::path::Path, head: &[u8]) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if matches!(ext.as_deref(), Some("nwk" | "newick" | "tree" | "treefile" | "tre" | "nhx" | "dnd" | "ph")) {
        return true;
    }
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    head.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'(')
}

/// Tree files next to an alignment with the same stem (`PF00042.sto` → `PF00042.nwk`), in the
/// order they are tried. `x.fa.gz` also tries `x.nwk`.
pub fn sibling_candidates(aln: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Some(name) = aln.file_name().and_then(|n| n.to_str()) else { return Vec::new() };
    let dir = aln.parent().unwrap_or(std::path::Path::new(""));
    let mut stems = Vec::new();
    let mut s = name;
    for _ in 0..2 {
        match s.rsplit_once('.') {
            Some((a, _)) if !a.is_empty() => {
                stems.push(a.to_string());
                s = a;
            }
            _ => break,
        }
    }
    let mut out = Vec::new();
    for stem in stems {
        for ext in ["nwk", "newick", "tree", "treefile"] {
            out.push(dir.join(format!("{stem}.{ext}")));
        }
    }
    out
}

/// How the tree's leaves matched the alignment's sequences.
#[derive(Debug, Default)]
pub struct Matching {
    /// Per node: the row of a matched leaf, else [`NONE`].
    pub row: Vec<u32>,
    pub matched: usize,
    pub leaves: usize,
    /// Labels of leaves that match no sequence (or a sequence already matched by another leaf).
    pub unmatched_leaves: Vec<String>,
    /// Rows no leaf matched, in file order. They are shown after the tree's rows.
    pub rows_not_in_tree: Vec<u32>,
}

impl Matching {
    /// "73 of 75 leaves matched; 2 leaves not in the alignment (a, b); 3 sequences not in the tree".
    pub fn report(&self) -> String {
        let g = crate::view::group_digits;
        let mut s = format!("{} of {} tree leaves matched", g(self.matched), g(self.leaves));
        if !self.unmatched_leaves.is_empty() {
            let shown: Vec<&str> = self.unmatched_leaves.iter().take(3).map(String::as_str).collect();
            let more = if self.unmatched_leaves.len() > 3 { ", …" } else { "" };
            s += &format!("; {} not in the alignment ({}{more})", g(self.unmatched_leaves.len()), shown.join(", "));
        }
        if !self.rows_not_in_tree.is_empty() {
            s += &format!("; {} sequences not in the tree, shown at the end", g(self.rows_not_in_tree.len()));
        }
        s
    }
}

/// Match leaves to rows, by the first rule that finds an unused row:
/// 1. the full sequence name (also with `_` in the label read as a space);
/// 2. the name's first word (FASTA headers with a description);
/// 3. the name before its last `/` (Pfam's `GLB2_TYLHE/32-143` against `GLB2_TYLHE`);
/// 4. the accession (`#=GS … AC`), with or without its `.version`.
///
/// Rules 2–4 only use keys that belong to one row. A leaf whose row was already taken by an
/// earlier leaf counts as unmatched.
pub fn match_rows(tree: &Tree, names: &[String], accessions: &[Option<Box<str>>]) -> Matching {
    fn unique<'a>(it: impl Iterator<Item = (&'a str, u32)>) -> HashMap<&'a str, u32> {
        let mut m: HashMap<&str, u32> = HashMap::new();
        let mut dup = Vec::new();
        for (k, r) in it {
            if m.insert(k, r).is_some() {
                dup.push(k);
            }
        }
        for k in dup {
            m.remove(k);
        }
        m
    }
    let full: HashMap<&str, u32> = names.iter().enumerate().rev().map(|(r, n)| (n.as_str(), r as u32)).collect();
    let word = unique(names.iter().enumerate().filter_map(|(r, n)| n.split_whitespace().next().filter(|w| w.len() < n.len()).map(|w| (w, r as u32))));
    let stem = unique(names.iter().enumerate().filter_map(|(r, n)| n.rsplit_once('/').map(|(a, _)| (a, r as u32))));
    let acc = unique(accessions.iter().enumerate().flat_map(|(r, a)| {
        let a = a.as_deref();
        let base = a.and_then(|a| a.rsplit_once('.').map(|(b, _)| b));
        a.into_iter().chain(base).map(move |k| (k, r as u32))
    }));
    let mut used = vec![false; names.len()];
    let mut m = Matching { row: vec![NONE; tree.len()], ..Default::default() };
    for n in tree.preorder() {
        if !tree.is_leaf(n) {
            continue;
        }
        m.leaves += 1;
        let l = tree.label(n);
        let spaced = l.contains('_').then(|| l.replace('_', " "));
        let found = [full.get(l), spaced.as_deref().and_then(|s| full.get(s)), word.get(l), stem.get(l), acc.get(l)]
            .into_iter()
            .flatten()
            .copied()
            .find(|&r| !used[r as usize]);
        match found {
            Some(r) => {
                used[r as usize] = true;
                m.row[n as usize] = r;
                m.matched += 1;
            }
            None => m.unmatched_leaves.push(l.to_string()),
        }
    }
    m.rows_not_in_tree = (0..names.len() as u32).filter(|&r| !used[r as usize]).collect();
    m
}

/// A tree matched to an alignment, with what does not depend on which clades are collapsed:
/// matched rows in tree order, each node's range in that order, and the drawing x of each node.
pub struct TreeIndex {
    pub tree: Tree,
    pub matching: Matching,
    /// Matched rows in the tree's leaf order.
    pub order: Vec<u32>,
    /// Per node: its leaves' range in `order` (empty for a subtree with no matched leaf).
    pub span: Vec<(u32, u32)>,
    /// Per node: distance from the root (branch lengths, or 1 per branch without them).
    pub x: Vec<f32>,
    /// Per node: the largest `x` of its subtree's matched leaves.
    pub x_max: Vec<f32>,
    /// The largest `x` of any matched leaf.
    pub depth: f32,
}

impl TreeIndex {
    pub fn new(tree: Tree, names: &[String], accessions: &[Option<Box<str>>]) -> TreeIndex {
        let matching = match_rows(&tree, names, accessions);
        let n = tree.len();
        let pre = tree.preorder();
        let mut x = vec![0f32; n];
        for &v in &pre[1..] {
            let node = &tree.nodes[v as usize];
            let len = if tree.has_lengths { node.len } else { 1.0 };
            x[v as usize] = x[node.parent as usize] + len;
        }
        let mut order = Vec::with_capacity(matching.matched);
        let mut span = vec![(0u32, 0u32); n];
        let mut x_max = vec![f32::NEG_INFINITY; n];
        // Leaves get their position in pre-order; a parent's span runs from its first child's
        // start to its last child's end, filled in post-order (reverse pre-order).
        for &v in &pre {
            let r = matching.row[v as usize];
            if r != NONE {
                span[v as usize] = (order.len() as u32, order.len() as u32 + 1);
                x_max[v as usize] = x[v as usize];
                order.push(r);
            }
        }
        for &v in pre.iter().rev() {
            if tree.is_leaf(v) {
                continue;
            }
            let (mut lo, mut hi, mut xm) = (u32::MAX, 0u32, f32::NEG_INFINITY);
            for c in tree.children(v) {
                let (a, b) = span[c as usize];
                if a < b {
                    lo = lo.min(a);
                    hi = hi.max(b);
                    xm = xm.max(x_max[c as usize]);
                }
            }
            if lo < hi {
                span[v as usize] = (lo, hi);
                x_max[v as usize] = xm;
            }
        }
        let depth = x_max[0].max(0.0);
        TreeIndex { tree, matching, order, span, x, x_max, depth }
    }

    /// Matched leaves under node `n`.
    pub fn size(&self, n: u32) -> usize {
        let (a, b) = self.span[n as usize];
        (b - a) as usize
    }

    /// The rows under node `n`, in tree order.
    pub fn rows(&self, n: u32) -> &[u32] {
        let (a, b) = self.span[n as usize];
        &self.order[a as usize..b as usize]
    }

    pub fn bytes(&self) -> usize {
        self.tree.bytes() + self.matching.row.len() * 4 + self.order.len() * 4 + self.span.len() * 8 + self.x.len() * 8
    }
}

/// Where each node is drawn for one row view: its row (centre, in view rows) and the rows its
/// subtree covers. `NAN` / empty for nodes not shown (no matched leaf, or inside a collapsed
/// clade).
pub struct TreeRows {
    pub y: Vec<f32>,
    /// View rows covered by the node's subtree, half-open.
    pub rows: Vec<(u32, u32)>,
    /// Node of each collapsed clade shown, by clade index of the row view.
    pub collapsed: Vec<u32>,
}

/// The row order to show: by the tree (collapsed clades as one row each, then the rows not in the
/// tree in file order) or the file's own order (nothing collapsed). Returns the row view and, in
/// tree order, the node positions for drawing.
///
/// `collapsed[n]` marks nodes the user collapsed; a collapsed node inside another collapsed one is
/// hidden with it. Clade consensus rows are computed here ([`clade_consensus`]).
pub fn row_view(aln: &Alignment, ix: &TreeIndex, collapsed: &[bool], tree_order: bool) -> (RowView, Option<TreeRows>) {
    let rows = aln.rows();
    if !tree_order {
        return (RowView::identity(rows), None);
    }
    let t = &ix.tree;
    let mut view: Vec<u32> = Vec::with_capacity(rows);
    let mut clade_nodes: Vec<u32> = Vec::new();
    let mut y = vec![f32::NAN; t.len()];
    let mut span = vec![(0u32, 0u32); t.len()];
    // Pre-order walk that skips collapsed subtrees: each node's first view row is set on entry;
    // the end is filled in post-order below.
    let mut stack = vec![0u32];
    let mut first = vec![u32::MAX; t.len()];
    let mut shown = Vec::with_capacity(t.len());
    let mut kids = Vec::new();
    while let Some(n) = stack.pop() {
        if ix.size(n) == 0 {
            continue;
        }
        shown.push(n);
        first[n as usize] = view.len() as u32;
        if collapsed.get(n as usize).copied().unwrap_or(false) && !t.is_leaf(n) {
            y[n as usize] = view.len() as f32 + 0.5;
            span[n as usize] = (view.len() as u32, view.len() as u32 + 1);
            view.push(RowView::CLADE | clade_nodes.len() as u32);
            clade_nodes.push(n);
            continue;
        }
        if t.is_leaf(n) {
            y[n as usize] = view.len() as f32 + 0.5;
            span[n as usize] = (view.len() as u32, view.len() as u32 + 1);
            view.push(ix.matching.row[n as usize]);
            continue;
        }
        kids.clear();
        kids.extend(t.children(n));
        stack.extend(kids.iter().rev());
    }
    // Internal nodes, children before parents: y halfway between the first and last shown child.
    for &n in shown.iter().rev() {
        if t.is_leaf(n) || !y[n as usize].is_nan() {
            continue;
        }
        let (mut lo, mut hi, mut a, mut b) = (f32::INFINITY, f32::NEG_INFINITY, u32::MAX, 0u32);
        for c in t.children(n) {
            let yc = y[c as usize];
            if !yc.is_nan() {
                lo = lo.min(yc);
                hi = hi.max(yc);
                a = a.min(span[c as usize].0);
                b = b.max(span[c as usize].1);
            }
        }
        if lo.is_finite() {
            y[n as usize] = (lo + hi) / 2.0;
            span[n as usize] = (a, b);
        }
    }
    let tree_end = view.len() as u32;
    view.extend_from_slice(&ix.matching.rows_not_in_tree);
    let members: Vec<Range<u32>> = clade_nodes.iter().map(|&n| ix.span[n as usize].0..ix.span[n as usize].1).collect();
    let consensus: Vec<Vec<u8>> = members.iter().map(|m| clade_consensus(aln, &ix.order[m.start as usize..m.end as usize])).collect();
    let rv = RowView::new(view, rows, tree_end, members, ix.order.clone(), consensus);
    (rv, Some(TreeRows { y, rows: span, collapsed: clade_nodes }))
}

/// The consensus of a set of rows, per column: a gap where more than half of them are gaps,
/// otherwise their most common residue (ties to the lower code).
pub fn clade_consensus(aln: &Alignment, rows: &[u32]) -> Vec<u8> {
    const BLOCK: usize = 1024;
    let cols = aln.cols();
    let mut out = vec![GAP; cols];
    out.par_chunks_mut(BLOCK).enumerate().for_each(|(blk, out)| {
        let (c0, w) = (blk * BLOCK, out.len());
        let mut acc = vec![0u32; w * NCODES];
        let mut buf = vec![0u8; w];
        for &r in rows {
            aln.matrix.decode_row(r as usize, c0, &mut buf);
            for (i, &code) in buf.iter().enumerate() {
                acc[i * NCODES + code as usize] += 1;
            }
        }
        for (i, h) in acc.as_chunks::<NCODES>().0.iter().enumerate() {
            if h[GAP as usize] as usize * 2 > rows.len() {
                continue;
            }
            let (best, &n) = h.iter().enumerate().skip(1).max_by_key(|&(i, &v)| (v, std::cmp::Reverse(i))).unwrap();
            if n > 0 {
                out[i] = best as u8;
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;

    fn leaves(t: &Tree) -> Vec<&str> {
        t.preorder().into_iter().filter(|&n| t.is_leaf(n)).map(|n| t.label(n)).collect()
    }

    #[test]
    fn parses_newick_edge_cases() {
        let t = Tree::parse("((A:0.1,B:0.2)90:0.05,'C d''x':1e-2,(E,F,G)inner)root;").unwrap();
        assert_eq!(leaves(&t), ["A", "B", "C d'x", "E", "F", "G"]);
        assert_eq!(t.label(0), "root");
        assert!(t.has_lengths);
        let kids: Vec<u32> = t.children(0).collect();
        assert_eq!(kids.len(), 3);
        assert_eq!(t.label(kids[0]), "90"); // support value as an internal label
        assert_eq!(t.label(kids[2]), "inner");
        assert_eq!(t.children(kids[2]).count(), 3); // multifurcation
        assert!((t.nodes[kids[1] as usize].len - 0.01).abs() < 1e-7);
        // comments, NHX, whitespace and line breaks, no final ';'
        let t = Tree::parse(" [a comment [nested]]\n( A[&&NHX:S=human] : 1 ,\n\tB:2 [x] )").unwrap();
        assert_eq!(leaves(&t), ["A", "B"]);
        assert_eq!(t.nodes[1].len, 1.0);
        // no lengths, single leaf, unicode in quotes, text after ';' ignored
        let t = Tree::parse("(a,(b,c));(x,y);").unwrap();
        assert!(!t.has_lengths);
        assert_eq!(leaves(&t), ["a", "b", "c"]);
        assert_eq!(leaves(&Tree::parse("only;").unwrap()), ["only"]);
        assert_eq!(leaves(&Tree::parse("('α β',\"q\")").unwrap()), ["α β", "q"]);
        // empty labels are leaves too
        assert_eq!(Tree::parse("(,);").unwrap().leaf_count(), 2);
        for bad in ["", "((a,b);", "(a,b));", "(a,b)c d;", "(a:x,b);", "a,b;", "('a,b);", "(a,b)[x"] {
            assert!(Tree::parse(bad).is_err(), "{bad:?} should fail");
        }
    }

    /// Deep trees parse and walk without recursion: a caterpillar 200,000 levels deep.
    #[test]
    fn deep_caterpillar() {
        let n = 200_000;
        let mut s = "(".repeat(n - 1);
        s += "l0";
        for i in 1..n {
            s += &format!(",l{i}:0.5)");
        }
        s += ";";
        let t = Tree::parse(&s).unwrap();
        assert_eq!(t.leaf_count(), n);
        let names: Vec<String> = (0..n).rev().map(|i| format!("l{i}")).collect();
        let ix = TreeIndex::new(t, &names, &[]);
        assert_eq!(ix.matching.matched, n);
        assert_eq!(ix.order[0] as usize, n - 1); // l0 is the last row of the file
        assert_eq!(ix.size(0), n);
    }

    #[test]
    fn matching_rules() {
        let aln = parse_bytes(
            b"# STOCKHOLM 1.0\n#=GS P1/1-3 AC Q9XYZ1.2\nGLB2_TYLHE/32-34 ACD\nGLB2_TYLHE/40-42 ACD\nP1/1-3 ACD\nfull_name ACD\nalone ACD\n//\n",
        )
        .unwrap();
        let t = Tree::parse("(GLB2_TYLHE,full_name,Q9XYZ1,'GLB2_TYLHE/40-42',ghost,full_name);").unwrap();
        let m = match_rows(&t, &aln.names, &aln.accessions);
        let row = |l: &str| t.preorder().into_iter().find(|&n| t.is_leaf(n) && t.label(n) == l).map(|n| m.row[n as usize]);
        // "GLB2_TYLHE" is the stem of two rows: ambiguous, no match by stem
        assert_eq!(row("GLB2_TYLHE"), Some(NONE));
        assert_eq!(row("GLB2_TYLHE/40-42"), Some(1));
        assert_eq!(row("Q9XYZ1"), Some(2)); // accession without its version
        assert_eq!(m.matched, 3);
        assert_eq!(m.unmatched_leaves, ["GLB2_TYLHE", "ghost", "full_name"]); // second full_name: row taken
        assert_eq!(m.rows_not_in_tree, [0, 4]);
        assert!(m.report().contains("3 of 6 tree leaves matched"));
        // FASTA headers with a description match by their first word; underscores as spaces
        let f = parse_bytes(b">seq1 some protein\nAC\n>two words\nAC\n").unwrap();
        let m = match_rows(&Tree::parse("(seq1,two_words);").unwrap(), &f.names, &[]);
        assert_eq!(m.matched, 2);
    }

    fn three_clades() -> (Alignment, TreeIndex) {
        // rows in file order: c, a1, b, a2, z (z is not in the tree)
        let aln = parse_bytes(b">c\nGGGG\n>a1\nAC-T\n>b\nTTTT\n>a2\nAA-T\n>z\nCCCC\n").unwrap();
        let t = Tree::parse("((a1:1,a2:1)A:1,(b:1,c:2)B:1);").unwrap();
        let ix = TreeIndex::new(t, &aln.names, &[]);
        (aln, ix)
    }

    #[test]
    fn tree_order_and_collapse() {
        let (aln, ix) = three_clades();
        assert_eq!(ix.order, [1, 3, 2, 0]);
        assert_eq!(ix.depth, 3.0);
        let none = vec![false; ix.tree.len()];
        let (rv, tr) = row_view(&aln, &ix, &none, true);
        assert_eq!(rv.expanded(), [1, 3, 2, 0, 4]);
        assert_eq!(rv.len(), 5);
        assert_eq!(rv.tree_end, 4);
        assert_eq!(rv.view_of(4), 4);
        assert_eq!(rv.view_of(0), 3);
        let tr = tr.unwrap();
        assert_eq!(tr.y[0], 2.0); // root halfway between clade A (1.0) and B (3.0)
        // collapse clade A (node 1)
        let mut c = none.clone();
        c[1] = true;
        let (rv, tr) = row_view(&aln, &ix, &c, true);
        assert_eq!(rv.len(), 4);
        assert_eq!(rv.clade_of(0), Some(0));
        assert_eq!(rv.view_of(1), 0);
        assert_eq!(rv.view_of(3), 0); // a2 is inside the collapsed row
        assert_eq!(rv.view_of(2), 1);
        assert_eq!(rv.members(0), [1, 3]);
        assert_eq!(tr.unwrap().collapsed, [1]);
        // consensus of a1 (AC-T) and a2 (AA-T): A, A (tie A/C to the lower code), gap, T
        let cons: Vec<u8> = rv.clade_codes(0).iter().map(|&x| aln.symbol(x)).collect();
        assert_eq!(cons, b"AA-T");
        assert_eq!(rv.expanded(), [1, 3, 2, 0, 4]);
        // collapsing inside a collapsed clade changes nothing; file order ignores collapse
        c[2] = true;
        assert_eq!(row_view(&aln, &ix, &c, true).0.len(), 4);
        let (rv, tr) = row_view(&aln, &ix, &c, false);
        assert!(tr.is_none());
        assert_eq!(rv.expanded(), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn consensus_gap_majority() {
        let aln = parse_bytes(b">a\nA-C\n>b\n--C\n>c\n-GA\n").unwrap();
        // column 1 is 2 gaps of 3: a gap, though G is the only residue
        let cons: Vec<u8> = clade_consensus(&aln, &[0, 1, 2]).iter().map(|&x| aln.symbol(x)).collect();
        assert_eq!(cons, b"--C");
        assert_eq!(clade_consensus(&aln, &[1, 2]).iter().map(|&x| aln.symbol(x)).collect::<Vec<_>>(), b"-GA");
        assert_eq!(clade_consensus(&aln, &[0]).iter().map(|&x| aln.symbol(x)).collect::<Vec<_>>(), b"A-C");
    }

    #[test]
    fn stockholm_nh_tree() {
        let aln = parse_bytes(b"# STOCKHOLM 1.0\n#=GF ID x\n#=GF NH (a:0.1,\n#=GF NH  b:0.2);\na AC\nb AG\n//\n").unwrap();
        let t = Tree::parse(aln.newick.as_deref().unwrap()).unwrap();
        assert_eq!(leaves(&t), ["a", "b"]);
        assert!(parse_bytes(b"# STOCKHOLM 1.0\na AC\n//\n").unwrap().newick.is_none());
    }

    #[test]
    fn sniffing_and_siblings() {
        use std::path::Path;
        assert!(looks_like_tree(Path::new("x.nwk"), b""));
        assert!(looks_like_tree(Path::new("x.txt"), b"\n  (a,b);"));
        assert!(!looks_like_tree(Path::new("x.fa"), b">a\nACGT"));
        let c = sibling_candidates(Path::new("/d/PF00042.sto.gz"));
        assert_eq!(c[0], Path::new("/d/PF00042.sto.nwk"));
        assert!(c.contains(&Path::new("/d/PF00042.nwk").to_path_buf()));
        assert!(c.contains(&Path::new("/d/PF00042.treefile").to_path_buf()));
    }

    /// A generated 230,000-leaf tree: parse, match and lay out, timed (printed with
    /// `--nocapture`); also a check that it finishes at all in a debug build.
    #[test]
    fn large_random_tree() {
        let n = 230_000;
        let mut buf = Vec::new();
        crate::synth::write_newick(n, 32, 7, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let t0 = std::time::Instant::now();
        let t = Tree::parse(&text).unwrap();
        let parse = t0.elapsed();
        assert_eq!(t.leaf_count(), n);
        let names: Vec<String> = (0..n).map(|r| crate::synth::row_name(r, n, 32)).collect();
        let t1 = std::time::Instant::now();
        let ix = TreeIndex::new(t, &names, &[]);
        assert_eq!(ix.matching.matched, n);
        eprintln!("230k leaves: {} nodes, parse {parse:?}, match + index {:?}, {} MiB", ix.tree.len(), t1.elapsed(), ix.bytes() >> 20);
    }
}
