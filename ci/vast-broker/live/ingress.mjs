// Only cloudflared can reach this loopback listener. Cloudflare overwrites the
// connecting-IP header; deny callbacks until the exact rented box IP is known.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
const root=process.argv[2];
const env=Object.fromEntries(fs.readFileSync(path.join(root,'.dev.vars'),'utf8').trim().split('\n').map(s=>{const n=s.indexOf('=');return [s.slice(0,n),s.slice(n+1)];}));
const read=name=>JSON.parse(fs.readFileSync(path.join(root,name),'utf8'));
const write=(name,value)=>fs.writeFileSync(path.join(root,name),JSON.stringify(value),{mode:0o600});
const event=value=>fs.appendFileSync(path.join(root,'events.jsonl'),JSON.stringify({at:Date.now(),...value})+'\n',{mode:0o600});
http.createServer(async(req,res)=>{
 if(req.url!=='/event'||req.method!=='POST'||req.headers.authorization!=='Bearer '+env.OWNER_STOP_TOKEN){res.writeHead(403);res.end();return;}
 try{const chunks=[];for await(const c of req)chunks.push(c);const d=JSON.parse(Buffer.concat(chunks));
  if(d.kind==='intent'){if(d.label!==read('run.json').label)throw Error();write('intent.json',d);}
  if(d.kind==='created'){if(d.label!==read('run.json').label||!Number.isSafeInteger(d.id))throw Error();write('owned.json',d);}
  event(d);res.end('ok');
 }catch{res.writeHead(400);res.end();}
}).listen(18792,'127.0.0.1');
http.createServer(async(req,res)=>{
 let owned,allowed;try{owned=read('owned.json');allowed=read('allowed.json').ip;}catch{}
 const run=read('run.json'),routes=['report','kill'].map(r=>'/'+env.BROKER_PATH_PREFIX+'/'+r+'/'+run.key);
 if(!allowed||req.headers['cf-connecting-ip']!==allowed||req.method!=='POST'||!routes.includes(req.url)){event({kind:'ingress_rejected',source_matches:req.headers['cf-connecting-ip']===allowed});res.writeHead(403);res.end();return;}
 const kind=req.url.includes('/kill/')?'kill':'report';
 // Snapshot the provider independently at callback arrival, before forwarding.
 try{const response=await fetch('https://console.vast.ai/api/v0/instances/'+owned.id+'/',{headers:{Authorization:'Bearer '+env.VAST_API_KEY}}),i=(await response.json()).instances;
  event({kind:'callback_arrival',route:kind,actual_status:i?.actual_status??null,cur_state:i?.cur_state??null,intended_status:i?.intended_status??null});
 }catch{event({kind:'callback_snapshot_unavailable',route:kind});}
 const headers={...req.headers,host:new URL(env.WORKER_URL).host};delete headers['cf-connecting-ip'];
 const target=http.request({hostname:'127.0.0.1',port:18787,path:req.url,method:'POST',headers},response=>{
  event({kind:'callback_response',route:kind,status:response.statusCode});res.writeHead(response.statusCode,response.headers);response.pipe(res);
 });target.on('error',()=>{res.writeHead(502);res.end();});req.pipe(target);
}).listen(18791,'127.0.0.1',()=>console.log('Exact-source callback ingress ready; deny until configured.'));
