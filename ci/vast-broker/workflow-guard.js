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
export function checkWorkflows(workflows){
  const errors=[];const gpuName='gpu-vast.yml';
  if(!workflows[gpuName])return ['GPU workflow missing'];
  const gpu=jobsIn(workflows[gpuName]);
  if(!gpu.some(x=>x.id==='pending')||!gpu.some(x=>x.id==='report'))errors.push('GPU status writers missing'); // MUTANT writers
  const report=gpu.find(x=>x.id==='report');
  if(report&&(!report.body.includes('    if: ${{ always() }}')||!['pending','gpu'].every(id=>report.needs.includes(id))))errors.push('Final status must always evaluate both dependencies');
  const primary=gpu.find(x=>x.id==='gpu');if(!primary)errors.push('GPU poll job missing');
  if(primary&&JSON.stringify(Object.entries(primary.grants).sort())!==JSON.stringify(Object.entries({contents:'read','pull-requests':'read','id-token':'write'}).sort()))errors.push('GPU permissions must be exactly contents/read, pull-requests/read, id-token/write');
  for(const job of gpu.filter(x=>['pending','report'].includes(x.id))){
    if(JSON.stringify(Object.entries(job.grants))!==JSON.stringify([['statuses','write']])||/X-GH-Token|WORKER_URL|id-token:/.test(job.body))errors.push('Status writer must not send its token to Worker');
  }
  for(const [filename,text] of Object.entries(workflows)){
    const jobs=jobsIn(text),map=new Map(jobs.map(j=>[j.id,j]));
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
