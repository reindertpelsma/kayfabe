import test from 'node:test';
import assert from 'node:assert/strict';
import {fetchJson} from '../src/policy.js';
// H3 FALSIFIER: the native workerd fetch contract rejects our redirect mode before I/O,
// or an upstream redirect can move an account bearer to a different origin.
for(const name of ['offers','balance','instance'])test('native fetch uses supported manual redirects '+name,async()=>{
 const native=async(_url,init)=>{
  if(!['follow','manual'].includes(init.redirect))throw TypeError('Unsupported native redirect mode');
  assert.equal(init.redirect,'manual');return Response.json({name});
 };
 assert.deepEqual(await fetchJson(native,'https://console.vast.ai/'+name),{name});
});
for(const status of [301,302,307,308])test('manual redirect is rejected without follow '+status,async()=>{
 let calls=0;
 const native=async(_url,init)=>{calls++;if(!['follow','manual'].includes(init.redirect))throw TypeError('Unsupported native redirect mode');return new Response('{"attacker":true}',{status,headers:{Location:'https://untrusted.example'}});};
 await assert.rejects(fetchJson(native,'https://console.vast.ai/api/v0/bundles'));assert.equal(calls,1);
});
// H4 FALSIFIER: provider canonical URLs redirect instead of reaching the API with manual mode.
for(const path of ['/api/v0/users/current','/api/v1/instances?limit=2','/api/v0/instances/900','/api/v0/asks/5','/api/v0/bundles'])test('Vast uses canonical slash endpoint '+path,async()=>{
 const {Vast}=await import('../src/vast.js');let calls=0;
 const v=new Vast(async(url,init)=>{calls++;assert.ok(new URL(url).pathname.endsWith('/'));assert.equal(init.redirect,'manual');return Response.json({success:true});},{},'mock-vast');
 assert.equal((await v.api(path)).success,true);assert.equal(calls,1);
});
