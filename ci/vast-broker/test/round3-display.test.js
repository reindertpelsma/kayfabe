import test from 'node:test';import assert from 'node:assert/strict';import {spawnSync} from 'node:child_process';
for(const kind of ['origin','path'])test('R3 summary locally redacts reflected endpoint '+kind,()=>{
 const p=spawnSync('python3',['-I',new URL('round3_main_fixture.py',import.meta.url).pathname,JSON.stringify({leak:true})],{encoding:'utf8',timeout:5000});assert.equal(p.status,0,p.stderr);const r=JSON.parse(p.stdout);assert.equal(r.status,0);assert.doesNotMatch(r.summary,kind==='origin'?/quiet-tenant|workers.example/:/unit-path-only/);assert.doesNotMatch(r.summary,/mock.jwt.value/);
});
for(const control of ['osc','dcs'])test('R3 drops complete C1 terminal sequence '+control,()=>{
 const p=spawnSync('python3',['-I','-c',`import importlib.util,sys; s=importlib.util.spec_from_file_location('r','runner.py');m=importlib.util.module_from_spec(s);sys.modules['r']=m;s.loader.exec_module(m);safe,_=m.scrub('before'+${control==='osc'?'chr(157)+"hidden"+chr(7)':'chr(144)+"hidden"+chr(156)'}+'after');assert 'hidden' not in safe;assert 'beforeafter' in safe`],{cwd:new URL('../',import.meta.url),encoding:'utf8',timeout:5000});assert.equal(p.status,0,p.stderr);
});
