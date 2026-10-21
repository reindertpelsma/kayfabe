// Explicit hand-authored mutations; each must kill >=2 independently named tests.
import {readFile,writeFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const root=new URL('../../',import.meta.url),base=new URL('./',import.meta.url);
const replace=(old,value)=>s=>{if(!s.includes(old))throw Error('Mutation anchor missing: '+old);return s.replace(old,value);};
const dropLine=fragment=>s=>{const lines=s.split('\n');if(lines.filter(x=>x.includes(fragment)).length!==1)throw Error('Ambiguous mutation anchor '+fragment);return lines.filter(x=>!x.includes(fragment)).join('\n');};
const mutations=[
 ['kill switch','src/index.js',replace("url.pathname==='/rent'&&env.RENTING_ENABLED!=='true'","false")],
 ['owner numeric identity','src/github.js',replace("const ownerPush=c.actor_id===cfg.owner&&String(r.triggering_actor.id)===cfg.owner;","const ownerPush=true;")],
 ['review head SHA','src/github.js',replace("latest?.state==='APPROVED'&&latest.commit_id===p.head.sha","latest?.state==='APPROVED'")],
 ['draft refusal','src/github.js',replace("if(p.draft===true&&!cfg.drafts)","if(false)")],
 ['OIDC claims','src/oidc.js',s=>s.replace(/if\(c\.iss!==ISSUER[\s\S]*?\)refuse\('UNAUTHORIZED',401\);/,"if(false)refuse('UNAUTHORIZED',401);")],
 ['credit reserve','src/index.js',dropLine('// MUTANT credit')],
 ['daily rental count','src/ledger.js',dropLine('// MUTANT daily-count')],
 ['daily spend','src/ledger.js',dropLine('if(daily+cost>cfg.daily)')],
 ['rolling 30-day spend','src/ledger.js',dropLine('// MUTANT rolling')],
 ['one live instance','src/index.js',replace('if(active&&active.ended===null){','if(false){')],
 ['superseding push','src/index.js',replace('if((active.pr!==null&&active.pr===target.pr||active.pr===null&&target.pr===null&&active.branch===target.branch)&&active.sha!==target.sha){','if(false){')],
 ['run idempotence','src/index.js',replace('if(prior){await recover(prior,vast);return json({...expose(prior),pr:target.pr});}','if(false){await recover(prior,vast);return json({...expose(prior),pr:target.pr});}')],
 ['SHA verdict cache','src/index.js',replace('if(cached&&!force){','if(false){')],
 ['same SHA minimum gap','src/index.js',dropLine('// MUTANT gap')],
 ['ledger-only operations','src/ledger.js',replace('const record=await this.granted(run);',"const record=await this.granted(run)||{key:id,id,label:'kf-ci-unrecorded',ended:null,pr:7};")],
 ['live label ownership','src/vast.js',replace('i.label!==record.label','false')],
 ['ambiguous-create binding','src/vast.js',dropLine('if(candidate.onstart!==record.script')],
 ['TTL destruction','src/index.js',replace('if(now()-record.started>=record.ttl){','if(false){')],
 ['final-marker destruction','src/index.js',s=>s.replaceAll('if(record.verdict){await cleanup(record,vast);return;}','')],
 ['SHA/nonce shell validation','src/onstart.js',dropLine("throw Error('Invalid fixed onstart inputs')")],
 ['log command sanitizer','src/sanitize.js',replace('export function sanitizeLog(bytes,cap=8192,secrets=[]){',"export function sanitizeLog(bytes,cap=8192,secrets=[]){return new TextDecoder().decode(bytes);")],
 ['marker and nonce verdict','src/sanitize.js',replace('export function parseResult(bytes,nonce){',"export function parseResult(bytes,nonce){return 'PASS';")],
 ['fixed offer admission','src/policy.js',replace('export function validOffer(offer,cfg){','export function validOffer(offer,cfg){return true;')],
 ['documented verification representation','src/policy.js',replace("(offer.verified===true||offer.verified===undefined&&offer.verification==='verified')","offer.verified===true")],
 ['documented model representation','src/policy.js',replace("MODELS.map(x=>x.toUpperCase()).includes(offer.gpu_name.replaceAll(' ','_').toUpperCase())","MODELS.includes(offer.gpu_name)")],
 ['documented offer pricing type','src/vast.js',replace("type:'ondemand'","type:'on-demand'")],
 ['poll permissions guard','workflow-guard.js',replace('if(primary&&JSON.stringify','if(false&&primary&&JSON.stringify')],
 ['attestation dependency guard','workflow-guard.js',replace('if(job.danger&&(filename===gpuName||reaches(job)))','if(false)')],
 ['cold-box disk guard','../../scripts/ci/vast_box_main.sh',replace('[[ "$available" -lt 60000000000 ]]','[[ false == true ]]')],
 ['must-fail control','../../scripts/ci/vast_box_main.sh',replace('exit "$control_rc"','exit 0')],
 ['scheduled cleanup wiring','src/index.js',replace('await this.service.sweep();','await Promise.resolve();')],
 ['Unicode format sanitizer','src/sanitize.js',dropLine('// MUTANT unicode-format')],
 ['excerpt command prefix','src/sanitize.js',replace("return excerpt.replace(/^(::|##\\[)/gm,'[box] $1');",'return excerpt;')],
 ['unsupported job syntax','workflow-guard.js',dropLine('// MUTANT job-shape')],
 ['distinct client refusal','../../.github/workflows/gpu-vast.yml',replace('description="GPU broker $state"',"description='GPU broker unavailable'")],
 ['terminal client completion','../../.github/workflows/gpu-vast.yml',replace('            if [[ "$state" = COMPLETE ]]; then exit 1; fi\n','')],
 ['public cron isolation','src/index.js',replace(String.raw`if(!/^\/(rent|status\/[1-9][0-9]*|destroy\/[1-9][0-9]*)$/.test(new URL(request.url).pathname))return new Response('Not found',{status:404});`,'')],
];
mutations.push(
 ['automatic base allowlist','src/github.js',s=>s.replaceAll('!cfg.bases.includes','false&&!cfg.bases.includes')],
 ['docs-only exclusion result','src/index.js',replace('if(target.notNeeded)','if(false)')],
 ['unknown file-list skip','src/github.js',s=>s.replaceAll('return false;','return true;')],
 ['canonical path validation','src/policy.js',dropLine("path.startsWith('ci/vast-broker/')")],
 ['renamed previous path','src/policy.js',replace('(x.previous_filename===undefined||skippedPath(x.previous_filename,patterns))','true')],
 ['cached PR display grant','src/index.js',s=>s.replace('await ledger.grant(key,record,target.pr);return json({...expose(record),pr:target.pr});','await ledger.grant(key,record);return json(expose(record));')],
 ['request bearer transport','src/index.js',replace("const token=request.headers.get('X-GH-Token');",'const token=null;')],
 ['request bearer retention','src/index.js',replace('await ledger.reserve(record);','record.githubToken=token;await ledger.reserve(record);')],
 ['public GitHub fallback','src/github.js',replace('if(this.headers.Authorization&&[401,403].includes(response.status))','if(false)')],
 ['owner branch without PR','src/github.js',replace('if(!p)return {','if(!p)refuse();if(!p)return {')],
 ['immutable API cache','src/github.js',replace('if(immutable&&cached&&','if(false&&immutable&&cached&&')],
 ['mandatory status writer jobs','workflow-guard.js',dropLine('// MUTANT writers')],
 ['token-shaped log redaction','src/sanitize.js',dropLine('// MUTANT token-shape')],
 ['isolated writer permissions','workflow-guard.js',dropLine('Status writer must not send its token')],
);
const results=[];
for(const [name,path,mutate] of mutations){
 const url=new URL(path,base),original=await readFile(url,'utf8');
 try{
   const changed=mutate(original);if(changed===original)throw Error('Mutation did not apply: '+name);
   await writeFile(url,changed);
   const syntaxChecks=path.endsWith('.yml')?changed.split('        run: |\n').slice(1).map(part=>{
     const lines=[];for(const line of part.split('\n')){if(line.trim()&&!line.startsWith('          '))break;lines.push(line.slice(10));}
     return spawnSync('bash',['-n'],{input:lines.join('\n'),encoding:'utf8'});
   }):[spawnSync(path.endsWith('.sh')?'bash':process.execPath,[path.endsWith('.sh')?'-n':'--check',fileURLToPath(url)],{encoding:'utf8'})];
   if(syntaxChecks.some(x=>x.status!==0))throw Error('Invalid mutant syntax: '+name+' '+syntaxChecks.map(x=>x.stderr).join(''));
   const r=spawnSync(process.execPath,['--test','ci/vast-broker/test'],{cwd:root,encoding:'utf8',maxBuffer:8*1024*1024,timeout:60000});
   const output=r.stdout+r.stderr,killed=[...output.matchAll(/^not ok \d+ - (.+)$/gm)].map(m=>m[1]);
   if(r.status===0||killed.length<2)throw Error(name+': FAILED mutation threshold ('+killed.length+' named killers)\n'+output.slice(-2000));
   results.push({name,count:killed.length,examples:killed.slice(0,3)});console.log(name+': '+killed.length+' tests failed');
 }finally{await writeFile(url,original);}
}
const rebuilt=spawnSync(process.execPath,['bundle.js'],{cwd:base,encoding:'utf8'});if(rebuilt.status!==0)throw Error(rebuilt.stderr);
const text='# Recorded manual mutation checks\n\n[MEASURED: 2026-10-21 mutation campaign] '+process.version+'; explicit patches in `../mutants.js`. Every source file restored in finally; syntax checked before tests. No live APIs or GPU. Each mutant requires >=2 named failing tests.\n\n| Mutation | Tests killed | Example killers |\n| --- | ---: | --- |\n'+results.map(r=>'| '+r.name+' | '+r.count+' | '+r.examples.join('; ')+' |').join('\n')+'\n';
await writeFile(new URL('test/mutants.md',base),text);console.log('All '+results.length+' mutants killed; source restored.');
