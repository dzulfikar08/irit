# ANI on a laptop: raw results

The raw outputs behind the measured numbers in
[docs/research/ani-laptop.md](../../../docs/research/ani-laptop.md), downloaded from the
`ani-laptop` and `ani-index-pack` workflow artifacts (10–11 Nov 2026) before they expired. Kept
here so the measurements can be checked without rerunning the 60.8 GB stream.

- `q1/`, `p1/`: `ani-laptop` runs, part 1: GTDB-Tk's ANI screen on the full database (skani 0.3.1
  and 0.4.0), the local index build, the client against a local `sketches.db`, timings, disk use.
- `p2/`, `p4/`: later parts: the client reading sketches from GTDB's server through the seek index
  (checkpoints every 4 and 16 MiB), and the over-50-genome run (skani's inverted-index path).
- `w1`, `w2`, `w3`, `wreal`, `w51a`, `w51b`: client work folders (assigned references,
  `ani_screen` logs and outputs) for the test genome sets; sketches, databases and genomes removed.
- `release-notes-gtdbtk-r232-ani-screen-v1.md`: the notes of the published index
  (`dzulfikar08/irit-data`, tag `gtdbtk-r232-ani-screen-v1`).

Files over 1 MB, genomes, sketches and databases were left out; the index itself is published.
