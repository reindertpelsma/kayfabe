// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
export class BrokerError extends Error {
  constructor(state='UNAVAILABLE',status=503){super(state);this.state=state;this.status=status;}
}
export function refuse(state='UNAVAILABLE',status=503){throw new BrokerError(state,status);}
export const numericId=v=>typeof v==='string'&&/^[1-9][0-9]{0,19}$/.test(v);
export const hexSha=v=>typeof v==='string'&&/^[0-9a-f]{40}$/.test(v);
export const IMAGE='docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16';
export const MODELS=['RTX_4070','RTX_4070_Super','RTX_4080','RTX_4090'];
export function configuration(env){
  const number=(name,fallback,max=1000)=>{const raw=env[name]??String(fallback);if(typeof raw!=='string'||!/^\d+(\.\d+)?$/.test(raw))refuse();const n=Number(raw);if(!Number.isFinite(n)||n<=0||n>max)refuse();return n;};
  if(!numericId(env.OWNER_GITHUB_ID)||!/^[-\w.]+\/[-\w.]+$/.test(env.REPOSITORY||''))refuse();
  let u;try{u=new URL(env.WORKER_URL);}catch{refuse();}
  if(u.protocol!=='https:'||u.username||u.password||u.search||u.hash||u.pathname!=='/')refuse();
  const mode=env.APPROVAL_MODE??'owner_push_or_review';
  if(!['owner_push_or_review','owner_push','review_or_author','review_only','actor_only'].includes(mode))refuse();
  if(!['true','false'].includes(env.RENT_DRAFTS??'false'))refuse();
  const list=(name,fallback)=>{let value;try{value=JSON.parse(env[name]??JSON.stringify(fallback));}catch{refuse();}if(!Array.isArray(value)||value.length>32||!value.every(x=>typeof x==='string'&&x.length>0&&x.length<=128&&/^[A-Za-z0-9_.*?/-]+$/.test(x)))refuse();return value;};
  const bases=list('ALLOWED_BASE_BRANCHES',['master']),patterns=list('PATH_SKIP_PATTERNS',['**/*.md','docs/**','traces/**','archive/**','LICENSE*','.github/ISSUE_TEMPLATE/**']);
  const cfg={bases,patterns,repo:env.REPOSITORY,owner:env.OWNER_GITHUB_ID,audience:env.WORKER_URL,mode,drafts:env.RENT_DRAFTS==='true',
    reserve:number('CREDIT_RESERVE_USD',8),price:number('MAX_PRICE_USD_HOUR',.5,2),ttl:number('TTL_MINUTES',150,150)*60000,
    dailyCount:number('DAILY_RENTALS',4,20),daily:number('DAILY_SPEND_USD',3),rolling:number('ROLLING_SPEND_USD',30),gap:number('SHA_GAP_HOURS',24,720)*3600000};
  if(!Number.isInteger(cfg.dailyCount)||!Number.isInteger(cfg.ttl/60000))refuse();
  return cfg;
}
export async function readBounded(response,limit=262144){
  if(Number(response.headers.get('content-length')||0)>limit)refuse();
  const reader=response.body?.getReader();if(!reader)return new Uint8Array();
  const chunks=[];let length=0;
  try{for(;;){const {done,value}=await reader.read();if(done)break;length+=value.byteLength;if(length>limit)refuse();chunks.push(value);}}
  finally{await reader.cancel().catch(()=>{});}
  const result=new Uint8Array(length);let at=0;for(const chunk of chunks){result.set(chunk,at);at+=chunk.byteLength;}return result;
}
export async function fetchBounded(fetcher,url,init={},limit=262144,allow404=false){
  const response=await fetcher(url,{...init,redirect:'error',signal:AbortSignal.timeout(8000)});
  if(allow404&&response.status===404)return null;
  if(!response.ok||response.redirected)refuse();
  return readBounded(response,limit);
}
export async function fetchJson(fetcher,url,init={},allow404=false){
  const bytes=await fetchBounded(fetcher,url,init,262144,allow404);if(bytes===null)return null;
  try{return JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes));}catch{refuse();}
}
export function validOffer(offer,cfg){
  return Number.isSafeInteger(offer.id)&&offer.id>0&&(offer.verified===true||offer.verified===undefined&&offer.verification==='verified')&&offer.vms_enabled===true&&
    offer.rentable===true&&offer.rented===false&&offer.num_gpus===1&&typeof offer.gpu_name==='string'&&MODELS.map(x=>x.toUpperCase()).includes(offer.gpu_name.replaceAll(' ','_').toUpperCase())&&
    typeof offer.reliability==='number'&&offer.reliability>=.99&&typeof offer.disk_space==='number'&&offer.disk_space>=120&&
    typeof offer.dph_total==='number'&&Number.isFinite(offer.dph_total)&&offer.dph_total>=0&&offer.dph_total<=cfg.price;
}

// Exclusions apply only to canonical API paths. Unknown names are always relevant.
export function skippedPath(path,patterns){
  if(typeof path!=='string'||!path||path.length>4096||!/^[A-Za-z0-9_ ./-]+$/.test(path)||path.split('/').some(x=>!x||x==='.'||x==='..')||path.startsWith('ci/vast-broker/'))return false;
  return patterns.some(pattern=>{
    let expression='^';
    for(let i=0;i<pattern.length;i++){
      const char=pattern[i];
      if(char==='*'&&pattern[i+1]==='*'){i++;if(pattern[i+1]==='/'){i++;expression+='(?:.*/)?';}else expression+='.*';}
      else if(char==='*')expression+='[^/]*';
      else if(char==='?')expression+='[^/]';
      else expression+=char==='.'?'\\.':char;
    }
    return new RegExp(expression+'$').test(path);
  });
}
export function skippedFiles(files,count,patterns){
  if(!Number.isSafeInteger(count)||count<=0||count>=3000||!Array.isArray(files)||files.length!==count||new Set(files.map(x=>x?.filename)).size!==count)return false;
  return files.every(x=>x&&['added','modified','removed','renamed','copied','changed'].includes(x.status)&&
    skippedPath(x.filename,patterns)&&(!['renamed','copied'].includes(x.status)||typeof x.previous_filename==='string')&&
    (x.previous_filename===undefined||skippedPath(x.previous_filename,patterns)));
}
