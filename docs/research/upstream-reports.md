# Problems reported upstream

Problems found in other projects' data or tools while building leanbio, and what was reported.

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
is updated. Status: reported to the GTDB-Tk issue tracker (link below once filed).
