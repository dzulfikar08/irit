#!/usr/bin/env python3
"""Write the synthetic FASTQ corpus leanqc is compared with FastQC on.

Usage: make_fastq_corpus.py OUT_DIR

Every file is generated from a fixed seed, so rerunning gives the same bytes. Each one targets
a behaviour that real data rarely isolates; see docs/benchmarks.md#synthetic-corpus.
"""

import gzip
import io
import random
import sys
from pathlib import Path


def rand_seq(rng, n, gc=0.5):
    out = []
    for _ in range(n):
        if rng.random() < gc:
            out.append(rng.choice("GC"))
        else:
            out.append(rng.choice("AT"))
    return "".join(out)


def phred33(qs):
    return "".join(chr(33 + max(0, min(41, q))) for q in qs)


def quals(rng, n, mean=36, spread=3):
    return [round(rng.gauss(mean, spread)) for _ in range(n)]


def record(name, seq, qual):
    return f"@{name}\n{seq}\n+\n{qual}\n"


def write_gz(path, text):
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0, filename="") as g:
        g.write(text.encode())
    path.write_bytes(buf.getvalue())


ADAPTER = "AGATCGGAAGAGCACACGTCTGAACTCCAGTCAC"


def varied_lengths(rng):
    out = []
    for i in range(4000):
        n = rng.randint(35, 250)
        out.append(record(f"r{i}", rand_seq(rng, n), phred33(quals(rng, n))))
    return out


def hiq_phred33(rng):
    # Every quality between '@' (Q31) and 'I' (Q40): valid Phred+33 that FastQC reads as Phred+64.
    out = []
    for i in range(2000):
        q = "".join(chr(rng.randint(ord("@"), ord("I"))) for _ in range(100))
        out.append(record(f"r{i}", rand_seq(rng, 100), q))
    return out


def phred64(rng):
    # Old Illumina 1.5 style: offset 64, 'B' (Q2) up to 'h' (Q40).
    out = []
    for i in range(2000):
        q = "".join(chr(64 + max(2, min(40, round(rng.gauss(34, 4))))) for _ in range(100))
        out.append(record(f"r{i}", rand_seq(rng, 100), q))
    return out


def adapters(rng):
    # 15% of reads run into the Illumina adapter (short inserts), 3% end in polyG.
    out = []
    for i in range(3000):
        r = rng.random()
        if r < 0.15:
            insert = rng.randint(30, 90)
            s = rand_seq(rng, insert) + (ADAPTER * 3)[: 100 - insert]
        elif r < 0.18:
            k = rng.randint(60, 90)
            s = rand_seq(rng, k) + "G" * (100 - k)
        else:
            s = rand_seq(rng, 100)
        out.append(record(f"r{i}", s, phred33(quals(rng, 100))))
    return out


def odd_headers(rng):
    # Starts with CASAVA 1.8 headers (tiles), then names with no tile, odd colons and spaces.
    out = []
    styles = [
        lambda i: f"M001:7:FC:1:{1101 + i % 4}:{i}:{i} 1:N:0:ACGT",
        lambda i: f"read {i} extra words",
        lambda i: f"x:y:{i}",
        lambda i: "::::::",
        lambda i: f"SRR000{i}.1 {i} length=75",
    ]
    for i in range(2000):
        name = styles[0](i) if i < 1000 else styles[i % len(styles)](i)
        out.append(record(name, rand_seq(rng, 75), phred33(quals(rng, 75))))
    return out


def long_reads(rng):
    # 5-6 kb reads: Q40 for the first 1,000 bases, Q2 after.
    out = []
    for i in range(150):
        n = rng.randint(5000, 6000)
        q = "I" * 1000 + "#" * (n - 1000)
        out.append(record(f"long{i}", rand_seq(rng, n), q))
    return out


def n_rich(rng):
    # Position 10: N in 60% of reads; of the rest, 70% A and 30% T. A/T imbalance counted over
    # A+C+G+T (FastQC) is 40 points; counted over all bases it would be 16.
    out = []
    for i in range(2000):
        s = list(rand_seq(rng, 50))
        r = rng.random()
        s[9] = "N" if r < 0.6 else ("A" if r < 0.88 else "T")
        out.append(record(f"r{i}", "".join(s), phred33(quals(rng, 50))))
    return out


def gc_bimodal(rng):
    out = []
    for i in range(3000):
        gc = 0.35 if rng.random() < 0.7 else 0.65
        out.append(record(f"r{i}", rand_seq(rng, 100, gc), phred33(quals(rng, 100))))
    return out


def duplicates(rng):
    common = [rand_seq(rng, 100) for _ in range(20)]
    out = []
    for i in range(3000):
        s = rng.choice(common) if rng.random() < 0.4 else rand_seq(rng, 100)
        out.append(record(f"r{i}", s, phred33(quals(rng, 100))))
    return out


def low_quality_tail(rng):
    out = []
    for i in range(2000):
        qs = quals(rng, 120) + [round(rng.gauss(36 - 1.0 * k, 3)) for k in range(30)]
        out.append(record(f"r{i}", rand_seq(rng, 150), phred33(qs)))
    return out


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    files = {
        "varied_lengths": varied_lengths,
        "hiq_phred33": hiq_phred33,
        "phred64": phred64,
        "adapters": adapters,
        "odd_headers": odd_headers,
        "long_reads": long_reads,
        "n_rich": n_rich,
        "gc_bimodal": gc_bimodal,
        "duplicates": duplicates,
        "low_quality_tail": low_quality_tail,
    }
    for i, (name, make) in enumerate(files.items()):
        rng = random.Random(1000 + i)
        write_gz(out / f"{name}.fastq.gz", "".join(make(rng)))
    # Broken inputs: a gzip stream cut in half, and a plain file whose last record has no quality.
    rng = random.Random(99)
    text = "".join(record(f"r{i}", rand_seq(rng, 75), phred33(quals(rng, 75))) for i in range(2000))
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0, filename="") as g:
        g.write(text.encode())
    data = buf.getvalue()
    (out / "truncated_gzip.fastq.gz").write_bytes(data[: len(data) // 2])
    lines = text.splitlines(keepends=True)
    (out / "truncated_record.fastq").write_text("".join(lines[: 4 * 500 + 3]))


if __name__ == "__main__":
    main()
