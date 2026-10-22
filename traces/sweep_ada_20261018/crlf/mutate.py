from pathlib import Path
import subprocess,json,sys
root=Path(__file__).resolve().parents[3];p=root/'scripts/bench/native_xid_guard.py';original=p.read_text()
mutants=[('omit_CRLF_framing',"    serial = serial.replace('\\r\\n', '\\n')\n",''),('strip_embedded_CR',"serial = serial.replace('\\r\\n', '\\n')", "serial = serial.replace('\\r\\n', '\\n').replace('\\r', '')")]
runner="""import unittest,json
r=unittest.TextTestRunner(verbosity=0).run(unittest.defaultTestLoader.discover('scripts/bench/tests',pattern='test_guest_xid_guard.py'))
print('MUTATION_JSON='+json.dumps({'failures':[t.id() for t,_ in r.failures],'errors':[t.id() for t,_ in r.errors],'tests':r.testsRun}))
"""
rows=[]
try:
 for name,before,after in mutants:
  assert original.count(before)==1;p.write_text(original.replace(before,after))
  r=subprocess.run([sys.executable,'-B','-c',runner],cwd=root,capture_output=True,text=True,timeout=30)
  data=json.loads(next(l.split('=',1)[1] for l in r.stdout.splitlines() if l.startswith('MUTATION_JSON=')))
  assert not data['errors'] and len(set(data['failures']))>=2,(name,data,r.stderr[-2000:]);rows.append(dict(mutant=name,**data));print(name,len(data['failures']),flush=True)
finally:p.write_text(original)
Path(__file__).with_name('kill_table.json').write_text(json.dumps(rows,indent=2)+'\n')
