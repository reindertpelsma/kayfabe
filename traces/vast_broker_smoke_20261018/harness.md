# Temporary native smoke harness

CODE READ: these are the local-only harness sources used by run v3-local-20261022 on 2026-10-22, copied as text with the lane worktree path replaced by <WORKTREE>. They are evidence, not production Worker entrypoints. `.dev.vars` values, private RSA material and callback bearers are omitted. The real probe has read-only Vast calls; the synthetic Worker intercepts all Vast calls and substitutes a fake key.

### identity.mjs

```text
import http from 'node:http';
import {generateKeyPairSync,sign} from 'node:crypto';
import fs from 'node:fs';
const root='<WORKTREE>/ci/vast-broker';
const readEnv=()=>Object.fromEntries(fs.readFileSync(root+'/.dev.vars','utf8').trim().split('\n').map(l=>{const i=l.indexOf('=');return [l.slice(0,i),l.slice(i+1)];}));
const {publicKey,privateKey}=generateKeyPairSync('rsa',{modulusLength:2048});
const jwk={...publicKey.export({format:'jwk'}),kid:'v3-local',alg:'RS256',use:'sig'};
const run=JSON.parse(fs.readFileSync('/tmp/v3-smoke/run.json','utf8'));
const repo='reindertpelsma/kayfabe',owner=229738317,issuer='http://127.0.0.1:18788';
const server=http.createServer((req,res)=>{
 const path=new URL(req.url,issuer).pathname;let value;
 if(path==='/.well-known/jwks')value={keys:[jwk]};
 else if(path==='/token'){
  const enc=x=>Buffer.from(JSON.stringify(x)).toString('base64url');const t=Math.floor(Date.now()/1000);
  const c={iss:issuer,aud:readEnv().WORKER_URL,nbf:t-1,exp:t+300,repository:repo,event_name:'pull_request',actor_id:String(owner),sha:'d'.repeat(40),ref:'refs/pull/7/merge',run_id:run.id,run_attempt:'1'};
  const input=enc({alg:'RS256',kid:'v3-local'})+'.'+enc(c);value={token:input+'.'+sign('RSA-SHA256',Buffer.from(input),privateKey).toString('base64url')};
 }else if(path.endsWith('/actions/runs/'+run.id))value={id:Number(run.id),run_attempt:1,event:'pull_request',head_sha:run.sha,head_branch:'v3-local-smoke',pull_requests:[{number:7,head:{sha:run.sha}}],actor:{id:owner},triggering_actor:{id:owner},status:'in_progress',repository:{full_name:repo}};
 else if(path.endsWith('/pulls/7'))value={number:7,state:'open',draft:false,user:{id:owner},head:{sha:run.sha,ref:'v3-local-smoke',repo:{full_name:repo}},base:{ref:'master',repo:{full_name:repo}},changed_files:1};
 else if(path.endsWith('/pulls/7/reviews'))value=[];
 else if(path.endsWith('/pulls/7/files'))value=[{filename:'ci/vast-broker/src/index.js',status:'modified'}];
 else {res.writeHead(404);res.end('{}');return;}
 res.writeHead(200,{'Content-Type':'application/json','Cache-Control':'no-store'});res.end(JSON.stringify(value));
});
server.listen(18788,'127.0.0.1',()=>console.log('Local identity ready; keys held in memory; tokens never logged.'));
```

### probe.mjs

```text
import {Github} from '<WORKTREE>/ci/vast-broker/src/github.js';
import {OidcVerifier} from '<WORKTREE>/ci/vast-broker/src/oidc.js';
import worker,{BrokerLedger} from '<WORKTREE>/ci/vast-broker/src/index.js';
import {Vast} from '<WORKTREE>/ci/vast-broker/src/vast.js';
import {configuration} from '<WORKTREE>/ci/vast-broker/src/policy.js';
export {BrokerLedger};
export default {
 ...worker,
 async fetch(request,env,ctx){
  const u=new URL(request.url);
  if(u.pathname.startsWith('/__smoke/')){
   if(u.hostname!=='127.0.0.1'||request.headers.get('Authorization')!=='Bearer '+env.OWNER_STOP_TOKEN)return new Response('no',{status:401});
   const calls=[];
   const tracked=async(url,init)=>{const t=performance.now();const r=await fetch(url,init);const detail={path:new URL(url).pathname,status:r.status,redirect:r.headers.get('location')!==null,ms:performance.now()-t};
if(new URL(url).pathname.includes('bundles')){const text=await r.clone().text();detail.bytes=new TextEncoder().encode(text).length;try{const d=JSON.parse(text);detail.keys=Object.keys(d);detail.count=d.offers?.length;detail.samples=d.offers?.slice(0,2).map(o=>Object.fromEntries(['id','gpu_name','verified','verification','vms_enabled','rentable','rented','num_gpus','reliability','disk_space','dph_total'].map(k=>[k,o[k]])));}catch{}}
if(new URL(url).pathname.includes('/instances/999999999999999/')){try{const d=await r.clone().json();detail.keys=Object.keys(d);detail.instance_type=d.instances===null?'null':typeof d.instances;detail.instance_keys=d.instances&&Object.keys(d.instances);}catch{}}
calls.push(detail);return r;};
   try{
    const vast=new Vast(tracked,{...configuration(env),price:Number(u.searchParams.get('price')||env.MAX_PRICE_USD_HOUR)},env.VAST_API_KEY);let result;
    if(u.pathname==='/__smoke/authorize'){const c=await new OidcVerifier(fetch,Date.now,{issuer:env.TEST_ISSUER,jwks:env.TEST_ISSUER+'/.well-known/jwks'}).verify(request.headers.get('X-Smoke-JWT'),env.WORKER_URL);const target=await new Github(fetch,{...configuration(env),githubOrigin:env.TEST_ISSUER}).authorize(c);result={authorized:true,pr:target.pr,sha:target.sha};}
    else if(u.pathname==='/__smoke/config'){result={testEnabled:env.ALLOW_TEST_ISSUER==='true',testIssuer:env.TEST_ISSUER==='http://127.0.0.1:18788'};}
    else if(u.pathname==='/__smoke/offer'){const o=await vast.offer();result={id:o.id,gpu_name:o.gpu_name,dph_total:o.dph_total,verified:o.verified,verification:o.verification,vms_enabled:o.vms_enabled,keys:Object.keys(o)};}
    else if(u.pathname==='/__smoke/balance'){const b=await vast.balance();result={balance_type:typeof b,reserve_met:typeof b==='number'&&b>=8};}
    else if(u.pathname==='/__smoke/list'){const r=await vast.api('/api/v1/instances?limit=2&select_filters='+encodeURIComponent(JSON.stringify({label:{eq:'kf-ci-1792671510122-1'}})));result={keys:Object.keys(r),instances_type:Array.isArray(r.instances)?'array':typeof r.instances,count:r.instances?.length,success:r.success};}
    else if(u.pathname==='/__smoke/absent'){const r=await vast.inspect({id:'999999999999999',label:'kf-ci-absent'});result={absent:r===null};}
    else return new Response('no',{status:404});
    return Response.json({result,calls});
   }catch(e){return Response.json({error:e.state||e.name,message:String(e.message).split(env.VAST_API_KEY).join('[REDACTED]'),calls},{status:503});}
  }
  return worker.fetch(request,env,ctx);
 }
};
```

### mock-native.mjs

```text
import worker,{createService} from '<WORKTREE>/ci/vast-broker/src/index.js';
export class BrokerLedger {
 constructor(ctx,env){
  this.storage=ctx.storage;this.offset=0;this.capability=null;
  const nativeFetch=async(url,init)=>{
   const u=new URL(url);if(u.hostname!=='console.vast.ai')return fetch(url,init);
   const path=u.pathname.replace(/\/$/,'');let out;
   if(path==='/api/v0/bundles')out={offers:[{id:5,verification:'verified',vms_enabled:true,rentable:true,rented:false,num_gpus:1,gpu_name:'RTX 4070',reliability:.999,disk_space:200,dph_total:.1}]};
   else if(path==='/api/v0/users/current')out={balance:20};
   else if(path==='/api/v0/asks/5'){
    const b=JSON.parse(init.body);await ctx.storage.put('mock-instance',{id:900,label:b.label,actual_status:'running'});
    this.capability=b.onstart.match(/^token='([^']+)'/m)[1];out={success:true,new_contract:900};
   }else if(path==='/api/v0/instances/900'){
    if(init.method==='DELETE'){await ctx.storage.delete('mock-instance');out={success:true};}
    else out={instances:await ctx.storage.get('mock-instance')??null};
   }else throw Error('Unexpected synthetic provider path');
   return Response.json(out);
  };
  this.service=createService({env:{...env,RENTING_ENABLED:'true',VAST_API_KEY:'mock-native-no-real-key'},storage:ctx.storage,fetch:nativeFetch,now:()=>Date.now()+this.offset});
 }
 async fetch(request){
  const u=new URL(request.url);
  if(u.origin==='https://internal.invalid'&&u.pathname==='/cron'){await this.service.sweep();return new Response('ok');}
  if(u.hostname==='127.0.0.1'&&u.pathname==='/__smoke/expire'){this.offset=601000;return new Response('expired synthetic clock');}
  if(u.hostname==='127.0.0.1'&&u.pathname==='/__smoke/capability'){
   const run=u.searchParams.get('run'),r=await this.storage.get('record:'+run);return Response.json({record:r,token:this.capability,mock_instance_exists:!!await this.storage.get('mock-instance')});
  }
  return this.service.fetch(request);
 }
}
export default {
 ...worker,
 fetch(request,env,ctx){if(new URL(request.url).pathname.startsWith('/__smoke/'))return env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account')).fetch(request);return worker.fetch(request,env,ctx);}
};
```

### cpu_routes.py

```text
from pathlib import Path
import subprocess,re,json,urllib.request,urllib.error,time
p=Path('<WORKTREE>/ci/vast-broker/.dev.vars');env=dict(line.split('=',1) for line in p.read_text().splitlines());run=json.loads(Path('/tmp/v3-smoke/run.json').read_text())['id']+'-1'
def req(path,method='GET',auth=None,port=18787):
 headers={} if auth is None else {'Authorization':'Bearer '+auth}
 if path=='/__smoke/authorize':headers['X-Smoke-JWT']=signed
 try:r=urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:'+str(port)+path,method=method,headers=headers),timeout=20)
 except urllib.error.HTTPError as e:r=e
 data=r.read();return r.status,len(data),data.decode()
def pids(port):
 out=subprocess.check_output(['ss','-ltnp','sport = :'+str(port)],text=True);pid=re.search(r'pid=(\d+)',out).group(1);parent=Path('/proc/'+pid+'/stat').read_text().split(')')[1].split()[1]
 return [p for p in Path('/proc').iterdir() if p.name.isdigit() and (p/'comm').exists() and (p/'comm').read_text().strip()=='workerd' and (p/'stat').read_text().split(')')[1].split()[1]==parent]
def cpu(group):
 total=0
 for p in group:
  for t in (p/'task').iterdir():
   try:total+=int((t/'schedstat').read_text().split()[0])
   except FileNotFoundError:pass
 return total
signed=json.load(urllib.request.urlopen('http://127.0.0.1:18788/token'))['token']
cases=[('unknown','GET','/unknown',None,18787),('rent-disabled','POST','/rent',None,18787),('status-missing','GET','/status/'+run,None,18787),('status-bad','GET','/status/'+run,'bad',18787),('status-local-jwt','GET','/status/'+run,signed,18787),('stop-bad','POST','/stop/'+run,'bad',18787),('report-bad','POST','/report/'+run,'bad',18787),('stop-all-bad','POST','/stop-all','bad',18787),('stop-all-owner','POST','/stop-all',env['OWNER_STOP_TOKEN'],18787),('scheduled','GET','/cdn-cgi/local/scheduled',None,18787),('local-authorization','GET','/__smoke/authorize',env['OWNER_STOP_TOKEN'],18789),('real-offer','GET','/__smoke/offer?price=2',env['OWNER_STOP_TOKEN'],18789),('real-balance','GET','/__smoke/balance',env['OWNER_STOP_TOKEN'],18789),('real-list','GET','/__smoke/list',env['OWNER_STOP_TOKEN'],18789),('real-absent','GET','/__smoke/absent',env['OWNER_STOP_TOKEN'],18789)]
rows=[]
for name,method,path,auth,port in cases:
 req(path,method,auth,port) # warm isolate and auth/JWKS cache
 group=pids(port);samples=[]
 for _ in range(5):
  before=cpu(group);start=time.perf_counter();status,size,body=req(path,method,auth,port);wall=(time.perf_counter()-start)*1000;used=(cpu(group)-before)/1e6
  samples.append({'cpu_ms':round(used,3),'wall_ms':round(wall,3),'status':status,'bytes':size})
 rows.append({'route':name,'n':5,'cpu_scope':'sum /proc task schedstat over own session workerd processes; includes local proxy/observability/native runtime, not Cloudflare billed CPU','cpu_min_ms':min(s['cpu_ms'] for s in samples),'cpu_max_ms':max(s['cpu_ms'] for s in samples),'samples':samples,'body':body})
Path('/tmp/v3-smoke/routes-final.json').write_text(json.dumps(rows,indent=2))
for r in rows:print(r['route'],r['samples'][0]['status'],r['samples'][0]['bytes'],'bytes; OS workerd CPU',r['cpu_min_ms'],'..',r['cpu_max_ms'],'ms')
```

### mock-lifecycle.py

```text
from pathlib import Path
import urllib.request,urllib.error,json,sys
run=json.loads(Path('/tmp/v3-smoke/run.json').read_text())['id']+'-1';jwt=json.load(urllib.request.urlopen('http://127.0.0.1:18788/token'))['token'];base='http://127.0.0.1:18790';rows=[]
def call(path,method='GET',token=jwt,body=None):
 headers={'Authorization':'Bearer '+token}
 data=json.dumps(body).encode() if body is not None else None
 if body is not None:headers['Content-Type']='application/json'
 try:r=urllib.request.urlopen(urllib.request.Request(base+path,method=method,headers=headers,data=data),timeout=20)
 except urllib.error.HTTPError as e:r=e
 data=r.read().decode();rows.append({'path':path,'status':r.status,'bytes':len(data),'body':data});return r.status,json.loads(data) if data.startswith('{') else data
if sys.argv[1]=='start':
 status,data=call('/rent','POST');assert status==200,(status,data)
 cap=json.load(urllib.request.urlopen(base+'/__smoke/capability?run='+run));assert cap['token'];Path('/tmp/v3-smoke/mock-cap.json').write_text(json.dumps(cap));Path('/tmp/v3-smoke/mock-cap.json').chmod(0o600)
 for payload in [{'seq':1,'kind':'progress','text':'Native SQLite smoke progress'},{'seq':2,'kind':'log','text':'::error::hostile synthetic log'}]:
  status,_=call('/report/'+run,'POST',cap['token'],payload);assert status==200
 status,data=call('/status/'+run);assert status==200 and ': :error: :' in data['log_tail_sanitised']
else:
 cap=json.loads(Path('/tmp/v3-smoke/mock-cap.json').read_text())
 persisted=json.load(urllib.request.urlopen(base+'/__smoke/capability?run='+run));assert 'Native SQLite' in persisted['record']['log'];rows.append({'sqlite_restart_verified':True,'id':persisted['record']['id']})
 call('/__smoke/expire');status,_=call('/cdn-cgi/local/scheduled');assert status==200
 persisted=json.load(urllib.request.urlopen(base+'/__smoke/capability?run='+run));r=persisted['record'];assert r['closed']=='TTL' and r['ended'] is not None and r['verdict']=='FAIL' and not persisted['mock_instance_exists'];rows.append({'native_ttl_verified':True,'closed':r['closed'],'verdict':r['verdict'],'mock_provider_absent':not persisted['mock_instance_exists'],'note':'Fake clock exceeds OIDC fixture expiry; inspect persisted ledger for terminal state.'})
 status,_=call('/report/'+run,'POST',cap['token'],{'seq':3,'kind':'progress','text':'stale'});assert status==401
 env=dict(line.split('=',1) for line in Path('<WORKTREE>/ci/vast-broker/.dev.vars').read_text().splitlines());status,data=call('/stop/'+run,'POST',env['OWNER_STOP_TOKEN']);assert status==200
 prev=json.loads(Path('/tmp/v3-smoke/mock-lifecycle.json').read_text());rows=prev+rows
Path('/tmp/v3-smoke/mock-lifecycle.json').write_text(json.dumps(rows,indent=2));print(json.dumps(rows,indent=2))
```
