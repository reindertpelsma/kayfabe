import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdtemp,rm} from 'node:fs/promises';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {spawnSync} from 'node:child_process';
const origin='https://quiet-tenant.workers.example';
const prefix='unit-path-only-'+'p'.repeat(32);
async function run({path=prefix,url=origin,failure='',pending=false,trace=false}={}){
 const directory=await mkdtemp(join(tmpdir(),'v1-secret-client-'));
 try{
  const yaml=await readFile(new URL('../../../.github/workflows/gpu-vast.yml',import.meta.url),'utf8');
  const script=yaml.split('  gpu:\n')[1].split('  report:\n')[0].split('        run: |\n')[1].split('\n').map(x=>x.startsWith('          ')?x.slice(10):x).join('\n');
  await writeFile(join(directory,'curl'),`#!/usr/bin/env node
const fs=require('node:fs'),args=process.argv.slice(2),url=args.at(-1),callsFile=process.env.CALLS;
const calls=fs.existsSync(callsFile)?JSON.parse(fs.readFileSync(callsFile,'utf8')):[];
const masks=fs.existsSync(process.env.MASKS)?fs.readFileSync(process.env.MASKS,'utf8').trim().split('\\n'):[];
calls.push({url,args,masks});fs.writeFileSync(callsFile,JSON.stringify(calls));
if(url.startsWith('https://oidc.example/')){process.stdout.write(JSON.stringify({value:'mock.jwt.value'}));process.exit(0);}
const api=calls.filter(x=>!x.url.startsWith('https://oidc.example/'));
if((process.env.FAILURE==='rent'&&api.length===1)||(process.env.FAILURE==='status'&&api.length>=2)){
 process.stderr.write('curl failure '+url+' host='+new URL(url).hostname+' path='+process.env.WORKER_PATH+'\\n');process.exit(6);
}
if(url.includes('/stop/')){process.stdout.write('{}');process.exit(0);}
const body={state:process.env.PENDING==='1'&&api.length===1?'RUNNING':'COMPLETE',verdict:process.env.PENDING==='1'&&api.length===1?'UNAVAILABLE':'PASS',run:'123-1',sha:'a'.repeat(40),pr:7,log_tail_sanitised:'[box] safe data\\n'};
fs.writeFileSync(args[args.indexOf('-o')+1],JSON.stringify(body));process.stdout.write('200');
`,{mode:0o700});
  await writeFile(join(directory,'sleep'),'#!/bin/sh\nexit 0\n',{mode:0o700});
  const prelude="printf() { if [[ \"${1:-}\" = '::add-mask::%s\\n' ]]; then builtin printf '%s\\n' \"$2\" >> \"$MASKS\"; fi; builtin printf \"$@\"; }\n";
  const output=join(directory,'output');await writeFile(output,'');
  const r=spawnSync('bash',[...(trace?['-x']:[]),'-c',prelude+script],{encoding:'utf8',timeout:10000,env:{...process.env,PATH:directory+':'+process.env.PATH,WORKER_URL:url,WORKER_PATH:path,PR_NUMBER:'7',TARGET_SHA:'a'.repeat(40),GH_TOKEN:'mock-job-token',ACTIONS_ID_TOKEN_REQUEST_TOKEN:'mock-request-token',ACTIONS_ID_TOKEN_REQUEST_URL:'https://oidc.example/request?test=1',GITHUB_OUTPUT:output,MASKS:join(directory,'masks'),CALLS:join(directory,'calls'),FAILURE:failure,PENDING:pending?'1':'0'}});
  const calls=await readFile(join(directory,'calls'),'utf8').then(JSON.parse).catch(()=>[]);
  return {...r,calls,outputs:await readFile(output,'utf8'),visible:r.stdout.split('\n').filter(x=>!x.startsWith('::add-mask::')).join('\n')+'\n'+r.stderr};
 }finally{await rm(directory,{recursive:true});}
}
for(const slash of ['', '/'])test('secret API base includes prefix while audience stays exact origin '+JSON.stringify(slash),async()=>{const r=await run({url:origin+slash,pending:true});assert.equal(r.status,0,r.stderr);const api=r.calls.filter(x=>!x.url.startsWith('https://oidc.example/'));assert.deepEqual(api.map(x=>x.url),[origin+'/'+prefix+'/rent',origin+'/'+prefix+'/status/123-1']);for(const x of r.calls.filter(x=>x.url.startsWith('https://oidc.example/')))assert.equal(new URL(x.url).searchParams.get('audience'),origin+slash);});
for(const path of ['',prefix])test('derived masks exist before the first request '+path.length,async()=>{const r=await run({path});assert.equal(r.status,0,r.stderr);const required=[origin,'quiet-tenant.workers.example','quiet-tenant','workers','example',encodeURIComponent(origin),...(path?[path,origin+'/'+path]:[])];for(const value of required)assert.ok(r.calls[0]?.masks.includes(value),'missing derived mask '+value);});
for(const failure of ['rent','status'])test('transport failures never print endpoint parts '+failure,async()=>{const r=await run({failure,pending:failure==='status'});assert.notEqual(r.status,0);for(const value of [origin,'quiet-tenant.workers.example',prefix])assert.ok(!r.visible.includes(value),r.visible);assert.doesNotMatch(r.outputs,/https:|quiet-tenant|unit-path-only/);});
for(const path of ['',prefix])test('shell tracing cannot reveal the endpoint '+path.length,async()=>{const r=await run({path,trace:true});assert.equal(r.status,0,r.stderr);for(const value of [origin,'quiet-tenant.workers.example','quiet-tenant',...(path?[path]:[])])assert.ok(!r.visible.includes(value),r.visible);assert.match(r.outputs,/^state=COMPLETE\nverdict=PASS\n$/);});
for(const bad of ['bad/path','x'.repeat(129)])test('invalid path secret fails without logging '+bad.length,async()=>{const r=await run({path:bad});assert.notEqual(r.status,0);assert.equal(r.calls.length,0);assert.ok(!r.visible.includes(bad));});
