#!/usr/bin/env python3
"""
diff_traces.py — compare two nvioctl_trace logs and report response-payload
differences for the same (cmd, inner_cmd) pair.

Usage: diff_traces.py HOST.log GUEST.log
"""

import re
import sys
from collections import defaultdict

REC_RE = re.compile(
    r'^PID=(\d+) TID=(\d+) FD=(\S+) PATH=(\S+) CMD=(\S+)'
    r'(?: (RM_CONTROL|RM_ALLOC) (inner|hClass)=(\S+))? RET=(\S+) ERRNO=(\S+) SIZE=(\d+)'
)


def parse(path):
    """Parse a trace file into a list of records.

    Each record is a dict with cmd, sub (inner cmd or hClass key),
    sub_val, ret, errno, size, pre (list of bytes), post (list of bytes).
    """
    records = []
    cur = None
    with open(path) as f:
        for line in f:
            line = line.rstrip()
            m = REC_RE.match(line)
            if m:
                if cur is not None:
                    records.append(cur)
                cur = {
                    'path': m.group(4),
                    'cmd': m.group(5),
                    'sub_kind': m.group(6),
                    'sub_val': m.group(8),
                    'ret': m.group(9),
                    'size': int(m.group(10)),
                    'pre': None,
                    'post': None,
                    'inner_pre': None,
                    'inner_post': None,
                }
            elif cur is None:
                continue
            elif line.startswith('  PRE  : '):
                hex_str = line[len('  PRE  : '):]
                if hex_str != '(read failed)' and hex_str != '(no buffer)':
                    cur['pre'] = bytes.fromhex(hex_str.replace(' ', ''))
            elif line.startswith('  POST : '):
                hex_str = line[len('  POST : '):]
                if hex_str != '(read failed)':
                    cur['post'] = bytes.fromhex(hex_str.replace(' ', ''))
            elif line.startswith('  INNER PRE  ['):
                # Skip past "[NN bytes @ 0xADDR]: "
                idx = line.find(']: ')
                if idx > 0:
                    hex_str = line[idx + 3:]
                    if hex_str not in ('(no pre)', '(read failed)'):
                        cur['inner_pre'] = bytes.fromhex(hex_str.replace(' ', ''))
            elif line.startswith('  INNER POST ['):
                idx = line.find(']: ')
                if idx > 0:
                    hex_str = line[idx + 3:]
                    if hex_str != '(read failed)':
                        cur['inner_post'] = bytes.fromhex(hex_str.replace(' ', ''))
    if cur is not None:
        records.append(cur)
    return records


def key(r):
    """Group records by ioctl identity (cmd + sub-cmd)."""
    return (r['cmd'], r['sub_kind'], r['sub_val'])


def fmt_bytes(b, mark=None):
    if b is None:
        return '(none)'
    parts = []
    for i, x in enumerate(b):
        s = f'{x:02x}'
        if mark and mark[i]:
            s = f'\033[1;31m{s}\033[0m'  # red
        parts.append(s)
    return ' '.join(parts)


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 1
    host = parse(sys.argv[1])
    guest = parse(sys.argv[2])

    # Index by (cmd, sub) — multiple records per key, paired in order.
    host_by_key = defaultdict(list)
    guest_by_key = defaultdict(list)
    for r in host:
        host_by_key[key(r)].append(r)
    for r in guest:
        guest_by_key[key(r)].append(r)

    all_keys = sorted(set(host_by_key) | set(guest_by_key))

    n_diffs = 0
    for k in all_keys:
        hlist = host_by_key.get(k, [])
        glist = guest_by_key.get(k, [])
        if not hlist and glist:
            print(f'!! Guest has but host does not: {k}  ({len(glist)} calls)')
            n_diffs += 1
            continue
        if hlist and not glist:
            print(f'!! Host has but guest does not: {k}  ({len(hlist)} calls)')
            n_diffs += 1
            continue
        # Compare first call of each side (response bytes), aligned.
        h = hlist[0]
        g = glist[0]
        hp, gp = h.get('post'), g.get('post')
        if hp is None and gp is None:
            continue
        if hp is None or gp is None:
            print(f'!! {k}  one side missing post buffer')
            n_diffs += 1
            continue
        n = min(len(hp), len(gp))
        diff_mask = [hp[i] != gp[i] for i in range(n)]
        hip, gip = h.get('inner_post'), g.get('inner_post')
        inner_diff_mask = None
        if hip and gip:
            ni = min(len(hip), len(gip))
            inner_diff_mask = [hip[i] != gip[i] for i in range(ni)]
            if any(inner_diff_mask):
                pass  # show below
            else:
                inner_diff_mask = None

        if any(diff_mask) or inner_diff_mask:
            print(f'\n=== {k}  (host calls={len(hlist)} guest calls={len(glist)}, size={h["size"]}) ===')
            print(f'  RET   host={h["ret"]:>3}   guest={g["ret"]:>3}')
            if any(diff_mask):
                print(f'  HOST  POST : {fmt_bytes(hp[:n], diff_mask)}')
                print(f'  GUEST POST : {fmt_bytes(gp[:n], diff_mask)}')
            if inner_diff_mask:
                ni = len(inner_diff_mask)
                print(f'  HOST  INNER: {fmt_bytes(hip[:ni], inner_diff_mask)}')
                print(f'  GUEST INNER: {fmt_bytes(gip[:ni], inner_diff_mask)}')
            n_diffs += 1
    print(f'\n{n_diffs} differences across {len(all_keys)} unique (cmd,sub) groups')


if __name__ == '__main__':
    sys.exit(main() or 0)
