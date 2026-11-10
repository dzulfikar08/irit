# Problems reported upstream

Problems found in other projects' data or tools while building irit, and what was reported.

## FastQC accepts a FASTQ file cut off part way through its last line

Found while building leanqc's test corpus; reproduced on FastQC 0.12.1 and 0.13.0 (2026-11-10).

A plain-text FASTQ whose last entry ends part way through its quality line is processed as complete:
FastQC exits 0, writes a normal report and counts the cut-off entry as a full sequence (0.12.1 prints
a bare `null`; 0.13.0 prints nothing with `-q`). A truncated `.fastq.gz` is correctly rejected, the
case fixed for [FastQC#45](https://github.com/s-andrews/FastQC/issues/45). FastQC's reader catches
files that end between lines, not within one.

Reported as [FastQC#221](https://github.com/s-andrews/FastQC/issues/221), with a fix and an
integration test in [FastQC#222](https://github.com/s-andrews/FastQC/pull/222): reject an entry whose
quality string length differs from its sequence (colourspace files excepted). FastQC's full test
suite passes with the fix; the new test fails without it. FastQC-Rust reproduces FastQC's behaviour
here by design. leanqc refuses such files.

## Not reported: FastQC's quality-encoding guess

FastQC treats a file whose lowest quality character is `@` or above as Phred+64, so very
high-quality Phred+33 data is misread. This was already raised as
[FastQC#212](https://github.com/s-andrews/FastQC/issues/212); the maintainer called it a known
limitation of guessing, and FastQC 0.13.0 added a `--phred64` option instead. leanqc uses a
narrower rule (Phred+64 only when the highest character is between `J` and `i`) and has `--phred`.

## GTDB R232 mock database doesn't match its published checksum

Found by `dblock` on 2026-11-10.

`r232_mockup.tar.gz` (the small test database for GTDB-Tk R232) does not match the MD5 listed in
the release's `MD5SUM.txt`:

| | Value |
|---|---|
| MD5 in `MD5SUM.txt` | `2c710f35d7e6296615bcf9026ee1c951` |
| MD5 of the file served | `59512f60fa48010647977460bb855666` |
| Size | 26,324,952 bytes (as served) |
| `MD5SUM.txt` last modified | 2026-06-04 03:54 GMT |
| Tarball last modified | 2026-06-09 11:09 GMT (data.gtdb.ecogenomic.org), 11:08 GMT (data.ace.uq.edu.au) |

Both mirrors serve the same file. The two other files in the same folder (`HOWTO.txt`,
`test_genome_G021783475.fna.gz`) and the full R232 package entry match their checksums, so the
likely cause is that the tarball was rebuilt after `MD5SUM.txt` was generated. Anyone verifying the
download (as GTDB-Tk's docs suggest) gets a failure.

Reproduce:

```sh
curl -sL https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/mockup_db/r232_mockup.tar.gz | md5sum
curl -s https://data.gtdb.ecogenomic.org/releases/release232/232.0/MD5SUM.txt | grep r232_mockup
```

`dblock` keeps GTDB's published checksum in its catalog and refuses the file until one of the two
is updated. Status: reported 2026-11-10 as [Ecogenomics/GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726).

## gPlace: identical to pplacer, and an OOM kill before the `--max_memory` check

[cmc-aau/gPlace#5](https://github.com/cmc-aau/gPlace/issues/5) (2026-11-10). Shares the
[gPlace checks](gplace-checks.md): placements identical to pplacer for 59 genomes on the R232 ar53
and bac120 backbone trees, and the `--max_memory` floors (1.9G on class tree 8, 5.2G on the full
bac120 tree). It also reports that on the full tree gPlace loads about 4.8 GiB before checking
`--max_memory`, so a hard memory limit below that kills it instead of printing "needs --max_memory
5.1G". Summary on [GTDBTk#728](https://github.com/Ecogenomics/GTDBTk/issues/728#issuecomment-6769095352).
