import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {env,clock,nonce,jwt,MemoryStorage,MockCloud,finalLine} from './helpers.js';
test('local HTTP integration: owner rent to async logs to automatic destroy',async()=>{
 const {createService}=await import('../src/index.js');const cloud=new MockCloud();
 const server=createServer(async(req,res)=>{try{
   const chunks=[];for await(const c of req)chunks.push(c);
   const r=await cloud.fetch(req.headers['x-mock-target'],{method:req.method,headers:req.headers,body:chunks.length?Buffer.concat(chunks).toString():undefined});
   res.writeHead(r.status,Object.fromEntries(r.headers));res.end(Buffer.from(await r.arrayBuffer()));
 }catch{res.writeHead(500);res.end('mock error');}});
 await new Promise(resolve=>server.listen(0,'localhost',resolve));
 try{
   const localFetch=(url,init={})=>fetch('http://localhost:'+server.address().port,{...init,headers:{...init.headers,'x-mock-target':String(url)}});
   const service=createService({env,storage:new MemoryStorage(),fetch:localFetch,now:()=>clock,random:()=>nonce});
   const authorization='Bearer '+await jwt();
   const r=await service.fetch(new Request(env.WORKER_URL+'/rent',{method:'POST',headers:{authorization}}));assert.equal(r.status,200);
   cloud.logReady=false;let response=await service.fetch(new Request(env.WORKER_URL+'/status/900',{headers:{authorization}}));assert.equal((await response.json()).verdict,'UNAVAILABLE');
   cloud.logReady=true;cloud.logText='::error::fake\n'+finalLine();response=await service.fetch(new Request(env.WORKER_URL+'/status/900',{headers:{authorization}}));
   const result=await response.json();assert.equal(result.verdict,'PASS');assert.match(result.log,/\[box\] ::error/);assert.deepEqual(cloud.deletes,['900']);
 }finally{server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
});
test('scheduled binding reaches Durable Object cleanup',async()=>{
 const source=await import('../src/index.js');let swept=0;
 const object=new source.BrokerLedger({storage:new MemoryStorage()},env);object.service={sweep:async()=>{swept++;},fetch:async()=>new Response('not cleanup',{status:404})};
 let promise;source.default.scheduled({}, {LEDGER:{idFromName:n=>n,get:()=>object}}, {waitUntil:p=>promise=p});await promise;assert.equal(swept,1);
});
test('public request cannot invoke the internal cron route',async()=>{
 const source=await import('../src/index.js');let forwarded=0;
 const response=await source.default.fetch(new Request('https://broker.example/cron',{method:'POST'}),{LEDGER:{idFromName:n=>n,get:()=>({fetch:()=>{forwarded++;return new Response('bad');}})}});
 assert.equal(response.status,404);assert.equal(forwarded,0);
});
