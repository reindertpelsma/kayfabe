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
 const native=async(_url,init)=>{calls++;if(!['follow','manual'].includes(init.redirect))throw TypeError('Unsupported native redirect mode');return new Response('',{status,headers:{Location:'https://untrusted.example'}});};
 await assert.rejects(fetchJson(native,'https://console.vast.ai/api/v0/bundles'));assert.equal(calls,1);
});
