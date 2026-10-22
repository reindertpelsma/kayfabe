"""Regenerate the partial comparison from completed vendor composites only."""
import csv,json,statistics
from pathlib import Path
R=Path(__file__).resolve().parents[1]
workloads=['blender-01','catia-07','creo-04','energy-04','enscape-01','maya-07','medical-04','solidworks-08','unreal_engine-01']
def read_ref(name):
 result={}
 for row in csv.DictReader((R/name).open()):
  if row['workload_status']=='Completed' and row['composite']:
   result.setdefault(row['workload'],[]).append(float(row['composite']))
 return result
baseline=read_ref('master-baseline-scores.csv');vfio=read_ref('vfio-reference-scores.csv');candidate={};trials=[]
for n in [4400,4401,4402,4403,4404]:
 path=R/str(n)/'result.json'
 if not path.exists():continue
 data=json.loads(path.read_text());r=data['result']
 if n!=4400:trials.append({'run':n,'verdict':r['verdict'],'events153':sum(e['id']==153 for e in r.get('events',[])),'events4101':sum(e['id']==4101 for e in r.get('events',[])),'xid_delta':r['host_stop']['new_host_xids'],'restored':r['restored']})
 # Run 4400's vendor logs completed these three before the Energy guard fired.
 accepted={'blender-01','catia-07','creo-04'} if n==4400 else ({'energy-04'} if r['verdict']=='PASS' else set())
 for row in data.get('vendor_scores',[]):
  if row['workload'] in accepted:candidate.setdefault(row['workload'],[]).append({'run':n,'composite':row['composite']})
rows=[]
for w in workloads:
 c=[x['composite'] for x in candidate.get(w,[])];b=baseline.get(w,[]);v=vfio.get(w,[])
 rows.append({'workload':w,'candidate_completed_scores':candidate.get(w,[]),'candidate_n':len(c),'candidate_mean':statistics.mean(c) if c else None,'candidate_range':[min(c),max(c)] if c else None,'master_completed_scores':b,'vfio_completed_scores':v,'candidate_over_master':statistics.mean(c)/statistics.mean(b) if c and b else None,'candidate_over_vfio':statistics.mean(c)/statistics.mean(v) if c and v else None})
result={'classification':'MEASURED','evidence':'comparison.json','candidate':'baa2977527c4509145e8033143bd664f768175a6','master':'89fc8a222','master_source':'origin/codex/kayfabe-specviewperf-20261022 traces/specviewperf_kayfabe_20261022/baseline-scores.csv','vfio_source':'benchmark-summary.json via K1 composite extraction','scope':'Partial full-suite run 4400 plus completed fresh Energy trials; survivor scores do not establish stability or full-suite repeatability. References are historical, not interleaved.','energy_trials':trials,'rows':rows}
(R/'comparison.json').write_text(json.dumps(result,indent=2)+'\n')
def scores(xs):return ', '.join(f'{x:.2f}' for x in xs) if xs else 'unavailable'
def ratio(x):return f'{x:.3f}' if x is not None else '—'
lines=['# Partial SPECviewperf comparison','','MEASURED on 2026-10-22: comparison.json combines vendor-completed scores from run 4400 and the passing fresh Energy arms. The full nine-workload/two-iteration attempt failed in Energy. Completed Energy scores describe survivors; they do not remove failures. Reference runs are historical, not interleaved.','','| Workload | 6b completed scores (n) | Master 89fc8a222 | VFIO (n=2) | 6b/master | 6b/VFIO |','|---|---|---|---|---|---|']
for row in rows:
 c=[x['composite'] for x in row['candidate_completed_scores']]
 lines.append(f"| {row['workload']} | {scores(c)} ({len(c)}) | {scores(row['master_completed_scores'])} | {scores(row['vfio_completed_scores'])} | {ratio(row['candidate_over_master'])} | {ratio(row['candidate_over_vfio'])} |")
lines+=['','CODE READ: the K1 baseline has only Blender completed; CATIA failed and later workloads were not run. Missing baseline rows are unavailable, not zero. Ratios use arithmetic means of valid completed composites.','','| Fresh Energy arm | Run | Verdict | Guest 153 / 4101 | New host Xids | Restored |','|---|---|---|---|---|---|']
for row in trials:lines.append(f"| E{row['run']-4400} | {row['run']} | {row['verdict']} | {row['events153']} / {row['events4101']} | {row['xid_delta']} | {row['restored']} |")
(R/'comparison.md').write_text('\n'.join(lines)+'\n')
