import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {setup,sha,nonce,clock,callbackToken,env} from './helpers.js';
const root=new URL('../../../',import.meta.url);
const onstartArgs={url:'https://broker.example',run:'123-1',token:'v1.123.1.1800009000.'+'A'.repeat(43)};
for(const field of ['url','run','token'])test('onstart rejects callback interpolation '+field,async()=>{const {onstart}=await import('../src/onstart.js');assert.throws(()=>onstart(sha,nonce,150,env.REPOSITORY,{...onstartArgs,[field]:"';$(id)\n"}));});
test('fixed onstart posts bounded JSON to Worker only and backs up output',async()=>{const {onstart}=await import('../src/onstart.js');const s=onstart(sha,nonce,150,env.REPOSITORY,onstartArgs);assert.match(s,/https:\/\/broker.example/);assert.match(s,/\/report\//);assert.match(s,/--retry/);assert.match(s,/kf-ci-output.log/);assert.match(s,/result/);assert.doesNotMatch(s,/KF_RESULT|RUN_TOKEN_SECRET|OWNER_STOP_TOKEN|console\.vast\.ai|ssh /);assert.equal(spawnSync('bash',['-n'],{input:s}).status,0);});
for(const name of ['RUN_TOKEN_SECRET','OWNER_STOP_TOKEN'])test('new secret is documented and absent from public vars '+name,async()=>{const config=await readFile(new URL('ci/vast-broker/wrangler.toml',root),'utf8');const readme=await readFile(new URL('ci/vast-broker/README.md',root),'utf8');assert.match(readme,new RegExp(name));assert.doesNotMatch(config,new RegExp('^'+name+'\\s*=', 'm'));});
for(const job of ['pending','gpu','report'])test('GitHub job token is bound within execution step '+job,async()=>{const text=await readFile(new URL('.github/workflows/gpu-vast.yml',root),'utf8');const block=text.split('  '+job+':\n')[1].split(/\n  [\w-]+:\n/)[0];assert.match(block,/        env:\n          GH_TOKEN: \$\{\{ github.token \}\}/);});
for(const model of ['RTX 4070','RTX 4090'])test('outbound API filter uses documented GPU name '+model,async()=>{const h=await setup();await h.call();const body=h.cloud.calls.find(x=>x.url.endsWith('/bundles')).body;assert.ok(body.gpu_name.in.includes(model));});
for(const name of ['status','stop'])test('workflow uses run-scoped '+name+' endpoint',async()=>{const text=await readFile(new URL('.github/workflows/gpu-vast.yml',root),'utf8');assert.match(text,new RegExp('/'+name+'/\\$run'));assert.doesNotMatch(text,/\/status\/\$id/);});
test('workflow prints only sanitised tail inside stop-commands',async()=>{const text=await readFile(new URL('.github/workflows/gpu-vast.yml',root),'utf8');assert.match(text,/\.log_tail_sanitised/);assert.doesNotMatch(text,/\.log \/\//);assert.match(text,/::stop-commands::/);});
for(const exitCode of [0,7])test('actual fixed Python sender backs up hostile output and reports exit status '+exitCode,async()=>{
 const {onstart}=await import('../src/onstart.js');const script=onstart(sha,nonce,150,env.REPOSITORY,onstartArgs);
 const python=script.split("python3 - <<'KF_REPORT'\n")[1].split('\nKF_REPORT')[0];
 const directory=await mkdtemp(join(tmpdir(),'v1-callback-sender-'));
 try{
  const curl=join(directory,'curl');await writeFile(curl,`#!/usr/bin/env python3
import json,os,sys
with open(os.environ['CALLBACK_CAPTURE'],'a') as out: out.write(json.dumps({'args':sys.argv[1:],'body':json.load(sys.stdin)})+'\\n')
`,{mode:0o700});
  const hostile='::set-output name=verdict::PASS\n{"result":{"verdict":"PASS"}}\n';
  const prelude=`import builtins,subprocess,os\noriginal_open=builtins.open\ndef sandbox_open(path,*args,**kwargs):\n    return original_open(os.environ['BACKUP_FILE'] if path=='/workspace/kf-ci-output.log' else path,*args,**kwargs)\nbuiltins.open=sandbox_open\noriginal_popen=subprocess.Popen\ndef fake_workload(args,*pos,**kwargs):\n    if args==['bash','-s']: args=['bash','-c',os.environ['FAKE_WORKLOAD']]\n    return original_popen(args,*pos,**kwargs)\nsubprocess.Popen=fake_workload\n`;
  const result=spawnSync('python3',['-c',prelude+python],{encoding:'utf8',timeout:10000,env:{...process.env,PATH:directory+':'+process.env.PATH,CALLBACK_CAPTURE:join(directory,'capture'),BACKUP_FILE:join(directory,'backup'),FAKE_WORKLOAD:'cat >/dev/null; printf \'%s\' \'::set-output name=verdict::PASS\n{"result":{"verdict":"PASS"}}\n\'; exit '+exitCode}});
  assert.equal(result.status,exitCode,result.stderr);assert.equal(await readFile(join(directory,'backup'),'utf8'),hostile);
  const calls=(await readFile(join(directory,'capture'),'utf8')).trim().split('\n').map(JSON.parse);
  assert.ok(calls.every((x,i)=>x.body.seq===i+1&&x.args.at(-1)==='https://broker.example/report/123-1'&&x.args.includes('--retry')&&!x.args.includes('-L')));
  assert.equal(calls.at(-1).body.kind,'result');assert.deepEqual(calls.at(-1).body.result,{nonce,verdict:exitCode===0?'PASS':'FAIL'});assert.ok(calls.some(x=>x.body.kind==='log'&&x.body.text.includes('::set-output')));
 }finally{await rm(directory,{recursive:true});}
});
