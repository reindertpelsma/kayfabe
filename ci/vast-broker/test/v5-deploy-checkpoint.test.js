import test from 'node:test';import assert from 'node:assert/strict';import {spawnSync} from 'node:child_process';import {onstart} from '../src/onstart.js';import {setup,env,sha,nonce} from './helpers.js';
// FALSIFIER: changing the approved repo-side suite requires changing Worker code,
// or boot fetches unrelated history rather than only the approved object.
for(const name of ['self_test','gpu_probe'])test('checkpoint accepts repo-defined successful suite '+name,async()=>{
 const h=await setup();await h.call();const suites=[{name,pass:1,fail:0,skip:0}];const r=await h.report({seq:1,kind:'result',result:{nonce,verdict:'PASS',suites,hardware:{gpu_name:'RTX_4070',compute_cap:890,driver_version:'580.159.04'}}});assert.equal(r.status,200);assert.deepEqual((await(await h.call('/status/123-1')).json()).suites,suites);
});
for(const name of ['self_test','gpu_probe'])test('checkpoint runner accepts repo-defined suite '+name,()=>{
 const probe="import importlib.util,sys;spec=importlib.util.spec_from_file_location('runner',sys.argv[1]);m=importlib.util.module_from_spec(spec);sys.modules['runner']=m;spec.loader.exec_module(m);m.validate_suites([{'name':sys.argv[2],'pass':1,'fail':0,'skip':0}],'PASS')";
 const r=spawnSync('python3',['-I','-c',probe,new URL('../runner.py',import.meta.url).pathname,name],{encoding:'utf8'});assert.equal(r.status,0,r.stderr);
});
for(const path of ['', 'x'.repeat(32)])test('checkpoint fetches only approved SHA '+path.length,()=>{
 const s=onstart(sha,nonce,180,env.REPOSITORY,{url:env.WORKER_URL,run:'123-1',token:'v1.123.1.1800030000.'+'A'.repeat(43),path});
 assert.ok(s.includes('git init /workspace/kf-ci-src'));assert.ok(s.includes('git fetch --depth=1 origin '+sha));assert.ok(!s.includes('git clone'));assert.ok(s.includes('git checkout --detach '+sha));
});
