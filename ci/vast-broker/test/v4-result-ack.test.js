import test from 'node:test';
import assert from 'node:assert/strict';
import {setup,nonce} from './helpers.js';
// FALSIFIER: the box prints configured grace rather than the shorter hard-TTL
// remainder. The authenticated Worker result acknowledgement supplies it.
for(const elapsed of [0,19*60000])test('V4 result acknowledges actual remaining grace '+elapsed,async()=>{
 const h=await setup();assert.equal((await h.call()).status,200);await h.report({seq:1,kind:'progress',text:'started'});h.time.value+=elapsed;
 const r=await h.report({seq:2,kind:'result',result:{nonce,verdict:'PASS'}});assert.equal(r.status,200);assert.equal((await r.json()).grace_remaining_seconds,elapsed?60:300);
});
