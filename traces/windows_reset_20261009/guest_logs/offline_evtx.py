#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""offline_evtx.py LABEL System.evtx [SINCE_UTC_PREFIX] -- read-only summary of an exported (cleanly stopped) guest System.evtx:
event count per provider, and every Level 1-3 event or event of the providers the live collector selects, newest 120. Needs python-evtx."""
import re, sys, collections
from Evtx.Evtx import Evtx
label, path = sys.argv[1], sys.argv[2]
since = sys.argv[3] if len(sys.argv) > 3 else ''
rx = re.compile(r'nvlddmkm|Display|DxgKrnl|Kernel-Power|WER-SystemErrorReporting|Service Control Manager|NVIDIA|Application Error|Windows Error Reporting|LiveKernel|BugCheck|WHEA|volmgr|Kernel-General|EventLog')
cnt = collections.Counter()
sel = []
total = 0
with Evtx(path) as log:
    for rec in log.records():
        try:
            x = rec.xml()
        except Exception:
            continue
        t = re.search(r'SystemTime="([^"]+)"', x)
        p = re.search(r'Provider Name="([^"]+)"', x)
        i = re.search(r'<EventID[^>]*>(\d+)<', x)
        lv = re.search(r'<Level>(\d+)<', x)
        t = t.group(1) if t else ''
        if since and t < since:
            continue
        total += 1
        pn = p.group(1) if p else '?'
        cnt[pn] += 1
        lvl = int(lv.group(1)) if lv else 4
        if lvl <= 3 or rx.search(pn):
            data = re.findall(r'<Data[^>]*>([^<]*)</Data>', x)
            sel.append((t, pn, i.group(1) if i else '?', lvl, ' '.join(data)[:160]))
print(f'== {label}: {path} events since {since or "start"}: {total}')
print('providers (count):', ', '.join(f'{k}={v}' for k, v in cnt.most_common(25)))
sel.sort()
print(f'selected {len(sel)} (Level 1-3 or provider match), newest 120:')
for t, pn, i, lvl, d in sel[-120:]:
    print(f'{t[:23]} {pn} id={i} L{lvl} {d}')
