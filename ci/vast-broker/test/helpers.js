import { webcrypto,createHmac } from 'node:crypto';
export const clock = 1800000000000;
export const sha = 'a'.repeat(40);
export const nonce = 'b'.repeat(32);
export const owner = '42';
export const repo = 'reindertpelsma/kayfabe';
export const env = {RENTING_ENABLED:'true',OWNER_GITHUB_ID:owner,REPOSITORY:repo,
  WORKER_URL:'https://broker.example',APPROVAL_MODE:'owner_push_or_review',RENT_DRAFTS:'false',
  CREDIT_RESERVE_USD:'8',MAX_PRICE_USD_HOUR:'0.5',TTL_MINUTES:'20',DAILY_RENTALS:'4',
  DAILY_SPEND_USD:'3',ROLLING_SPEND_USD:'30',SHA_GAP_HOURS:'24',
  RUN_TOKEN_SECRET:'mock-run-token-binding-only-for-unit-tests',OWNER_STOP_TOKEN:'mock-owner-stop-binding-only-for-unit-tests',
  VAST_API_KEY:'mock-vast-binding'};
const pair = await webcrypto.subtle.generateKey({name:'RSASSA-PKCS1-v1_5',modulusLength:2048,
  publicExponent:new Uint8Array([1,0,1]),hash:'SHA-256'},true,['sign','verify']);
export const jwk = {...await webcrypto.subtle.exportKey('jwk',pair.publicKey),kid:'test-key',alg:'RS256',use:'sig'};
export const baseClaims = {iss:'https://token.actions.githubusercontent.com',aud:env.WORKER_URL,
  exp:clock/1000+300,nbf:clock/1000-1,repository:repo,event_name:'pull_request',actor_id:owner,
  sha:'d'.repeat(40),ref:'refs/pull/7/merge',run_id:'123',run_attempt:'1'};
export async function jwt(patch={},header={},key=pair.privateKey) {
  const enc = v=>Buffer.from(JSON.stringify(v)).toString('base64url');
  const input=enc({alg:'RS256',kid:'test-key',...header})+'.'+enc({...baseClaims,...patch});
  const sig=await webcrypto.subtle.sign('RSASSA-PKCS1-v1_5',key,new TextEncoder().encode(input));
  return input+'.'+Buffer.from(sig).toString('base64url');
}
export class MemoryStorage {
  data=new Map();
  async get(key) { return structuredClone(this.data.get(key)); }
  async put(key,value) { this.data.set(key,structuredClone(value)); }
  async delete(key) { return this.data.delete(key); }
  async list({prefix='',limit=1000}={}) {
    return new Map([...this.data].filter(([k])=>k.startsWith(prefix)).sort(([a],[b])=>a.localeCompare(b)).slice(0,limit).map(([k,v])=>[k,structuredClone(v)]));
  }
}
export class MockCloud {
  credit=20; price=.2; creates=0; deletes=[]; inspected=[]; calls=[]; ambiguous=false;
  githubError=0; vastError=0; logReady=true; logText='building\n'; logStatus=200; logHeaders={};
  run={id:123,run_attempt:1,event:'pull_request',head_sha:sha,head_branch:'owner-pr',
    pull_requests:[{number:7,head:{sha}}],actor:{id:42},triggering_actor:{id:42},status:'in_progress',repository:{full_name:repo}};
  prs=[{number:7,state:'open',draft:false,user:{id:42},head:{sha,ref:'owner-pr',repo:{full_name:repo}},base:{ref:'master',repo:{full_name:repo}},changed_files:1}];
  reviews=[]; reviewPages=null; files=[{filename:'crates/kf-core/src/lib.rs',status:'modified'}]; filePages=null; filesError=0;
  offer={id:5,verified:true,vms_enabled:true,rentable:true,rented:false,num_gpus:1,
    gpu_name:'RTX_4070',reliability:.999,disk_space:200};
  instances=new Map([['777',{id:777,label:'owner-dev'}],['888',{id:888,label:'kf-ci-unrecorded'}]]);
  fetch=async (input,init={})=>{
    const u=new URL(String(input));const method=init.method||'GET';const body=init.body?JSON.parse(init.body):undefined;
    this.calls.push({url:u.href,method,body,headers:new Headers(init.headers)});
    u.pathname=u.pathname.replace(/\/$/,''); // Mock routing tolerates canonical provider slash.
    if(u.hostname==='token.actions.githubusercontent.com')return Response.json({keys:[jwk]});
    if(u.hostname==='api.github.com'){
      if(this.githubError)return new Response('PRIVATE-UPSTREAM', {status:this.githubError});
      if(/\/actions\/runs\/\d+$/.test(u.pathname))return Response.json(this.run);
      if(/\/commits\/[a-f0-9]+$/.test(u.pathname))return Response.json({sha:this.run.head_sha});
      if(/\/pulls\/\d+\/files$/.test(u.pathname)){if(this.filesError)return new Response('PRIVATE-UPSTREAM',{status:this.filesError});return Response.json(this.filePages?.[Number(u.searchParams.get('page')||1)-1]??this.files);}
      if(/\/commits\/[a-f0-9]+\/pulls$/.test(u.pathname))return Response.json(this.prs);
      if(/\/pulls\/[0-9]+$/.test(u.pathname))return Response.json(this.prs.find(p=>String(p.number)===u.pathname.split('/').at(-1)));
      if(/\/pulls\/\d+\/reviews$/.test(u.pathname))return Response.json(this.reviewPages?.[Number(u.searchParams.get('page')||1)-1]||this.reviews);
      throw Error('Unexpected GitHub endpoint '+u.pathname);
    }
    if(u.hostname==='s3.amazonaws.com'){
      if(!this.logReady)return new Response('',{status:404});
      return new Response(this.logText,{status:this.logStatus,headers:this.logHeaders});
    }
    if(u.hostname!=='console.vast.ai')throw Error('Unapproved external host');
    if(this.vastError)return new Response('PRIVATE-UPSTREAM',{status:this.vastError});
    if(u.pathname==='/api/v0/users/current')return Response.json({credit:this.credit,balance:0});
    if(u.pathname==='/api/v0/bundles'){if(body.type!=='ondemand')return new Response('invalid type',{status:400});return Response.json({offers:[{...this.offer,dph_total:this.price}]});}
    if(u.pathname==='/api/v0/asks/5'){
      this.creates++;
      this.instances.set('900',{id:900,label:body.label,onstart:body.onstart,image_uuid:body.image,actual_status:'running'});
      if(this.ambiguous)throw Error('PRIVATE-UPSTREAM');
      return Response.json({success:true,new_contract:900,instance_api_key:'PRIVATE-UPSTREAM'});
    }
    if(u.pathname==='/api/v1/instances')return Response.json({success:true,instances:[...this.instances.values()],next_token:null});
    const logs=u.pathname.match(/^\/api\/v0\/instances\/request_logs\/(\d+)$/);
    if(logs)return Response.json({success:true,result_url:this.logUrl||'https://s3.amazonaws.com/vast.ai/instance_logs/mock.log'});
    const m=u.pathname.match(/^\/api\/v0\/instances\/(\d+)$/);
    if(!m)throw Error('Unexpected instance endpoint '+u.pathname);
    if(method==='DELETE'){this.deletes.push(m[1]);this.instances.delete(m[1]);return Response.json({success:true});}
    this.inspected.push(m[1]);
    return this.instances.has(m[1])?Response.json({instances:this.instances.get(m[1])}):new Response('',{status:404});
  };
}
export function callbackToken(record,secret=env.RUN_TOKEN_SECRET,patch={}) {
 const r={...record,...patch},expiry=r.reportExpiry??Math.floor((r.started+r.ttl)/1000);
 const message=['v1',r.run,r.attempt,r.sha,r.label,expiry].join('|');
 return ['v1',r.run,r.attempt,expiry,createHmac('sha256',secret).update(message).digest('base64url')].join('.');
}
export async function setup(overrides={}) {
  const {createService}=await import('../src/index.js');
  const cloud=new MockCloud();const storage=new MemoryStorage();const time={value:clock};
  const bindings={...env,...overrides};
  let service=createService({env:bindings,storage,fetch:cloud.fetch,now:()=>time.value,random:()=>nonce});
  const harness={cloud,storage,time,bindings,get service(){return service;},
    restart(){service=createService({env:bindings,storage,fetch:cloud.fetch,now:()=>time.value,random:()=>nonce});},
    async report(payload,{run='123-1',token,raw,headers={}}={}){
      const record=await storage.get('record:'+run);
      const bearer=token??callbackToken(record,bindings.RUN_TOKEN_SECRET);
      return service.fetch(new Request(bindings.WORKER_URL+'/report/'+run,{method:'POST',headers:{Authorization:'Bearer '+bearer,'Content-Type':'application/json',...headers},body:raw??JSON.stringify(payload)}));
    },
    async call(path='/rent',{claims={},body,method=path.startsWith('/status')?'GET':'POST',token,ghToken='mock-job-token'}={}){
      const authorization=token??await jwt({exp:time.value/1000+300,nbf:time.value/1000-1,...claims});
      return service.fetch(new Request(bindings.WORKER_URL+path,{method,headers:{Authorization:'Bearer '+authorization,...(ghToken?{'X-GH-Token':ghToken}:{})},...(body===undefined?{}:{body:JSON.stringify(body)})}));
    }};
  return harness;
}
export const finalLine=(verdict='PASS',n=nonce)=>`KF_RESULT ${n} ${verdict} ${'c'.repeat(64)}\n`;
