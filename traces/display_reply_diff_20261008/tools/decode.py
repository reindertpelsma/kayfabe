"""decode.py TSV REG VALUE... -- the set fields of a class register value, from the derived class TSV."""
import sys
fields = []
for l in open(sys.argv[1]):
    f = l.rstrip('\n').split('\t')
    if f[0] == 'F' and f[1].startswith(sys.argv[2] + '_'):
        fields.append((f[1][len(sys.argv[2]) + 1:], int(f[2]), int(f[3])))
for v in sys.argv[3:]:
    x = int(v, 16)
    out = []
    for n, a, b in fields:
        lo, hi = min(a, b), max(a, b)
        val = (x >> lo) & ((1 << (hi - lo + 1)) - 1)
        if val:
            out.append('%s=%d' % (n, val))
    print(v, ' '.join(out))
