import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
// FALSIFIER: a hostile Worker crosses the runner's display/schema/binding
// boundary, or any structural violation grants a successful required check.
for(const name of ['valid','controls','commands','caps','invalid-utf8','extra','duplicate','wrong-sha','wrong-nonce','bad-state','bad-verdict','wrong-hash','redirect','timeout','html','huge','wrong-run','wrong-version','display-only'])test('R3 hostile Worker: '+name,()=>{
 const p=spawnSync('python3',['-I',new URL('round3_runner_fixture.py',import.meta.url).pathname,name],{encoding:'utf8',timeout:15000,maxBuffer:65536});
 assert.equal(p.status,0,p.stderr+p.stdout);
});
