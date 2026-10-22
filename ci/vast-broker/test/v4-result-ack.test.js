import test from 'node:test';
import assert from 'node:assert/strict';
import {setup,nonce} from './helpers.js';
// FALSIFIER: the box prints configured grace rather than the shorter hard-TTL
// remainder. The authenticated Worker result acknowledgement supplies it.
for(const elapsed of [0,18*60000,19*60000])test('V4 result acknowledges actual remaining grace '+elapsed,async()=>{
 const h=await setup();assert.equal((await h.call()).status,200);await h.report({seq:1,kind:'progress',text:'started'});let seq=2;if(elapsed>600000){h.time.value+=600000;assert.equal((await h.report({seq:2,kind:'log',text:''})).status,200);h.time.value+=elapsed-600000;seq=3;}else h.time.value+=elapsed;
 const r=await h.report({seq,kind:'result',result:{nonce,verdict:'PASS'}});assert.equal(r.status,200);assert.equal((await r.json()).grace_remaining_seconds,Math.min(300,(1200000-elapsed)/1000));
});
