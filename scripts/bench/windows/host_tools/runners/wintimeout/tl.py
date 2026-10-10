#!/usr/bin/env python3
"""Timeline of a kf3 qemu.log: every rpc-trace / refusal / relay / birth / free / RC line with the last
maplog time before it; consecutive duplicates (same text after the time) collapsed with a count.
usage: tl.py LOG [from_line] [to_line]"""
import re
import sys

log = sys.argv[1]
lo = int(sys.argv[2]) if len(sys.argv) > 2 else 0
hi = int(sys.argv[3]) if len(sys.argv) > 3 else 10**12
t = 0.0
keep = re.compile(r'rpc-trace|GSP REFUSED|USERD relay: GP_PUT|BORN|act free: .*(passthrough|translated)|RC_|Xid|RESET|NSI RELAY|birth REFUSED')
prev, cnt, first = None, 0, None
with open(log, errors='replace') as f:
    for n, line in enumerate(f, 1):
        m = re.search(r'maplog t=([0-9.]+)', line)
        if m:
            t = float(m.group(1))
        if n < lo or n > hi or not keep.search(line):
            continue
        s = re.sub(r'seq=\d+ ', '', line.strip())[:170]
        if s == prev:
            cnt += 1
            continue
        if prev is not None:
            print(f'{first[0]:7d} {first[1]:.3f} {prev}' + (f'  (x{cnt})' if cnt > 1 else ''))
        prev, cnt, first = s, 1, (n, t)
if prev is not None:
    print(f'{first[0]:7d} {first[1]:.3f} {prev}' + (f'  (x{cnt})' if cnt > 1 else ''))
