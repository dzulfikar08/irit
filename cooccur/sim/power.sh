#!/bin/sh
# Planted pairs found and false pairs, over REPS simulated tables per sample count.
# usage: power.sh GENOMES PAIRS NOISE REPS SAMPLES...   (extra cooccur options in $EXTRA)
# The cooccur binary is $COOCCUR (default: cooccur on PATH); scratch goes in $TMPDIR.
set -e
B=${COOCCUR:-cooccur}; S=$(dirname "$0")/simulate.py; D=$(mktemp -d)
k=$1; pairs=$2; noise=$3; reps=$4; shift 4
for n in "$@"; do
  : > "$D/res.txt"
  for rep in $(seq 1 "$reps"); do
    python3 -I "$S" "$k" "$n" "$pairs" "$noise" 0.01 "$rep" "$D/t.tsv"
    # shellcheck disable=SC2086
    "$B" "$D/t.tsv" $EXTRA -o "$D/e.tsv" 2>/dev/null
    awk -F'\t' -v P="$pairs" 'NR>1 { a=substr($1,2)+0; b=substr($2,2)+0; if (a>b) {c=a; a=b; b=c}
      if (a%2==0 && b==a+1 && a<2*P) t++; else f++ } END { print t+0, f+0 }' "$D/e.tsv" >> "$D/res.txt"
  done
  awk -v k="$k" -v P="$pairs" -v noise="$noise" -v n="$n" -v reps="$reps" -v x="$EXTRA" '
    { t+=$1; f+=$2; if ($1+$2>0) fdp+=$2/($1+$2); if ($2>0) any++ }
    END { printf "genomes %d, samples %d, noise %s %s: found %d of %d planted, %d false, tables with a false pair %d of %d, mean FDP %.3f\n", k, n, noise, x, t, P*reps, f, any, reps, fdp/reps }' "$D/res.txt"
done
rm -r "$D"
