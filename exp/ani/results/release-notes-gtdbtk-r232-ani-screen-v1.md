**Unofficial, derived data.** A screen index for running GTDB-Tk's ANI species screen without its 85 GB skani database, with [`gtdbprep ani`](https://github.com/dzulfikar08/irit/blob/main/docs/tools/gtdbprep.md) from [irit](https://github.com/dzulfikar08/irit). It is computed from GTDB release 232's GTDB-Tk reference package. It is not made or endorsed by the GTDB team.

- **Source:** [`gtdbtk_r232_data.tar.gz`](https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/full_package/gtdbtk_r232_data.tar.gz)
  (60,806,405,195 bytes, MD5 `25a59e0352b1fd150c589f56559767d4`, checked against GTDB's
  `MD5SUM.txt` while streaming), folder `release232/skani/database/` (skani 0.3.1 database of all
  199,923 species representatives; GTDB's `metadata.tsv` gives `sketches.db` MD5
  `c58e686b29bfc41c91b7e7be69e5c5c7`, `markers.bin` `6c719d7c95d9293e5f8669d5ec727ac4`, `index.db`
  `fcde0614e71f33cdfe9ede678a3f6206`).
- **What it is:** for every representative, its exact marker k-mer count, 32-bit fingerprints of
  its marker k-mers (Rice-coded), where its sketch sits in `sketches.db`, and a SHA-256 prefix of
  that sketch; plus a gzip seek index of GTDB's archive (a checkpoint every 4 MiB of
  `sketches.db`) so single sketches can be range-read from GTDB's own file. It contains no sketch
  and no genome sequence.
- **How it is used:** `gtdbprep ani` screens your genomes against it, keeps every reference skani
  would compare in full, range-reads only those sketches, and runs GTDB-Tk 2.8.0's own ANI-screen
  code on them. On test genomes the output was byte-identical to GTDB-Tk against the full
  database. Write-up: [The ANI species check on a laptop](https://github.com/dzulfikar08/irit/blob/main/docs/research/ani-laptop.md).
- **Reading sketches from GTDB's archive is a stopgap:** it downloads about 5.7 times the sketch
  bytes and loads GTDB's server. Keep a cache (`--cache`) and avoid large batches.
- **Licence:** derived from GTDB data, so [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)
  ([GTDB licences](https://gtdb.ecogenomic.org/licenses)), like the source.
- **Credit:** Parks, D.H., et al. and the GTDB team ([gtdb.ecogenomic.org](https://gtdb.ecogenomic.org));
  GTDB-Tk (Chaumeil, P.-A., et al.); skani (Shaw, J. & Yu, Y.W., *Nature Methods* 2023), which
  made the sketches and computes every ANI. Cite GTDB, GTDB-Tk and skani for results.

Fetch with [`dblock`](https://github.com/dzulfikar08/irit/blob/main/docs/tools/dblock.md):
`dblock add gtdbtk-r232-ani-screen && dblock fetch`. The three archives unpack into one
`ani-screen-r232/` folder. Built with gtdbprep 0.2.1 (`ani-index build`, `gz-scan`,
`gz-attach --every 4`) on a GitHub runner,
[run 42936739602](https://github.com/dzulfikar08/irit/actions/runs/42936739602).

Files: `SHA256SUMS` and `MD5SUMS` (archives).

## Assets

| Archive | Size (bytes) | Unpacked | Contents |
|---|---:|---:|---|
| `gtdbtk-r232-ani-screen-core.tar.zst` | 5,705,190 | `meta.json` 2,583 + `refs.bin` 8,196,843 | index metadata; one 41-byte row per reference |
| `gtdbtk-r232-ani-screen-markers.tar.zst` | 1,748,864,709 | `markers.idx` 1,748,824,109 | fingerprints of 644,245,721 marker k-mers |
| `gtdbtk-r232-ani-screen-gzindex.tar.zst` | 347,291,901 | `gz.idx` 571,990,880 | 17,443 checkpoints into GTDB's archive |
