import test from 'node:test';
import assert from 'node:assert/strict';
import {setup,nonce} from './helpers.js';
// FALSIFIER: a newly created provider row lacking actual_status is destroyed
// before its provisioning bound, or cur_state admits an authenticated result.
for(const shape of ['missing','null']){
 const pending=async()=>{
  const h=await setup();assert.equal((await h.call()).status,200);
  const i=h.cloud.instances.get('900');
  if(shape==='missing')delete i.actual_status;else i.actual_status=null;
  i.cur_state='running';i.intended_status='running';return h;
 };
 test('new provider '+shape+' actual_status survives status and cron only to provision bound',async()=>{
  const h=await pending();
  assert.equal((await (await h.call('/status/123-1')).json()).state,'RUNNING');
  await h.service.sweep();assert.deepEqual(h.cloud.deletes,[]);
  h.time.value+=600000;await h.service.sweep();assert.deepEqual(h.cloud.deletes,['900']);
 });
 test('new provider '+shape+' actual_status refuses report without killing or accepting cur_state',async()=>{
  const h=await pending();
  const response=await h.report({seq:1,kind:'result',result:{nonce,verdict:'PASS'}});
  assert.equal(response.status,409);assert.equal((await response.json()).state,'NOT_READY');
  assert.deepEqual(h.cloud.deletes,[]);assert.equal((await h.storage.get('record:123-1')).resultAt,undefined);
  h.cloud.instances.get('900').actual_status='running';
  assert.equal((await h.report({seq:1,kind:'result',result:{nonce,verdict:'PASS'}})).status,200);
 });
}
