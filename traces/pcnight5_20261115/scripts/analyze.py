#!/usr/bin/env python3 -I
"""pcnight5 analysis. usage: analyze.py <boot-dir containing run1/ run2/ (3dmark|superposition)> [dmon.txt ...]
Prints 3DMark scores, GPU-Z style monitoring means and, for each dmon file, mean util / SM clock over active rows (sm util >= 20)."""
import sys, glob, zipfile, re, statistics as st, pathlib
def tdm(zp):
    z = zipfile.ZipFile(zp)
    x = z.read('Result.xml').decode('utf8', 'replace')
    sc = {k: float(v) for k, v in re.findall(r'<(\w+(?:Score|Test))>([\d.]+)</', x) if 'ForPass' not in k}
    out = dict(scores=sc)
    rows = z.read('Monitoring.csv').decode('utf8', 'replace').splitlines()
    hdr = rows[0].rstrip(';').split(';'); data = [r.split(';') for r in rows[1:]]
    def col(sub):
        i = next((n for n, h in enumerate(hdr) if sub in h), None)
        if i is None: return []
        v = []
        for r in data:
            try: v.append(float(r[i]))
            except (ValueError, IndexError): pass
        return v
    load, clk, tmp = col('GPUZ/GpuLoad'), col('GPUZ/GpuCoreClock'), col('Temperature')
    act = [c for l, c in zip(load, clk) if l >= 20] if load and clk else []
    out['mon'] = dict(n=len(data), gpu_load_mean_active=round(st.mean([l for l in load if l >= 20]), 1) if any(l >= 20 for l in load) else None,
                      gpu_clock_mean_active=round(st.mean(act), 1) if act else None, gpu_clock_max=max(clk) if clk else None,
                      gpu_temp_max=max(tmp) if tmp else None, hdr_gpu=[h for h in hdr if 'GPUZ' in h][:12])
    return out
def dmon(p):
    sm = []; pclk = []; pw = []; tp = []; fb = []
    for ln in open(p, errors='replace'):
        f = ln.split()
        if not f or f[0].startswith('#') or len(f) < 17: continue
        try: s = int(f[6]); pc = int(f[13])
        except ValueError: continue
        if s >= 20: sm.append(s); pclk.append(pc); pw.append(float(f[3])); tp.append(int(f[4])); fb.append(int(f[16]))
    return dict(active_rows=len(sm), sm_util_mean=round(st.mean(sm), 1) if sm else None, sm_clock_mean_mhz=round(st.mean(pclk), 1) if pclk else None,
                power_mean_w=round(st.mean(pw), 1) if pw else None, temp_max=max(tp) if tp else None, fb_max_mb=max(fb) if fb else None)
d = pathlib.Path(sys.argv[1])
for z in sorted(glob.glob(str(d / 'run*/3dmark/*/result.3dmark-result'))):
    print(z.split('/run')[1][:5], tdm(z))
for p in sys.argv[2:]:
    print(p, dmon(p))
