"""Simulated relative-abundance table in coverm genome layout, for checking cooccur.

usage: python3 simulate.py GENOMES SAMPLES PAIRS NOISE DETECTION SEED OUT.tsv

Absolute abundances are log-normal (sd 1.5 on the natural-log scale), independent between genomes,
except PAIRS planted pairs: genome 2i+1 = 3 x genome 2i x log-normal noise (sd NOISE). Each sample
is closed to 100 %, and values below DETECTION (%) become zeros.
"""
import math, random, sys

k, n, pairs = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
noise, det, seed, out = float(sys.argv[4]), float(sys.argv[5]), int(sys.argv[6]), sys.argv[7]
r = random.Random(seed)
x = [[math.exp(r.gauss(0, 1.5)) for _ in range(n)] for _ in range(k)]
for p in range(pairs):
    for s in range(n):
        x[2 * p + 1][s] = 3 * x[2 * p][s] * math.exp(r.gauss(0, noise))
for s in range(n):
    t = sum(g[s] for g in x)
    for g in x:
        g[s] = 100 * g[s] / t
        if g[s] < det:
            g[s] = 0.0
with open(out, "w") as f:
    f.write("Genome\t" + "\t".join(f"S{s}.fq.gz Relative Abundance (%)" for s in range(n)) + "\n")
    for i, g in enumerate(x):
        f.write(f"g{i}\t" + "\t".join(f"{v:.6g}" for v in g) + "\n")
