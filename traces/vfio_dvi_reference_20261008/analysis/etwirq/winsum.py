import collections, sys
# winsum.py MERGED.txt -- counts of MSI sources, doorbell tokens, ETW event kinds in a merged window
msi, db, etw = collections.Counter(), collections.Counter(), collections.Counter()
for line in open(sys.argv[1]):
    p = line.split()
    if len(p) < 3:
        continue
    if p[1] == 'MSI':
        msi['DISP' if p[2].startswith('DISP') else p[2].split('+DISP')[0] + ('+DISP' if '+DISP' in p[2] else '')] += 1
    elif p[1] == 'DB':
        db[p[2]] += 1
    elif p[1] == 'ETW':
        k = p[2]
        if '"' in line:
            k += ' ' + line.split('"')[1].strip()
        etw[k] += 1
print('MSI by source:', dict(msi.most_common()))
print('doorbells by token:', dict(db.most_common(12)))
print('ETW:', dict(etw.most_common(16)))
