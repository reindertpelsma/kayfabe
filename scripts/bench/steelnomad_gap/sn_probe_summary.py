# sn_probe_summary.py 'KAYFABE_GLOB' 'VFIO_GLOB': side-by-side median of per-run medians of the sn_gpu_probe.ps1 outputs
import re, sys, statistics as S, glob
def parse(files):
    out = {}
    for f in files:
        for l in open(f, errors='replace'):
            m = re.match(r'GP (\S+)\s+(.*?)\s+(?:round(\d) )?(?:us|time) n=\d+ med=([\d.]+)', l)
            if not m: continue
            key = m.group(1) + ' ' + m.group(2).strip()
            if m.group(3) is not None and m.group(3) != '2': continue
            if l.startswith('GP C') or l.startswith('GP D') or l.startswith('GP E'):
                g = re.search(r'GiB/s n=\d+ med=([\d.]+)', l); v = float(g.group(1)); key += ' GiB/s'
            else: v = float(m.group(4))
            out.setdefault(key, []).append(v)
    return out
k = parse(sorted(glob.glob(sys.argv[1]))); v = parse(sorted(glob.glob(sys.argv[2])))
print('| probe (median of per-run medians [min..max over 3 runs]) | kayfabe | VFIO native | ratio |'); print('|---|---|---|---|')
for key in sorted(set(k) | set(v)):
    a = k.get(key); b = v.get(key)
    f = lambda x: '-' if not x else f'{S.median(x):.4g} [{min(x):.4g}..{max(x):.4g}] (n={len(x)})'
    r = f'{S.median(a)/S.median(b):.3g}' if a and b else '-'
    print(f'| {key} | {f(a)} | {f(b)} | {r} |')
