import test from 'node:test';
import assert from 'node:assert/strict';
import {setup,nonce,callbackToken} from './helpers.js';
// FALSIFIER: CI polling after the box's early kill misses a valid structured
// result because the terminal state is changed to STOPPED.
for(const verdict of ['PASS','FAIL'])test('V4 polling after box kill preserves COMPLETE '+verdict,async()=>{
 const h=await setup();await h.call();await h.report({seq:1,kind:'result',result:{nonce,verdict}});
 const r=await h.storage.get('record:123-1');assert.equal((await h.call('/kill/123-1',{token:callbackToken(r)})).status,200);
 const status=await(await h.call('/status/123-1')).json();assert.equal(status.state,'COMPLETE');assert.equal(status.verdict,verdict);assert.equal(status.cleanup_pending,false);
});
