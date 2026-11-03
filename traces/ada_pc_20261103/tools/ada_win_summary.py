#!/usr/bin/env python3
"""Summarize one Windows boot's host-side evidence: display fps lines (kms/flip rate over time), STOPPED /
STALL / REFUSED counts, walk timing (vat[...] windows), kf3 thread CPU from cpu.jsonl.
usage: ada_win_summary.py <run dir> [<qemu.log of a reference run>]"""
import json, re, sys
from pathlib import Path
from collections import defaultdict

A = Path(sys.argv[1])
log = (A / 'qemu.log').read_text(errors='replace').splitlines()


def vat(lines, tag):
    ws = []
    for l in lines:
        m = re.search(r'vat\[invals=(\d+) arrive->clear_avg_us=(\d+) max_us=(\d+) walks=(\d+) walk_avg_us=(\d+) '
                      r'gpu_avg_us=(\d+)', l)
        if m and int(m.group(4)) > 0:
            ws.append(tuple(int(x) for x in m.groups()))
    if not ws:
        print(f'WALK {tag}: no window with walks'); return
    tw = sum(w[3] for w in ws)
    wavg = sum(w[3] * w[4] for w in ws) / tw
    gavg = sum(w[3] * w[5] for w in ws) / tw
    ti = sum(w[0] for w in ws)
    iavg = sum(w[0] * w[1] for w in ws) / ti if ti else 0
    srt = sorted(w[4] for w in ws)
    print(f'WALK {tag}: windows={len(ws)} walks={tw} walk_avg_us(weighted)={wavg:.0f} gpu_avg_us={gavg:.0f} '
          f'inval_arrive_to_clear_avg_us={iavg:.0f} invals={ti} per-window walk_avg_us p50={srt[len(srt)//2]} '
          f'max={srt[-1]}')
    lt = [w for w in ws if w[3] <= 50]
    if lt:
        n = sum(w[3] for w in lt)
        print(f'WALK {tag} light windows (<=50 walks): windows={len(lt)} walks={n} '
              f'walk_avg_us={sum(w[3]*w[4] for w in lt)/n:.0f} gpu_avg_us={sum(w[3]*w[5] for w in lt)/n:.0f} '
              f'p50 window walk_avg_us={sorted(w[4] for w in lt)[len(lt)//2]}')
    first = ws[:60]
    n = sum(w[3] for w in first)
    print(f'WALK {tag} first 60 windows with walks (boot+desktop): walks={n} '
          f'walk_avg_us={sum(w[3]*w[4] for w in first)/n:.0f} gpu_avg_us={sum(w[3]*w[5] for w in first)/n:.0f}')


fps = [l for l in log if l.startswith('kf3: display fps[')]
kms = []
for l in fps:
    m = re.search(r'h0\[armed=\S+ tick=(\S+) .*? kms=(\S+) tear=(\S+)', l)
    if m:
        kms.append(m.group(2))
print(f'DISPLAY fps lines={len(fps)}; kms series (first 10 / last 10): {kms[:10]} ... {kms[-10:]}')
nz = [k for k in kms if k not in ('0.0', '', '-')]
print(f'DISPLAY windows with kms>0: {len(nz)} of {len(kms)}')
for pat in ['STOPPED', 'STALL', 'REFUSED', 'Xid', 'panicked', 'USER_RING_REFUSED', 'RESET_CHANNEL', 'WATCHDOG',
            'flip timeout', 'kf3: display: TRACE']:
    print(f'COUNT {pat!r}: {sum(pat in l for l in log)}')
for l in [l for l in log if 'STOPPED' in l or 'STALL' in l][:8]:
    print('  ', l[:240])
vat(log, 'this run')
if len(sys.argv) > 2:
    vat(Path(sys.argv[2]).read_text(errors='replace').splitlines(), 'reference ' + sys.argv[2])
cj = A / 'cpu.jsonl'
if cj.exists():
    rows = [json.loads(l) for l in cj.read_text().splitlines() if l.strip()]
    if len(rows) >= 2:
        a, b = rows[0], rows[-1]
        dt = b['t'] - a['t']
        per = defaultdict(int)
        for tid, (comm, ticks) in b['th'].items():
            per[comm] += ticks - a['th'].get(tid, (comm, 0))[1]
        tot = sum(per.values())
        print(f'CPU over {dt:.0f}s: qemu total {tot/100/dt*100:.1f}% of one core')
        for comm, t in sorted(per.items(), key=lambda x: -x[1])[:14]:
            print(f'  {comm:20s} {t/100/dt*100:6.2f}%')
