#!/usr/bin/env python3
"""Emit consttab rows (crates/kf-rm/data/measured_constants.tsv syntax) from scrubbed census bodies of Windows kfwc captures.
usage: gen_rows.py DIE:GSPFW:DATE:EVIDENCE_PREFIX:CONTROL_SET ...   (lane C, 2026-10-14)
Each spec reads EVIDENCE_PREFIX.bodies.tsv (allow-listed bodies only).  CONTROL_SET: 'static' (GMMU static info, CE fault-method buffer)
or 'pka' (the small group P/K/A controls).  A control is emitted only if EXACTLY ONE distinct (request, reply) pair was captured
(a reply that depends on a request word needs one row per request: the table's key is (key, die, branch)).
The branch is the GSP firmware major: the version the guest believes it talks to (matrix D2)."""
import csv, sys
KEYS = {
    'static': ['0x20800a59', '0x20802a08'],
    'pka': ['0x20802806', '0x20802801', '0x2080121f', '0x50700709'],
}
def words(h):
    return [int.from_bytes(bytes.fromhex(h[i:i + 8].ljust(8, '0')), 'little') for i in range(0, len(h), 8)]
def main():
    for spec in sys.argv[1:]:
        die, fw, date, prefix, cset = spec.split(':')
        pairs = {}
        rows = list(csv.DictReader(open(prefix + '.bodies.tsv'), delimiter='\t'))
        for key in KEYS[cset]:
            reqs = [r for r in rows if r['cmd'] == key and r['dir'] == 'cmd']
            reps = [r for r in rows if r['cmd'] == key and r['dir'] == 'status']
            if len({r['hex'] for r in reqs}) != 1 or len({r['hex'] for r in reps}) != 1 or reps[0]['status'] != '0x0':
                print(f'# skipped {key} on {die} {fw}: not exactly one request/reply pair', file=sys.stderr); continue
            size = int(reps[0]['size']); rq = words(reqs[0]['hex']); rp = words(reps[0]['hex'])
            inputs = ','.join(f'{4 * i:#x}={v:#x}' for i, v in enumerate(rq) if v) or '-'
            # the row stores the reply words up to the last non-zero word (the tail is measured zero), at least one word
            last = max([i for i, v in enumerate(rp) if v] or [0])
            w = ' '.join(f'{v:#x}' for v in rp[:last + 1])
            major = fw.split('.')[0]
            print('\t'.join([f'rm.{key}', die, major, str(size), inputs, w, f'measured:{die},{fw},{date}', prefix.rsplit('/', 1)[0] + ' (Windows kfwc capture, scrubbed bodies; GSP firmware ' + fw + ')']))
main()
