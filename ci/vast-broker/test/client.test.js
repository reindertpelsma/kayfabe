import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,readFile,writeFile,chmod,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
async function client(state,verdict='UNAVAILABLE',code=200,pending=false){
 const directory=await mkdtemp(join(tmpdir(),'v1-client-test-'));
 try{
  const yaml=await readFile(new URL('../../../.github/workflows/gpu-vast.yml',import.meta.url),'utf8');
  const script=yaml.split('        run: |\n')[1].split('\n').map(x=>x.startsWith('          ')?x.slice(10):x).join('\n');
  const payload={state,verdict,log:'::error::hostile\n',id:'900',sha:'a'.repeat(40),pr:7};
  const curl=join(directory,'curl');
  await writeFile(curl,`#!/usr/bin/env node
const fs=require('node:fs');const args=process.argv.slice(2);const url=args.at(-1);
if(url.includes('/statuses/')){const body=args[args.indexOf('--data-binary')+1].slice(1);fs.appendFileSync(process.env.STATUS_LOG,fs.readFileSync(body)+'\\n');process.stdout.write('{}');}
else if(url.includes('oidc.example'))process.stdout.write(JSON.stringify({value:'mock.jwt.value'}));
else{const previous=fs.existsSync(process.env.CALL_LOG)?fs.readFileSync(process.env.CALL_LOG,'utf8').trim().split('\\n').length:0;fs.appendFileSync(process.env.CALL_LOG,'worker\\n');if(previous>=2)process.exit(77);let response=JSON.parse(process.env.MOCK_RESPONSE);if(process.env.FIRST_PENDING==='1'&&previous===0){response.state='RUNNING';response.verdict='UNAVAILABLE';}fs.writeFileSync(args[args.indexOf('-o')+1],JSON.stringify(response));process.stdout.write(process.env.MOCK_CODE);}
`);await chmod(curl,0o700);
  await writeFile(join(directory,'sleep'),'#!/bin/sh\nexit 0\n');await chmod(join(directory,'sleep'),0o700);
  const response=spawnSync('bash',['-c',script],{encoding:'utf8',timeout:10000,maxBuffer:1048576,env:{...process.env,
    PATH:directory+':'+process.env.PATH,WORKER_URL:'https://broker.example',PR_NUMBER:'7',TARGET_SHA:'a'.repeat(40),GH_TOKEN:'mock-job-token',
    GITHUB_SERVER_URL:'https://github.com',GITHUB_REPOSITORY:'reindertpelsma/kayfabe',GITHUB_RUN_ID:'123',
    ACTIONS_ID_TOKEN_REQUEST_TOKEN:'mock-request-token',ACTIONS_ID_TOKEN_REQUEST_URL:'https://oidc.example/request?test=1',
    FIRST_PENDING:pending?'1':'0',MOCK_RESPONSE:JSON.stringify(payload),MOCK_CODE:String(code),STATUS_LOG:join(directory,'statuses'),CALL_LOG:join(directory,'calls')}});
  const statuses=(await readFile(join(directory,'statuses'),'utf8')).trim().split('\n').map(x=>JSON.parse(x));
  const calls=(await readFile(join(directory,'calls'),'utf8')).trim().split('\n').length;
  return {...response,statuses,calls};
 }finally{await rm(directory,{recursive:true});}
}
test('workflow client posts success on the tested head and wraps hostile log',async()=>{const r=await client('COMPLETE','PASS');assert.equal(r.status,0,r.stderr);assert.deepEqual(r.statuses.map(x=>x.state),['pending','success']);assert.match(r.stdout,/::stop-commands::[0-9a-f]+\n::error::hostile/);assert.equal(r.calls,1);});
test('workflow client posts failure when box verdict fails',async()=>{const r=await client('COMPLETE','FAIL');assert.equal(r.status,1);assert.equal(r.statuses.at(-1).state,'failure');});
test('workflow client exposes budget refusal as a distinct failing state',async()=>{const r=await client('UNAVAILABLE_BUDGET','UNAVAILABLE',409);assert.equal(r.status,1);assert.match(r.statuses.at(-1).description,/UNAVAILABLE_BUDGET/);});
test('workflow client exposes missing approval as a distinct failing state',async()=>{const r=await client('NEEDS_APPROVAL','UNAVAILABLE',403);assert.equal(r.status,1);assert.match(r.statuses.at(-1).description,/NEEDS_APPROVAL/);});
test('workflow client exits on a terminal unavailable verdict without another poll',async()=>{const r=await client('COMPLETE','UNAVAILABLE');assert.equal(r.status,1);assert.equal(r.calls,1);});

test('workflow client exits when a pending run completes unavailable',async()=>{const r=await client('COMPLETE','UNAVAILABLE',200,true);assert.equal(r.status,1);assert.equal(r.calls,2);});
