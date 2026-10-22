import test from 'node:test';import assert from 'node:assert/strict';import {spawnSync} from 'node:child_process';import {mkdtemp,writeFile,rm} from 'node:fs/promises';import {tmpdir} from 'node:os';import {join} from 'node:path';
import {onstart} from '../src/onstart.js';import {sha,nonce,env} from './helpers.js';
const token='v1.123.1.1800030000.'+'A'.repeat(43);
const script=path=>onstart(sha,nonce,180,env.REPOSITORY,{url:env.WORKER_URL,run:'123-1',token,path,runnerNonce:'f'.repeat(64)});
// FALSIFIER: an SSH login does not hold, a local shell holds, or the installed
// helper leaks account authority/uses redirects. All execution is mocked.
for(const prefix of ['', 'hold-prefix-'+'h'.repeat(32)])test('V5 generated hold helper is scoped HTTPS '+prefix.length,async()=>{
 const source=script(prefix),helper=source.split("<<'KF_HOLD'\n")[1]?.split('\nKF_HOLD')[0];assert.ok(helper,'missing hold helper');
 const directory=await mkdtemp(join(tmpdir(),'v5-hold-test-'));try{
  await writeFile(join(directory,'curl'),'#!/usr/bin/env python3\nimport json,sys\nprint(json.dumps(sys.argv[1:]))\n',{mode:0o700});
  const r=spawnSync('sh',['-s'],{input:helper,encoding:'utf8',env:{...process.env,PATH:directory+':'+process.env.PATH}});assert.equal(r.status,0,r.stderr);const args=JSON.parse(r.stdout);
  assert.equal(args.at(-1),env.WORKER_URL+(prefix?'/'+prefix:'')+'/hold/123-1');assert.ok(args.includes('Authorization: Bearer '+token));assert.ok(args.includes('=https'));assert.ok(!args.includes('-L'));assert.ok(!source.includes(env.VAST_API_KEY));assert.ok(!source.includes(env.OWNER_STOP_TOKEN));
 }finally{await rm(directory,{recursive:true});}
});
for(const ssh of [false,true])test('V5 profile hold hook runs only on SSH login '+ssh,async()=>{
 const source=script(''),hook=source.split("<<'KF_PROFILE'\n")[1]?.split('\nKF_PROFILE')[0];assert.ok(hook,'missing login profile hook');assert.ok(!hook.includes(token));
 const directory=await mkdtemp(join(tmpdir(),'v5-profile-test-'));try{
  await writeFile(join(directory,'hold'),'#!/bin/sh\nprintf called\n',{mode:0o700});
  const r=spawnSync('bash',['--noprofile','--norc','-c',hook.replaceAll('/usr/local/bin/kf-box-hold',join(directory,'hold')).replaceAll('>/dev/null','')],{encoding:'utf8',env:{...process.env,SSH_CONNECTION:ssh?'synthetic connection':''}});assert.equal(r.status,0,r.stderr);assert.equal(r.stdout.includes('called'),ssh);
 }finally{await rm(directory,{recursive:true});}
});
for(const exhausted of ['attempts','bytes'])test('V5 empty-log heartbeat survives sender cap '+exhausted,()=>{
 const s=script(''),python=s.split("python3 - <<'KF_REPORT'\n")[1].split('\nKF_REPORT')[0].split("work=r'''")[0];
 const probe=`import json,sys,types\nscope={};exec(sys.argv[1],scope);calls=[]\nscope['subprocess'].run=lambda *a,**k:(calls.append(json.loads(k['input'])) or types.SimpleNamespace(returncode=0,stdout=b'{}'))\nscope['attempted' if sys.argv[2]=='attempts' else 'sent']=600 if sys.argv[2]=='attempts' else 900000\nscope['post']('log','')\nassert len(calls)==1 and calls[0]['kind']=='log' and calls[0]['text']==''\n`;
 const r=spawnSync('python3',['-I','-c',probe,python,exhausted],{encoding:'utf8',timeout:5000});assert.equal(r.status,0,r.stderr);
});
for(const exhausted of ['attempts','bytes'])test('V5 sender busy output cannot starve heartbeat '+exhausted,()=>{
 const s=script(''),python=s.split("python3 - <<'KF_REPORT'\n")[1].split('\nKF_REPORT')[0];
 const r=spawnSync('python3',['-I',new URL('v5_sender_fixture.py',import.meta.url).pathname,python,exhausted],{encoding:'utf8',timeout:5000});assert.equal(r.status,0,r.stderr);
});
