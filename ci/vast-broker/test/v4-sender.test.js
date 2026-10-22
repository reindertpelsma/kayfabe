import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {spawnSync} from 'node:child_process';
import {onstart} from '../src/onstart.js';
import {sha,nonce,env} from './helpers.js';
// FALSIFIER: a fast bounded workload loses its first bytes before callback flush,
// or its final-result tail repeats already-delivered log lines.
for(const exitCode of [0,7])test('V4 sender preserves fast full bounded output once '+exitCode,async()=>{
 const dir=await mkdtemp(tmpdir()+'/v4-sender-');
 try{
  const script=onstart(sha,nonce,20,env.REPOSITORY,{url:env.WORKER_URL,run:'123-1',token:'v1.123.1.1800009000.'+'A'.repeat(43)});
  const python=script.split("python3 - <<'KF_REPORT'\n")[1].split('\nKF_REPORT')[0];
  await writeFile(dir+'/curl',"#!/usr/bin/env python3\nimport json,os,sys\nwith open(os.environ['CAPTURE'],'a') as out: out.write(json.dumps(json.load(sys.stdin))+'\\n')\nprint(json.dumps({'grace_remaining_seconds':60}))\n",{mode:0o700});
  const output=Array.from({length:3000},(_,i)=>`message ${i} of bounded output\n`).join('');await writeFile(dir+'/payload',output);
  const prelude="import builtins,subprocess,os\noriginal_open=builtins.open\ndef sandbox_open(path,*args,**kwargs):\n    return original_open(os.environ['BACKUP'] if path=='/workspace/kf-ci-output.log' else path,*args,**kwargs)\nbuiltins.open=sandbox_open\noriginal_popen=subprocess.Popen\ndef workload(args,*pos,**kwargs):\n    if args==['bash','-s']: args=['bash','-c',os.environ['WORK']]\n    return original_popen(args,*pos,**kwargs)\nsubprocess.Popen=workload\n";
  const run=spawnSync('python3',['-c',prelude+python],{encoding:'utf8',timeout:10000,env:{...process.env,PATH:dir+':'+process.env.PATH,CAPTURE:dir+'/capture',BACKUP:dir+'/backup',WORK:'cat >/dev/null; cat '+dir+'/payload; exit '+exitCode}});
  assert.equal(run.status,exitCode,run.stderr);assert.match(run.stdout,/Worker reports 1 minutes remaining/);const calls=(await readFile(dir+'/capture','utf8')).trim().split('\n').map(JSON.parse);
  assert.equal(calls.filter(x=>x.kind==='log').map(x=>x.text).join(''),output);
  assert.doesNotMatch(calls.at(-1).text??'',/message 2999/);assert.equal(calls.at(-1).result.verdict,exitCode===0?'PASS':'FAIL');
 }finally{await rm(dir,{recursive:true});}
});
