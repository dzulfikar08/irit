#!/usr/bin/env python3
"""Compare FastQC verdicts with leanqc's, module by module.

Usage: compare_fastqc.py FASTQC_DIR LEANQC_DIR
FASTQC_DIR holds unzipped `<name>_fastqc/summary.txt` folders; LEANQC_DIR holds `<name>_leanqc.json`.
Prints a Markdown table and exits non-zero if any shared module disagrees.
"""

import json
import sys
from pathlib import Path

# FastQC module -> leanqc check
MODULES = {
    "Per base sequence quality": "Per-position quality",
    "Per tile sequence quality": "Per-tile quality",
    "Per sequence quality scores": "Per-read quality",
    "Per base sequence content": "Per-position base content",
    "Per sequence GC content": "Per-read GC",
    "Per base N content": "N content",
    "Sequence Length Distribution": "Read length",
    "Sequence Duplication Levels": "Duplication",
    "Overrepresented sequences": "Overrepresented sequences",
    "Adapter Content": "Adapter content",
}


def main() -> int:
    fastqc_dir, leanqc_dir = Path(sys.argv[1]), Path(sys.argv[2])
    rows, mismatches, compared = [], 0, 0
    for summary in sorted(fastqc_dir.glob("*_fastqc/summary.txt")):
        name = summary.parent.name.removesuffix("_fastqc")
        lq = leanqc_dir / f"{name}_leanqc.json"
        if not lq.exists():
            rows.append(f"| {name} | (no leanqc report) | | | ✗ |")
            mismatches += 1
            continue
        ours = {c["name"]: c["status"] for c in json.loads(lq.read_text())["checks"]}
        for line in summary.read_text().splitlines():
            status, module, _ = line.split("\t")
            check = MODULES.get(module)
            if check is None or check not in ours:
                continue
            compared += 1
            theirs = status.lower()
            ok = theirs == ours[check]
            mismatches += not ok
            if not ok or theirs != "pass":
                rows.append(f"| {name} | {check} | {theirs} | {ours[check]} | {'✓' if ok else '✗'} |")
    print(f"Compared {compared} module verdicts; {mismatches} differ.\n")
    print("| File | Module | FastQC | leanqc | Match |\n|---|---|---|---|---|")
    print("\n".join(rows) if rows else "| all | all | pass | pass | ✓ |")
    print("\n(Rows where both tools say pass are omitted.)")
    return 1 if mismatches or compared == 0 else 0


if __name__ == "__main__":
    sys.exit(main())
