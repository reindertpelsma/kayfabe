"""regname.py ADDR... -- name display-range BAR0 addresses from ogkm-595.84 published dev_disp.h (all disp versions)."""
import glob, re, sys
root = '/workspace/ogkm-tars/open-gpu-kernel-modules-595.84/src/common/inc/swref/published/disp/'
plain = re.compile(r'#define\s+(NV_\w+)\s+0x([0-9A-Fa-f]{8})\s*/\*\s*R')
arr = re.compile(r'#define\s+(NV_\w+)\((\w)\)\s+\(0x([0-9A-Fa-f]{8})\s*\+\s*\(\w\)\s*\*\s*(\d+|0x[0-9A-Fa-f]+)\)')
arr2 = re.compile(r'#define\s+(NV_\w+)\((\w),(\w)\)\s+\(0x([0-9A-Fa-f]{8})\s*\+\s*\(\w\)\s*\*\s*(\d+|0x[0-9A-Fa-f]+)\s*\+\s*\(\w\)\s*\*\s*(\d+|0x[0-9A-Fa-f]+)\)')
regs = []
for f in sorted(glob.glob(root + '*/dev_disp.h')):
    v = f.split('/')[-2]
    for line in open(f, errors='replace'):
        m = plain.search(line)
        if m:
            regs.append((v, m.group(1), int(m.group(2), 16), 0, 1, 0, 1)); continue
        m = arr2.search(line)
        if m:
            regs.append((v, m.group(1), int(m.group(4), 16), int(m.group(5), 0), 64, int(m.group(6), 0), 64)); continue
        m = arr.search(line)
        if m:
            regs.append((v, m.group(1), int(m.group(3), 16), int(m.group(4), 0), 64, 0, 1))
def name(a):
    out = []
    for v, n, b, s, cnt, s2, cnt2 in regs:
        if s == 0:
            if a == b:
                out.append('%s:%s' % (v, n))
            continue
        r = a - b
        if r < 0:
            continue
        i, rem = divmod(r, s)
        if i < 8 and (rem == 0 or (s2 and rem % s2 == 0 and rem // s2 < 8)):
            out.append('%s:%s(%d%s)' % (v, n, i, '' if rem == 0 else ',%d' % (rem // s2)))
    return out
if __name__ == '__main__':
    for x in sys.argv[1:]:
        a = int(x, 16)
        ns = name(a)
        # prefer the newest version's names
        print('0x%06x %s' % (a, ' | '.join(sorted(set(n.split(':', 1)[1] for n in ns))) or '?'), ' [' + ','.join(sorted(set(n.split(':')[0] for n in ns))) + ']')
