import sys, bisect
# flips whose QI..QE window contains a VSync interrupt (VI): what VD mode that VSync reported, and whether the
# flip was then retired by a later VD IMMEDIATE_SW_FLIP_QUEUE (real-HW behaviour under the kayfabe race)
qi, qe, vi, vd = {}, {}, [], []
for line in open(sys.argv[1], errors='replace'):
    p = line.split(None, 6)
    if len(p) < 7 or not line.startswith("D ") or not p[2].startswith("t="): continue
    t = int(p[2][2:])
    if p[1] == 'VI': vi.append(t)
    elif p[1] == 'VD': vd.append((t, p[6].split('"')[1].strip()))
    elif 'MMIOFLIP' in line and p[1] in ('QI', 'QE'):
        k = (p[6].split(',')[0], p[6].split(',')[2].strip()); (qi if p[1] == 'QI' else qe)[k] = t
vi.sort(); vd.sort(); vt = [x[0] for x in vd]
n = 0
for k in qe:
    if k not in qi: continue
    a, b = qi[k], qe[k]
    i = bisect.bisect_left(vi, a)
    if i < len(vi) and vi[i] < b:
        n += 1
        j = bisect.bisect_left(vt, b)
        print('race flip', k[1], 'QI->QE %.2f ms' % ((b - a) / 1e4), 'VI inside; next VD after QE: %s at +%.2f ms' % (vd[j][1], (vd[j][0] - b) / 1e4) if j < len(vd) else 'none')
print('flips with a VSync between QI and QE:', n, 'of', len(qe))
