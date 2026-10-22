import test from 'node:test';import assert from 'node:assert/strict';import {Ledger} from '../src/ledger.js';import {MemoryStorage,env,clock} from './helpers.js';import {configuration} from '../src/policy.js';
// FALSIFIER: a short-lived box releases unmeasured transfer liability merely
// because its allowance was amortised into an hourly admission price.
for(const ended of [false,true])test('V5 full network allowance remains reserved '+ended,async()=>{
 const storage=new MemoryStorage(),ledger=new Ledger(storage),r={key:'123-1',run:'123',sha:'a'.repeat(40),id:'900',profile:'check',started:clock-300000,ended:null,price:.1256666667,computeStoragePrice:.12,bandwidthCost:.017,ttl:10800000};
 await ledger.reserve(r);const row=await storage.get('cost:'+r.key);assert.equal(row.price,.12);assert.equal(row.bandwidthCost,.017);
 if(ended){await ledger.settle(r,clock);const cfg={...configuration(env),daily:.04};await assert.rejects(ledger.budgets(cfg,clock,.025),e=>e.state==='UNAVAILABLE_BUDGET');}
 else assert.equal((await storage.get('cost:'+r.key)).ended,null);
});
