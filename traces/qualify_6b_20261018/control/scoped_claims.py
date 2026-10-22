import importlib.util,json,subprocess,sys,datetime
from pathlib import Path
R=Path('/workspace/q4-wt');BASE='6e0ac33871eee5acb3bef709bdf64cea9ec4ded1'
spec=importlib.util.spec_from_file_location('debt',R/'scripts/ci/debt.py');d=importlib.util.module_from_spec(spec);spec.loader.exec_module(d)
files=set(subprocess.check_output(['git','diff','--name-only',BASE],cwd=R,text=True).splitlines())
files.update(subprocess.check_output(['git','ls-files','--others','--exclude-standard'],cwd=R,text=True).splitlines())
actual=d.claims();baseline=json.loads((R/'scripts/ci/claims-debt.json').read_text())
new=d.compare(actual,baseline);scoped=[k for k in new if k.split('|',1)[0] in files]
r={'classification':'MEASURED','evidence':'scoped-claims.json','base':BASE,'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=R,text=True).strip(),'scope':'Owner-approved Q4-added/changed files only; evidence-only [skip ci]; inherited debt untouched','files':sorted(files),'new':len(scoped),'sites':scoped,'inherited_ignored':len(new)-len(scoped),'time':datetime.datetime.now(datetime.timezone.utc).isoformat()}
(R/'traces/qualify_6b_20261018/scoped-claims.json').write_text(json.dumps(r,indent=2)+'\n')
for k in scoped:print('NEW',k,actual[k])
print(f"claims: scope=Q4 changed-files={len(files)} new={len(scoped)} inherited-ignored={len(new)-len(scoped)}")
sys.exit(bool(scoped))
