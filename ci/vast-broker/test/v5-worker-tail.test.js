import test from 'node:test';import assert from 'node:assert/strict';import {setup,callbackToken} from './helpers.js';
// FALSIFIER: adding the required Worker-authored status bypasses the shared
// tail line/UTF-8 byte limits that apply to box logs.
for(const kind of ['lines','bytes'])test('V5 Worker-authored tail shares bounds '+kind,async()=>{
 const h=await setup({HARD_RUNTIME_MINUTES:'180'});assert.equal((await h.call()).status,200);
 const text=kind==='lines'?'short\n'.repeat(200):('é'.repeat(495)+' abcdefg\n').repeat(8);
 await h.report({seq:1,kind:'log',text});const r=await h.storage.get('record:123-1');assert.equal((await h.call('/hold/123-1',{token:callbackToken(r)})).status,200);
 for(let i=0;i<3;i++){h.time.value+=60000;await h.service.sweep();}
 const tail=(await h.storage.get('record:123-1')).log;
 assert.ok(tail.trimEnd().split('\n').length<=200);assert.ok(Buffer.byteLength(tail)<=8192);assert.match(tail,/\[Worker\] MANUAL/);
});
