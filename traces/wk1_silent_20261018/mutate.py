import json, os, subprocess
from pathlib import Path
ROOT=Path.cwd()
ORIGINAL={n:(ROOT/n).read_text() for n in ['runner.py','src/ledger.js']}
CASES=[]
def mutate(name,file,old,new,count=1):
 assert ORIGINAL[file].count(old)==count,(name,ORIGINAL[file].count(old))
 CASES.append((name,file,ORIGINAL[file].replace(old,new)))
mutate('runner-failure-line-omitted','runner.py',
       'if not (result["verified"] == 1 and result["verdict"] == "PASS"):', 'if False:')
mutate('setup-failure-line-omitted','runner.py','if not success and not diagnosed:', 'if False:')
mutate('main-duplicates-runner-line','runner.py','if not success and not diagnosed:', 'if not success:')
mutate('phase-lost','runner.py','suffix = " phase=" + phase + " state=" + state + " http="',
       'suffix = " phase=setup state=" + state + " http="')
mutate('http-class-lost','runner.py',
       'code = status if type(status) is int and 100 <= status <= 599 else None', 'code = None')
mutate('diagnostic-unbounded','runner.py', 'line)[:800] + suffix', 'line) + suffix')
mutate('diagnostic-newline-injection','runner.py',
       're.sub(r"[^\\x20-\\x7e]", "?", line)[:800]', 'line[:800]')
mutate('oidc-phase-lost','runner.py',
       'if isinstance(error, Diagnosed) and getattr(error, "phase", None):', 'if False:')
mutate('legacy-receipt-marker-required','src/ledger.js',
       'if (v.created === undefined) {', 'if (false) {')
mutate('legacy-absent-id-not-recognized','src/ledger.js',
       'record.id == null', 'record.id === null', 3)
# Scope these to the new block; existing interrupted/zero-row fallbacks retain their checks.
s=ORIGINAL['src/ledger.js'];start=s.index('      if (v.created === undefined) {')
end=s.index('      // Recover old partial settlements',start);body=s[start:end]
for name,old,new in [
 ('legacy-gone-proof-skipped',"record.closed === 'GONE' &&",'true &&'),
 ('legacy-time-binding-skipped','record.started === v.started','true'),
 ('legacy-real-instance-proof-skipped','record.id == null &&','true &&'),
 ('legacy-admin-writes-storage','v = { ...v, ended: record.ended, created: false, price: 0, bandwidthCost: 0 };',
  "await this.storage.delete(key);\n          v = { ...v, ended: record.ended, created: false, price: 0, bandwidthCost: 0 };"),
]:
 assert body.count(old)==1,name
 CASES.append((name,'src/ledger.js',s[:start]+body.replace(old,new)+s[end:]))
mutate('legacy-admission-proof-not-persisted','src/ledger.js',
       '          if (prune) await this.put(key, v);', '')
mutate('legacy-expiry-proof-not-persisted','src/ledger.js',
       "if (cost && cost.created === undefined && cost.started === record.started)", 'if (false)')
env={**os.environ,'PYTHONDONTWRITEBYTECODE':'1',
     'PYTHONPYCACHEPREFIX':'/tmp/wk1-silent-no-bytecode'}
baseline=subprocess.run(['nice','-n','19','node','--test','test/silent.test.js',
                         'test/silent-retention.test.js','workerd/silent.test.js'],
                        capture_output=True,text=True,env=env)
assert baseline.returncode==0,(baseline.stdout[-4000:],baseline.stderr)
try:
 for name,file,source in CASES:
  (ROOT/file).write_text(source)
  p=subprocess.run(['nice','-n','19','node','--test','--test-reporter=tap',
                    'test/silent.test.js','test/silent-retention.test.js','workerd/silent.test.js'],capture_output=True,text=True,env=env)
  killers=[l for l in p.stdout.splitlines() if l.lstrip().startswith('not ok ')]
  print(json.dumps({'mutant':name,'killers':killers,'count':len(killers)}),flush=True)
  (ROOT/file).write_text(ORIGINAL[file])
  assert p.returncode and len(killers)>=2,(name,p.stdout[-3500:],p.stderr)
finally:
 for file,source in ORIGINAL.items():(ROOT/file).write_text(source)
