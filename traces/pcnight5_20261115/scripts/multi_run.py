#!/usr/bin/env python3
"""pcnight5: run one benchmark app N times in an already-held guest (hold-until-file harness boot).
usage: KF_WORK=<name> multi_run.py <app 3dmark|superposition> <n> <outdir> [preset-note]
Uses propimage_check.run_check unchanged except SUPERPOSITION (longer wait, extra screenshots, results harvest)."""
import importlib.util, json, sys, pathlib, os, time
PCS = pathlib.Path(os.environ.get('PCSRC', '/workspace/bench/pcnight2_20261115/src/scripts/bench/windows/propimage_check.py'))
W = pathlib.Path('/workspace/bench/propimage_20261112/work_' + os.environ['KF_WORK'])
spec = importlib.util.spec_from_file_location('propimage_check', PCS)
m = importlib.util.module_from_spec(spec); sys.modules['propimage_check'] = m; spec.loader.exec_module(m)
app, n, out = sys.argv[1], int(sys.argv[2]), pathlib.Path(sys.argv[3])
out.mkdir(parents=True, exist_ok=True)
st = json.loads((W / 'state.json').read_text())
g = m.Guest(st, str(W / 'id_kf'), st['ids']['net']['ssh_port'])

# Superposition: same click as the harness, but wait 20 s after 'Benchmark stopped' so the result screen is drawn,
# take a second screenshot, and harvest any result files the app wrote (text/html/json/png, bounded) into extra.txt.
base = m.SUPERPOSITION
tail_old = 'Save-Screen "$D\\result.png"\n$f=Get-Item $source'
assert tail_old in base
tail_new = r'''Start-Sleep 20
Save-Screen "$D\result.png"
Start-Sleep 15
Save-Screen "$D\result2.png"
$ex=@()
foreach($root in @("$env:USERPROFILE\Superposition","$env:USERPROFILE\Documents\Superposition","$env:USERPROFILE\Documents\Unigine","$env:APPDATA\Unigine","$env:LOCALAPPDATA\Unigine")){
 if(Test-Path $root){ foreach($f in Get-ChildItem $root -Recurse -File -ErrorAction SilentlyContinue){
  $ex += "FILE $($f.FullName) $($f.Length) $($f.LastWriteTimeUtc.ToString('o'))"
  if($f.LastWriteTimeUtc -gt $r_start -and $f.Length -lt 200000 -and $f.Extension -match 'txt|json|html|xml|log|csv'){ $ex += "--- BEGIN $($f.Name)"; $ex += (Get-Content $f.FullName -Raw); $ex += "--- END" }
 } }
}
$ex | Set-Content -Encoding UTF8 "$D\extra.txt"
$f=Get-Item $source'''
pre = r'''$r_start=(Get-Date).ToUniversalTime().AddMinutes(-1)
'''
m.SUPERPOSITION = pre + base.replace(tail_old, tail_new) + '\nGet-Process | Where-Object {$_.Name -match "^(launcher|superposition|Superposition)"} | Stop-Process -Force -ErrorAction SilentlyContinue\n'
# 3DMark title: at fb-mb 10240 / the 12 GB card the HOME page recommends Speed Way and Steel Nomad, not Time Spy
# (Time Spy is only on the lazily rendered BENCHMARKS list, whose UIA tree gave no usable rect). KF_3DM_TITLE picks the home-page card.
T = os.environ.get('KF_3DM_TITLE', 'Steel Nomad')
m.THREEDMARK_RUN = m.THREEDMARK_RUN.replace("'Time Spy'", "'" + T + "'")
rows = []
for i in range(1, n + 1):
    t0 = time.time()
    print(f'RUN {i} {app} start {time.strftime("%FT%TZ", time.gmtime())}', flush=True)
    row = m.run_check(app, {}, out / f'run{i}', g, superposition_run=(app == 'superposition'))
    print(f'RUN {i} done {time.time()-t0:.0f}s ok={row.get("ok")} {row.get("signal") or row.get("error")}', flush=True)
    d = sorted((out / f'run{i}' / app).iterdir())[-1]
    remote = m.GUEST_DIR.replace('\\', '/') + '/' + d.name + '/' + app
    for f in ('extra.txt', 'result2.png', 'Monitoring.csv'):
        try: g.scp(remote + '/' + f, str(d / f), False)
        except Exception as e: print('pull', f, e)
    rows.append(dict(run=i, secs=round(time.time() - t0), row=row, dir=str(d)))
    time.sleep(20)
(out / 'multi_rows.json').write_text(json.dumps(rows, indent=1, default=str))
