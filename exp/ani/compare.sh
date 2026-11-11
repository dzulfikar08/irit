#!/usr/bin/env bash
# compare.sh REF_OUT TEST_OUT: GTDB-Tk ANI-screen outputs must match.
# ani_summary.tsv files byte for byte; the unassigned-hits file after sorting its rows (GTDB-Tk
# writes them in set order) and ignoring the gzip header.
set -uo pipefail
ref=$1/classify/ani_screen; test=$2/classify/ani_screen
status=0
for f in gtdbtk.bac120.ani_summary.tsv gtdbtk.ar53.ani_summary.tsv; do
  if [[ ! -e $ref/$f && ! -e $test/$f ]]; then echo "$f: absent in both"; continue; fi
  if cmp -s "$ref/$f" "$test/$f"; then echo "$f: identical ($(($(wc -l < "$ref/$f") - 1)) rows)"; else echo "$f: DIFFERENT"; diff "$ref/$f" "$test/$f" | head -20; status=1; fi
done
f=gtdbtk.ani_screen_unassigned_hits.tsv.gz
if cmp -s <(zcat "$ref/$f" | sort) <(zcat "$test/$f" | sort); then echo "$f: identical rows ($(($(zcat "$ref/$f" | wc -l) - 1)))"; else echo "$f: DIFFERENT"; diff <(zcat "$ref/$f" | sort) <(zcat "$test/$f" | sort) | head -20; status=1; fi
exit $status
