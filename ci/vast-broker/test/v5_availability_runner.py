import importlib.util,sys,json,io
from pathlib import Path
spec=importlib.util.spec_from_file_location('runner',Path(__file__).resolve().parents[1]/'runner.py');m=importlib.util.module_from_spec(spec);sys.modules['runner']=m;spec.loader.exec_module(m)
scope={};exec(Path(__file__).with_name('v5_fixture_data.py').read_text(),scope)
case=sys.argv[1];ctx={'run':'123-1','sha':'a'*40,'nonce':'f'*64,'pr':7,'profile':'check','preferences':['AD104']};manifest={'protocol_version':4,'worker_version':'v5','bundle_sha256':'b'*64}
a=scope['availability']();a['offers']=[{'die':'GA106','gpu_name':'RTX_3060','compute_cap':860,'count':2,'effective_hourly':.5,'estimated_total':1.51,'kvm':True}];a['verified_counts']=[{'die':'GA106','total':3,'verified':2}]
if case=='id-field':a['offers'][0]['offer_id']=5
if case=='command-name':a['offers'][0]['gpu_name']='::error::'
if case=='negative-price':a['offers'][0]['effective_hourly']=-1
if case=='string-price':a['offers'][0]['estimated_total']='1.5'
if case=='bool-count':a['offers'][0]['count']=True
if case=='bad-compute':a['offers'][0]['compute_cap']=7.5
if case=='extra-threshold':a['thresholds']['command']='::error::'
if case=='verified-off':a['thresholds']['verified']=0
if case=='unknown-reason':a['rejected']['SHELL_COMMAND']=1
if case=='oversize':a['offers']*=101
if case=='cache-age':a['cache']['age_ms']=60001
status={'state':'GRACE','verdict':'PASS','run':ctx['run'],'id':'900','sha':ctx['sha'],'pr':7,'cleanup_pending':True,'result_at':2000,'ended_at':None,'started_at':1000,'deadline_at':302000,'gpu_model':'RTX_4070','cost_estimate_usd':.01,'log_cursor':0,'log_tail_sanitised':'','bytes_dropped':0,'tail_bytes_dropped':0,'runner_nonce':ctx['nonce'],'log_sha256':'0'*64,**manifest,**scope['extra_status']()}
calls=[];printed=[]
def client(route,method='GET',body=None):
 calls.append((route,body))
 data=a if route.startswith('/availability') else status if route=='/rent' else {}
 return m.Reply(200,{'content-type':'application/json'},json.dumps(data).encode())
r=m.Runner(ctx,manifest,client,emit=printed.append,summary=io.StringIO()).execute()
if case=='good':
 assert r['verdict']=='PASS',r
 request=next(body for route,body in calls if route=='/rent');assert request['preferences']==['AD104'];assert 'Availability: check' in ''.join(printed)
else:assert r['verified']==0 and not any(route=='/rent' for route,_ in calls),(r,calls)
