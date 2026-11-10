#!/usr/bin/env bash
# Put first on PATH as `pplacer`: GTDB-Tk calls `pplacer -m WAG -j N -c REFPKG -o OUT.json MSA`.
# This runs `gplace place` with the same arguments, a gPlace memory limit, and an enforced
# cgroup cap of the same size, measured by measure.sh. `pplacer --version` goes to the real one.
set -uo pipefail
if [[ ${1:-} == --version ]]; then exec "$REAL_PPLACER" --version; fi
n=$(( $(ls "$MEASURE_DIR"/gplace_call_*.time 2>/dev/null | wc -l) + 1 ))
refpkg=""; prev=""
for a in "$@"; do [[ $prev == -c ]] && refpkg=$a; prev=$a; done
echo "gplace_call_$n: $(basename "$refpkg") :: gplace place --max_memory $GPLACE_MAX_MEMORY $*" >> "$MEASURE_DIR/calls.txt"
exec "$MEASURE" "gplace_call_$n" --cap "$GPLACE_CGROUP_CAP" -- \
  gplace place --max_memory "$GPLACE_MAX_MEMORY" "$@"
