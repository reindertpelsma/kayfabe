import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {readFile} from 'node:fs/promises';
for(const name of ['valid','wrong-sha','wrong-status','wrong-creator','wrong-url','wrong-workflow','wrong-event','wrong-run-sha','wrong-conclusion','wrong-repo'])test('R3 release same-SHA provenance gate: '+name,()=>{
 const p=spawnSync('python3',['-I',new URL('round3_release_fixture.py',import.meta.url).pathname,name],{encoding:'utf8',timeout:5000});assert.equal(p.status,0,p.stderr+p.stdout);
});
test('R3 final binaries built on GitHub without artifact inputs',async()=>{
 const t=await readFile(new URL('../../../.github/workflows/release-build.yml',import.meta.url),'utf8');
 assert.match(t,/workflow_dispatch:/);assert.match(t,/refs\/heads\/master/);assert.match(t,/python3 -I ci\/vast-broker\/release_gate.py/);assert.match(t,/cargo build --locked --release --workspace/);assert.match(t,/actions\/attest-build-provenance@[0-9a-f]{40}/);assert.match(t,/persist-credentials: false/);assert.doesNotMatch(t,/upload-artifact|download-artifact|cache@|workflow_run:|release create|secrets\./);
 for(const ref of t.matchAll(/^\s*- uses: (\S+)/gm))assert.match(ref[1],/^actions\/[a-z-]+@[0-9a-f]{40}$/);
});
test('R3 bundle manifest pins exact emitted Worker bytes',async()=>{
 const p=spawnSync(process.execPath,['bundle.js'],{cwd:new URL('../',import.meta.url),encoding:'utf8'});assert.equal(p.status,0,p.stderr);
 const {createHash}=await import('node:crypto');const raw=await readFile(new URL('../dist/worker.js',import.meta.url));const manifest=JSON.parse(await readFile(new URL('../dist/deployment.json',import.meta.url),'utf8'));assert.deepEqual(manifest,{protocol_version:3,worker_version:'v4-r3',bundle_sha256:createHash('sha256').update(raw).digest('hex')});
});

test('R3 generated manifest can be independently checked with sha256sum',async()=>{
 const p=spawnSync(process.execPath,['bundle.js'],{cwd:new URL('../',import.meta.url),encoding:'utf8'});assert.equal(p.status,0,p.stderr);
 const expected=spawnSync('sha256sum',['dist/worker.js'],{cwd:new URL('../',import.meta.url),encoding:'utf8'});assert.equal(expected.status,0);
 const manifest=JSON.parse(await readFile(new URL('../dist/deployment.json',import.meta.url),'utf8'));assert.equal(manifest.bundle_sha256,expected.stdout.split(' ')[0]);
});
