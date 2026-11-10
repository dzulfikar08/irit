#!/usr/bin/env bash
# Run FastQC 0.12.1 on the synthetic corpus and record its verdicts, for leanqc's corpus test.
#
# Usage: scripts/fastqc_corpus.sh            (from the repository root; needs Docker)
# Writes leanqc/tests/corpus/fastqc_verdicts.tsv: file, module, verdict (or `error` when
# FastQC could not read the file).
set -euo pipefail
image=quay.io/biocontainers/fastqc:0.12.1--hdfd78af_0
corpus=leanqc/tests/corpus
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
python3 scripts/make_fastq_corpus.py "$work/in"
if ! diff -rq "$work/in" "$corpus" --exclude='*.tsv' --exclude='README.md' >/dev/null; then
  echo "corpus files differ from scripts/make_fastq_corpus.py output; regenerate them first" >&2
  exit 1
fi
mkdir -p "$work/out"
out="$corpus/fastqc_verdicts.tsv"
printf '# FastQC 0.12.1 (%s) verdicts on the synthetic corpus\nfile\tmodule\tverdict\n' "$image" > "$out"
for f in "$work"/in/*; do
  name=$(basename "$f")
  docker run --rm -u "$(id -u):$(id -g)" -v "$work:/d" "$image" fastqc -q --extract -o /d/out "/d/in/$name" >/dev/null 2>&1 || true
  stem=${name%.gz}; stem=${stem%.fastq}
  summary="$work/out/${stem}_fastqc/summary.txt"
  if [[ -f $summary ]]; then
    awk -F'\t' -v f="$name" '{ print f "\t" $2 "\t" tolower($1) }' "$summary" >> "$out"
  else
    printf '%s\t(any)\terror\n' "$name" >> "$out"
  fi
done
echo "wrote $out"
