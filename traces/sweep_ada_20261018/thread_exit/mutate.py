from pathlib import Path
import subprocess,json,sys
root=Path(__file__).resolve().parents[3];path=root/'scripts/bench/native_xid_guard.py';original=path.read_text()
mutants=[
 ('reject_legitimate_leader_exit',"except FileNotFoundError:\n        # A thread-group", "except FileNotFoundError:\n        raise GuardRefused('mutant rejects thread exit')\n        # A thread-group"),
 ('ignore_live_worker_image',"if retained != context['exe']:",'if False:'),
 ('accept_missing_exe_without_exit_state','return bool(int(fields[6]) & 0x4)','return True'),
]
runner="""import unittest,json
r=unittest.TextTestRunner(verbosity=0).run(unittest.defaultTestLoader.discover('scripts/bench/tests',pattern='test_guest_thread_exit_guard.py'))
print('MUTATION_JSON='+json.dumps({'failures':[t.id() for t,_ in r.failures],'errors':[t.id() for t,_ in r.errors],'tests':r.testsRun}))
"""
rows=[]
try:
 for name,before,after in mutants:
  assert original.count(before)==1
  path.write_text(original.replace(before,after))
  p=subprocess.run([sys.executable,'-B','-c',runner],cwd=root,capture_output=True,text=True,timeout=30)
  row=json.loads(next(l.split('=',1)[1] for l in p.stdout.splitlines() if l.startswith('MUTATION_JSON=')))
  assert not row['errors'] and len(set(row['failures']))>=2,(name,row,p.stderr[-2000:])
  rows.append(dict(mutant=name,**row));print(name,len(row['failures']),flush=True)
finally:path.write_text(original)
Path(__file__).with_name('kill_table.json').write_text(json.dumps(rows,indent=2)+'\n')
