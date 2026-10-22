// Explicit hand-authored mutations; each must kill >=2 independently named tests.
import {readFile,writeFile,readdir} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const root=new URL('../../',import.meta.url),base=new URL('./',import.meta.url);
const replace=(old,value)=>s=>{if(!s.includes(old))throw Error('Mutation anchor missing: '+old);return s.replace(old,value);};
const dropLine=fragment=>s=>{const lines=s.split('\n');if(lines.filter(x=>x.includes(fragment)).length!==1)throw Error('Ambiguous mutation anchor '+fragment);return lines.filter(x=>!x.includes(fragment)).join('\n');};
// R3 replaced seven inline-client/cache cases; see traces/vast_broker_v4_round3_20261018/mutation-migration.json.
const mutations=[
 ['SSH onstart launch','src/vast.js',replace("runtype:'ssh_direct'","runtype:'args'")],
 ['pending provider status','src/index.js',replace("i.actual_status==null||['created'","['created'")],
 ['resolved launch preflight','smoke.py',replace("else 'ELIGIBLE_OFFER_FOUND'","else 'BLOCKED_LAUNCH_CONTRACT'")],
 ['live tunnel readiness','smoke-live.py',s=>replace(" if match and match[0]!='https://api.trycloudflare.com' else None"," if match else None")(replace("if 'Your quick Tunnel has been created!' not in log or 'Registered tunnel connection' not in log:return None","if False:return None")(s))],
 ['live ingress duplicate probes','live/ingress.mjs',replace("const kind=req.url.includes('/kill/')?'kill':'report';","const kind=req.url.includes('/kill/')?'kill':'report';try{await fetch('https://console.vast.ai/api/v0/instances/'+owned.id+'/');}catch{}")],
 ['inherited endpoint secrets','workflow-guard.js',dropLine('// MUTANT secret-inherit')],
 ['endpoint aliases in step metadata','workflow-guard.js',dropLine('// MUTANT secret-metadata')],
 ['optional prefix admission','src/routes.js',dropLine('// MUTANT prefix-match')],
 ['native constant-time prefix comparison','src/routes.js',replace("if(!await constantTimeEqual(prefix,parts[1]||''))return null;","if(prefix!==(parts[1]||''))return null;")],
 ['public prefix gate','src/index.js',replace('async fetch(request,env){const path=await routePath(request,env);','async fetch(request,env){const path=new URL(request.url).pathname;')],
 ['ledger service prefix gate','src/index.js',replace('const url=new URL(request.url),path=await routePath(request,env);','const url=new URL(request.url),path=new URL(request.url).pathname;')],
 ['prefix charset and bounds','src/routes.js',replace('export const validPathPrefix=value=>','export const validPathPrefix=value=>true||')],
 ['fixed callback prefix','src/onstart.js',replace("url='${origin}${prefix?'/'+prefix:''}/report/${callback.run}'","url='${origin}/report/${callback.run}'")],
 ['endpoint text redaction','src/routes.js',replace('export function endpointSecrets(env){','export function endpointSecrets(env){return [];')],
 ['Actions secret instead of variable','../../.github/workflows/gpu-vast.yml',s=>s.replaceAll('secrets.VAST_BROKER_','vars.VAST_BROKER_')],
 ['shell tracing disabled','../../.github/workflows/gpu-vast.yml',dropLine('          set +x')],
 ['transport diagnostic suppression','../../.github/workflows/gpu-vast.yml',s=>s.replaceAll('2>/dev/null','').replaceAll('2>&1','')],
 ['secret context placement guard','workflow-guard.js',dropLine('// MUTANT secret-reference')],
 ['secret object access guard','workflow-guard.js',dropLine('// MUTANT secret-object')],
 ['secret alias logging guard','workflow-guard.js',dropLine('// MUTANT secret-print')],
 ['secret job output guard','workflow-guard.js',dropLine('// MUTANT secret-output')],
 ['secret alias job isolation','workflow-guard.js',dropLine('// MUTANT secret-alias')],
 ['literal run and renderer enclosure','workflow-guard.js',s=>dropLine('// MUTANT secret-run-style')(s).replace("if(!approvedRenderer)fail('requires exact trusted BASE controller bootstrap');",'')],
 ['HMAC tag and commit binding','src/tokens.js',replace("return await crypto.subtle.verify('HMAC',await this.cryptoKey(),bytes,tokenEncoder.encode(this.message(r,Number(parts[3]))));",'return true;')],
 ['closed ledger rejects reports','src/index.js',replace('||r.ended!==null||r.closed||r.resultAt!==undefined)', '||r.ended!==null||false||r.resultAt!==undefined)')],
 ['actual Vast liveness','src/index.js',dropLine('// MUTANT report-live')],
 ['liveness cache expiration','src/index.js',replace('now()-r.liveCheckedAt>=3000','false')],
 ['report count cap','src/index.js',s=>s.replaceAll('r.reportCount>=cfg.reportCount','false')],
 ['cumulative report byte cap','src/reports.js',replace('remaining=cfg.reportBytes-record.reportBytes','remaining=cfg.reportBytes')],
 ['streamed request byte cap','src/reports.js',replace('limit=Math.min(65536,remaining)','limit=remaining')],
 ['report rate cap','src/index.js',dropLine('// MUTANT report-rate')],
 ['preparse JSON depth gate','src/reports.js',replace('if(++depth>4)','if(++depth>40000)')],
 ['strict report schema','src/reports.js',dropLine("if(!object(p)||!keys(p,")],
 ['strict result schema','src/reports.js',dropLine('if(!object(p.result)')],
 ['report sequence immutability','src/index.js',replace('if(payload.seq<=r.lastSeq)','if(false)')],
 ['structured result nonce','src/index.js',dropLine("if(payload.kind==='result'&&payload.result.nonce")],
 ['free text cannot decide verdict','src/index.js',replace('r.lastSeq=payload.seq;r.lastDigest=digest;',"r.lastSeq=payload.seq;r.lastDigest=digest;if(payload.text?.includes('PASS'))r.verdict='PASS';")],
 ['structured result owned cleanup','src/index.js',replace("if(payload.kind==='result'){r.resultAt=now();r.verdict=payload.result.verdict;r.liveCheckedAt=null;}", "if(payload.kind==='result'){r.closed='RESULT';r.verdict=payload.result.verdict;await ledger.settle(r,now());}")],
 ['native owner secret comparison','src/tokens.js',replace('export async function constantTimeEqual(expected,provided){','export async function constantTimeEqual(expected,provided){return true;')],
 ['owner-only stop-all','src/index.js',dropLine('// MUTANT stop-auth')],
 ['OIDC run-scoped status and stop','src/index.js',dropLine('// MUTANT stop-run')],
 ['ledger-only owner stop','src/index.js',replace("const r=await ledger.granted(match[2])||await ledger.get('record:'+match[2]);", "const r=await ledger.granted(match[2])||await ledger.get('record:'+match[2])||{key:match[2],id:match[2].split('-')[0],label:'kf-ci-unrecorded',ended:null};")],
 ['stop-all cannot adopt account entries','src/index.js',replace('const records=await ledger.openRecords();', "const records=await ledger.openRecords();records.push({key:'888-1',id:'888',label:'kf-ci-unrecorded',ended:null});")],
 ['revocation precedes failed delete','src/index.js',replace('record.closed=reason;','record.closed=null;')],
 ['all log lines prefixed','src/sanitize.js',replace("const rendered='[box] '+safe+'\\n';", "const rendered=safe+'\\n';")],
 ['embedded command delimiters','src/sanitize.js',dropLine('// MUTANT command-delimiters')],
 ['per-line UTF8 byte cap','src/sanitize.js',replace('const safe=utf8Prefix(line,1018);','const safe=line;')],
 ['log line count cap','src/sanitize.js',s=>s.replaceAll('slice(-200)','slice(-20000)').replaceAll('length>200','length>20000')],
 ['log tail byte cap','src/sanitize.js',s=>s.replaceAll('size>cap','false').replaceAll('size>8192','false')],
 ['orphan index cannot block cleanup','src/ledger.js',replace("if(!record){console.log(JSON.stringify({event:'incomplete_ledger_index'}));continue;}",'if(!record)refuse();')],
 ['kill switch','src/index.js',replace("path==='/rent'&&env.RENTING_ENABLED!=='true'",'false')],
 ['owner numeric identity','src/github.js',replace('const ownerPush=c.actor_id===cfg.owner&&String(r.triggering_actor.id)===cfg.owner;','const ownerPush=true;')],
 ['review head SHA','src/github.js',replace("latest?.state==='APPROVED'&&latest.commit_id===p.head.sha","latest?.state==='APPROVED'")],
 ['draft refusal','src/github.js',replace('if(p.draft===true&&!cfg.drafts)','if(false)')],
 ['OIDC claims','src/oidc.js',s=>s.replace(/if\(c\.iss!==this\.issuer[\s\S]*?\)refuse\('UNAUTHORIZED',401\);/,"if(false)refuse('UNAUTHORIZED',401);")],
 ['credit reserve','src/index.js',dropLine('// MUTANT credit')],
 ['daily rental count','src/ledger.js',dropLine('// MUTANT daily-count')],
 ['daily spend','src/ledger.js',dropLine('if(daily+cost>cfg.daily)')],
 ['rolling 30-day spend','src/ledger.js',dropLine('// MUTANT rolling')],
 ['one live instance','src/index.js',replace('if(active&&active.ended===null){','if(false){')],
 ['superseding push','src/index.js',replace('if((active.pr!==null&&active.pr===target.pr||active.pr===null&&target.pr===null&&active.branch===target.branch)&&active.sha!==target.sha){','if(false){')],
 ['same SHA minimum gap','src/index.js',dropLine('// MUTANT gap')],
 ['live label ownership','src/vast.js',replace('i.label!==record.label','false')],
 ['ambiguous-create binding','src/vast.js',dropLine("if(typeof candidate.onstart!==")],
 ['TTL destruction','src/index.js',replace('if(now()-record.started>=record.ttl){','if(false){')],
 ['SHA/nonce shell validation','src/onstart.js',dropLine("throw Error('Invalid fixed onstart inputs')")],
 ['callback shell validation','src/onstart.js',dropLine("throw Error('Invalid callback capability')")],
 ['fixed offer admission','src/policy.js',replace('export function validOffer(offer,cfg){','export function validOffer(offer,cfg){return true;')],
 ['documented verification representation','src/policy.js',replace("(offer.verified===true||offer.verified===undefined&&offer.verification==='verified')",'offer.verified===true')],
 ['documented model representation','src/policy.js',replace("MODELS.map(x=>x.toUpperCase()).includes(offer.gpu_name.replaceAll(' ','_').toUpperCase())",'MODELS.includes(offer.gpu_name)')],
 ['outbound model names','src/vast.js',replace("MODELS.map(x=>x.replaceAll('_',' '))",'MODELS')],
 ['documented offer pricing type','src/vast.js',replace("type:'ondemand'","type:'on-demand'")],
 ['poll permissions guard','workflow-guard.js',replace('if(primary&&JSON.stringify','if(false&&primary&&JSON.stringify')],
 ['attestation dependency guard','workflow-guard.js',replace('if(job.danger&&(filename===gpuName||reaches(job)))','if(false)')],
 ['cold-box disk guard','../../scripts/ci/vast_box_main.sh',replace('[[ "$available" -lt 60000000000 ]]','[[ false == true ]]')],
 ['must-fail control','../../scripts/ci/vast_box_main.sh',replace('exit "$control_rc"','exit 0')],
 ['scheduled cleanup wiring','src/index.js',replace('await this.service.sweep();','await Promise.resolve();')],
 ['Unicode format sanitizer','src/sanitize.js',dropLine('// MUTANT unicode-format')],
 ['unsupported job syntax','workflow-guard.js',dropLine('// MUTANT job-shape')],
 ['distinct client refusal','../../.github/workflows/gpu-vast.yml',replace('description="GPU broker $state"',"description='GPU broker unavailable'")],
 ['public cron isolation','src/index.js',replace("if(path===null||!/^\\/(?:rent|stop-all|(?:report|kill|status|stop|logs)\\/[1-9][0-9]{0,19}-[1-9][0-9]{0,19})$/.test(path))return notFound();",'')],
 ['automatic base allowlist','src/github.js',s=>s.replaceAll('!cfg.bases.includes','false&&!cfg.bases.includes')],
 ['docs-only exclusion result','src/index.js',replace('if(target.notNeeded)','if(false)')],
 ['unknown file-list skip','src/github.js',s=>s.replaceAll('return false;','return true;')],
 ['canonical path validation','src/policy.js',dropLine("path.startsWith('ci/vast-broker/')")],
 ['renamed previous path','src/policy.js',replace('(x.previous_filename===undefined||skippedPath(x.previous_filename,patterns))','true')],
 ['request bearer transport','src/index.js',replace("const token=request.headers.get('X-GH-Token');",'const token=null;')],
 ['request bearer retention','src/index.js',replace('await ledger.reserve(r);','r.githubToken=token;await ledger.reserve(r);')],
 ['public GitHub fallback','src/github.js',replace('if(this.headers.Authorization&&[401,403].includes(response.status))','if(false)')],
 ['owner branch without PR','src/github.js',replace('if(!p)return {','if(!p)refuse();if(!p)return {')],
 ['immutable API cache','src/github.js',replace('if(immutable&&cached&&','if(false&&immutable&&cached&&')],
 ['mandatory status writer jobs','workflow-guard.js',dropLine('// MUTANT writers')],
 ['token-shaped log redaction','src/sanitize.js',dropLine('// MUTANT token-shape')],
 ['isolated writer permissions','workflow-guard.js',dropLine('Status writer must not send its token')],
];
if(process.env.MUTANT_PREFLIGHT){for(const [name,path,mutate] of mutations){try{const source=await readFile(new URL(path,base),'utf8');if(mutate(source)===source)throw Error('did not change');}catch(e){console.log(name+': '+e.message);process.exitCode=1;}}if(!process.exitCode)console.log('All '+mutations.length+' anchors valid');process.exit(process.exitCode||0);}
// Legacy source campaign; R3 tests exercise the controller separately below.
const workerTests=(await readdir(new URL('test/',base))).filter(x=>x.endsWith('.test.js')&&!x.endsWith('client.test.js')).map(x=>'ci/vast-broker/test/'+x);
const results=[];
for(const [name,path,mutate] of mutations.filter(m=>!process.env.MUTANT_FILTER||m[0].includes(process.env.MUTANT_FILTER))){
 const url=new URL(path,base),original=await readFile(url,'utf8');
 try{
   const changed=mutate(original);if(changed===original)throw Error('Mutation did not apply: '+name);
   await writeFile(url,changed);
   const syntaxChecks=path.endsWith('.yml')?changed.split('        run: |\n').slice(1).map(part=>{
     const lines=[];for(const line of part.split('\n')){if(line.trim()&&!line.startsWith('          '))break;lines.push(line.slice(10));}
     return spawnSync('bash',['-n'],{input:lines.join('\n'),encoding:'utf8'});
   }):[path.endsWith('.py')?spawnSync('python3',['-c','import ast,sys;ast.parse(open(sys.argv[1]).read())',fileURLToPath(url)],{encoding:'utf8'}):spawnSync(path.endsWith('.sh')?'bash':process.execPath,[path.endsWith('.sh')?'-n':'--check',fileURLToPath(url)],{encoding:'utf8'})];
   if(syntaxChecks.some(x=>x.status!==0))throw Error('Invalid mutant syntax: '+name+' '+syntaxChecks.map(x=>x.stderr).join(''));
   const r=spawnSync(process.execPath,['--test',...(path.endsWith('.yml')?['ci/vast-broker/test']:workerTests)],{cwd:root,encoding:'utf8',maxBuffer:8*1024*1024,timeout:60000});
   const output=r.stdout+r.stderr,killed=[...output.matchAll(/^not ok \d+ - (.+)$/gm)].map(m=>m[1]);
   if(r.status===0||killed.length<2)throw Error(name+': FAILED mutation threshold ('+killed.length+' named killers)\n'+output.slice(-2000));
   results.push({name,count:killed.length,examples:killed.slice(0,3)});if(process.env.MUTANT_RESULTS_FILE)await writeFile(process.env.MUTANT_RESULTS_FILE,JSON.stringify(results,null,2)+'\n');console.log(name+': '+killed.length+' tests failed');
 }finally{await writeFile(url,original);}
}
const rebuilt=spawnSync(process.execPath,['bundle.js'],{cwd:base,encoding:'utf8'});if(rebuilt.status!==0)throw Error(rebuilt.stderr);
if(process.env.MUTANT_RESULTS_FILE)await writeFile(process.env.MUTANT_RESULTS_FILE,JSON.stringify(results,null,2)+'\n');
if(process.env.MUTANT_FILTER){console.log('Filtered mutants passed; full table unchanged.');process.exit(0);}
const text='# Recorded manual mutation checks\n\n[MEASURED: 2026-10-22 endpoint secrecy plus Amendment 3/3b mutation campaign] '+process.version+'; explicit patches in `../mutants.js`. Every source file restored in finally; syntax checked before tests. Worker mutations run all source-facing test files explicitly in parallel; workflow mutations also run shell-client fixtures. Those fixtures read YAML only and never import Worker source. No live APIs or GPU. Each mutant requires >=2 named failing tests.\n\n| Mutation | Tests killed | Example killers |\n| --- | ---: | --- |\n'+results.map(r=>'| '+r.name+' | '+r.count+' | '+r.examples.join('; ')+' |').join('\n')+'\n';
await writeFile(new URL('test/mutants.md',base),text);console.log('All '+results.length+' mutants killed; source restored.');

// All campaigns mutate files sequentially; never run them alongside baseline tests.
const v4=spawnSync('python3',['v4-mutants.py',...(process.env.V4_MUTANT_RESULTS_FILE?['--results',process.env.V4_MUTANT_RESULTS_FILE]:[])],{cwd:base,stdio:'inherit'});
if(v4.status!==0)throw Error('V4 mutation campaign failed');
console.log('Full campaign passed: '+results.length+' legacy plus 32 V4 mutants.');

const round3=spawnSync('python3',['round3-mutants.py',...(process.env.ROUND3_MUTANT_RESULTS_FILE?['--results',process.env.ROUND3_MUTANT_RESULTS_FILE]:[])],{cwd:base,stdio:'inherit'});
if(round3.status!==0)throw Error('Round 3 mutants failed');
