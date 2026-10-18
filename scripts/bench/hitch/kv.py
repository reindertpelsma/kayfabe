#!/usr/bin/env python3
"""kv.py: structured parse of kf3 'family=' and 'display fps[' lines from a dispts file (epoch-stamped).
Library: load(path) -> list of (t, kind, dict) with dict keys 'stack.key' -> float (first occurrence wins).
CLI: kv.py DISPTS F9_EPOCH col1 col2 ... : per-family-interval table; a column is KEY (cumulative->delta) or KEY@ (value as is)
"""
import re, sys

TOK = re.compile(r'([A-Za-z_][\w>\-]*)\[|\]|([A-Za-z_][\w>\-]*)=(-?(?:0x[0-9a-fA-F]+|[0-9]+(?:\.[0-9]+)?))')

def parse(s):
    d, stack = {}, []
    for m in TOK.finditer(s):
        if m.group(1):
            stack.append(m.group(1))
        elif m.group(0) == ']':
            if stack:
                stack.pop()
        else:
            k = '.'.join(stack + [m.group(2)]) if stack else m.group(2)
            v = m.group(3)
            v = float(int(v, 16)) if v.startswith('0x') else float(v)
            d.setdefault(k, v)
    return d

def load(path):
    out = []
    for ln in open(path, errors='replace'):
        p = ln.split(' ', 1)
        if len(p) < 2:
            continue
        try:
            t = float(p[0])
        except ValueError:
            continue
        s = p[1]
        if 'display fps[' in s:
            out.append((t, 'disp', parse(s)))
        elif 'family=' in s:
            out.append((t, 'fam', parse(s)))
    return out

if __name__ == '__main__':
    rows = load(sys.argv[1])
    t0 = float(sys.argv[2])
    cols = sys.argv[3:]
    disp = [(t, d) for t, k, d in rows if k == 'disp']
    fam = [(t, d) for t, k, d in rows if k == 'fam']
    print('t_rel   ' + ' '.join('%10s' % c[-10:] for c in cols))
    prev = None
    for t, d in fam:
        if prev is not None:
            dl = min(disp, key=lambda x: abs(x[0] - t))[1] if disp else {}
            vals = []
            for c in cols:
                absolute = c.endswith('@')
                k = c.rstrip('@')
                src = d if k in d else dl
                if k in dl and k not in d:
                    v = dl[k]
                elif absolute:
                    v = d.get(k, float('nan'))
                else:
                    v = d.get(k, float('nan')) - prev.get(k, float('nan'))
                vals.append(v)
            print('%6.1f  ' % (t - t0) + ' '.join('%10.1f' % v for v in vals))
        prev = d
