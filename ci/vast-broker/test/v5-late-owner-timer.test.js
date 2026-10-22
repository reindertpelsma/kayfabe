import test from 'node:test';import assert from 'node:assert/strict';import {setup,env,callbackToken} from './helpers.js';
// FALSIFIER: owner timer removal revives an expired hard/absolute manual
// deadline because cleanup cron has not yet consumed the stored timestamp.
for(const restart of [false,true])for(const ceiling of [false,true])test('V5 expired timer cannot be removed '+restart+' '+ceiling,async()=>{
 const h=await setup({HARD_RUNTIME_MINUTES:'180',BOOT_LIMIT_MINUTES:'40',DAILY_SPEND_USD:'10'});await h.call();await h.report({seq:1,kind:'log',text:''});const token=callbackToken(await h.storage.get('record:123-1'));assert.equal((await h.call('/hold/123-1',{token,ghToken:null})).status,200);
 if(ceiling)assert.equal((await h.call('/stop-timer/123-1',{token:env.OWNER_STOP_TOKEN,ghToken:null})).status,200);
 h.time.value+=(ceiling?481:181)*60000;if(restart)h.restart();assert.equal((await h.call('/stop-timer/123-1',{token:env.OWNER_STOP_TOKEN,ghToken:null})).status,401);assert.equal(h.cloud.instances.has('900'),false);
});
