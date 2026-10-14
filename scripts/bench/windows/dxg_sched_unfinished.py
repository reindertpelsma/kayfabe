#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""dxg_sched_unfinished.py — host-side, DIAGNOSTIC (2026-10-08, after run84): read the compact `D ...`
lines dxg_etw_stop.ps1 prints (the guest's DxgKrnl scheduler events) and name what never finished: DMA
packets started (or reported by DmaPacket/Info) with no DmaPacket/Stop, queue packets with no
QueuePacket/Stop, and the AttemptPreemption reasons. Times are guest UTC (FILETIME).
usage: dxg_sched_unfinished.py ETW_OUTPUT.txt"""
import collections
import datetime
import re
import sys


def wall(t):
    return datetime.datetime.fromtimestamp(t / 1e7 - 11644473600, datetime.timezone.utc).strftime('%H:%M:%S.%f')


def main(path):
    starts, stops, infos, qs, qe = {}, {}, {}, {}, {}
    pre = collections.Counter()
    last = 0
    with open(path, encoding='utf-8', errors='replace') as fh:
        for line in fh:
            m = re.match(r'(?:ETW )?D (\S+) t=(\d+) pid=(\S+) cpu=(\S+)\s+(.*)', line.strip())
            if not m:
                continue
            tag, t, pid, _cpu, ud = m.groups()
            t = int(t)
            last = max(last, t)
            f = [x.strip().strip('"').strip() for x in ud.split(',')]
            if tag == 'S' and len(f) > 2:
                starts[(f[1], f[2])] = (t, f)
            elif tag == 'E' and len(f) > 2:
                stops[(f[1], f[2])] = (t, f)
            elif tag == 'I' and len(f) > 3:
                infos[(f[2], f[3])] = (t, f)
            elif tag == 'QS' and len(f) > 2:
                qs[f[2]] = (t, f, pid)
            elif tag == 'QE' and len(f) > 2:
                qe[f[2]] = (t, f)
            elif tag == 'P' and len(f) > 1:
                pre[f[-1]] += 1
    print(f'trace ends {wall(last)} UTC; DMA starts {len(starts)} stops {len(stops)}; queue starts {len(qs)} stops {len(qe)}')
    for k, (t, f) in sorted(starts.items(), key=lambda x: x[1][0]):
        if k not in stops:
            print('DMA STARTED, NEVER STOPPED', wall(t), f)
    for k, (t, f) in sorted(infos.items(), key=lambda x: x[1][0]):
        if k not in stops:
            print('DMA INFO, NEVER STOPPED', wall(t), f)
    for k, (t, f, pid) in sorted(qs.items(), key=lambda x: x[1][0]):
        if k not in qe:
            print('QUEUE PACKET NEVER STOPPED', wall(t), pid, f)
    for r, n in pre.most_common():
        print('AttemptPreemption', n, r)


if __name__ == '__main__':
    main(sys.argv[1])
