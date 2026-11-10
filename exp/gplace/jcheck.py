#!/usr/bin/env python3
"""jcheck.py A.jplace B.jplace LABEL_A LABEL_B
Independent per-genome comparison of two jplace files (pplacer JSON format), without gPlace code.
Prints a markdown table: best edge, LWR, distal and pendant length of the best placement in each,
the absolute differences, the number of placements listed, and a verdict:
  identical  - every placement row equal (same edges, same values as printed)
  same       - same best edge, |dLWR| <= 1e-4, |d distal| and |d pendant| <= 1e-5 (or 1e-4 relative)
  DIFFERENT  - otherwise
Exit code 0 always; the summary line starts with 'SUMMARY'."""
import json
import sys


def load(path):
    with open(path) as fh:
        j = json.load(fh)
    fields = j['fields']
    out = {}
    for p in j['placements']:
        rows = [dict(zip(fields, r)) for r in p['p']]
        names = p.get('n') or [x[0] for x in p.get('nm', [])]
        for n in names:
            out[n] = rows
    return j['tree'], out


def close(a, b, abs_tol, rel_tol=1e-4):
    return abs(a - b) <= max(abs_tol, rel_tol * max(abs(a), abs(b)))


def main():
    pa, pb, la, lb = sys.argv[1:5]
    ta, a = load(pa)
    tb, b = load(pb)
    names = sorted(set(a) | set(b))
    counts = {'identical': 0, 'same': 0, 'DIFFERENT': 0, 'missing': 0}
    print(f'Tree strings identical: {ta == tb}\n')
    print(f'| Genome | Best edge {la} / {lb} | LWR {la} / {lb} | |dLWR| | |d distal| | |d pendant| | '
          f'|d logL| | Rows {la}/{lb} | Verdict |')
    print('|---|---|---|---:|---:|---:|---:|---|---|')
    for n in names:
        if n not in a or n not in b:
            counts['missing'] += 1
            print(f'| {n} | only in {"A" if n in a else "B"} | | | | | | | missing |')
            continue
        ra = max(a[n], key=lambda r: r['like_weight_ratio'])
        rb = max(b[n], key=lambda r: r['like_weight_ratio'])
        d = {k: abs(ra[k] - rb[k]) for k in ('like_weight_ratio', 'distal_length', 'pendant_length', 'likelihood')}
        if a[n] == b[n]:
            v = 'identical'
        elif (ra['edge_num'] == rb['edge_num'] and d['like_weight_ratio'] <= 1e-4
              and close(ra['distal_length'], rb['distal_length'], 1e-5)
              and close(ra['pendant_length'], rb['pendant_length'], 1e-5)):
            v = 'same'
        else:
            v = 'DIFFERENT'
        counts[v] += 1
        print(f"| {n} | {ra['edge_num']} / {rb['edge_num']} | {ra['like_weight_ratio']:.6g} / "
              f"{rb['like_weight_ratio']:.6g} | {d['like_weight_ratio']:.3g} | {d['distal_length']:.3g} | "
              f"{d['pendant_length']:.3g} | {d['likelihood']:.3g} | {len(a[n])}/{len(b[n])} | {v} |")
    print()
    print('SUMMARY ' + ' '.join(f'{k}={v}' for k, v in counts.items()) + f' total={len(names)}')


if __name__ == '__main__':
    main()
