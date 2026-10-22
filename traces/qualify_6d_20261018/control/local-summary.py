import importlib.util,json,re,sys,hashlib
from pathlib import Path
R=Path('/workspace/q4-wt');P=Path('/workspace/q4-6d-private')
log=P/'candidate-tests.log';text=log.read_text()
rows=re.findall(r'test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored;',text)
j={'classification':'MEASURED','evidence':'candidate-tests.json','candidate':'70da04ffaebdee3b51e5eca6aa1eb3919e92e80d','command':"KAYFABE_NO_KVM=1 cargo test --release --locked -j2 -p 'kf-*' --no-fail-fast",'summaries':len(rows),'passed':sum(int(x[1]) for x in rows),'failed':sum(int(x[2]) for x in rows),'ignored':sum(int(x[3]) for x in rows),'log_sha256':hashlib.sha256(log.read_bytes()).hexdigest(),'log_bytes':log.stat().st_size,'test_failures':re.findall(r'^test (.+) \.\.\. FAILED$',text,re.M)}
checks=P/'local-checks.json'
if checks.exists():
 c=json.loads(checks.read_text());done=next((x for x in c['checks'] if x['name']=='candidate-tests'),None)
 if done:j.update(exit=done['exit'],start=done['start'],end=done['end'])
(P/'candidate-tests.json').write_text(json.dumps(j,indent=2)+'\n');print(json.dumps(j))
