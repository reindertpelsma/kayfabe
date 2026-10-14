"""condense.py ALIGNED NAMES -- the non-equal rows of align.py's output, consecutive same-kind rows merged, named."""
import re, sys
names = {}
for l in open(sys.argv[2]):
    a, n = l.split()[:2]
    names.setdefault(int(a, 16), n)
row = re.compile(r'^(DIFF|HWONLY|KFONLY)\s+(?:after )?h(-?\d+)\s*([\d.]+)?\s*(.*)$')
groups = []
for l in open(sys.argv[1]):
    if l.startswith('#') or l.startswith('     h'):
        continue
    m = row.match(l)
    if not m:
        continue
    kind, h = m.group(1), int(m.group(2))
    rest = m.group(4)
    left, _, right = rest.partition('|')
    t = m.group(3)
    item = (left if kind != 'KFONLY' else right).strip()
    cm = re.search(r'ctrl (0x[0-9a-f]{8})', item)
    wm = re.match(r'k\d+ (.*)', right.strip()) if kind == 'KFONLY' else None
    if kind == 'KFONLY':
        item = wm.group(1) if wm else item
        cm = re.search(r'ctrl (0x[0-9a-f]{8})', item)
    key_item = re.sub(r' = 0x[0-9a-f]+', '', re.sub(r' st=\S+', '', item))
    if kind == 'DIFF':
        diff = right.split('  ', 1)[1].strip() if '  ' in right else ''
        dsig = re.sub(r'0x[0-9a-f]+|\d+', '#', diff)[:40]
    else:
        diff, dsig = '', ''
    nm = names.get(int(cm.group(1), 16), '') if cm else ''
    k = (kind, key_item, dsig)
    if groups and groups[-1]['k'] == k:
        g = groups[-1]
        g['n'] += 1
        g['t1'] = t or g['t1']
        g['h1'] = h
    else:
        groups.append(dict(k=k, n=1, h0=h, h1=h, t0=t, t1=t, item=item, nm=nm, diff=diff, kfi=right.strip()[:70]))
for g in groups:
    print('%-6s h%05d%s %s%s x%-4d %s %s %s' % (
        g['k'][0], g['h0'], '' if g['h1'] == g['h0'] else '-%05d' % g['h1'], g['t0'] or '      ',
        '' if g['t1'] == g['t0'] or not g['t1'] else '..' + g['t1'], g['n'], g['item'][:60], g['nm'], g['diff'][:160]))
