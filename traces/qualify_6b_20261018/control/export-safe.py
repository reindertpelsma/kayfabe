import datetime as dt,hashlib,json,re,sys
from pathlib import Path
R=Path('/data/codex-Q4');N=int(sys.argv[1]);S=R/'energy-bisect'/str(N);A=R/'windows-20261022'/f'boundary-kayfabe-{N}';P=R/'public'/str(N);P.mkdir(parents=True,exist_ok=True)
full=S/'result.json';result=json.loads(full.read_text()) if full.exists() else None
counters=['pointer_large_entries','pointer_entry_max_bytes','refused_scan_budget','poisoned'];maxima={k:None for k in counters};sums={k:0 for k in counters};budget=[];strands=[];refused=0;retired=0
q=A/'qemu.log';h=hashlib.sha256()
with q.open('rb') as f:
 for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
with q.open(errors='replace') as f:
 for number,line in enumerate(f,1):
  if 'USER_RING_LIFECYCLE ' in line:retired+=1
  for key in counters:
   values=re.findall(key+r'[=:] ?(\d+)',line)
   if values:
    maxima[key]=max(maxima[key] or 0,max(map(int,values)))
    if 'USER_RING_LIFECYCLE ' in line:sums[key]+=int(values[-1])
  if 'USER_RING_REFUSED' in line:refused+=1
  if 'USER_RING' in line and ('Budget' in line or '_BUDGET' in line):
   if len(budget)<30:budget.append({'line':number,'diagnostic':line.split('; first words:',1)[0].strip()[:1000]})
  if 'PROGRESS_STRAND for_ms=' in line or 'PROGRESS_STRAND_END' in line:
   if len(strands)<100:strands.append({'line':number,'diagnostic':line.strip()[:1000]})
stats=[];errors=[]
for log in A.glob('broker-*.log'):
 for number,line in enumerate(log.read_text(errors='replace').splitlines(),1):
  if 'stats t=' in line:stats.append({k:int(v) for k,v in re.findall(r'(\w+)=(\d+)',line)})
  elif re.search(r'error|failed|refus|protocol violation',line,re.I):
   if len(errors)<30:errors.append({'line':number,'diagnostic':line[:1000]})
seconds=(stats[-1]['t']-stats[0]['t'])/1000+1 if len(stats)>1 else None
broker={'stats_windows':len(stats),'duration_seconds_approx':seconds,'feedbacks':sum(x['feedbacks'] for x in stats),'new_content_presents':sum(x['new'] for x in stats),'repeat_presents':sum(x['repeat'] for x in stats),'discarded':sum(x['discarded'] for x in stats),'errors':errors,'new_content_fps_approx':sum(x['new'] for x in stats)/seconds if seconds else None,'repeat_per_second_approx':sum(x['repeat'] for x in stats)/seconds if seconds else None,'max_new_content_gap_us':max((x['cgap_max_us'] for x in stats),default=None),'gaps_per_minute_approx':{key:sum(x[key] for x in stats)*60/seconds if seconds else None for key in ['cg33','cg50','cg100']},'one_percent_low':None,'point_one_percent_low':None,'quantile_limitation':'Stock master broker stats provide threshold counts and window maxima, not a raw new-content gap series. No percentile values inferred.','scope':'whole VM session; no-present intervals are not rendering performance scores'}
config=json.loads((A/'command.json').read_text());after=A/'backing-after.json'
metadata={'classification':'MEASURED','evidence':f'{N}/result.json','run':N,'candidate':'baa2977527c4509145e8033143bd664f768175a6','captured':dt.datetime.now(dt.timezone.utc).isoformat(),'complete':result is not None,'result':result,'qemu_bytes':q.stat().st_size,'qemu_sha256':h.hexdigest(),'retirement_lines':retired,'counter_maxima':maxima,'observed_retirement_counter_sums':sums,'budget_diagnostics_first30':budget,'progress_strands_first100':strands,'user_ring_refusal_lines':refused,'broker':broker,'backing':json.loads(after.read_text()) if after.exists() else None,'flags':config['flags'],'process_controls':json.loads((S/'observer-controls-proof.json').read_text()) if (S/'observer-controls-proof.json').exists() else None}
if (S/'export.json').exists():
 e=json.loads((S/'export.json').read_text(encoding='utf-8-sig'));metadata['vendor_scores']=e.get('scores',[]);metadata['export_errors']=e.get('errors',[])
(P/'result.json').write_text(json.dumps(metadata,indent=2)+'\n');print(json.dumps({'run':N,'complete':metadata['complete'],'counter_maxima':maxima,'broker_feedbacks':broker['feedbacks'],'budget':len(budget),'strands':len(strands)}))
