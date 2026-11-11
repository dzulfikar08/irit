#!/usr/bin/env bash
# pack.sh INDEX_DIR OUT_DIR [SKETCHES_DB]
#
# Prepares, but does not upload, the files a hosted ANI screen would need:
#   - the screen index (meta.json, refs.bin, markers.idx, gz.idx) as .tar.zst archives, each
#     under GitHub's 2 GiB asset limit, under the folder name ani-screen-r232/ (one dblock
#     multi-part entry);
#   - optionally sketches.db cut into 2,000,000,000-byte parts sketches.db.000, .001, ... for
#     `gtdbprep ani --source '<base>/sketches.db.{part}' --part-size 2000000000`;
#   - SHA256SUMS, MD5SUMS and a draft dblock catalog entry (dblock-entry.json) to fill in a
#     base_url once the files are published somewhere.
# GTDB data is CC BY-SA 4.0: a release of these files must say so and credit GTDB.
set -euo pipefail
idx=${1:?index folder}; out=${2:?output folder}; sk=${3:-}
limit=$((2 * 1024 * 1024 * 1024 - 1))
mkdir -p "$out"
name=ani-screen-r232
stage=$(mktemp -d); trap 'rm -rf "$stage"' EXIT
mkdir "$stage/$name"
for f in meta.json refs.bin markers.idx gz.idx; do
  [[ -e $idx/$f ]] && ln -s "$(realpath "$idx/$f")" "$stage/$name/$f"
done
parts=()
pack() { # archive-name files...
  local a=$1; shift
  tar -C "$stage" -ch --owner=0 --group=0 --numeric-owner --mtime=@0 --sort=name "$@" \
    | zstd -19 -T0 -q -o "$out/$a"
  local s; s=$(stat -c %s "$out/$a")
  (( s <= limit )) || { echo "$a is $s bytes, over 2 GiB: split it" >&2; exit 1; }
  parts+=("$a")
}
pack gtdbtk-r232-ani-screen-core.tar.zst "$name/meta.json" "$name/refs.bin"
pack gtdbtk-r232-ani-screen-markers.tar.zst "$name/markers.idx"
[[ -e $stage/$name/gz.idx ]] && pack gtdbtk-r232-ani-screen-gzindex.tar.zst "$name/gz.idx"

if [[ -n $sk ]]; then
  split -b 2000000000 -d -a 3 "$sk" "$out/sketches.db."
fi
(cd "$out" && sha256sum -- * | grep -v SUMS > SHA256SUMS && md5sum -- * | grep -v SUMS > MD5SUMS)

python3 - "$out" "$idx" "${parts[@]}" <<'EOF'
import hashlib, json, os, sys
out, idx, parts = sys.argv[1], sys.argv[2], sys.argv[3:]
def h(path, algo):
    d = hashlib.new(algo)
    with open(path, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            d.update(b)
    return d.hexdigest()
extracted = sum(os.path.getsize(os.path.join(idx, f)) for f in ('meta.json', 'refs.bin', 'markers.idx', 'gz.idx') if os.path.exists(os.path.join(idx, f)))
ps = [{'file': p, 'size': os.path.getsize(os.path.join(out, p)), 'md5': h(os.path.join(out, p), 'md5'),
       'sha256': h(os.path.join(out, p), 'sha256')} for p in parts]
entry = {
    'id': 'gtdbtk-r232-ani-screen',
    'name': 'Screen index for the GTDB-Tk R232 ANI species screen (gtdbprep ani; derived from GTDB data)',
    'version': 'R232',
    'tool': 'gtdbprep',
    'format': 'tar.zst',
    'base_url': 'FILL-IN-AFTER-PUBLISHING',
    'parts': ps,
    'size': sum(p['size'] for p in ps),
    'extracted': extracted,
    'md5_source': "the release's MD5SUMS and SHA256SUMS (derived files; GTDB publishes none)",
    'licence': 'CC BY-SA 4.0. Derived from the Genome Taxonomy Database (GTDB, https://gtdb.ecogenomic.org; Parks et al.) skani database in https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/full_package/gtdbtk_r232_data.tar.gz; not made or endorsed by GTDB.',
    'non_commercial': False,
    'notes': 'For gtdbprep ani --index <path>: marker fingerprints of every R232 species representative, their sketch offsets and hashes, and a seek index for GTDB\'s archive. Sketches are range-read from GTDB\'s server (or another source) at run time.',
}
with open(os.path.join(out, 'dblock-entry.json'), 'w') as f:
    json.dump(entry, f, indent=2)
    f.write('\n')
EOF
ls -la "$out"
