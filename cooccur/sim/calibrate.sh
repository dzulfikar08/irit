#!/bin/sh
# Share of independent pairs (no planted pairs) with p <= 0.01 and <= 0.05, with a fixed 2,000
# permutations per pair. usage: calibrate.sh GENOMES SAMPLES REPS METHOD
set -e
B=${COOCCUR:-cooccur}; S=$(dirname "$0")/simulate.py; D=$(mktemp -d)
: > "$D/all.tsv"
for rep in $(seq 1 "$3"); do
  python3 -I "$S" "$1" "$2" 0 0.3 0.01 $((rep + 100)) "$D/t.tsv"
  "$B" "$D/t.tsv" --method "$4" --all-pairs --permutations 2000 --max-permutations 2000 --stop-after 2001 2>/dev/null | tail -n +2 >> "$D/all.tsv"
done
awk -F'\t' -v k="$1" -v n="$2" -v m="$4" '{ c++; if ($4<=0.01) a++; if ($4<=0.05) b++; s+=$3 }
  END { printf "%s, genomes %d, samples %d, %d pairs: p<=0.01 %.4f, p<=0.05 %.4f, mean statistic %.4f\n", m, k, n, c, a/c, b/c, s/c }' "$D/all.tsv"
rm -r "$D"
