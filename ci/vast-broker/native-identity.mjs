// Local-only fake GitHub/JWKS; RSA private material stays in memory.
import http from 'node:http';
import {generateKeyPairSync,sign} from 'node:crypto';
import fs from 'node:fs';
const {publicKey,privateKey}=generateKeyPairSync('rsa',{modulusLength:2048});
const jwk={...publicKey.export({format:'jwk'}),kid:'v4-native',alg:'RS256',use:'sig'};
const variables=()=>Object.fromEntries(fs.readFileSync(new URL('./.dev.vars',import.meta.url),'utf8').trim().split('\n').map(l=>{const n=l.indexOf('=');return [l.slice(0,n),l.slice(n+1)];}));
const repo=variables().REPOSITORY??'reindertpelsma/kayfabe',owner=Number(variables().OWNER_GITHUB_ID),issuer='http://127.0.0.1:18788';
let selected={run:'1001',sha:'a'.repeat(40)};
http.createServer(async(req,res)=>{
 const path=new URL(req.url,issuer).pathname;let value;
 if(path==='/select'&&req.method==='POST'){const chunks=[];for await(const c of req)chunks.push(c);selected=JSON.parse(Buffer.concat(chunks));value={ok:true};}
 else if(path==='/.well-known/jwks')value={keys:[jwk]};
 else if(path==='/token'){
  const env=variables();
  const enc=x=>Buffer.from(JSON.stringify(x)).toString('base64url'),t=Math.floor(Date.now()/1000);
  const c={iss:issuer,aud:env.WORKER_URL,nbf:t-1,exp:t+1200,repository:repo,event_name:'pull_request',actor_id:String(owner),sha:'d'.repeat(40),ref:'refs/pull/7/merge',run_id:selected.run,run_attempt:'1'};
  const input=enc({alg:'RS256',kid:'v4-native'})+'.'+enc(c);value={token:input+'.'+sign('RSA-SHA256',Buffer.from(input),privateKey).toString('base64url')};
 }else if(path.endsWith('/actions/runs/'+selected.run))value={id:Number(selected.run),run_attempt:1,event:'pull_request',head_sha:selected.sha,head_branch:'v4-native',pull_requests:[{number:7,head:{sha:selected.sha}}],actor:{id:owner},triggering_actor:{id:owner},status:'in_progress',repository:{full_name:repo}};
 else if(path.endsWith('/pulls/7'))value={number:7,state:'open',draft:false,user:{id:owner},head:{sha:selected.sha,ref:'v4-native',repo:{full_name:repo}},base:{ref:'master',repo:{full_name:repo}},changed_files:1};
 else if(path.endsWith('/pulls/7/reviews'))value=[];
 else if(path.endsWith('/pulls/7/files'))value=[{filename:'ci/vast-broker/src/index.js',status:'modified'}];
 else {res.writeHead(404);res.end('{}');return;}
 res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify(value));
}).listen(18788,'127.0.0.1',()=>console.log('Native identity ready; key held in memory.'));
