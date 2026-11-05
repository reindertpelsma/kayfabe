import subprocess,json,time,datetime,os,signal,sys
from pathlib import Path
id=int(sys.argv[1]);L=Path('/workspace/bw-run/owned_instances.json');rows=json.loads(L.read_text());match=[r for r in rows if r['id']==id];assert len(match)==1,'not our ID'
r=match[0]
def api(args):
 p=subprocess.run(['vastai',*args,'--raw'],capture_output=True,text=True,check=True);return json.loads(p.stdout)
before=api(['show','instances']);entry=next((x for x in before if int(x['id'])==id),None)
if entry:r['observed_dph_total']=float(entry.get('dph_total',r.get('observed_dph_total',r['quote_dph'])))
subprocess.run(['vastai','destroy','instance',str(id),'-y'],capture_output=True,text=True,check=True)
for i in range(6):
 absent=not any(int(x['id'])==id for x in api(['show','instances']))
 if absent:break
 time.sleep(3)
assert absent,'teardown not yet API-verified; keep timer armed and do not rent'
r['destroyed']=True;r['absence_verified_utc']=datetime.datetime.now(datetime.timezone.utc).isoformat()
credit=float(api(['show','user']).get('credit',0));r['credit_after_observed']=credit;r['observed_account_credit_delta']=max(0,r['credit_before']-credit)
elapsed=(datetime.datetime.now(datetime.timezone.utc)-datetime.datetime.fromisoformat(r['created_utc'])).total_seconds();r['elapsed_hours_until_absence']=elapsed/3600;r['runtime_cost_estimate_usd']=r['observed_dph_total']*elapsed/3600
r['cost_upper_bound_usd']=max(r['runtime_cost_estimate_usd']+1.0,r['observed_account_credit_delta'])
r['cost_note']='Runtime estimate and shared-account credit delta are observations, not settled attribution. Budget guard reserves1USD for delayed transfer billing until invoices reconcile.'
pid=r.get('auto_destroy_pid')
if pid:
 try:
  if os.getpgid(pid)==pid:os.killpg(pid,signal.SIGTERM)
  else:raise RuntimeError('timer PID not its own process group; inspect without killing others')
 except ProcessLookupError:pass
 r['auto_destroy_cancelled_after_absence']=True
L.write_text(json.dumps(rows,indent=2)+'\n')
print(json.dumps({k:r.get(k) for k in ('id','destroyed','absence_verified_utc','observed_dph_total','runtime_cost_estimate_usd','observed_account_credit_delta','cost_upper_bound_usd','auto_destroy_cancelled_after_absence')}))
