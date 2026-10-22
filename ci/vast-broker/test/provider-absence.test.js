import test from 'node:test';
import assert from 'node:assert/strict';
import {Vast} from '../src/vast.js';
// H5 FALSIFIER: real Vast's HTTP 200 {instances:null} cannot establish absence,
// or accepting that exact absent shape weakens ownership/malformed-response checks.
const record={id:'900',label:'kf-ci-123-1'};
test('provider HTTP 200 null instance is absent',async()=>{
 const v=new Vast(async()=>Response.json({instances:null}),{},'mock-vast');assert.equal(await v.inspect(record),null);
});
test('destroy already absent provider instance is idempotent',async()=>{
 const methods=[];const v=new Vast(async(_url,init)=>{methods.push(init.method);return Response.json({instances:null});},{},'mock-vast');
 await v.destroy(record);assert.deepEqual(methods,['GET']);
});
for(const instances of [undefined,{},[],{id:901,label:record.label},{id:900,label:'foreign'}])test('absence does not accept malformed or foreign '+JSON.stringify(instances),async()=>{
 const v=new Vast(async()=>Response.json({instances}),{},'mock-vast');await assert.rejects(v.inspect(record));
});
