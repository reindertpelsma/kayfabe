from pathlib import Path
import csv,json,statistics
R=Path('/workspace/q3-wt/traces/energy_rates_20261018')
series=json.loads((R/'series-results.json').read_text());rows=[]
for run in series:
 n=run['run'];p=R/str(n);j=json.loads((p/'result.json').read_text());c=json.loads((p/'channels.json').read_text());r=j['result'];scores=[s['composite'] for s in j.get('vendor_scores',[]) if s['workload']=='energy-04'];deaths=[q for q in c['channels'] if q['gp'] is not None]
 rows.append({'order':run['order'],'run':n,'arm':run['arm'],'build':run['arm'][0],'revision':run['rev'],'started':run['started'],'ended':run['ended'],'verdict':r['verdict'],'score':scores[0] if scores else None,'first153_guest':c['first153_guest'],'first153_detected_host':c['first153_detected_host'],'guest153':c['guest153'],'guest4101':c['guest4101'],'host_xids':c['new_host_xids'],'poisoned':c['maxima']['poisoned'],'restored':c['restored'],'failing_channels':[{'host':d['host'],'token':d['token'],'forwarded':d['forwarded'],'gp':d['gp']} for d in deaths],'matching_forwarded':[q['forwarded'] for q in c['channels'] if q['host']=='0x1002d'],'rpc_limit':c['rpc_limit']})
prior={'D':{'pass':0,'fail':0,'runs':[]},'A':{'pass':0,'fail':2,'runs':[4263,4266]},'B':{'pass':1,'fail':0,'runs':[4264]},'C':{'pass':1,'fail':1,'runs':[4262,4267]}}
counts={}
for arm in 'DABC':
 records=[r for r in rows if r['build']==arm];p=sum(r['verdict']=='PASS' for r in records);f=sum(r['verdict']=='FAIL' for r in records);old=prior[arm];scores=[r['score'] for r in records if r['score'] is not None]
 counts[arm]={'q3_pass':p,'q3_fail':f,'q3_n':p+f,'q3_failure_rate':f/(p+f) if p+f else None,'prior_energy_only_pass':old['pass'],'prior_energy_only_fail':old['fail'],'prior_energy_only_runs':old['runs'],'pooled_pass':p+old['pass'],'pooled_fail':f+old['fail'],'pooled_n':p+f+old['pass']+old['fail'],'scores':scores,'score_min':min(scores) if scores else None,'score_max':max(scores) if scores else None,'score_mean':statistics.mean(scores) if scores else None}
summary={'classification':'MEASURED','evidence':'table.json','scope':'Q3 fresh standalone Energy trials plus explicitly separated prior standalone observations; K1 mixed3454 and candidate mixed4260 excluded from rate denominators','completed':len(rows),'counts':counts,'run_order':rows}
(R/'table.json').write_text(json.dumps(summary,indent=2)+'\n')
with (R/'run-order.csv').open('w',newline='') as out:
 keys=[k for k in rows[0] if k not in ['failing_channels','matching_forwarded','rpc_limit']]
 writer=csv.DictWriter(out,keys,lineterminator="\n");writer.writeheader();writer.writerows({k:r[k] for k in keys} for r in rows)
lines=['# Q3 Energy rate table','',f'MEASURED at rev 468a35c9 (run4301) and the per-run revisions in table.json: runs4301–{rows[-1]["run"]}, exact revisions and first-event/channel facts in table.json and per-run channels.json; {len(rows)}/11 trials complete.','', '| Build | Q3 pass/fail | Prior standalone pass/fail | Combined fail/n | Q3 exported guest scores |','|---|---:|---:|---:|---|']
for arm in 'DABC':
 q=counts[arm];lines.append(f'| {arm} | {q["q3_pass"]}/{q["q3_fail"]} | {q["prior_energy_only_pass"]}/{q["prior_energy_only_fail"]} | {q["pooled_fail"]}/{q["pooled_n"]} | '+', '.join(map(str,q['scores']))+' |')
lines+=['','CODE READ: D = descriptor-only468a35c9; A = descriptor+SA-02 40583bc2; B = descriptor+batched-walk a83394e6; C = candidate afe00b3c. Prior standalone runs are A4263/4266, B4264, C4262/4267. K1 mixed3454 and candidate mixed4260 are reference observations only.','', '| Order/run | Build | Host start CEST | Result | First153 guest | Host detection CEST | Failing forwarded/GP |','|---|---|---|---|---|---|---|']
for r in rows:
 ff='; '.join(str(c['forwarded'])+'/'+str(c['gp']) for c in r['failing_channels']) or 'none'
 lines.append(f'| {r["order"]}/run{r["run"]} | {r["arm"]} | {r["started"][11:19]} | {r["verdict"]} | {r["first153_guest"] or "none"} | {r["first153_detected_host"] or "none"} | {ff} |')
lines+=['','MEASURED at rev 468a35c9, run4301 (4301/channels.json): run-order.csv and per-run channels.json retain the guest clock and host clock separately; run4301 guest captured09:18 while host captured11:18 CEST, approximately two hours apart. Times are not silently converted.','', 'CODE READ: production logs expose RmAlloc seq=0 context, CHAN-FACTS and token/host birth linkage; they do not identify a unique complete RPC transaction replay. Full logs and CoWs stay under /data/codex-Q3/windows-20261022/ and /data/codex-Q3/energy-bisect/.']
(R/'rates.md').write_text('\n'.join(lines)+'\n')
print('completed',len(rows),'counts',json.dumps({k:{'q3':(v['q3_pass'],v['q3_fail']),'pooled_fail_n':(v['pooled_fail'],v['pooled_n'])} for k,v in counts.items()}))
