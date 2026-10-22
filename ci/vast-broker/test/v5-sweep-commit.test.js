import test from 'node:test';import assert from 'node:assert/strict';import {setup,sha,env} from './helpers.js';
// FALSIFIER: a master-based owner dispatch cannot approve an exact alternative
// repo SHA for sweep, or a different identity can use the same route.
for(const target of ['c'.repeat(40),'e'.repeat(40)])test('V5 owner sweep pins chosen commit '+target[0],async()=>{
 const h=await setup(),base=h.cloud.fetch;h.cloud.run.event='workflow_dispatch';h.cloud.run.head_branch='master';h.cloud.prs=[];h.cloud.run.pull_requests=[];h.cloud.fetch=async(u,i)=>new URL(String(u)).pathname.endsWith('/commits/'+target)?Response.json({sha:target}):base(u,i);h.restart();
 const r=await h.call('/rent',{claims:{event_name:'workflow_dispatch',sha,ref:'refs/heads/master',workflow_ref:env.REPOSITORY+'/.github/workflows/gpu-vast-sweep.yml@refs/heads/master'},body:{runner_nonce:'f'.repeat(64),tested_sha:target,pr_number:null,profile:'sweep',preferences:['AD104']}});assert.equal(r.status,200);assert.equal((await h.storage.get('record:123-1')).sha,target);assert.match(h.cloud.instances.get('900').onstart,new RegExp(target));
});
