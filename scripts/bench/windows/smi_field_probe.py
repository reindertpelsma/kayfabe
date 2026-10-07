#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Host-side: which GSP requests does each `nvidia-smi --query-gpu=<field>` send? (owner question, 2026-10-08)
For each field (and an idle window with no query), twice, interleaved: record the qemu.log line count before
and after one QGA-run query, and list the distinct `rpc-trace` requests (fn, cmd or class, result) in the window.
usage: smi_field_probe.py RUN OUTFILE"""
import json, re, subprocess, sys, time
run, out = sys.argv[1], sys.argv[2]
rd = f'/var/lib/kf-windows-20261005/boundary-kayfabe-{run}'
qmp = '/var/lib/kf-windows-20261005/boundary-tools/qmp.py'
log = rd + '/qemu.log'
FIELDS = ['name', 'pstate', 'utilization.gpu', 'memory.used', 'memory.total']
def lines():
    return int(subprocess.run(['wc', '-l', log], capture_output=True, text=True).stdout.split()[0])
TR = re.compile(r'rpc-trace fn=(\d+) (\w+) seq=\d+ ?(?:cmd=(0x[0-9a-f]+))?(?: client=\S+ object=\S+)?(?:.*?class=(0x[0-9a-f]+))?.*?result=(\S+)')
def window(a, b):
    txt = subprocess.run(['sed', '-n', f'{a+1},{b}p', log], capture_output=True, text=True, errors='replace').stdout
    seen = {}
    for l in txt.splitlines():
        if 'rpc-trace' not in l: continue
        m = re.search(r'rpc-trace fn=(\d+) (\w+)', l)
        if not m or m.group(2) in ('Free',): continue
        cmd = re.search(r'cmd=(0x[0-9a-f]+)', l); cls = re.search(r'class=(0x[0-9a-f]+)', l); res = re.search(r'result=(\S+)', l)
        key = f'{m.group(2)} {cmd.group(1) if cmd else (cls.group(1) if cls else "")} result={res.group(1) if res else "?"}'
        seen[key] = seen.get(key, 0) + 1
    refused = [l for l in txt.splitlines() if 'REFUSED' in l or 'W349REFUSE' in l]
    return seen, [r[:200] for r in refused]
res = []
for rep in (1, 2):
    for f in FIELDS + [None]:
        b = lines()
        if f is None:
            time.sleep(6); smi = '(idle window, no query)'
        else:
            p = subprocess.run(['timeout', '100', 'python3', qmp, f'{rd}/qga.sock', 'qga-exec', 'nvidia-smi.exe', f'--query-gpu={f}', '--format=csv,noheader'], capture_output=True, text=True)
            smi = p.stdout.strip()
        time.sleep(2)
        a = lines()
        seen, ref = window(b, a)
        res.append({'rep': rep, 'field': f or 'IDLE', 'output': smi, 'lines': [b, a], 'requests': seen, 'refusal_lines': ref})
        print('DONE', rep, f, smi, len(seen), flush=True)
json.dump(res, open(out, 'w'), indent=1)
