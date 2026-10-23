"""K4 typed telemetry summary, excludes proprietary result scores and content."""
import csv,datetime as dt,json,math,re,statistics as st,sys
from collections import Counter,defaultdict
from pathlib import Path
import attribution as A
import clock_envelope as C
P=Path('/workspace/q4-k4-private');N=int(sys.argv[1]);S=P/str(N);native=N in (4641,4643,4646,4647,4648)
status=json.loads((S/'status-final.json').read_text());ws=[]
pat=re.compile(r'^\[(\d{4}-\d\d-\d\d)\|(\d\d:\d\d:\d\d)\] \[workload: ([^ /]+) / subtest: ([^]]+)\] (Started|Completed)')
for log in status['run_logs']:
 for line in log['text'].splitlines():
  m=pat.match(line)
  if not m:continue
  date,clock,w,sub,event=m.groups();t=A.log_epoch(date,clock,status['captured'])
  if event=='Started':ws.append({'workload':w,'subtest':sub,'start':t})
  else:
   found=[r for r in ws if r['workload']==w and r['subtest']==sub and 'end' not in r]
   assert len(found)==1,'AMBIGUOUS_SUBTEST';found[0]['end']=t
assert len(ws)==19 and all(x.get('end',0)>x['start'] for x in ws),'INCOMPLETE_SUBTEST_WINDOWS'

def stats(v):
 if not v:return {'n':0,'mean':None,'min':None,'max':None,'median':None}
 assert all(math.isfinite(x) for x in v),'NONFINITE_TELEMETRY'
 return {'n':len(v),'mean':st.mean(v),'min':min(v),'max':max(v),'median':st.median(v)}

def text(p):
 raw=p.read_bytes();return raw.decode('utf-16' if raw.startswith((b'\xff\xfe',b'\xfe\xff')) else 'utf-8-sig')

# Host/guest clocks are distinct. Native physical readings and memory share the guest
# epoch. Only kayfabe's physical-host readings require an offset, from explicit brackets.
anchors=[]
if (S/'guest-response-brackets.jsonl').exists():
 for line in (S/'guest-response-brackets.jsonl').read_text().splitlines():
  b=json.loads(line);f=S/b['file']
  if not b['file'].startswith('status-') or not f.exists():continue
  j=json.loads(text(f));g=dt.datetime.fromisoformat(j['captured']).timestamp()*1000
  lo,hi=A.clock_bounds(b['host_before_ns'],b['host_after_ns'],g)
  anchors.append({'host_mid_s':(b['host_before_ns']+b['host_after_ns'])/2e9,'offset_low_s':lo,'offset_high_s':hi})
if (S/'clock-anchor.json').exists():
 j=json.loads((S/'clock-anchor.json').read_text());lo,hi=A.clock_bounds(j['host_before_epoch_ns'],j['host_after_epoch_ns'],j['guest']['epoch_ms']);anchors.append({'host_mid_s':(j['host_before_epoch_ns']+j['host_after_epoch_ns'])/2e9,'offset_low_s':lo,'offset_high_s':hi})
assert native or anchors,'NO_HOST_GUEST_CLOCK_ANCHOR'
original_offset=0 if native else st.median((a['offset_low_s']+a['offset_high_s'])/2 for a in anchors)
original_uncertainty=0 if native else max(max(abs(original_offset-a['offset_low_s']),abs(original_offset-a['offset_high_s'])) for a in anchors)
bounds=(0.0,0.0) if native else C.envelope([(a['offset_low_s'],a['offset_high_s']) for a in anchors])
offset=(bounds[0]+bounds[1])/2
uncertainty=(bounds[1]-bounds[0])/2
minimum_margin=max(3.0,uncertainty+1.0)
margin=float(sys.argv[2]) if len(sys.argv)>2 else minimum_margin
assert margin>=minimum_margin,'COMMON_MARGIN_DOES_NOT_COVER_FULL_CLOCK_ENVELOPE'
physical=list(csv.DictReader(text(S/('guest-nvml.csv' if native else 'host-nvml.csv')).splitlines()))
assert physical,'NO_PHYSICAL_GPU_SAMPLES'
mem=[json.loads(x) for x in text(S/'guest-memory.jsonl').splitlines() if x.strip()]
ad=[x for x in mem if x.get('kind')=='adapter'];assert len(ad)==1,'MISSING_NVIDIA_DXGI_ADAPTER'
luid=f"luid_0x{ad[0]['luid_high']&0xffffffff:08x}_0x{ad[0]['luid_low']:08x}"
nv=[x for x in mem if x.get('kind')=='nvapi_memory' and x.get('status')==0]
fields=['util_gpu','util_mem','clock_sm_mhz','clock_mem_mhz','pstate','power_mw','power_limit_mw','temp_c','pcie_gen','pcie_width','pcie_rx_kb_s','pcie_tx_kb_s','fb_used_bytes','bar1_used_bytes','enc_util','dec_util','pcie_replays']
rows=[]
for workload in ('catia-07','creo-04'):
 windows=[x for x in ws if x['workload']==workload]
 selected=[r for r in physical if A.inside(float(r['epoch_ms'])/1000-offset,windows,margin)]
 assert all(any(C.certified_inside(float(r['epoch_ms'])/1000,w['start'],w['end'],bounds,1) for w in windows) for r in selected),'UNCERTIFIED_PHYSICAL_SAMPLE'
 duration=sum(max(0,w['end']-w['start']-2*margin) for w in windows)
 density=len(selected)/duration;assert 3.5<=density<=5.3,(N,workload,'SAMPLING_DENSITY',density)
 metrics={k:stats([v for r in selected if (v:=A.valid_value(r,k)) is not None]) for k in fields}
 errors={k:dict(Counter(str(r.get(k+'_rc','MISSING')) for r in selected)) for k in fields}
 masks=[int(r['throttle_active']) for r in selected if r.get('throttle_active_rc')=='0']
 links=Counter((r['pcie_gen'],r['pcie_width']) for r in selected if r.get('pcie_gen_rc')=='0' and r.get('pcie_width_rc')=='0')
 mrows=[x for x in mem if x.get('epoch_ms') is not None and A.inside(x['epoch_ms']/1000,windows,margin)]
 nvrows=[x for x in mrows if x.get('kind')=='nvapi_memory' and x.get('status')==0]
 budgets={str(g):stats([x['budget'] for x in mrows if x.get('kind')=='dxgi_budget' and x.get('group')==g and x.get('hr')==0]) for g in (0,1)}
 pdh=[x for x in mrows if x.get('kind')=='pdh' and x.get('status') in (0,1) and luid in x.get('instance','').lower()]
 badpdh=[x for x in mrows if x.get('kind')=='pdh' and x.get('status') not in (0,1)]
 peaks={k:max([x['value'] for x in pdh if x['counter']==k],default=None) for k in ('adapter_dedicated','adapter_shared','adapter_committed')}
 engines=defaultdict(list)
 for x in pdh:
  if x['counter']!='engine_util':continue
  match=re.search(r'_engtype_(.+)$',x['instance']);assert match
  engines[match[1]].append(x['value'])
 rows.append({'workload':workload,'completed_subtests':len(windows),'interior_seconds':duration,'physical_samples':len(selected),'samples_per_second':density,'physical_metrics':metrics,'physical_status_codes':errors,'throttle_masks':dict(Counter(map(str,masks))),'link_gen_width_counts':{g+'x'+w:c for (g,w),c in links.items()},'query_duration_ms':stats([int(r['query_end_epoch_ms'])-int(r['epoch_ms']) for r in selected]),'guest_memory':{'observer_DXGI_budget_bytes':budgets,'NVAPI_supported_samples':len(nvrows),'NVAPI_available_bytes':stats([x['current_available'] for x in nvrows]),'NVAPI_evicted_count_range':stats([x['evicted_count'] for x in nvrows]),'NVAPI_promoted_count_range':stats([x['promoted_count'] for x in nvrows]),'PDH_adapter_peak_bytes':peaks,'PDH_invalid_rows':len(badpdh),'observed_nonzero_engine_rows':{k:stats(v) for k,v in engines.items()},'limits':'DXGI observer budget is not the viewperf process usage. PDH collector omits zero rows; absent engine types/instances are not asserted idle. Per-engine rows can overlap across processes and are not summed into physical SM occupancy.'}})
result={'classification':'MEASURED','evidence':f'telemetry-{N}.json','run':N,'candidate':'714ec29d3fdca0a013d9d478755346b0f1af6fa7','arm':'VFIO' if native else 'kayfabe','physical_location':'guest' if native else 'host','auxiliary':N in (4644,4646),'scope':'Diagnostic profiling; driver rolling windows are correlated, boot is replication unit; no vendor score fields','windows':ws,'clock_alignment':{'guest_capture':status['captured'],'host_minus_guest_seconds_used':offset,'offset_uncertainty_s':uncertainty,'observed_envelope':bounds,'original_fixed3_model':{'status':'FAIL_RETAINED' if original_uncertainty>=3 else 'PASS','point_offset':original_offset,'worst_distance_from_point':original_uncertainty},'anchors':len(anchors),'margin_per_subtest_seconds':margin,'minimum_certified_margin':minimum_margin,'method':'Vendor local timestamps use the captured explicit guest timezone. Every physical sample must remain inside the subtest for ALL offsets in the complete observed request/response bracket union plus1s vendor-boundary exclusion. Offset center is only a coordinate origin, not a constant-clock claim. Final cohort comparisons require the same maximum margin across all four clean boots; broader uncertainty removes more data. VFIO physical and guest memory share guest epoch. Between observed brackets, stock guest clock continuity remains a measurement assumption; harness makes no clock changes.'},'whole_session_NVAPI':{'supported_samples':len(nv),'evicted_count_delta':A.counter_delta([x['evicted_count'] for x in nv]) if nv else None,'evicted_bytes_delta':A.counter_delta([x['evicted_bytes'] for x in nv]) if nv else None,'promoted_count_delta':A.counter_delta([x['promoted_count'] for x in nv]) if nv else None,'promoted_bytes_delta':A.counter_delta([x['promoted_bytes'] for x in nv]) if nv else None,'meaning':'Adapter-reported counters, whole observer session including boot/setup/proxy. No assertion that all OS paging mechanisms are covered.'},'DXGI_adapter':ad[0],'rows':rows}
(S/'telemetry-summary.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'run':N,'auxiliary':N in (4644,4646),'rows':[{'workload':r['workload'],'metrics':{k:round(v['mean'],3) if v['mean'] is not None else None for k,v in r['physical_metrics'].items()},'PDH_peak_bytes':r['guest_memory']['PDH_adapter_peak_bytes'],'observed_engine_types':list(r['guest_memory']['observed_nonzero_engine_rows'])} for r in rows],'NVAPI_session':result['whole_session_NVAPI']},indent=2))
