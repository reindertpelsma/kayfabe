import json
from pathlib import Path
R=Path('/workspace/q3-wt');T=R/'traces/energy_rates_20261018';expected=json.loads((R/'traces/energy_bisect_20261018/4267/config.json').read_text())['flags'];rows=json.loads((T/'series-results.json').read_text());proofs=[]
for row in rows:
 n=row['run'];P=T/str(n);cfg=json.loads((P/'config.json').read_text());j=json.loads((P/'result.json').read_text());c=json.loads((P/'channels.json').read_text());r=j['result']
 assert cfg['flags']==expected,(n,'flags')
 assert cfg['resolution']=='FullHD' and cfg['workloads']==['energy-04'] and cfg['iterations']==1
 assert cfg['revision']==row['rev']==r['rev']==c['revision']
 assert j['process_observer_controls_passed'],(n,'observer')
 assert j['backing']=={'classification':'MEASURED','size_unchanged':True,'mtime_unchanged':True},(n,'backing')
 h=r['restoration'];assert r['restored'] and h['cuda']['after']==0 and h['gdm']=='active' and h['nvidia_bound'] and h['smi_exit']==0
 assert c['boot']['gpu_ok']
 assert all(v['CurrentHorizontalResolution']==1920 and v['CurrentVerticalResolution']==1080 for v in c['boot']['video'])
 if r['verdict']=='PASS':
  assert c['guest153']==c['guest4101']==c['new_host_xids']==c['maxima']['poisoned']==c['maxima']['refused_scan_budget']==0
  assert r['sample']['completed'] and r['sample']['boot_unchanged']
  assert j['NoCeObject_refusal_count']==0
 proofs.append({'run':n,'revision':r['rev'],'verdict':r['verdict'],'flags_match':True,'observer_controls_passed':True,'independent_cow_backing_unchanged':True,'guest1920x1080':True,'restored':True})
result={'classification':'MEASURED','evidence':'validation.json','runs_checked':len(rows),'passed':len(proofs),'failed':0,'proofs':proofs}
(T/'validation.json').write_text(json.dumps(result,indent=2)+'\n');print('Result/config/restoration validation:',len(proofs),'passed, 0 failed')
