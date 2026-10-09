#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""make_subset.py FULL.txt OUT.txt --drop KEY[,KEY...] | --keep-only KEY[,KEY...] | --family NAME

Derives a bisect refusal list from refusal-full.txt. Dropped / not-kept rules stay in the output as commented lines (so
the file still documents the full list). Families: ctl (all fn76), alloc (fn103), legacy (fn76 with cmd & 0x8000),
display (fn76 0x73xxxx), rest (fn76 neither legacy nor display). A key given without `fn` means fn76."""
import sys

src, dst = sys.argv[1], sys.argv[2]
mode, arg = sys.argv[3], sys.argv[4]


def fam(fn, key):
    f = set()
    if fn == 103:
        f.add('alloc')
    else:
        f.add('ctl')
        if (key >> 16) == 0x73:
            f.add('display')
        elif key & 0x8000:
            f.add('legacy')
        else:
            f.add('rest')
    return f


keys = {(103 if k.startswith('103:') else 76, int(k.split(':')[-1], 0)) for k in arg.split(',')} if mode != '--family' else set()
out = []
n = d = 0
for line in open(src):
    t = line.split('#')[0].split()
    if len(t) < 2:
        out.append(line)
        continue
    fn, key = int(t[0]), int(t[1], 0)
    if mode == '--drop':
        keep = (fn, key) not in keys
    elif mode == '--keep-only':
        keep = (fn, key) in keys
    else:
        keep = arg in fam(fn, key)
    if keep:
        out.append(line)
        n += 1
    else:
        out.append('# not in this subset: ' + line)
        d += 1
out.insert(0, '# subset of %s (%s %s): %d rules kept, %d left out\n' % (src.split('/')[-1], mode, arg, n, d))
open(dst, 'w').write(''.join(out))
print(dst, n, 'kept', d, 'left out')
