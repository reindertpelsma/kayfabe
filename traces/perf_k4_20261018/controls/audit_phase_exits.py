"""Audit completed primary phases separately from the failed optional control."""
import json
from pathlib import Path
P=Path('/workspace/q4-k4-private');O=Path('/workspace/q4-wt/traces/perf_k4_20261018');rows=[]
for n in (4642,4648,4649,4643):
 files=json.loads((P/str(n)/'proxy-export.json').read_text());f=next(x for x in files if x['name']=='phases.jsonl');events=[json.loads(l) for l in f['text'].splitlines() if l.strip()];ends=[x for x in events if 'exit' in x];expected=18 if n in (4649,4643) else 12
 assert len(ends)==expected and len(events)==expected*2 and all(x['exit']==0 for x in ends)
 assert len({x['phase'] for x in ends})==expected
 rows.append({'run':n,'primary_and_windowed_phase_exit_count':len(ends),'nonzero_exits':0,'last_phase_end_guest_UTC':ends[-1]['end'],'optional_mode_control':'FAIL_APPLIED_MODE_VERIFICATION_AFTER_THESE_PHASES' if n==4643 else 'NOT_REQUESTED'})
(O/'main-phase-exit-receipts.json').write_text(json.dumps({'classification':'MEASURED','evidence':'main-phase-exit-receipts.json','scope':'Actual owned child exit records for primary/windowed phases only; does not override original proxyFAIL or whole-run rejection in4643','runs':rows},indent=2)+'\n');print('Verified60 individual phase exits0; whole4643 verdict remains rejected')
