# Live local smoke harness

CODE READ: temporary harness source from run v3-live-credit-20261022 on 2026-10-22; source path replaced by <WORKTREE>. No account credentials, endpoint binding values or capability values are included. The native entry inherits the source Durable Object and injects only a diagnostic fetch wrapper; all admission, ledger, crypto, provider and report logic comes from the repo. The loopback-only ledger diagnostic was added after provider absence to read settlement metadata. The onstart workload was unchanged.

### ingress.mjs

```text
import http from 'node:http';
import fs from 'node:fs';
const dir='/tmp/v3-live',root='<WORKTREE>/ci/vast-broker';
const env=Object.fromEntries(fs.readFileSync(root+'/.dev.vars','utf8').trim().split('\n').map(l=>{const n=l.indexOf('=');return [l.slice(0,n),l.slice(n+1)];}));
const run=JSON.parse(fs.readFileSync(dir+'/run.json','utf8'));
http.createServer((req,res)=>{
 let allowed=null;try{allowed=JSON.parse(fs.readFileSync(dir+'/allowed-ip.json','utf8')).ip;}catch{}
 const source=req.headers['cf-connecting-ip'];
 const path='/'+env.BROKER_PATH_PREFIX+'/report/'+run.id+'-1';
 if(!allowed||source!==allowed||req.method!=='POST'||req.url!==path){console.log(JSON.stringify({event:'ingress_rejected',ip_allowed:!!allowed,source_matches:source===allowed,route_matches:req.url===path}));res.writeHead(403);res.end('Forbidden');return;}
 const headers={...req.headers,host:new URL(env.WORKER_URL).host};delete headers['cf-connecting-ip'];
 const target=http.request({hostname:'127.0.0.1',port:18787,path:req.url,method:'POST',headers},response=>{res.writeHead(response.statusCode,response.headers);response.pipe(res);console.log(JSON.stringify({event:'callback_forwarded',status:response.statusCode}));});
 target.on('error',()=>{res.writeHead(502);res.end('Unavailable');});req.pipe(target);
}).listen(18791,'127.0.0.1',()=>console.log('Callback ingress ready: report route only, exact Vast source-IP allowlist, deny until IP configured.'));
```

### live-entry.mjs

```text
import worker,{BrokerLedger as SourceBrokerLedger,createService} from '<WORKTREE>/ci/vast-broker/src/index.js';
export class BrokerLedger extends SourceBrokerLedger {
 constructor(ctx,env){
  super(ctx,env);this.storage=ctx.storage;this.owner=env.OWNER_STOP_TOKEN;
  const redact=value=>{let s=String(value??'');for(const k of ['VAST_API_KEY','RUN_TOKEN_SECRET','OWNER_STOP_TOKEN','WORKER_URL','BROKER_PATH_PREFIX'])if(env[k])s=s.split(env[k]).join('[REDACTED]');return s.replace(/v1\.[0-9]+\.[0-9]+\.[0-9]+\.[A-Za-z0-9_-]{43}/g,'[run-capability]').slice(0,1000);};
  const emit=async d=>{const r=await fetch('http://127.0.0.1:18792/event',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(d)});if(!r.ok)throw Error('Diagnostic sink unavailable');};
  const tracked=async(url,init)=>{
   const u=new URL(url);if(u.hostname!=='console.vast.ai')return fetch(url,init);
   const creating=init.method==='PUT'&&u.pathname.includes('/asks/');let body;
   if(creating){body=JSON.parse(init.body);const digest=await crypto.subtle.digest('SHA-256',new TextEncoder().encode(body.onstart));await emit({event:'intent',label:body.label,scriptHash:Array.from(new Uint8Array(digest),b=>b.toString(16).padStart(2,'0')).join(''),fields:Object.keys(body),vm:body.vm,runtype:body.runtype,disk:body.disk,path:u.pathname});}
   const t=performance.now(),r=await fetch(url,init);const detail={event:'provider_response',method:init.method,path:u.pathname,status:r.status,wall_ms:performance.now()-t};
   try{const d=await r.clone().json();detail.keys=Object.keys(d);if(!r.ok){detail.error=redact(d.error);detail.msg=redact(d.msg??d.message);}if(creating){detail.success=d.success;if(d.success===true&&Number.isSafeInteger(d.new_contract))await emit({event:'created',id:d.new_contract,label:body.label});}}catch{}
   await emit(detail);return r;
  };
  this.service=createService({env,storage:ctx.storage,fetch:tracked});
 }
 async fetch(request){
  const u=new URL(request.url);
  if(u.hostname==='127.0.0.1'&&u.pathname==='/__v3/ledger'&&request.headers.get('Authorization')==='Bearer '+this.owner){const r=await this.storage.get('record:'+u.searchParams.get('run'));return Response.json(r?Object.fromEntries(['key','id','price','started','ended','ttl','closed','verdict','reportCount','reportBytes'].map(k=>[k,r[k]])):{});}
  return super.fetch(request);
 }
}
export default {...worker,fetch(request,env,ctx){if(new URL(request.url).pathname==='/__v3/ledger')return env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account')).fetch(request);return worker.fetch(request,env,ctx);}};
```

### control.mjs

```text
import http from 'node:http';
import fs from 'node:fs';
const dir='/tmp/v3-live';
http.createServer(async(req,res)=>{
 if(req.method!=='POST'||req.url!=='/event'){res.writeHead(404);res.end();return;}
 const chunks=[];let bytes=0;for await(const b of req){bytes+=b.length;if(bytes>16384){res.writeHead(413);res.end();return;}chunks.push(b);}
 try{
  const d=JSON.parse(Buffer.concat(chunks));const run=JSON.parse(fs.readFileSync(dir+'/run.json'));
  fs.appendFileSync(dir+'/provider-events.jsonl',JSON.stringify(d)+'\n',{mode:0o600});
  if(d.event==='intent'&&d.label==='kf-ci-'+run.id+'-1')fs.writeFileSync(dir+'/intent.json',JSON.stringify(d),{mode:0o600});
  if(d.event==='created'&&d.label==='kf-ci-'+run.id+'-1'&&Number.isSafeInteger(d.id))fs.writeFileSync(dir+'/owned-id.json',JSON.stringify({id:d.id,label:d.label}),{mode:0o600});
  res.writeHead(200);res.end('ok');
 }catch{res.writeHead(400);res.end('invalid');}
}).listen(18792,'127.0.0.1',()=>console.log('Local provider diagnostics ready; allowlisted metadata only.'));
```

### reaper.py

```text
from pathlib import Path
import urllib.request,urllib.parse,urllib.error,json,time,os
from vastai._base import _resolve_api_key,_APIKEY_SENTINEL
root=Path('/tmp/v3-live');key=_resolve_api_key(_APIKEY_SENTINEL);run=json.loads((root/'run.json').read_text());label='kf-ci-'+run['id']+'-1'
def api(path,method='GET'):
 r=urllib.request.urlopen(urllib.request.Request('https://console.vast.ai'+path,method=method,headers={'Authorization':'Bearer '+key}),timeout=20)
 return json.load(r)
def owned():
 p=root/'owned-id.json'
 if p.exists():
  item=json.loads(p.read_text());d=api('/api/v0/instances/'+str(item['id'])+'/');i=d.get('instances')
  if i is None:return []
  if str(i.get('id'))!=str(item['id']) or i.get('label')!=label:raise RuntimeError('Ownership mismatch')
  return [item['id']]
 q=urllib.parse.urlencode({'limit':2,'select_filters':json.dumps({'label':{'eq':label}})})
 d=api('/api/v1/instances/?'+q)
 if d.get('success') is not True or d.get('next_token'):raise RuntimeError('Incomplete exact-run list')
 matches=[i for i in d.get('instances',[]) if i.get('label')==label]
 if len(matches)>1:raise RuntimeError('Multiple exact-run instances')
 return [i['id'] for i in matches]
deadline=time.time()+900
(root/'reaper-armed.json').write_text(json.dumps({'pid':os.getpid(),'deadline_epoch':deadline,'run':run['id'],'minutes':15}))
print('Independent own-run reaper armed for 15 minutes.',flush=True)
time.sleep(max(0,deadline-time.time()))
for attempt in range(8):
 try:
  ids=owned()
  if not ids:print('Exact own run absent; reaper complete.',flush=True);break
  for i in ids:
   result=api('/api/v0/instances/'+str(i)+'/','DELETE');print('Own instance destroy acknowledgement:',i,result.get('success'),flush=True)
 except Exception as e:print('Reaper API unavailable:',type(e).__name__,flush=True)
 time.sleep(5)
else:print('Reaper exhausted retries; manual cleanup required.',flush=True)
```
