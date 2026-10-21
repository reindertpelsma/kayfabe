"""Page B = the DERIVED page (PR65 build) with ONLY the PRECOMP region 0x780..0x87c taken from a native own-die read.
Diagnostic input for KF3_DIAG_CAPS_PAGE_FILE; never committed, never shipped.
usage: make_page_b.py ORACLE.tsv NATIVE.txt OUT.txt"""
import sys
orc, nat, out = sys.argv[1:4]
native = {}
for l in open(nat):
    f = l.split()
    if f and f[0] == 'WORD': native[int(f[1], 16)] = int(f[2], 16)
derived = dict(native)  # unlisted-in-diff words are equal between derived and native
for l in open(orc):
    if l.startswith('#') or l.startswith('offset'): continue
    f = l.rstrip('\n').split('\t')
    off, d = int(f[0], 16), int(f[1], 16)
    if 0x780 <= off <= 0x87c: continue          # PRECOMP stays the die's
    if d == 0: derived.pop(off, None)
    else: derived[off] = d
# words only the derivation sets (native 0) appear in the oracle diff with native 0: handled above (set).
with open(out, 'w') as o:
    o.write('# DIAGNOSTIC page B: derived (PR65) except PRECOMP 0x780..0x87c from a native own-die read\n')
    for off in sorted(derived): o.write('WORD 0x%03x 0x%08x\n' % (off, derived[off]))
    o.write('# end words=%d\n' % len(derived))
print(len(derived), 'words')
