g = {512:dict(med=126.132,first=127.129,p90=135.005,mn=102.870,mx=138.051,gf=2.128,bpl=115.014,bgf=2.334,h2d=232.069,d2h=102.575),
     1024:dict(med=198.084,first=294.814,p90=208.871,mn=175.704,mx=284.748,gf=10.841,bpl=107.540,bgf=19.969,h2d=1019.768,d2h=446.494),
     2048:dict(med=623.790,first=699.719,p90=653.765,mn=585.071,mx=690.403,gf=27.541,bpl=471.661,bgf=36.424,h2d=3459.421,d2h=1729.489)}
n = {512:dict(med=0.386,first=7.348,p90=0.388,mn=0.384,mx=0.396,gf=695.740,bpl=0.377,bgf=711.108,h2d=0.615,d2h=0.252),
     1024:dict(med=3.010,first=3.008,p90=3.020,mn=2.783,mx=3.027,gf=713.453,bpl=2.773,bgf=774.487,h2d=1.101,d2h=0.746),
     2048:dict(med=22.469,first=22.382,p90=22.481,mn=22.372,mx=22.518,gf=764.616,bpl=22.468,bgf=764.636,h2d=4.195,d2h=2.136)}
print(f"{'N':>5} {'g_med':>9} {'n_med':>8} {'RATIO':>8} {'slowdown':>9} {'gap_ms':>9} {'BATCHRAT':>9} {'g_bpl':>8}")
for N in (512,1024,2048):
    r = g[N]['gf']/n[N]['gf']; gap = g[N]['med']-n[N]['med']; sd = g[N]['med']/n[N]['med']
    br = n[N]['bpl']/g[N]['bpl']
    print(f"{N:>5} {g[N]['med']:>9.3f} {n[N]['med']:>8.3f} {r:>8.5f} {sd:>8.1f}x {gap:>9.2f} {br:>9.5f} {g[N]['bpl']:>8.2f}")
print()
print("COPY BANDWIDTH — per 4 KiB page cost")
print(f"{'N':>5} {'dir':>5} {'MiB':>6} {'g_ms':>10} {'n_ms':>8} {'g_MiB/s':>9} {'n_MiB/s':>9} {'g_us/4KiBpg':>12} {'ratio':>8}")
for N in (512,1024,2048):
    szM = N*N*4/1048576
    for d,mult in (('h2d',2),('d2h',1)):
        mib = szM*mult; pages = mib*256
        gm=g[N][d]; nm=n[N][d]
        print(f"{N:>5} {d:>5} {mib:>6.0f} {gm:>10.3f} {nm:>8.3f} {mib/(gm/1000):>9.2f} {mib/(nm/1000):>9.1f} {gm*1000/pages:>12.1f} {(mib/(gm/1000))/(mib/(nm/1000)):>8.5f}")
print()
print("FIXED-vs-PROPORTIONAL model on SOLO launches: guest = C + k*native")
import itertools
for a,b in itertools.combinations((512,1024,2048),2):
    k=(g[b]['med']-g[a]['med'])/(n[b]['med']-n[a]['med']); C=g[a]['med']-k*n[a]['med']
    print(f"  from N={a},{b}:  k={k:6.2f}x   C={C:8.2f} ms")
print()
print("BATCHED: guest batch per-launch vs solo")
for N in (512,1024,2048):
    print(f"  N={N:>5}: solo {g[N]['med']:8.2f}  batched {g[N]['bpl']:8.2f}  recovered {g[N]['med']-g[N]['bpl']:7.2f} ms ({100*(1-g[N]['bpl']/g[N]['med']):.1f}%)")
