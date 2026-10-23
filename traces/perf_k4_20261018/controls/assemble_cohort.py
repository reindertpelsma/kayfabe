"""Assemble own typed evidence; keep vendor exports private until stamped."""
import collections,hashlib,importlib.util,json,re,statistics as st,subprocess
from pathlib import Path
P=Path('/workspace/q4-k4-private');O=Path('/workspace/q4-wt/traces/perf_k4_20261018')
IDS=(4642,4648,4649,4643);KF=(4642,4649);VF=(4648,4643)
for n in IDS:
 subprocess.run(['python3',str(P/'analyze_case.py'),str(n)],stdout=subprocess.DEVNULL,check=True)
margin=max(json.loads((P/str(n)/'telemetry-summary.json').read_text())['clock_alignment']['minimum_certified_margin'] for n in IDS)
for n in IDS:subprocess.run(['python3',str(P/'analyze_case.py'),str(n),str(margin)],stdout=subprocess.DEVNULL,check=True)
cohort={'classification':'MEASURED','evidence':'cohort.json','candidate':'714ec29d3fdca0a013d9d478755346b0f1af6fa7','clean_runs':list(IDS),'common_boundary_margin_seconds':margin,'replication':'Two independent fresh boots per arm; polls and within-boot proxy phases are correlated. No inferential confidence intervals from n2.','runs':[]}
proxies=[]
for n in IDS:
 S=P/str(n);j=json.loads((S/'telemetry-summary.json').read_text());j.pop('windows');j.pop('DXGI_adapter')
 res=json.loads((S/'result.json').read_text());assert res['verdict']=='PASS' and res['restored'] and res['telemetry_exit']==0
 assert res['proxy']['verdict']=='PASS' and res['proxy']['phases']==12
 final=json.loads((S/'status-final.json').read_text());assert final['currentBoot']==final['initialBoot']
 exits=[json.loads(line) for line in final['process_exit_log'].splitlines() if line.strip()];viewperf=[x for x in exits if x['name'].lower()=='viewperf.exe'];assert len(viewperf)==19 and all(x['exit']==0 for x in viewperf)
 j['viewperf_process_exit_count']=len(viewperf)
 assert res['sample']['counters']['poisoned']==0 and res['sample']['counters']['refused_scan_budget']==0
 assert res['host_stop']['new_host_xids']==0 and res['sample']['guest153']==0 and res['sample']['guest4101']==0 and res['sample']['boot_unchanged'] and res['sample']['budget_refusals']==0 and not res['sample']['viewperf_nonzero_exits']
 j['guards']={k:res['sample'][k] for k in ('guest153','guest4101','boot_unchanged','budget_refusals','viewperf_nonzero_exits','counters')};j['guards'].update(verdict=res['verdict'],restored=True,new_host_xids=0,telemetry_exit=0,controller_exit=res['controller_exit'])
 j['host_load']={k:json.loads((S/f'load-before-{k}.json').read_text()) for k in ('vm','spec')}
 j['identity_controls']=json.loads((S/'identity-controls.json').read_text());assert j['identity_controls']['verdict']=='PASS'
 j['raw_sha256']={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in (S/'guest-nvml.csv',S/'guest-memory.jsonl',S/'export.json',S/'proxy-export.json')}
 if (S/'host-nvml.csv').exists():j['raw_sha256']['host-nvml.csv']=hashlib.sha256((S/'host-nvml.csv').read_bytes()).hexdigest()
 j['raw_evidence_directory']=f'/data/codex-Q4/round-6d/r1/energy-bisect/{n}'
 phase=[]
 for f in json.loads((S/'proxy-export.json').read_text()):
  if not f['name'].endswith('.txt') or f['name'].startswith('control-'):continue
  lines=f['text'].splitlines()
  if f['name']=='mode-control.txt':
   assert n==4643 and any(l=='MODE_CONTROL_RESULT verdict=PASS saved_mode_restored=1' for l in lines)
   data=[l for l in lines if l.startswith(('MODE_CONTROL','PROXY_DISPLAY ','PROXY_RESULT '))];results=[l for l in data if l.startswith('PROXY_RESULT ')];assert len(results)==3
   assert len([l for l in lines if l.startswith('PROXY_PIXEL ')])==36 and all('bad=0' in l for l in lines if l.startswith('PROXY_PIXEL '))
   for line in results:
    values=dict(re.findall(r'(\w+)=(\S+)',line));assert values['real_pixels']=='1' and values['device_reason']=='0x00000000' and values['disjoint_batches']=='0'
   j['fullscreen_coverage_control']={'records':data,'pixel_checks':36,'verdict':'PASS','saved_mode_restored':True}
   continue
  results=[l for l in lines if l.startswith('PROXY_RESULT ')];assert len(results)==1
  fields=dict(re.findall(r'(\w+)=(\S+)',results[0]));assert fields['real_pixels']=='1' and fields['device_reason']=='0x00000000' and fields['disjoint_batches']=='0'
  pixels=[l for l in lines if l.startswith('PROXY_PIXEL ')];assert len(pixels)==12 and all('bad=0' in l for l in pixels)
  row={'run':n,'arm':j['arm'],'file':f['name'],'variant':'windowed' if f['name'].startswith('windowed-') else 'primary','result':fields,'pixel_checks':12}
  for prefix,key in [('PROXY_DISPLAY ','display'),('PROXY_WINDOW ','window'),('PROXY_PRESENT_STATUS ','present_status')]:
   found=[l for l in lines if l.startswith(prefix)];assert len(found)<=1
   if found:row[key]=dict(re.findall(r'(\w+)=(-?\w+)',found[0]))
  if row['variant']=='windowed' and fields['mode']=='present':
   w=row['window'];assert w['client_width']=='1280' and w['client_height']=='720' and w['visible']=='1' and w['iconic']=='0' and w['cloak_hr']=='0' and w['cloaked']=='0'
   assert row['present_status']=={'occluded':'0','other_nonzero_status':'0'},'NOT_A_VISIBLE_PRESENT_CONTROL'
  phase.append(row)
 assert sum(r['variant']=='primary' for r in phase)==12
 assert sum(r['variant']=='windowed' for r in phase)==(6 if n in (4649,4643) else 0)
 proxies+=phase;j['proxy_phase_count']=len(phase);cohort['runs'].append(j)
cohort['aggregate']=[]
for w in ('catia-07','creo-04'):
 for arm in ('kayfabe','VFIO'):
  rows=[next(r for r in j['rows'] if r['workload']==w) for j in cohort['runs'] if j['arm']==arm]
  entry={'workload':w,'arm':arm,'boot_n':2,'physical_metrics':{},'PDH_adapter_peak_bytes':{},'observer_local_budget_mean_bytes':{}}
  for k in rows[0]['physical_metrics']:
   v=[r['physical_metrics'][k]['mean'] for r in rows];assert all(x is not None for x in v)
   entry['physical_metrics'][k]={'boot_means':v,'mean':st.mean(v),'min':min(v),'max':max(v)}
  for k in rows[0]['guest_memory']['PDH_adapter_peak_bytes']:
   v=[r['guest_memory']['PDH_adapter_peak_bytes'][k] for r in rows];entry['PDH_adapter_peak_bytes'][k]={'boot_peaks':v,'min':min(v),'max':max(v)}
  v=[r['guest_memory']['observer_DXGI_budget_bytes']['0']['mean'] for r in rows];entry['observer_local_budget_mean_bytes']={'boot_means':v,'min':min(v),'max':max(v)}
  cohort['aggregate'].append(entry)
(O/'cohort.json').write_text(json.dumps(cohort,indent=2)+'\n')
proxy={'classification':'MEASURED','evidence':'proxy-results.json','candidate':cohort['candidate'],'metric':'Completed render / Present-call loop iterations per second, not displayed new-content fps. Timestamp interval includes CPU/queue gaps, not active SM kernel duration.','primary_visibility_limit':'Primary windows have unequal desktop coverage; primary did not log Present success-status/occlusion.','windowed_limit':'Additional n1 boot/arm, same1280x720 client with1920x1080 render target; no claim to replay CATIA/Creo OpenGL.','phases':proxies,'groups':[]}
for variant in ('primary','windowed'):
 for arm in ('kayfabe','VFIO'):
  for vertices in ('3000','300000') if variant=='primary' else ('3000',):
   for mode in ('offscreen','present'):
    selected=[r for r in proxies if r['variant']==variant and r['arm']==arm and r['result']['vertices']==vertices and r['result']['mode']==mode]
    byboot=collections.defaultdict(list)
    for r in selected:byboot[r['run']].append(float(r['result']['fps']))
    b=[st.mean(v) for v in byboot.values()];assert all(len(v)==3 for v in byboot.values())
    proxy['groups'].append({'variant':variant,'arm':arm,'vertices':int(vertices),'mode':mode,'boot_n':len(b),'phase_n':len(selected),'boot_means':dict(zip(map(str,byboot),b)),'mean':st.mean(b),'boot_mean_range':[min(b),max(b)],'phase_range':[min(float(r['result']['fps']) for r in selected),max(float(r['result']['fps']) for r in selected)]})
(O/'proxy-results.json').write_text(json.dumps(proxy,indent=2)+'\n')
spec=importlib.util.spec_from_file_location('stamp','/workspace/q4-6d-r1-private/spec_stamp.py');stamp=importlib.util.module_from_spec(spec);spec.loader.exec_module(stamp)
comps={}
for n in IDS:
 scores=json.loads((P/str(n)/'export.json').read_text())['scores'];assert len(scores)==2
 comps[n]={s['workload']:float(s['composite']) for s in scores}
fields=['workload']+[f'{n} composite' for n in IDS]+['kayfabe mean composite','kayfabe min composite','kayfabe max composite','VFIO mean composite','VFIO min composite','VFIO max composite',stamp.ratio_header('kayfabe mean','VFIO mean')]
rows=[]
for w in ('catia-07','creo-04'):
 row={'workload':w};row.update({f'{n} composite':comps[n][w] for n in IDS})
 for arm,ns in [('kayfabe',KF),('VFIO',VF)]:
  v=[comps[n][w] for n in ns]
  for label,func in [('mean',st.mean),('min',min),('max',max)]:row[f'{arm} {label} composite']=round(func(v),5)
 row[fields[-1]]=round(st.mean(comps[n][w] for n in KF)/st.mean(comps[n][w] for n in VF),5);rows.append(row)
release='SPECviewperf 15.x (patch level not recorded)';config='Diagnostic runs4642/4649 kayfabe714ec29d3 and4648/4643 VFIO; RTX4070; Win11 driver580.88/Linux595.91.07;12GiB guest RAM,8vCPU;FullHD;9216MiB kayfabe store/test broker60Hz/1920x1080 virtual desktop versus12282MiB VFIO/physical primary3840x2160,virtual8640x2160;200ms NVML/query and1s memory observers, Linux dmon1s;two fresh boots per arm.'
for kind in ('md','csv'):
 text=stamp.render_markdown('MEASURED K4 diagnostic composites: runs4642/4648/4649/4643',rows,fields,fields[1:],release,config) if kind=='md' else stamp.render_csv(rows,fields,fields[1:],release,config)
 stamp.require_valid(text,kind);(O/f'spec-comparison.{kind}').write_text(text)
print('COMMON_MARGIN',margin,'all four guards PASS; stamped composites written')
for r in cohort['aggregate']:print(r['workload'],r['arm'],{k:round(v['mean'],2) for k,v in r['physical_metrics'].items()})
for r in proxy['groups']:print('PROXY',r['variant'],r['arm'],r['vertices'],r['mode'],r['mean'],r['phase_range'])
