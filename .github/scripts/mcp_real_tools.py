"""Drives irit-mcp over stdio against the real fastp, Bakta, CheckM2 and GTDB-Tk.

Used by .github/workflows/mcp-real-tools.yml. Standard library only. The run:

1. dblock_add / dblock_fetch the Bakta light and CheckM2 databases, dblock_env for their paths.
2. fastp, bakta, checkm2 and gtdbtk_classify_wf once each, then isotable over the results.
3. Moves those outputs aside and runs the four tools again with the same paths: any output whose
   SHA-256 differs between the two identical runs is listed (the "volatile" outputs replay.sh
   can't hash-check). For JSON files, the fields that differ are listed too.
4. Deletes the first run's outputs and runs replay.sh.

Writes a Markdown report to $GITHUB_STEP_SUMMARY (or stdout) and exits non-zero if a tool call
failed or the replay failed.
"""

import argparse
import hashlib
import json
import os
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
    ap.add_argument("--amrfinder-update", help="amrfinder_update to run on the fetched Bakta database (its AMRFinderPlus data must match the installed amrfinder)")
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

    call("dblock_add", {"ids": ["bakta-db-v6-light", "checkm2-db-v3"], "store": "dbs"})
    call("dblock_fetch", {"confirm": True, "stream": True})
    dbs = call("dblock_env", {}).get("env", {})
    if a.amrfinder_update and dbs.get("BAKTA_DB"):
        amr = Path(dbs["BAKTA_DB"]) / "amrfinderplus-db"
        r = subprocess.run([a.amrfinder_update, "--force_update", "--database", str(amr)], env=env, capture_output=True, text=True)
        log(f"amrfinder_update --force_update --database {amr}: exit {r.returncode}")
        log("")

    def run_all(suffix=""):
        os.makedirs(work / "trim", exist_ok=True)
        call(
            "fastp",
            {
                "in1": "reads/mg_R1.fastq.gz",
                "in2": "reads/mg_R2.fastq.gz",
                "out1": "trim/mg_R1.fastq.gz",
                "out2": "trim/mg_R2.fastq.gz",
                "json": "trim/mg.fastp.json",
                "detect_adapter_for_pe": True,
                "thread": 4,
            },
            f"fastp{suffix}",
        )
        call(
            "bakta",
            {
                "genome": "asm/mg.fna",
                "db": dbs.get("BAKTA_DB", ""),
                "outdir": "bakta",
                "translation_table": 4,
                "genus": "Mycoplasmoides",
                "species": "genitalium",
                "threads": 4,
            },
            f"bakta{suffix}",
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
                "data_path": "gtdb_mock/release232_mockup",
                "place_species": True,
                # GTDB's mock database has an empty split/, so it needs the full tree (its HOWTO's -f)
                "full_tree": True,
                "cpus": 4,
                "pplacer_cpus": 1,
            },
            f"gtdbtk_classify_wf{suffix}",
        )

    dirs = ["trim", "bakta", "checkm2", "gtdbtk"]
    run_all()
    call("isotable", {"dirs": [d for d in ["trim", "checkm2", "gtdbtk"] if (work / d).is_dir()]})
    for d in dirs:
        if (work / d).exists():
            os.rename(work / d, work / f"{d}_first")
    run_all("_again")
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

    run_dir = work / "irit-runs" / "real-tools"
    for d in dirs:
        shutil.rmtree(work / d, ignore_errors=True)
    os.makedirs(work / "trim")
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
