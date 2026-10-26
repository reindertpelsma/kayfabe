#!/usr/bin/env python3
"""One line per tier-2 probe output (traces/.../tier2/*.txt): the die the HOST reported, the driver, and the answers that matter.

usage: summarize_tier2.py OUT.tsv FILE...      (lane C, 2026-10-14; reads the kit's text output only)
"""
import re, sys, os

def pr(t, cmd, size=None, nth=0):
    hits = []
    for l in t:
        m = re.match(r'PROBE kind=subdevice cmd=%s size=(\d+) status=(0x\w+) (\w+)(?: reply_head=(\w+))?' % cmd, l)
        if m and (size is None or int(m.group(1)) == size):
            hits.append(m.group(3) + (':' + m.group(4) if m.group(4) else ''))
    return hits[nth] if len(hits) > nth else '-'

def gss(t, cmd, size):
    for l in t:
        m = re.match(r'GSSROW cmd=%s size=(\d+) .*probe=zero status=(0x\w+) (\w+)' % cmd, l)
        if m and int(m.group(1)) == size:
            return m.group(3)
    return '-'

def short(s):
    return {'INVALID_ARGUMENT': 'inval', 'OK': 'OK', '-': '-'}.get(s.split(':')[0], s.split(':')[0]) if not s.startswith('OK:') else 'OK'

def main():
    out = ['\t'.join(['file', 'gpu', 'pci', 'die_by_host', 'driver', 'session', 'cap_sys_admin',
                      '852e@528', '852e@596', '853a@1456', '853a@15352', 'a618@8620', 'a618@8632', '853b@14320', 'b201@2188', 'b201@2444',
                      'scan_852e', 'scan_853a', 'scan_a618', 'scan_b201', '8162', 'a001', '9001', '9009', 'zbc_color', 'zbc_depth', 'zbc_stencil'])]
    for f in sys.argv[2:]:
        t = open(f).read().splitlines()
        gpu = next((l for l in t if l.startswith('GPU ')), 'GPU ?, ?, ?')
        mm = re.search(r'0x([0-9A-F]{4})10DE', gpu)
        parts = [x.strip() for x in gpu[4:].split(',')]
        die = next((l.split('die=')[1] for l in t if l.startswith('PROBE_DIE')), '-')
        sess = any(l.startswith('PROBE_SESSION') for l in t)
        adm = 'admin' if any('CapEff' in l and not l.endswith('0000000000000000') and 'a80405fb' not in l and '000001ffffffffff' in l for l in t) else 'no'
        scan = {}
        for c in ('852e', '853a', 'a618', 'b201'):
            sizes = [re.search(r'size=(\d+)', l).group(1) for l in t if l.startswith('SCAN cmd=0x2080%s ' % c) and ' size=' in l]
            scan[c] = '/'.join(sizes) if sizes else ('none' if any(l.startswith('SCAN cmd=0x2080%s range' % c) for l in t) else '-')
        z = {}
        for ty in '123':
            v = []
            for l in t:
                m = re.match(r'ZBC table=%s index=(\d+) valid=1 format=(\d+) value=(.*)' % ty, l)
                if m:
                    w = m.group(3).split()
                    v.append('i%s:f%s:%s' % (m.group(1), m.group(2), ''.join(x[:4] if x != '00000000' else '0' for x in w[:10:2] if True)[:12]))
            z[ty] = ';'.join(v) if v else '-'
        row = [os.path.basename(f), parts[0].replace('NVIDIA ', '').replace('GeForce ', ''), mm.group(1) if mm else '?', die, parts[-1], str(sess), adm,
               short(gss(t, '0x2080852e', 528)), short(pr(t, '0x2080852e', 596)), short(gss(t, '0x2080853a', 1456)), short(pr(t, '0x2080853a', 15352)),
               short(gss(t, '0x2080a618', 8620)), short(pr(t, '0x2080a618', 8632)), short(pr(t, '0x2080853b', 14320)),
               short(pr(t, '0x2080b201', 2188)), short(pr(t, '0x2080b201', 2444)),
               scan['852e'], scan['853a'], scan['a618'], scan['b201'],
               pr(t, '0x20808162').split(':')[-1], pr(t, '0x2080a001').split(':')[-1][:24], pr(t, '0x20809001').split(':')[-1], pr(t, '0x20809009').split(':')[-1],
               z['1'], z['2'], z['3']]
        out.append('\t'.join(row))
    open(sys.argv[1], 'w').write('\n'.join(out) + '\n')

if __name__ == '__main__':
    main()
