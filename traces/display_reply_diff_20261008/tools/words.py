"""words.py BODIES CMD [CMD..] [--tmax S] [--all] -- distinct (req, rep, status) of each control as u32 words (offset: req -> rep)."""
import collections, json, struct, sys
a = sys.argv
tmax = float(a[a.index('--tmax') + 1]) if '--tmax' in a else 1e9
allw = '--all' in a
cmds = [int(x, 16) for x in a[2:] if x.startswith('0x')]
groups = collections.OrderedDict()
for l in open(a[1]):
    e = json.loads(l)
    if e['fn'] != 76 or e['cmd'] not in cmds or e['t'] > tmax:
        continue
    k = (e['cmd'], e['req'], e.get('rep'), e.get('status'))
    groups.setdefault(k, []).append(e)
for (cmd, req, rep, st), es in sorted(groups.items(), key=lambda kv: kv[1][0]['n']):
    rq = bytes.fromhex(req); rp = bytes.fromhex(rep or '')
    print('== 0x%08x status=0x%x x%d first n=%d t=%.6f psz=%d' % (cmd, st, len(es), es[0]['n'], es[0]['t'] % 60, len(rq)))
    n = max(len(rq), len(rp))
    lines = []
    for o in range(0, n, 4):
        x = struct.unpack_from('<I', rq.ljust(n + 4, b'\0'), o)[0]
        y = struct.unpack_from('<I', rp.ljust(n + 4, b'\0'), o)[0]
        if allw or x or y:
            lines.append('  +0x%03x %08x -> %08x%s' % (o, x, y, '' if x == y else '  *'))
    if len(lines) > 40:
        lines = lines[:40] + ['  ... (%d more non-zero words)' % (len(lines) - 40)]
    print('\n'.join(lines) if lines else '  (all zero)')
