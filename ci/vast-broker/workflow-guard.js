import {runnerBootstrap} from './workflow-bootstrap.js';
// Security-focused reader for the repository's indentation-based workflow subset.
// Unsupported dynamic permissions/dependencies fail closed; this is not a YAML evaluator.
import {readdir,readFile} from 'node:fs/promises';
function jobsIn(text){
  const lines=text.split('\n');const jobs=[];let inside=false,current;
  for(const line of lines){
    if(line==='jobs:'){inside=true;continue;}
    if(!inside)continue;
    if(/^\S/.test(line)&&!line.startsWith('#')){inside=false;continue;}
    const match=/^  ([\w-]+):\s*(?:#.*)?$/.exec(line);
    if(match){current={id:match[1],lines:[]};jobs.push(current);}
    else if(/^  [^\s#].*:/.test(line)){current={id:'unsupported-'+jobs.length,lines:[],unsupported:true};jobs.push(current);} // MUTANT job-shape
    else if(current)current.lines.push(line);
  }
  return jobs.map(job=>{
    const body=job.lines.join('\n');const needs=[];let dependency=false,permissions=false;const grants={};let unsupported=job.unsupported===true;
    for(const line of job.lines){
      if(/^    needs:/.test(line)){
        const v=line.replace(/^    needs:\s*/,'').replace(/#.*/,'').trim();dependency=!v;
        if(v){if(!/^[\w\s,\[\]'"-]+$/.test(v))unsupported=true;else needs.push(...v.replace(/[\[\]'",]/g,' ').split(/\s+/).filter(Boolean));}
        continue;
      }
      if(dependency){const m=/^      - ([\w-]+)\s*$/.exec(line);if(m){needs.push(m[1]);continue;}if(/^    \S/.test(line))dependency=false;else if(line.trim())unsupported=true;}
      if(/^    permissions:/.test(line)){permissions=true;if(line.trim()!=='permissions:')unsupported=true;continue;}
      if(permissions){const m=/^      ([\w-]+): (read|write|none)\s*$/.exec(line);if(m){grants[m[1]]=m[2];continue;}if(/^    \S/.test(line))permissions=false;else if(line.trim()&&!line.trim().startsWith('#'))unsupported=true;}
    }
    const danger=/attest|release|publish/i.test(job.id)||/uses:.*(?:attest|release)|gh\s+(?:attestation|release)\s|attestations:\s*write/i.test(body);
    return {...job,body,needs,grants,unsupported,danger,opaque:/^    uses:/m.test(body)};
  });
}
function endpointBoundary(filename,text,jobs){
  const errors=[],fail=reason=>errors.push('Broker endpoint '+reason);
  if(/^[ \t]*['"]?secrets['"]?[ \t]*:[ \t]*[^#\s]/m.test(text))fail('secret forwarding requires explicit block mappings'); // MUTANT secret-inherit
  const primary=filename==='gpu-vast.yml'?jobs.find(x=>x.id==='gpu'):null;
  let remaining=text,outside=text;
  if(primary){
    const parts=primary.body.split(/(?=^      - )/m),steps=parts.slice(1);
    if(steps.length!==1||!/^        id: broker\s*$/m.test(steps[0]))fail('secrets require the sole broker step'); // MUTANT secret-step
    const header=parts[0];
    if(/\b(?:WORKER_URL|WORKER_PATH|BROKER_BASE)\b/.test(header))fail('secrets cannot enter job env or outputs');
    const output=header.split('    outputs:\n')[1]?.split(/^    \S/m)[0];
    if(output&&output.trim()!=="state: ${{ steps.broker.outputs.state }}\n      verdict: ${{ steps.broker.outputs.verdict }}\n      verified: ${{ steps.broker.outputs.verified }}")fail('job outputs must be fixed state and verdict'); // MUTANT secret-output
    let sanitised=header;
    for(const step of steps){
      if(!/^        run: \|[ \t]*$/m.test(step))fail('requires literal broker run syntax'); // MUTANT secret-run-style
      let environment=false;const found=new Set(),kept=[];
      for(const line of step.split('\n')){
        if(/^        env:\s*$/.test(line))environment=true;
        else if(/^        \S/.test(line))environment=false;
        const match=/^          (WORKER_URL|WORKER_PATH): \$\{\{ secrets\.(VAST_BROKER_URL|VAST_BROKER_PATH) \}\}$/.exec(line);
        if(environment&&steps.length===1&&/^        id: broker\s*$/m.test(step)&&match&&((match[1]==='WORKER_URL'&&match[2]==='VAST_BROKER_URL')||(match[1]==='WORKER_PATH'&&match[2]==='VAST_BROKER_PATH'))){found.add(match[1]);continue;}
        kept.push(line);
      }
      if(!['WORKER_URL','WORKER_PATH'].every(x=>found.has(x)))fail('requires two step-env secret bindings'); // MUTANT secret-binding
      if(/\b(?:WORKER_URL|WORKER_PATH|BROKER_BASE|broker_origin|broker_host|broker_base|audience)\b/.test(kept.join('\n').split(/^        run:/m)[0]))fail('aliases cannot enter step metadata'); // MUTANT secret-metadata
      const run=step.split(/^        run: \|\s*$/m)[1]||'';
      const approvedRenderer=run.split('\n').map(x=>x.startsWith('          ')?x.slice(10):x).join('\n').trim()===runnerBootstrap.trim();
      if(!approvedRenderer)fail('requires exact trusted BASE controller bootstrap');
      for(const line of run.split('\n')){
        if(/GITHUB_ENV/.test(line))fail('cannot enter environment channels');
        if(/GITHUB_STEP_SUMMARY|GITHUB_OUTPUT/.test(line)&&!(approvedRenderer&&line.trim()==='GITHUB_OUTPUT="$GITHUB_OUTPUT" GITHUB_STEP_SUMMARY="$GITHUB_STEP_SUMMARY" \\'))fail('only BASE controller may write summaries/enum outputs');
        if(/^\s*(?:echo|printf|cat|tee)\b/.test(line)&&/\b(?:WORKER_URL|WORKER_PATH|broker_origin|broker_host|broker_base|host_parts|audience)\b/.test(line))fail('cannot be printed'); // MUTANT secret-print
      }
      sanitised+=kept.join('\n');
    }
    remaining=text.replace(primary.body,sanitised);outside=text.replace(primary.body,'');
  }
  if(/VAST_BROKER_(?:URL|PATH)/.test(remaining))fail('secret reference outside approved step env'); // MUTANT secret-reference
  if(/\b(?:WORKER_URL|WORKER_PATH|BROKER_BASE)\b/.test(outside))fail('aliases outside broker step'); // MUTANT secret-alias
  for(const expression of text.match(/\$\{\{[\s\S]*?\}\}/g)||[])if(/\bsecrets\b(?!\s*\.)/.test(expression))fail('opaque secret context is unsupported'); // MUTANT secret-object
  return errors;
}
export function checkWorkflows(workflows){
  const errors=[];const gpuName='gpu-vast.yml';
  if(!workflows[gpuName])return ['GPU workflow missing'];
  const gpu=jobsIn(workflows[gpuName]);
  if(!/^  pull_request_target:/m.test(workflows[gpuName])||/^  pull_request:/m.test(workflows[gpuName])||!workflows[gpuName].includes('bash --noprofile --norc -eo pipefail {0}')||!workflows[gpuName].includes('concurrency:'))errors.push('GPU runner must use trusted base trigger, fixed shell and head concurrency');
  if(/actions\/checkout|cache@|upload-artifact|download-artifact/.test(workflows[gpuName]))errors.push('GPU runner cannot consume source/artifact/cache actions');

  if(!gpu.some(x=>x.id==='pending')||!gpu.some(x=>x.id==='report'))errors.push('GPU status writers missing'); // MUTANT writers
  const report=gpu.find(x=>x.id==='report');
  if(report&&(!report.body.includes('    if: ${{ always() }}')||!['pending','gpu'].every(id=>report.needs.includes(id))))errors.push('Final status must always evaluate both dependencies');
  const primary=gpu.find(x=>x.id==='gpu');if(!primary)errors.push('GPU poll job missing');
  if(primary&&JSON.stringify(Object.entries(primary.grants).sort())!==JSON.stringify(Object.entries({'id-token':'write'}).sort()))errors.push('GPU permissions must be exactly id-token/write');
  for(const job of gpu.filter(x=>['pending','report'].includes(x.id))){
    if(JSON.stringify(Object.entries(job.grants))!==JSON.stringify([['statuses','write']])||/X-GH-Token|WORKER_URL|id-token:/.test(job.body))errors.push('Status writer must not send its token to Worker');
  }
  for(const [filename,text] of Object.entries(workflows)){
    const jobs=jobsIn(text),map=new Map(jobs.map(j=>[j.id,j]));
    errors.push(...endpointBoundary(filename,text,jobs));
    const reaches=(job,seen=new Set())=>{if(seen.has(job.id))return false;seen.add(job.id);return job.needs.some(n=>gpu.some(g=>g.id===n)||map.has(n)&&reaches(map.get(n),seen));};
    for(const job of jobs){
      if(job.unsupported)errors.push(filename+': unsupported dynamic permission/needs in '+job.id);
      if(filename===gpuName&&job.opaque)errors.push('GPU workflow cannot call opaque reusable workflow');
      if(job.danger&&(filename===gpuName||reaches(job)))errors.push(filename+': attestation/release consumes GPU workflow');
    }
    if(filename!==gpuName&&/workflow_run:/.test(text)&&(/Vast GPU|gpu-vast|workflows:\s*\[?\s*['"]?\*/.test(text)))errors.push(filename+': downstream workflow_run GPU consumer');
  }
  if(/merge_group:|upload-artifact|download-artifact/.test(workflows[gpuName]))errors.push('GPU workflow has forbidden trigger/artifact channel');
  return errors;
}
export async function checkRepository(root){
  const folder=new URL('.github/workflows/',root),workflows={};
  for(const name of await readdir(folder))if(/\.ya?ml$/.test(name))workflows[name]=await readFile(new URL(name,folder),'utf8');
  return checkWorkflows(workflows);
}
