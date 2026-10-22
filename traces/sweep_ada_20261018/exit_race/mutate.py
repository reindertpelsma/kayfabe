from pathlib import Path
import subprocess,sys,json
root=Path(__file__).resolve().parents[3]
g=root/'scripts/bench/native_xid_guard.py';s=root/'scripts/bench/guarded_fast_suite.py'
originals={g:g.read_text(),s:s.read_text()}
block="""if ([record['arm'] for record in records] != completed + [arm] or
                any(record['revision'] != a.revision or record['pid'] <= 1 or record['starttime'] <= 0
                    for record in records)):"""
mutants=[
 ('uncaught_proc_exit',g,'except (GuardRefused, FileNotFoundError):','except GuardRefused:'),
 ('accept_missing_exe_while_live',g,'if child.poll() is None:\n                        raise','if False:\n                        raise'),
 ('ignore_child_exit_status',g,'if child.wait() != 0:','if False:'),
 ('drop_success_completion',g,"state.setdefault('completed_arms', []).append(context)",'pass  # no durable completion'),
 ('trust_shell_PASS_without_completion',s,block,'if False:'),
 ('trust_completion_without_recovery',s,"if [proof['accepted_faults'] for proof in state.get('guest_proofs', [])] != expected:",'if False:'),
]
runner="""import unittest,json
r=unittest.TextTestRunner(verbosity=0).run(unittest.defaultTestLoader.discover('scripts/bench/tests',pattern='test_*guard*.py'))
print('MUTATION_JSON='+json.dumps({'failures':[t.id() for t,_ in r.failures],'errors':[t.id() for t,_ in r.errors],'tests':r.testsRun}))
"""
rows=[]
try:
 for name,path,before,after in mutants:
  assert originals[path].count(before)==1,(name,originals[path].count(before))
  path.write_text(originals[path].replace(before,after))
  p=subprocess.run([sys.executable,'-B','-c',runner],cwd=root,capture_output=True,text=True,timeout=30)
  data=json.loads(next(l.split('=',1)[1] for l in p.stdout.splitlines() if l.startswith('MUTATION_JSON=')))
  path.write_text(originals[path])
  assert not data['errors'] and len(set(data['failures']))>=2,(name,data,p.stderr[-3000:])
  rows.append(dict(mutant=name,**data));print(name,len(data['failures']),flush=True)
finally:
 for path,source in originals.items():path.write_text(source)
Path(__file__).with_name('kill_table.json').write_text(json.dumps(rows,indent=2)+'\n')
