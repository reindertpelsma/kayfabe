#!/usr/bin/env python3
"""Summarise one kf-census run (PREFIX.controls.tsv + the scrubbed PREFIX.bodies.tsv) for the constants lane.

usage: windows_rows.py PREFIX [--bodies-only-small N]
Prints, for the rows of docs/design/V3_CONSTANT_MATRIX_20261014.md section 3 and the P/K/I/A groups of section 7, the paramsSize
(request = reply size), call count, RM status and, where the scrub kept the body, the request's non-zero word count and the
reply's first words.  Reads only census text; never a capture file.  (Lane C, 2026-10-14.)
"""
import csv, sys

WINDOWS_ONLY = {
    '0x2080852a': 'GSS', '0x20808524': 'GSS', '0x2080a70a': 'GSS', '0x2080a088': 'GSS', '0x2080852e': 'GSS', '0x2080853a': 'GSS',
    '0x2080a618': 'GSS', '0x2080852f': 'GSS', '0x2080853b': 'GSS', '0x2080a619': 'GSS', '0x20809001': 'cudart', '0x20809009': 'cudart',
    '0x20809064': 'cudart', '0x20809004': 'GSS', '0x2080a060': 'GSS', '0x2080a630': 'GSS', '0x2080a801': 'GSS', '0x2080b201': 'GSS',
    '0x2080a02f': 'GSS', '0x20808162': 'GSS', '0x00809908': 'NV0080', '0x20810108': 'NV2081', '0x2081010d': 'NV2081', '0x20801111': 'swrl',
    '0x20802806': 'A', '0x00730122': 'A', '0x00730128': 'A', '0x007302a3': 'A', '0x007302a5': 'A', '0x0073011a': 'display',
    '0x00731369': 'P', '0x20802801': 'P', '0x20802a12': 'P', '0x50700709': 'P', '0x50700118': 'P',
    '0x0073014b': 'K', '0x007302a2': 'K', '0x2080121f': 'K', '0x20802a08': 'K',
    '0x20800a59': 'I', '0x20800a3f': 'I', '0x20800a80': 'I', '0x20800a2e': 'I', '0x20800a87': 'I', '0x20800ab8': 'I', '0x20800b03': 'I',
}

def main():
    pre = sys.argv[1]
    ctl = {r['cmd']: r for r in csv.DictReader(open(pre + '.controls.tsv'), delimiter='\t')}
    bod = {}
    try:
        for r in csv.DictReader(open(pre + '.bodies.tsv'), delimiter='\t'):
            bod.setdefault(r['cmd'], []).append(r)
    except FileNotFoundError:
        pass
    print('cmd\tgroup\tname\tcalls\treq_sizes\treply_sizes\tstatuses\tbody_kept\treply_first_words')
    for k, g in sorted(WINDOWS_ONLY.items()):
        r = ctl.get(k)
        if not r:
            print(f'{k}\t{g}\t-\tABSENT\t-\t-\t-\t-\t-'); continue
        rep = [b for b in bod.get(k, []) if b['dir'] == 'status']
        first = ''
        if rep:
            h = rep[0]['hex']; w = [h[i:i + 8] for i in range(0, min(len(h), 96), 8)]
            first = ' '.join(w)
        print(f"{k}\t{g}\t{r['name'][:36]}\t{r['requests']}\t{r['req_sizes']}\t{r['reply_sizes']}\t{r['statuses']}\t{len(rep)}\t{first}")

if __name__ == '__main__':
    main()
