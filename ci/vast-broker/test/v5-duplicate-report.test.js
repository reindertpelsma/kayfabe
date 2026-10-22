import test from 'node:test';import assert from 'node:assert/strict';import {setup,nonce,sha,sampleResult} from './helpers.js';
// FALSIFIER: duplicate or escaped-equivalent keys survive authenticated JSON
// parsing and select a result by parser precedence.
for(const key of ['kind','nonce','pass','driver_version','escaped-nonce'])test('V5 duplicate report field refuses '+key,async()=>{
 const h=await setup();await h.call();let raw=JSON.stringify({seq:1,kind:'result',result:{nonce,sha,runner_nonce:'f'.repeat(64),verdict:'PASS',...sampleResult()}});
 if(key==='escaped-nonce')raw=raw.replace('"nonce":','"\\u006eonce":"'+nonce+'","nonce":');
 else{const field=key==='kind'?'"kind":"result"':key==='nonce'?'"nonce":"'+nonce+'"':key==='pass'?'"pass":1':'"driver_version":"580.159.04"';raw=raw.replace(field,field+','+field);}
 assert.equal((await h.report(null,{raw})).status,400);assert.equal((await h.storage.get('record:123-1')).resultAt,undefined);
});
