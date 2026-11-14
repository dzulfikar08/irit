"""Drives irit-mcp over stdio against the real fastp, Shovill, Bakta, CheckM2, GTDB-Tk, mlst,
AMRFinderPlus, MEGAHIT, CoverM, MetaBAT2 and Kleborate.

Used by .github/workflows/mcp-real-tools.yml. Standard library only. The run:

1. dblock_add / dblock_fetch the Bakta light, CheckM2 and AMRFinderPlus databases (dblock indexes the
   last with amrfinder_index), dblock_env for their paths. Bakta gets the AMRFinderPlus database as
   `amrfinderplus-db/latest` of a folder of links to its own database, so neither dblock folder changes.
2. Two samples. mg: Mycoplasma genitalium G37 (simulated reads, the published assembly) through
   fastp, bakta, checkm2, gtdbtk_classify_wf and amrfinder on Bakta's proteins. sa: real Illumina reads
   of Staphylococcus aureus (ERR044595, as nf-core/bacass tests with) through fastp, shovill (SKESA),
   mlst, amrfinder and coverm_genome (depth by mapping), and shovill with SPAdes and its default
   correction steps, typed again. kp: K. pneumoniae HS11286's published genome through kleborate,
   with M. genitalium as an assembly Kleborate doesn't type; assembly_stats on the M. genitalium
   genome. Then isotable over all results, with mlst and AMR required.
3. Moves those outputs aside and runs the tools again with the same paths: any output whose
   SHA-256 differs between the two identical runs is listed (the "volatile" outputs replay.sh
   can't hash-check). For JSON files, the fields that differ are listed too.
4. Deletes the first run's outputs and runs replay.sh; then dblock_verify with deep: every database
   file, including the AMRFinderPlus indexes dblock built, still has its locked SHA-256.

Writes a Markdown report to $GITHUB_STEP_SUMMARY (or stdout) and exits non-zero if a tool call
failed, a result is not what the sample should give, or the replay failed.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path


class Client:
    def __init__(self, exe, cwd, env):
        self.p = subprocess.Popen(
            [exe, "--run-id", "real-tools"], cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True
        )
        self.next = 1
        init = self.request(
            "initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "ci", "version": "0"}}
        )
        assert init["result"]["serverInfo"]["name"] == "irit-mcp", init
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, msg):
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()

    def request(self, method, params):
        i = self.next
        self.next += 1
        self.send({"jsonrpc": "2.0", "id": i, "method": method, "params": params})
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("irit-mcp closed stdout")
            msg = json.loads(line)
            if msg.get("id") == i:
                return msg

    def call(self, tool, args):
        t = time.time()
        r = self.request("tools/call", {"name": tool, "arguments": args})
        res = r.get("result", r)
        return res, time.time() - t

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=60)


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def tree_hashes(root):
    out = {}
    for dirpath, _, files in os.walk(root):
        for f in files:
            p = Path(dirpath) / f
            if p.is_file() and not p.is_symlink():
                out[str(p.relative_to(root))] = sha(p)
    return out


def json_diff(p1, p2, limit=15):
    """Dotted paths of the values that differ between two JSON files."""
    out = []

    def walk(x, y, path):
        if len(out) >= limit:
            return
        if isinstance(x, dict) and isinstance(y, dict):
            for k in sorted(x.keys() | y.keys()):
                walk(x.get(k), y.get(k), f"{path}.{k}" if path else k)
        elif isinstance(x, list) and isinstance(y, list) and len(x) == len(y):
            for i, (a, b) in enumerate(zip(x, y)):
                walk(a, b, f"{path}[{i}]")
        elif x != y:
            out.append(path or "(root)")

    try:
        walk(json.load(open(p1)), json.load(open(p2)), "")
    except Exception as e:
        return f"unreadable: {e}"
    return ", ".join(f"`{x}`" for x in out) or "none"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mcp", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--path", required=True, help="PATH for irit-mcp and replay.sh (tool wrappers first)")
    a = ap.parse_args()
    work = Path(a.work).resolve()
    env = dict(os.environ, PATH=a.path)
    report = ["# irit-mcp with the real tools", ""]
    failures = []

    def log(s=""):
        print(s, flush=True)
        report.append(s)

    c = Client(a.mcp, work, env)

    def call(tool, args, label=None):
        res, secs = c.call(tool, args)
        sc = res.get("structuredContent", {})
        ok = res.get("isError") is not True
        log(f"## {label or tool} ({'ok' if ok else 'FAILED'}, {secs:.0f} s)")
        log("")
        log("```json")
        log(json.dumps({"arguments": args, "result": sc or res}, indent=1)[:6000])
        log("```")
        log("")
        if not ok:
            failures.append(label or tool)
        return sc

    amr_id = "amrfinderplus-db-4.2-2026-10-29.1"
    call("dblock_add", {"ids": ["bakta-db-v6-light", "checkm2-db-v3", amr_id, "gtdbtk-r232-mockup"], "store": "dbs"})
    call("dblock_fetch", {"confirm": True, "stream": True})
    dbs = call("dblock_env", {}).get("env", {})
    # Bakta 1.12.2 runs `amrfinder --database <db>/amrfinderplus-db/latest` (bakta/expert/amrfinder.py).
    # Instead of amrfinder_update writing into the dblock Bakta folder, Bakta gets a folder of links to
    # its database whose amrfinderplus-db/latest is the dblock AMRFinderPlus database.
    bakta_db = dbs.get("BAKTA_DB", "")
    if bakta_db and dbs.get("AMRFINDER_DB"):
        src, linked = work / bakta_db, work / "bakta-db-amr"
        shutil.rmtree(linked, ignore_errors=True)
        linked.mkdir()
        for entry in src.iterdir():
            if entry.name != "amrfinderplus-db":
                (linked / entry.name).symlink_to(entry.resolve())
        (linked / "amrfinderplus-db").mkdir()
        (linked / "amrfinderplus-db" / "latest").symlink_to((work / dbs["AMRFINDER_DB"]).resolve())
        log(f"Bakta database with dblock's AMRFinderPlus data: `{linked.name}` (links to `{bakta_db}`, latest -> `{dbs['AMRFINDER_DB']}`)")
        log("")
        bakta_db = linked.name

    def expect(label, ok, detail):
        log(f"- check {label}: {'ok' if ok else 'WRONG'} ({detail})")
        log("")
        if not ok:
            failures.append(f"check {label}")

    def run_all(suffix=""):
        # trim/, shovill/ and typing/ don't exist yet (the second run starts after they're moved
        # away): irit-mcp creates output folders itself.
        for s, reads in [("mg", "reads/mg"), ("sa", "reads/ERR044595_1M")]:
            call(
                "fastp",
                {
                    "in1": f"{reads}_1.fastq.gz" if s == "sa" else f"{reads}_R1.fastq.gz",
                    "in2": f"{reads}_2.fastq.gz" if s == "sa" else f"{reads}_R2.fastq.gz",
                    "out1": f"trim/{s}_R1.fastq.gz",
                    "out2": f"trim/{s}_R2.fastq.gz",
                    "json": f"trim/{s}.fastp.json",
                    "detect_adapter_for_pe": True,
                    "thread": 4,
                },
                f"fastp {s}{suffix}",
            )
        # S. aureus: real reads (53x), assembled with SKESA at 30x without the optional read
        # correction, stitching and Pilon steps, to keep three Shovill runs within the job's time.
        asm = call(
            "shovill",
            {
                "r1": "trim/sa_R1.fastq.gz",
                "r2": "trim/sa_R2.fastq.gz",
                "outdir": "shovill/sa",
                "assembler": "skesa",
                "gsize": "2.8M",
                "depth": 30,
                "noreadcorr": True,
                "nostitch": True,
                "nocorr": True,
                "cpus": 4,
                "ram": 8,
            },
            f"shovill sa{suffix}",
        )
        size = asm.get("assembly", {}).get("total_length", 0)
        expect(f"shovill sa{suffix}", 2_600_000 < size < 3_100_000, f"{size} bp; S. aureus is about 2.8 Mb")
        st = call("mlst", {"contigs": ["shovill/sa/contigs.fa"], "outfile": "typing/sa.mlst.tsv", "json": "typing/sa.mlst.json"}, f"mlst sa{suffix}")
        row = (st.get("results") or [{}])[0]
        # ERR044595 is an EMRSA-15 isolate: ST22.
        expect(f"mlst sa{suffix}", (row.get("sample"), row.get("scheme"), row.get("st")) == ("sa", "saureus", "22"), json.dumps(row)[:300])
        # mlst's bundled PubMLST data is in the record: its date, and the files it types with.
        expect(f"mlst sa{suffix} data version", re.fullmatch(r"\d{4}-\d{2}-\d{2}", st.get("database_version") or "") is not None,
               str(st.get("database_version")))
        # Shovill's default assembler, SPAdes, with its default read correction, stitching and Pilon
        # polishing; the SKESA run above skips them to save time. Typed again: same ST.
        asm = call(
            "shovill",
            {"r1": "trim/sa_R1.fastq.gz", "r2": "trim/sa_R2.fastq.gz", "outdir": "shovill/sa_spades", "assembler": "spades",
             "gsize": "2.8M", "depth": 30, "cpus": 4, "ram": 8},
            f"shovill spades sa{suffix}",
        )
        size = asm.get("assembly", {}).get("total_length", 0)
        expect(f"shovill spades sa{suffix}", 2_600_000 < size < 3_100_000, f"{size} bp; S. aureus is about 2.8 Mb")
        st = call("mlst", {"contigs": ["shovill/sa_spades/contigs.fa"], "outfile": "shovill/sa_spades.mlst.tsv"}, f"mlst spades sa{suffix}")
        row = (st.get("results") or [{}])[0]
        expect(f"mlst spades sa{suffix}", (row.get("scheme"), row.get("st")) == ("saureus", "22"), json.dumps(row)[:300])
        # The MAG route on a two-genome mock metagenome: the S. aureus reads (53x) and the
        # M. genitalium pairs (about 10x) assembled together with MEGAHIT, CoverM depth with the two
        # read sets as two samples, then MetaBAT2. mag/ doesn't exist yet: the server creates it.
        pairs = {"r1": ["trim/sa_R1.fastq.gz", "trim/mg_R1.fastq.gz"], "r2": ["trim/sa_R2.fastq.gz", "trim/mg_R2.fastq.gz"]}
        meta = call("megahit", {**pairs, "outdir": "mag/asm", "memory_gb": 4, "threads": 4}, f"megahit{suffix}")
        size = meta.get("assembly", {}).get("total_length", 0)
        expect(f"megahit{suffix}", 2_600_000 < size < 4_500_000, f"{size} bp; S. aureus is about 2.8 Mb and M. genitalium 0.58 Mb")
        depth = call(
            "coverm_contig",
            {"reference": "mag/asm/final.contigs.fa", **pairs, "output": "mag/depth.tsv", "methods": ["metabat"], "threads": 4},
            f"coverm_contig{suffix}",
        )
        mapped = [m.get("percent", 0) for m in depth.get("mapped", [])]
        expect(f"coverm_contig{suffix}", len(mapped) == 2 and min(mapped) > 50, json.dumps(depth.get("mapped"))[:300])
        bins = call(
            "metabat2",
            {"contigs": "mag/asm/final.contigs.fa", "depth": "mag/depth.tsv", "outdir": "mag/bins", "threads": 4},
            f"metabat2{suffix}",
        )
        longest = max((b.get("total_length", 0) for b in bins.get("bins", [])), default=0)
        expect(f"metabat2{suffix}", 2_000_000 < longest < 3_200_000, f"{bins.get('bin_count')} bins, largest {longest} bp; S. aureus is about 2.8 Mb")
        # Depth by mapping the trimmed reads to the SKESA assembly (about 53x of raw reads; Shovill
        # logs the depth it estimated before subsampling to 30x).
        cov = call(
            "coverm_genome",
            {"assembly": "shovill/sa/contigs.fa", "r1": "trim/sa_R1.fastq.gz", "r2": "trim/sa_R2.fastq.gz", "output": "typing/sa.coverm.tsv", "threads": 4},
            f"coverm_genome sa{suffix}",
        )
        g = (cov.get("genomes") or [{}])[0]
        expect(f"coverm_genome sa{suffix}", 30 < g.get("mapped_depth", 0) < 80 and g.get("covered_fraction", 0) > 95, json.dumps(cov.get("genomes"))[:300])
        # Kleborate: HS11286 is ST11 with KPC-2; M. genitalium is no species Kleborate types, so it gets no row
        kl = call("kleborate", {"assemblies": ["kp/HS11286.fna", "asm/mg.fna"], "outdir": "kleb"}, f"kleborate{suffix}")
        rows = {r.get("strain"): r for r in kl.get("assemblies", [])}
        hs = rows.get("HS11286", {})
        expect(
            f"kleborate{suffix}",
            (hs.get("species"), hs.get("ST"), hs.get("amr", {}).get("Bla_Carb_acquired")) == ("Klebsiella pneumoniae", "ST11", "KPC-2") and "mg" not in rows,
            json.dumps(hs)[:400],
        )
        expect(f"kleborate{suffix} warns about mg", any(w.startswith("mg: Kleborate wrote no row") for w in kl.get("warnings", [])), json.dumps(kl.get("warnings"))[:400])
        expect(f"kleborate{suffix} data hashed", kl.get("bundled_data_files_hashed", 0) > 50, str(kl.get("bundled_data_files_hashed")))
        # M. genitalium G37 is one 580,076 bp chromosome
        st = call("assembly_stats", {"assemblies": ["asm/mg.fna"]}, f"assembly_stats{suffix}")
        a = (st.get("assemblies") or [{}])[0]
        expect(f"assembly_stats{suffix}", (a.get("contigs"), a.get("n50"), a.get("l50"), a.get("total_length")) == (1, 580076, 1, 580076), json.dumps(a)[:300])
        amr = call(
            "amrfinder",
            {
                "nucleotide": "shovill/sa/contigs.fa",
                "database": dbs.get("AMRFINDER_DB", ""),
                "organism": "Staphylococcus_aureus",
                "plus": True,
                "output": "typing/sa.amrfinder.tsv",
                "threads": 4,
            },
            f"amrfinder sa{suffix}",
        )
        genes = (amr.get("summary") or {}).get("amr_genes", "")
        expect(f"amrfinder sa{suffix}", {"mecA", "blaZ"} <= set(genes.split(",")), genes)
        call(
            "bakta",
            {
                "genome": "asm/mg.fna",
                "db": bakta_db,
                "outdir": "bakta",
                "translation_table": 4,
                "genus": "Mycoplasmoides",
                "species": "genitalium",
                "threads": 4,
            },
            f"bakta{suffix}",
        )
        # AMRFinderPlus on Bakta's proteins, with Bakta's GFF3 and contigs (its own contig names).
        call(
            "amrfinder",
            {
                "protein": "bakta/mg.faa",
                "gff": "bakta/mg.gff3",
                "nucleotide": "bakta/mg.fna",
                "annotation_format": "bakta",
                "translation_table": 4,
                "database": dbs.get("AMRFINDER_DB", ""),
                "output": "typing/mg.amrfinder.tsv",
                "threads": 4,
            },
            f"amrfinder mg{suffix}",
        )
        call(
            "checkm2",
            {"genomes": ["asm/mg.fna"], "db": dbs.get("CHECKM2DB", ""), "outdir": "checkm2", "lowmem": True, "threads": 4},
            f"checkm2{suffix}",
        )
        call(
            "gtdbtk_classify_wf",
            {
                "genome_dir": "gtdb_genomes",
                "extension": "fna.gz",
                "out_dir": "gtdbtk",
                "data_path": dbs.get("GTDBTK_DATA_PATH", ""),
                "place_species": True,
                # GTDB's mock database has an empty split/, so it needs the full tree (its HOWTO's -f)
                "full_tree": True,
                "cpus": 4,
                "pplacer_cpus": 1,
            },
            f"gtdbtk_classify_wf{suffix}",
        )

    dirs = ["trim", "shovill", "typing", "bakta", "checkm2", "gtdbtk", "mag", "kleb"]
    run_all()
    table = call(
        "isotable",
        {"dirs": [d for d in ["trim", "checkm2", "gtdbtk", "typing", "kleb"] if (work / d).is_dir()], "require": ["reads", "q30", "mlst", "amr"]},
    )
    rows = {s.get("sample"): s for s in table.get("samples", [])}
    sa = rows.get("sa", {}).get("values", {})
    expect(
        "isotable sa",
        sa.get("mlst_st") == "22" and "mecA" in sa.get("amr_genes", "").split(",") and sa.get("reads_trimmed", "") != "" and sa.get("mapped_depth", "") != "",
        json.dumps(rows.get("sa"))[:600],
    )
    expect("isotable HS11286", rows.get("HS11286", {}).get("values", {}).get("kleborate_st") == "ST11", json.dumps(rows.get("HS11286"))[:600])
    # AMRFinderPlus on Bakta's proteins joined to mg (`none` when it found nothing).
    expect("isotable mg", rows.get("mg", {}).get("values", {}).get("amr_genes", "") != "", json.dumps(rows.get("mg"))[:600])
    # Bakta's own AMRFinderPlus step ran on dblock's database (Bakta 1.12.2 stops if amrfinder fails).
    run_dir = work / "irit-runs" / "real-tools"
    log_json = json.loads((run_dir / "run.json").read_text())
    bakta_call = next((x for x in log_json["calls"] if x["mcp_tool"] == "bakta" and x["status"] == "ok"), None)
    out = (run_dir / bakta_call["stdout"]).read_text() if bakta_call and bakta_call.get("stdout") else ""
    line = next((l.strip() for l in out.splitlines() if l.strip().startswith("amrfinder:")), None)
    expect("bakta's AMRFinderPlus step", line is not None, line or "no `amrfinder:` line in Bakta's output")
    for d in dirs:
        if (work / d).exists():
            os.rename(work / d, work / f"{d}_first")
    run_all("_again")
    # Nothing above may have changed a fetched database (the old way, amrfinder_update, wrote into Bakta's).
    # deep: every file's SHA-256 against the lock, including the indexes dblock built for AMRFinderPlus.
    verify = call("dblock_verify", {"deep": True}, "dblock_verify deep")
    report_text = verify.get("report", "x")
    expect("databases unchanged", "CHANGED" not in report_text, report_text[:600])
    expect("AMRFinderPlus database verified deep", amr_id in report_text and "CHANGED" not in report_text, report_text[:600])
    c.close()

    log("## Outputs that differ between two identical runs (same paths)")
    log("")
    for d in dirs:
        a1, a2 = tree_hashes(work / f"{d}_first"), tree_hashes(work / d)
        diff = sorted(k for k in a1.keys() | a2.keys() if a1.get(k) != a2.get(k))
        log(f"- `{d}`: {len(a1)} files, {len(diff)} differ" + (": " + ", ".join(f"`{x}`" for x in diff) if diff else ""))
        for k in diff:
            if k.endswith(".json") and k in a1 and k in a2:
                log(f"  - `{k}` fields: {json_diff(work / f'{d}_first' / k, work / d / k)}")
    log("")

    # replay.sh from a clean folder: it creates the output folders (mkdir -p) itself.
    for d in dirs:
        shutil.rmtree(work / d, ignore_errors=True)
    r = subprocess.run(["sh", str(run_dir / "replay.sh")], cwd=work, env=env, capture_output=True, text=True)
    log(f"## replay.sh: exit {r.returncode}")
    log("")
    log("```")
    log(r.stderr[-6000:])
    log("```")
    if r.returncode != 0:
        failures.append("replay")

    log("")
    log("Run record: " + ", ".join(sorted(p.name for p in run_dir.iterdir())))
    log(f"Failures: {failures or 'none'}")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write("\n".join(report) + "\n")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
