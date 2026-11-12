#!/usr/bin/env python3
"""Write a synthetic long-read FASTQ for timing leanqc on long reads (nanopore-like).

Usage: make_long_fastq.py OUT.fastq GIGABYTES [SEED]

Read lengths are log-normal (median 5 kb, a long tail, capped at 300 kb); each read gets a mean
quality between Q6 and Q25 and per-base scores scattered around it. Sequence and qualities are
sliced from random pools, so the file is quick to make and not meant to look like a genome.
"""
import math
import random
import sys


def main():
    out, gb = sys.argv[1], float(sys.argv[2])
    rng = random.Random(int(sys.argv[3]) if len(sys.argv) > 3 else 1)
    target = int(gb * 1e9)
    pool = "".join(rng.choices("ACGT", k=4_000_000)).encode()
    # Per mean quality, a pool of scores scattered around it (Phred+33).
    qpools = {}
    for q in range(6, 26):
        qpools[q] = bytes(33 + max(1, min(50, int(rng.gauss(q, 4)))) for _ in range(1_000_000))
    written = 0
    n = 0
    with open(out, "wb", buffering=1 << 22) as f:
        while written < target:
            n += 1
            length = min(300_000, max(1, int(math.exp(rng.gauss(math.log(5000), 1.0)))))
            s = rng.randrange(0, len(pool) - length) if length < len(pool) else 0
            seq = pool[s : s + length]
            qp = qpools[rng.randint(6, 25)]
            o = rng.randrange(0, len(qp) - length)
            rec = b"@read%d\n%s\n+\n%s\n" % (n, seq, qp[o : o + length])
            f.write(rec)
            written += len(rec)
    print(f"{n} reads, {written} bytes", file=sys.stderr)


if __name__ == "__main__":
    main()
