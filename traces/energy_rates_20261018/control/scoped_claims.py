import importlib.util,json,subprocess,sys,datetime
from pathlib import Path
R=Path('/workspace/q3-wt');BASE='b6f6d3010600e77d02e1e48b5e497d24fd4306d5'
spec=importlib.util.spec_from_file_location('debt',R/'scripts/ci/debt.py');d=importlib.util.module_from_spec(spec);spec.loader.exec_module(d)
files=set(subprocess.check_output(['git','diff','--name-only',BASE],cwd=R,text=True).splitlines())
files.update(subprocess.check_output(['git','ls-files','--others','--exclude-standard'],cwd=R,text=True).splitlines())
actual=d.claims();baseline=json.loads((R/'scripts/ci/claims-debt.json').read_text())
new=d.compare(actual,baseline);scoped=[k for k in new if k.split('|',1)[0] in files]
r={'classification':'MEASURED','evidence':'scoped-claims.json','base':BASE,'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=R,text=True).strip(),'scope':'Owner-approved Q3-added/changed files only; evidence-only [skip ci]; inherited debt untouched','files':sorted(files),'new':len(scoped),'sites':scoped,'inherited_ignored':len(new)-len(scoped),'time':datetime.datetime.now(datetime.timezone.utc).isoformat()}
(R/'traces/energy_rates_20261018/scoped-claims.json').write_text(json.dumps(r,indent=2)+'\n')
for k in scoped:print('NEW',k,actual[k])
print(f"claims: scope=Q3 changed-files={len(files)} new={len(scoped)} inherited-ignored={len(new)-len(scoped)}")
sys.exit(bool(scoped))
