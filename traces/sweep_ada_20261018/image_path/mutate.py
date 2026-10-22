"""Manual plumbing mutants; no GPU, require >=2 distinct assertion failures."""
from pathlib import Path
import json, subprocess, sys
root=Path(__file__).resolve().parents[3]
target=root/'scripts/bench/guarded_fast_suite.py';original=target.read_text()
mutants=[
 ('drop_image_environment', ', KF_FASTGUEST_DIR=str(images)', ''),
 ('skip_selected_image_preflight', "for name in ('vmlinuz', 'initrd.cpio.gz'):", 'for name in ():'),
 ('ignore_explicit_directory', 'images = Path(a.fastguest_dir).resolve()', "images = Path(os.environ.get('KF_FASTGUEST_DIR', a.fastguest_dir)).resolve()"),
]
runner='''import unittest,json
s=unittest.defaultTestLoader.discover('scripts/bench/tests',pattern='test_guarded_fast_suite.py')
r=unittest.TextTestRunner(verbosity=0).run(s)
print('MUTATION_JSON='+json.dumps({'failures':[t.id() for t,_ in r.failures],'errors':[t.id() for t,_ in r.errors],'tests':r.testsRun}))
'''
rows=[]
try:
 for name,before,after in mutants:
  assert original.count(before)==1
  target.write_text(original.replace(before,after))
  p=subprocess.run([sys.executable,'-B','-c',runner],cwd=root,capture_output=True,text=True,timeout=30)
  data=json.loads(next(l.split('=',1)[1] for l in p.stdout.splitlines() if l.startswith('MUTATION_JSON=')))
  assert not data['errors'] and len(set(data['failures']))>=2,(name,data,p.stderr)
  rows.append(dict(mutant=name,**data));print(name,len(data['failures']),flush=True)
finally:target.write_text(original)
Path(__file__).with_name('kill_table.json').write_text(json.dumps(rows,indent=2)+'\n')
