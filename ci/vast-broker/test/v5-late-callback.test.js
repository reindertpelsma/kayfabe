import test from 'node:test';import assert from 'node:assert/strict';import {setup,nonce,callbackToken} from './helpers.js';
// FALSIFIER: a callback/hold wins a race with overdue cron and extends an
// already-expired boot, heartbeat or result-grace deadline.
for(const restart of [false,true])for(const deadline of ['boot','heartbeat','grace'])test('V5 late callback cannot revive '+deadline+' '+restart,async()=>{
 const h=await setup({HARD_RUNTIME_MINUTES:'180',BOOT_LIMIT_MINUTES:'40'});await h.call();
 if(deadline==='heartbeat')await h.report({seq:1,kind:'log',text:''});
 if(deadline==='grace')await h.report({seq:1,kind:'result',result:{nonce,verdict:'PASS'}});
 h.time.value+=(deadline==='boot'?40:deadline==='heartbeat'?15:5)*60000;if(restart)h.restart();
 const response=deadline==='grace'?await h.call('/hold/123-1',{token:callbackToken(await h.storage.get('record:123-1')),ghToken:null}):await h.report({seq:deadline==='heartbeat'?2:1,kind:'log',text:''});
 assert.equal(response.status,401);assert.equal(h.cloud.instances.has('900'),false);assert.notEqual((await h.storage.get('record:123-1')).ended,null);
});
