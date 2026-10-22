import test from 'node:test';import assert from 'node:assert/strict';import {mkdtemp,writeFile,readFile,rm} from 'node:fs/promises';import {join} from 'node:path';import {tmpdir} from 'node:os';import {spawnSync} from 'node:child_process';
// FALSIFIER: raw download uses the PR SHA, Python is not isolated, or any inherited
// credential/control variable reaches the controller (even with bash -x).
for(const path of ['', 'unit-path-only-'+'p'.repeat(32)])test('R3 actual bootstrap isolates BASE controller '+path.length,async()=>{
 const d=await mkdtemp(join(tmpdir(),'v3-r3-bootstrap-'));try{
  const t=await readFile(new URL('../../../.github/workflows/gpu-vast.yml',import.meta.url),'utf8');const script=t.split('  gpu:\n')[1].split('  report:\n')[0].split('        run: |\n')[1].split('\n').map(x=>x.startsWith('          ')?x.slice(10):x).join('\n');
  await writeFile(join(d,'curl'),`#!/usr/bin/env python3
import os,sys
args=sys.argv[1:];url=args[-1];file=url.rsplit('/',1)[1]
assert url=='https://raw.githubusercontent.com/reindertpelsma/kayfabe/'+'d'*40+'/ci/vast-broker/'+file
assert file in {'runner.py','deployment.json'} and '-L' not in args and '--location' not in args
with open(args[args.index('-o')+1],'w') as f:f.write(os.environ['STUB_CONTROLLER'] if file=='runner.py' else '{}')
`,{mode:0o700});
  const stub=`import os,sys\nassert sys.flags.isolated==1\nassert not any(x in os.environ for x in ['GH_TOKEN','GITHUB_TOKEN','BASH_ENV','PYTHONPATH','LD_PRELOAD','INHERITED_SECRET'])\nassert os.environ['PATH']=='/usr/bin:/bin'\nassert os.environ['GITHUB_SHA']=='d'*40\nassert len(os.environ)<=20\nwith open(os.environ['GITHUB_OUTPUT'],'w') as f:f.write('state=COMPLETE\\nverdict=PASS\\nverified=1\\n')\n`;
  const origin='https://quiet-tenant.workers.example';const r=spawnSync('bash',['--noprofile','--norc','-x','-eo','pipefail','-c',script],{encoding:'utf8',timeout:5000,env:{...process.env,PATH:d+':'+process.env.PATH,GITHUB_SHA:'d'.repeat(40),GITHUB_REF:'refs/heads/master',GITHUB_REPOSITORY:'reindertpelsma/kayfabe',GITHUB_EVENT_PATH:join(d,'event'),GITHUB_EVENT_NAME:'pull_request_target',GITHUB_RUN_ID:'123',GITHUB_RUN_ATTEMPT:'1',GITHUB_OUTPUT:join(d,'out'),GITHUB_STEP_SUMMARY:join(d,'summary'),WORKER_URL:origin,WORKER_PATH:path,ACTIONS_ID_TOKEN_REQUEST_URL:'https://oidc.example/?a=1',ACTIONS_ID_TOKEN_REQUEST_TOKEN:'fake-request-token',GH_TOKEN:'fake-github-token',GITHUB_TOKEN:'fake-github-token',INHERITED_SECRET:'must-not-cross',STUB_CONTROLLER:stub}});
  assert.equal(r.status,0,r.stderr);assert.doesNotMatch(r.stdout+r.stderr,/quiet-tenant|unit-path-only|fake-github-token/);assert.equal(await readFile(join(d,'out'),'utf8'),'state=COMPLETE\nverdict=PASS\nverified=1\n');
 }finally{await rm(d,{recursive:true});}
});
