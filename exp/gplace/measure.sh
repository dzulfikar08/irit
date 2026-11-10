#!/usr/bin/env bash
# measure.sh NAME [--cap BYTES] -- CMD...
# Runs CMD in its own cgroup v2 group and records: GNU time -v (max RSS of the largest process),
# the cgroup's memory.peak (all processes, incl. page cache) and the peak of anon memory
# (all processes, no page cache) sampled every 0.5 s. With --cap, sets memory.max and
# memory.swap.max=0 so going over the cap is an OOM kill, not a slowdown.
# Also samples the sum of VmRSS over all processes in the group (KiB).
set -uo pipefail
name=$1; shift
cap=""
if [[ ${1:-} == --cap ]]; then cap=$2; shift 2; fi
[[ ${1:-} == -- ]] && shift
out=${MEASURE_DIR:-$PWD/measure}
mkdir -p "$out"
cg=/sys/fs/cgroup/m_${name}_$$
sudo mkdir "$cg"
if [[ -n $cap ]]; then
  echo "$cap" | sudo tee "$cg/memory.max" >/dev/null
  echo 0 | sudo tee "$cg/memory.swap.max" >/dev/null
fi
(
  peak_anon=0; peak_rss=0
  while [[ -d $cg ]]; do
    a=$(awk '$1=="anon"{print $2}' "$cg/memory.stat" 2>/dev/null || echo 0)
    [[ -n $a && $a -gt $peak_anon ]] && { peak_anon=$a; echo "$peak_anon" > "$out/$name.peak_anon"; }
    # sum of VmRSS (KiB) over every process in the group (counts pages shared after fork twice)
    r=$(for p in $(cat "$cg/cgroup.procs" 2>/dev/null); do awk '/^VmRSS/{print $2}' /proc/$p/status 2>/dev/null; done | awk '{s+=$1} END{print s+0}')
    [[ -n $r && $r -gt $peak_rss ]] && { peak_rss=$r; echo "$peak_rss" > "$out/$name.peak_rss_sum"; }
    sleep 0.5
  done
) &
sampler=$!
start=$(date +%s.%N)
bash -c 'echo $$ | sudo tee '"$cg"'/cgroup.procs >/dev/null; exec /usr/bin/time -v -o '"$out/$name.time"' "$@"' _ "$@"
rc=$?
end=$(date +%s.%N)
peak=$(cat "$cg/memory.peak" 2>/dev/null || echo NA)
oom=$(awk '$1=="oom_kill"{print $2}' "$cg/memory.events" 2>/dev/null || echo NA)
sleep 1
# move any leftovers out, then remove the group (stops the sampler)
for p in $(cat "$cg/cgroup.procs" 2>/dev/null); do echo "$p" | sudo tee /sys/fs/cgroup/cgroup.procs >/dev/null || true; done
sudo rmdir "$cg" || true
wait $sampler 2>/dev/null
pa=$(cat "$out/$name.peak_anon" 2>/dev/null || echo 0)
rss=$(awk -F': ' '/Maximum resident set size/{print $2}' "$out/$name.time" 2>/dev/null)
wall=$(awk "BEGIN{printf \"%.1f\", $end-$start}")
rs=$(cat "$out/$name.peak_rss_sum" 2>/dev/null || echo 0)
line="| $name | $rc | $wall | ${rss:-NA} | ${rs} | $(( ${pa:-0} / 1024 )) | $(( ${peak/NA/0} / 1024 )) | ${oom} | ${cap:-none} |"
echo "$line" | tee -a "$out/table.md"
exit $rc
