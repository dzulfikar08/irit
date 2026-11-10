//! Residue codes. Every residue is stored as a small code (< 32) so histograms stay tiny.
//! DNA and RNA use 16 codes (gap + 15 IUPAC symbols, T or U) and pack two per byte; an alignment
//! that has both T and U keeps them apart with a 17th code and one byte per residue. Nucleotides
//! with hard-masked `X` or a `*` also take one byte per residue, so those are stored as written.
//! Colours are in [`crate::scheme`]; this module only maps bytes to codes and back.

pub const GAP: u8 = 0;
pub const NCODES: usize = 32;
const INVALID: u8 = 0xFF;

/// Residue type chosen by the user instead of guessed from the residues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqType {
    Dna,
    Rna,
    Protein,
}

impl SeqType {
    pub fn kind(self) -> Kind {
        if self == SeqType::Protein { Kind::Protein } else { Kind::Nucleotide }
    }
}

impl std::str::FromStr for SeqType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "dna" => Ok(SeqType::Dna),
            "rna" => Ok(SeqType::Rna),
            "protein" | "aa" => Ok(SeqType::Protein),
            _ => Err(format!("unknown sequence type {s:?}: expected dna, rna or protein")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Nucleotide,
    Protein,
}

const DNA_SYMBOLS: &[u8] = b"-ACGTRYSWKMBDHVN";
const RNA_SYMBOLS: &[u8] = b"-ACGURYSWKMBDHVN";
const NT_MIXED_SYMBOLS: &[u8] = b"-ACGTRYSWKMBDHVNUX*";
const DNA_WIDE_SYMBOLS: &[u8] = b"-ACGTRYSWKMBDHVNX*";
const RNA_WIDE_SYMBOLS: &[u8] = b"-ACGURYSWKMBDHVNX*";
const AA_SYMBOLS: &[u8] = b"-ACDEFGHIKLMNPQRSTVWYBZXJUO*";

pub struct Alphabet {
    pub kind: Kind,
    pub name: &'static str,
    /// Bits per stored residue: 4 (two per byte) or 8.
    pub bits: u32,
    enc: [u8; 256],
    symbols: &'static [u8],
}

/// DNA: T, no U. Four bits per residue.
pub static DNA: Alphabet = Alphabet::new(Kind::Nucleotide, "DNA", DNA_SYMBOLS, 4);
/// RNA: U, no T. Four bits per residue.
pub static RNA: Alphabet = Alphabet::new(Kind::Nucleotide, "RNA", RNA_SYMBOLS, 4);
/// Nucleotides with both T and U, kept apart (U is code 16). One byte per residue.
pub static NT_MIXED: Alphabet = Alphabet::new(Kind::Nucleotide, "DNA/RNA", NT_MIXED_SYMBOLS, 8);
/// DNA that also has hard-masked `X` or `*` (no 4-bit code is free for them). One byte per residue.
pub static DNA_WIDE: Alphabet = Alphabet::new(Kind::Nucleotide, "DNA", DNA_WIDE_SYMBOLS, 8);
/// RNA that also has `X` or `*`. One byte per residue.
pub static RNA_WIDE: Alphabet = Alphabet::new(Kind::Nucleotide, "RNA", RNA_WIDE_SYMBOLS, 8);
pub static PROTEIN: Alphabet = Alphabet::new(Kind::Protein, "protein", AA_SYMBOLS, 8);

impl Alphabet {
    const fn new(kind: Kind, name: &'static str, symbols: &'static [u8], bits: u32) -> Self {
        let mut enc = [INVALID; 256];
        let mut i = 0;
        while i < symbols.len() {
            let s = symbols[i];
            enc[s as usize] = i as u8;
            enc[s.to_ascii_lowercase() as usize] = i as u8;
            i += 1;
        }
        enc[b'.' as usize] = GAP;
        enc[b'~' as usize] = GAP;
        enc[b'?' as usize] = match kind {
            Kind::Nucleotide => 15, // N
            Kind::Protein => 23,    // X
        };
        Alphabet { kind, name, bits, enc, symbols }
    }

    #[inline]
    pub fn encode(&self, b: u8) -> Option<u8> {
        let c = self.enc[b as usize];
        (c != INVALID).then_some(c)
    }

    #[inline]
    pub fn symbol(&self, code: u8) -> u8 {
        self.symbols.get(code as usize).copied().unwrap_or(b'?')
    }

    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// The alphabet to move to when `self` rejects byte `b`: T in RNA or U in DNA keeps both apart,
    /// `X` or `*` in 4-bit DNA/RNA moves to its one-byte form, any other protein letter means
    /// protein. `None` if no alphabet takes `b`.
    pub fn widen(&self, b: u8) -> Option<&'static Alphabet> {
        let u = b.to_ascii_uppercase();
        let is = |a: &Alphabet| std::ptr::eq(self, a);
        if self.kind == Kind::Protein {
            None
        } else if ((is(&DNA) || is(&DNA_WIDE)) && u == b'U') || ((is(&RNA) || is(&RNA_WIDE)) && u == b'T') {
            Some(&NT_MIXED)
        } else if matches!(u, b'X' | b'*') && (is(&DNA) || is(&RNA)) {
            Some(if is(&DNA) { &DNA_WIDE } else { &RNA_WIDE })
        } else {
            PROTEIN.encode(b).map(|_| &PROTEIN)
        }
    }
}

impl std::fmt::Debug for Alphabet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

impl PartialEq for Alphabet {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

// Byte classes for `Census`.
const SKIP: u8 = 0; // gap, whitespace, digit
const BASE: u8 = 1; // A, C, G, N
const T: u8 = 2;
const U: u8 = 3;
const AMBIG: u8 = 4; // other IUPAC nucleotide codes
const MASK: u8 = 5; // X, '*' and '?': unknown or masked in both DNA and protein
const AA_ONLY: u8 = 6; // valid only in protein
const OTHER: u8 = 7; // rejected by every alphabet (the encoder reports it)

static CLASS: [u8; 256] = {
    let mut t = [OTHER; 256];
    let mut b = 0;
    while b < 256 {
        let x = (b as u8).to_ascii_uppercase();
        t[b] = match x {
            b'-' | b'.' | b'~' | b'0'..=b'9' | b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C => SKIP,
            b'A' | b'C' | b'G' | b'N' => BASE,
            b'T' => T,
            b'U' => U,
            b'R' | b'Y' | b'S' | b'W' | b'K' | b'M' | b'B' | b'D' | b'H' | b'V' => AMBIG,
            b'X' | b'*' | b'?' => MASK,
            _ if PROTEIN.enc[x as usize] != INVALID => AA_ONLY,
            _ => OTHER,
        };
        b += 1;
    }
    t
};

/// Residue counts used to choose the alphabet. Gaps are not residues.
#[derive(Clone, Copy, Debug, Default)]
pub struct Census {
    pub residues: usize,
    /// A, C, G, T, U and N.
    pub plain: usize,
    pub t: usize,
    pub u: usize,
    /// X, `*` and `?`: they say nothing about the residue type.
    pub masked: usize,
    /// Some residue is X or `*` (which the 4-bit nucleotide codes can't hold).
    pub x_or_stop: bool,
    /// A letter that only an amino acid can be.
    pub protein_only: bool,
}

/// Share of informative residues (not gaps, X, `*` or `?`) that must be A, C, G, T, U or N for
/// an alignment to be read as nucleotides. Proteins rich in A, G, S, T, C, R, Y, K, M, W, D, H or V
/// (silk fibroin, antifreeze glycoproteins, glycine-rich and cysteine-rich peptides) can have no
/// protein-only letter at all, but they are far below this share; real DNA, even with IUPAC
/// ambiguity codes, is far above it. `--type` overrides the guess.
pub const NUCLEOTIDE_SHARE: f64 = 0.9;

impl Census {
    /// Count the residues of `seq`, stopping after `limit` of them. Returns how many were counted.
    pub fn add(&mut self, seq: &[u8], limit: usize) -> usize {
        let mut n = 0;
        for &b in seq {
            if n == limit {
                break;
            }
            self.add_byte(b, 1);
            n += (CLASS[b as usize] != SKIP && CLASS[b as usize] != OTHER) as usize;
        }
        n
    }

    /// Count `n` occurrences of byte `b`.
    #[inline]
    pub fn add_byte(&mut self, b: u8, n: usize) {
        match CLASS[b as usize] {
            SKIP | OTHER => return,
            BASE => self.plain += n,
            T => {
                self.plain += n;
                self.t += n
            }
            U => {
                self.plain += n;
                self.u += n
            }
            AMBIG => {}
            MASK => {
                self.masked += n;
                self.x_or_stop |= b != b'?';
            }
            _ => self.protein_only = true,
        }
        self.residues += n;
    }

    /// Nucleotides when no residue is protein-only and at least [`NUCLEOTIDE_SHARE`] of the
    /// informative residues are A, C, G, T, U or N; otherwise protein. X, `*` and `?` don't count
    /// either way (hard-masked DNA is common), except that residues made only of X and `*` are
    /// protein. T and U together are kept apart.
    pub fn alphabet(&self) -> &'static Alphabet {
        let informative = self.residues - self.masked;
        let nucleotide = if informative == 0 {
            !self.x_or_stop
        } else {
            !self.protein_only && self.plain as f64 >= NUCLEOTIDE_SHARE * informative as f64
        };
        if !nucleotide {
            &PROTEIN
        } else if self.t > 0 && self.u > 0 {
            &NT_MIXED
        } else if self.u > 0 {
            if self.x_or_stop { &RNA_WIDE } else { &RNA }
        } else if self.x_or_stop {
            &DNA_WIDE
        } else {
            &DNA
        }
    }

    /// Like [`Census::alphabet`], but with the residue type given by the user (`--type`): the
    /// census then only decides between the nucleotide alphabets (T, U, X and `*`).
    pub fn alphabet_as(&self, ty: Option<SeqType>) -> &'static Alphabet {
        let Some(ty) = ty else { return self.alphabet() };
        match ty {
            SeqType::Protein => &PROTEIN,
            _ if self.t > 0 && self.u > 0 => &NT_MIXED,
            _ if self.u > 0 || (ty == SeqType::Rna && self.t == 0) => {
                if self.x_or_stop { &RNA_WIDE } else { &RNA }
            }
            _ if self.x_or_stop => &DNA_WIDE,
            _ => &DNA,
        }
    }

    pub fn of(seq: &[u8]) -> Self {
        let mut c = Census::default();
        c.add(seq, usize::MAX);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_types() {
        assert_eq!(Census::of(b"ACGT--NNacgt").alphabet(), &DNA);
        assert_eq!(Census::of(b"ACGU--NNacgu").alphabet(), &RNA);
        assert_eq!(Census::of(b"ACGU--NNacgt").alphabet(), &NT_MIXED);
        assert_eq!(Census::of(b"MKVLAAGIVE-").alphabet(), &PROTEIN);
        assert_eq!(Census::of(b"ACGTRYKM").alphabet(), &PROTEIN); // half ambiguity codes: not a DNA alignment
        // 6 ambiguity codes in 66 residues (91% A/C/G/T): still DNA
        let dna = [&b"--ACRYKMSW-ACGT--"[..], &b"ACGTACGTAC".repeat(5)].concat();
        assert_eq!(Census::of(&dna).alphabet(), &DNA);
        // 6 in 30 (80%): protein
        assert_eq!(Census::of(b"--ACRYKMSW-ACGT--AAACGTACGTACGTACGT").alphabet(), &PROTEIN);
        assert_eq!(Census::of(b"--------").alphabet(), &DNA);
        let mut c = Census::default();
        assert_eq!(c.add(b"--AC-GT", 3), 3);
        assert_eq!((c.residues, c.plain), (3, 3));
    }

    /// Real proteins with no protein-only letter (reviewer repros): all came out DNA under the
    /// old "no protein-only letter and more than half A/C/G/T/U/N" rule.
    #[test]
    fn proteins_without_protein_only_letters() {
        let silk = b"GAGAGSGAGAGSGAGAGYGAGAGS GAGAGSGAGAGSGAGAGSGAGAGS GAGAGYGAGAGSGAGAGSGAGAGT";
        let glycine = b"MGSGGGRGGGSGGRGYGGGGRGGG MGSGGGRGGGNGGRGYGGGGRGGG";
        let cys = b"GCKGKGAKCSRTMYDCCTGSCRSGKC";
        for (name, s) in [("silk fibroin", &silk[..]), ("glycine-rich", glycine), ("Cys-rich", cys)] {
            assert_eq!(Census::of(s).alphabet(), &PROTEIN, "{name}");
        }
        assert!(!Census::of(silk).protein_only && !Census::of(cys).protein_only);
        // (Ala-Ala-Thr)n antifreeze glycoprotein is letter for letter the (AAT)n DNA
        // microsatellite: no composition rule can tell them apart, so it needs `--type protein`.
        let afgp = b"AATAATAATAATAATAATAATAAT";
        assert_eq!(Census::of(afgp).alphabet(), &DNA);
        assert_eq!(Census::of(afgp).alphabet_as(Some(SeqType::Protein)), &PROTEIN);
    }

    #[test]
    fn type_override() {
        let silk = Census::of(b"GAGAGSGAGAGS");
        assert_eq!(silk.alphabet_as(Some(SeqType::Dna)), &DNA);
        assert_eq!(silk.alphabet_as(Some(SeqType::Rna)), &RNA);
        assert_eq!(Census::of(b"ACGTXX").alphabet_as(Some(SeqType::Rna)), &DNA_WIDE); // T wins over the flag
        assert_eq!(Census::of(b"ACGU").alphabet_as(Some(SeqType::Dna)), &RNA);
        assert_eq!(Census::of(b"ACGT").alphabet_as(Some(SeqType::Protein)), &PROTEIN);
        assert_eq!(Census::of(b"ACGT").alphabet_as(None), &DNA);
        assert_eq!("Protein".parse::<SeqType>(), Ok(SeqType::Protein));
        assert!("nt".parse::<SeqType>().is_err());
    }

    #[test]
    fn masked_and_stop() {
        // hard-masked DNA stays DNA, stored one byte per residue so X and * survive
        assert_eq!(Census::of(b"ACGTXXXXXXXXACGTACGT").alphabet(), &DNA_WIDE);
        assert_eq!(Census::of(b"ACGTACGTAC*").alphabet(), &DNA_WIDE);
        assert_eq!(Census::of(b"ACGUXXACGU").alphabet(), &RNA_WIDE);
        assert_eq!(Census::of(b"ACGUXXACGT").alphabet(), &NT_MIXED);
        assert_eq!(Census::of(b"ACGT??ACGT").alphabet(), &DNA); // '?' is stored as N
        // a protein with a stop stays protein; only X / * is protein (X is the usual unknown residue)
        assert_eq!(Census::of(b"GAGAGSGAGAGS*").alphabet(), &PROTEIN);
        assert_eq!(Census::of(b"MKVLE*").alphabet(), &PROTEIN);
        assert_eq!(Census::of(b"XXXX*").alphabet(), &PROTEIN);
        for a in [&DNA_WIDE, &RNA_WIDE, &NT_MIXED] {
            assert_eq!(a.symbol(a.encode(b'x').unwrap()), b'X');
            assert_eq!(a.symbol(a.encode(b'*').unwrap()), b'*');
        }
    }

    #[test]
    fn round_trips() {
        for a in [&DNA, &RNA, &NT_MIXED, &DNA_WIDE, &RNA_WIDE, &PROTEIN] {
            for i in 0..a.len() as u8 {
                assert_eq!(a.encode(a.symbol(i)), Some(i), "{a:?}");
            }
            assert!(a.len() <= NCODES);
            assert_eq!(a.encode(b'.'), Some(GAP));
        }
        assert_eq!(DNA.encode(b'u'), None);
        assert_eq!(RNA.encode(b't'), None);
        assert_eq!(RNA.encode(b'u'), DNA.encode(b'T'));
        assert_eq!(NT_MIXED.symbol(NT_MIXED.encode(b'u').unwrap()), b'U');
        assert_eq!(DNA.encode(b'E'), None);
        assert_eq!((DNA.symbol(DNA.encode(b'?').unwrap()), PROTEIN.symbol(PROTEIN.encode(b'?').unwrap())), (b'N', b'X'));
    }

    #[test]
    fn widens() {
        assert_eq!(DNA.widen(b'u'), Some(&NT_MIXED));
        assert_eq!(RNA.widen(b'T'), Some(&NT_MIXED));
        assert_eq!(DNA.widen(b'E'), Some(&PROTEIN));
        assert_eq!(NT_MIXED.widen(b'E'), Some(&PROTEIN));
        assert_eq!(DNA.widen(b'x'), Some(&DNA_WIDE));
        assert_eq!(RNA.widen(b'*'), Some(&RNA_WIDE));
        assert_eq!(DNA_WIDE.widen(b'U'), Some(&NT_MIXED));
        assert_eq!(RNA_WIDE.widen(b'T'), Some(&NT_MIXED));
        assert_eq!(DNA_WIDE.widen(b'E'), Some(&PROTEIN));
        assert_eq!(DNA.widen(b'%'), None);
        assert_eq!(PROTEIN.widen(b'%'), None);
    }
}
