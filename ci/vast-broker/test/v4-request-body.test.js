import test from 'node:test';
import assert from 'node:assert/strict';
import {setup,env} from './helpers.js';
// H-V4-stream FALSIFIER: a rejected callback returns while it still owns an
// unread upload; native workerd then reports a read after response completion.
for(const action of ['report','kill'])test('V4 rejected '+action+' relinquishes unread upload before returning',async()=>{
 const h=await setup();let cancelled=false;
 const body=new ReadableStream({start(c){c.enqueue(new TextEncoder().encode('{"text":"untrusted"}'));},cancel(){cancelled=true;}});
 const response=await h.service.fetch(new Request(env.WORKER_URL+'/'+action+'/123-1',{method:'POST',headers:{Authorization:'Bearer invalid'},body,duplex:'half'}));
 assert.equal(response.status,401);assert.equal(cancelled,true);assert.equal(h.cloud.creates,0);
});
