import {BrokerError,refuse,configuration,numericId,localIdentity,readBounded} from './policy.js';
import {RunTokens,constantTimeEqual,validSecret,sha256Hex} from './tokens.js';
import {readReportBody,parseReport} from './reports.js';
import {routePath,notFound,endpointSecrets} from './routes.js';
import {OidcVerifier} from './oidc.js';
import {Github} from './github.js';
import {Vast} from './vast.js';
import {Ledger} from './ledger.js';
import {onstart} from './onstart.js';
import {appendBoxText,sanitizeBoxText} from './sanitize.js';
export function createService({env,storage,fetch:fetcher=fetch,now=Date.now,random}={}){
  const ledger=new Ledger(storage),verifier=new OidcVerifier(fetcher,now),tokens=new RunTokens(env.RUN_TOKEN_SECRET),githubCache=new Map();let busy=false,localVerifier;
  const runKey=c=>c.run_id+'-'+c.run_attempt;
  const json=(data,status=200)=>Response.json(data,{status,headers:{'Cache-Control':'no-store','X-Content-Type-Options':'nosniff'}});
  const secrets=[env.VAST_API_KEY,env.RUN_TOKEN_SECRET,env.OWNER_STOP_TOKEN,...endpointSecrets(env)].filter(Boolean);
  const expose=(r,key=r.key,pr=r.pr)=>({run:key,id:r.id,sha:r.sha,pr,
    state:r.closed==='STOPPED'?'STOPPED':r.ended!==null||r.resultAt!==undefined?'COMPLETE':r.closed?'STOPPING':'RUNNING',
    verdict:r.ended!==null||r.resultAt!==undefined?r.verdict||'UNAVAILABLE':'UNAVAILABLE',
    cleanup_pending:r.ended===null,result_at:r.resultAt??null,ended_at:r.ended,started_at:r.started,
    deadline_at:r.resultAt===undefined?r.started+r.ttl:Math.min(r.started+r.ttl,r.resultAt+(r.grace??300000)),
    gpu_model:r.gpu??'unknown',cost_estimate_usd:r.price*((r.ended??now())-r.started)/3600000,log_cursor:r.logChunks??0,log_tail_sanitised:r.log||'',bytes_dropped:r.fullBytesDropped??r.bytesDropped??0,tail_bytes_dropped:r.bytesDropped||0});
  async function recover(record,vast){if(!record.id){record.id=await vast.reconcile(record);await ledger.save(record);}return record;}
  async function cleanup(record,vast){
    await recover(record,vast);await vast.destroy(record);await ledger.settle(record,now());
    console.log(JSON.stringify({event:'destroyed'}));
  }
  async function close(record,vast,reason,verdict='FAIL'){
    record.closed=reason;record.verdict||=verdict;record.liveCheckedAt=null;
    await ledger.save(record);if(record.ended===null)await cleanup(record,vast);
  }
  async function update(record,vast,github){
    if(record.ended!==null)return;
    await recover(record,vast);
    if(now()-record.started>=record.ttl){await close(record,vast,'TTL');return;} // MUTANT ttl
    if(record.closed){await cleanup(record,vast);return;}
    if(record.resultAt!==undefined){
      if(now()>=Math.min(record.started+record.ttl,record.resultAt+record.grace))await close(record,vast,'RESULT');
      return; // GitHub completion must not cancel the owner's inspection grace.
    }
    if(record.firstReportAt===undefined&&now()-record.started>=(record.provision??600000)){await close(record,vast,'PROVISION','UNAVAILABLE');return;}
    let instance;
    try{instance=await vast.inspect(record);}catch{record.closed='UNAVAILABLE';record.verdict='UNAVAILABLE';await ledger.save(record);throw Error('UNAVAILABLE');}
    if(instance===null){record.closed='GONE';record.verdict='UNAVAILABLE';await ledger.settle(record,now());return;}
    if(instance.actual_status!=='running'){
      // Pending is not proof of liveness; keep only to the bounded provisioning deadline.
      if(record.firstReportAt===undefined&&['created','loading','starting'].includes(instance.actual_status))return;
      await close(record,vast,'ENDED');return;
    }
    record.liveCheckedAt=now();await ledger.save(record);
    const current=record.pr===null?null:await github.pr(record.pr);
    if(current&&(current.head?.sha!==record.sha||current.state!=='open')){await close(record,vast,'SUPERSEDED');return;}
    const run=await github.run(record.run,true);
    if(run===null||run.status==='completed')await close(record,vast,'RUN_GONE');
  }
  async function oidc(bearer,cfg){
    const selected=cfg.testIdentity?(localVerifier??=new OidcVerifier(fetcher,now,cfg.testIdentity)):verifier;
    const c=await selected.verify(bearer,cfg.audience);
    if(c.repository!==cfg.repo||!numericId(c.actor_id)||!numericId(c.run_id)||!numericId(c.run_attempt)||!['pull_request','workflow_dispatch'].includes(c.event_name))refuse('UNAUTHORIZED',401);
    return c;
  }
  async function rate(r,cfg){
    const window=Math.floor(now()/10000);
    if(r.rateWindow!==window){r.rateWindow=window;r.rateCount=0;}
    if(r.rateCount>=cfg.reportRate)refuse('RATE_LIMITED',429); // MUTANT report-rate
    r.rateCount++;await ledger.save(r);
  }
  async function kill(request,key,bearer,cfg,vast){
    const r=await ledger.get('record:'+key);
    if(!r||r.key!==key||!await tokens.verify(bearer,r)||now()>=r.reportExpiry*1000)refuse('UNAUTHORIZED',401);
    // An ended kill receipt is idempotent but grants no live authority or provider calls.
    if(r.ended!==null){if(r.boxKilled)return json({state:'STOPPED'});refuse('UNAUTHORIZED',401);}
    if(r.closed&&!r.boxKilled)refuse('UNAUTHORIZED',401);
    await rate(r,cfg);await readBounded(request,256);
    r.boxKilled=true;await close(r,vast,'STOPPED');return json({state:'STOPPED'});
  }
  async function report(request,key,bearer,cfg,vast){
    const r=await ledger.get('record:'+key);
    if(!await tokens.verify(bearer,r)||!r||r.key!==key||r.ended!==null||r.closed||r.resultAt!==undefined)refuse('UNAUTHORIZED',401); // MUTANT report-ledger
    if(now()>=r.reportExpiry*1000||now()-r.started>=r.ttl){try{await close(r,vast,'TTL');}catch{}refuse('UNAUTHORIZED',401);}
    if(r.reportCount>=cfg.reportCount||r.reportBytes>=cfg.reportBytes){try{await close(r,vast,'LIMIT');}catch{}refuse('UNAUTHORIZED',401);} // MUTANT report-caps
    await rate(r,cfg);r.reportCount++;
    try{
      if(r.liveCheckedAt===null||r.liveCheckedAt===undefined||now()-r.liveCheckedAt>=3000){
        try{
          await recover(r,vast);const instance=await vast.inspect(r);
          if(instance&&r.firstReportAt===undefined&&['created','loading','starting'].includes(instance.actual_status))refuse('NOT_READY',409);
          if(instance===null||instance.actual_status!=='running')throw Error('NOT_LIVE'); // MUTANT report-live
          r.liveCheckedAt=now();
        }catch(error){
          if(error instanceof BrokerError&&error.state==='NOT_READY')throw error;
          try{await close(r,vast,'UNAVAILABLE','UNAVAILABLE');}catch{}
          refuse('UNAUTHORIZED',401);
        }
      }
      const raw=await readReportBody(request,r,cfg),payload=parseReport(raw);
      const digest=await sha256Hex(JSON.stringify(payload));
      if(payload.seq<=r.lastSeq){if(payload.seq===r.lastSeq&&digest===r.lastDigest)return json({state:'ACCEPTED'});refuse('REPORT_SEQUENCE',409);}
      if(payload.kind==='result'&&payload.result.nonce!==r.nonce)refuse('BAD_REPORT',400);
      if(payload.text!==undefined){
        const bytes=new TextEncoder().encode(payload.text),redactions=[...secrets,bearer,r.id,r.label,r.nonce];
        const safe=sanitizeBoxText(bytes,Math.max(0,cfg.reportBytes-(r.fullBytes??0)),redactions,{tail:false});
        await ledger.appendLog(r,safe.text);r.fullBytes=(r.fullBytes??0)+new TextEncoder().encode(safe.text).length;
        r.fullBytesDropped=(r.fullBytesDropped??0)+safe.bytesDropped;
        appendBoxText(r,bytes,redactions);
      }
      r.lastSeq=payload.seq;r.lastDigest=digest;r.firstReportAt??=now();
      if(payload.kind==='result'){r.resultAt=now();r.verdict=payload.result.verdict;r.liveCheckedAt=null;} // MUTANT structured-verdict
      else if(r.reportCount>=cfg.reportCount||r.reportBytes>=cfg.reportBytes)await close(r,vast,'LIMIT');
      return json({state:'ACCEPTED',...(payload.kind==='result'?{grace_remaining_seconds:Math.max(0,Math.floor((Math.min(r.started+r.ttl,r.resultAt+r.grace)-now())/1000))}:{})});
    }catch(error){
      if(r.reportBytes>=cfg.reportBytes&&!r.closed){try{await close(r,vast,'LIMIT');}catch{}}
      throw error;
    }finally{await ledger.save(r);}
  }
  async function perform(request){
    const url=new URL(request.url),path=await routePath(request,env);
    if(path===null)return notFound();
    if(path==='/rent'&&env.RENTING_ENABLED!=='true')return json({state:'RENTING_DISABLED',verdict:'UNAVAILABLE',log_tail_sanitised:'',bytes_dropped:0},503);
    const cfg=configuration(env);
    cfg.testIdentity=localIdentity(env,request.url);
    if(cfg.testIdentity)cfg.githubOrigin=cfg.testIdentity.githubOrigin;
    if(url.search&&!/^\/logs\//.test(path))refuse('BAD_REQUEST',400);
    const match=/^\/(report|kill|status|stop|logs)\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})$/.exec(path);
    if(!(path==='/rent'&&request.method==='POST')&&!(path==='/stop-all'&&request.method==='POST')&&!(match&&request.method===(['status','logs'].includes(match[1])?'GET':'POST')))return notFound();
    const authorization=request.headers.get('Authorization');if(!authorization?.startsWith('Bearer '))refuse('UNAUTHORIZED',401);
    const bearer=authorization.slice(7),vast=new Vast(fetcher,cfg,env.VAST_API_KEY);
    if(match?.[1]==='report')return report(request,match[2],bearer,cfg,vast);
    if(match?.[1]==='kill')return kill(request,match[2],bearer,cfg,vast);
    // workerd exposes even an empty HTTP POST as a stream; forbid bytes, not streams.
    if(request.body!==null){try{await readBounded(request,0);}catch{refuse('BAD_REQUEST',400);}}
    const owner=validSecret(env.OWNER_STOP_TOKEN)&&await constantTimeEqual(env.OWNER_STOP_TOKEN,bearer);
    if(path==='/stop-all'){
      if(!owner)refuse('UNAUTHORIZED',401); // MUTANT stop-auth
      const records=await ledger.openRecords();
      for(const r of records){r.closed='STOPPED';r.verdict||='FAIL';r.liveCheckedAt=null;await ledger.save(r);}
      let failed=false;for(const r of records){try{await cleanup(r,vast);}catch{failed=true;}}
      if(failed)refuse();return json({state:'STOPPED'});
    }
    if(match?.[1]==='stop'&&owner){
      const r=await ledger.granted(match[2])||await ledger.get('record:'+match[2]);if(!r)refuse('NOT_FOUND',404);
      await close(r,vast,'STOPPED');return json(expose(r,match[2]));
    }
    const c=await oidc(bearer,cfg),key=runKey(c);
    const token=request.headers.get('X-GH-Token');
    if(token&&(token.length>1024||!/^[\x21-\x7e]+$/.test(token)))refuse();
    const github=new Github(fetcher,cfg,token,{cache:githubCache,now});
    if(match){
      if(match[2]!==key)refuse('UNAUTHORIZED',401); // MUTANT stop-run
      const r=await ledger.granted(key);if(!r)refuse('NOT_FOUND',404);
      if(match[1]==='logs'){
        if(!/^\?cursor=(0|[1-9][0-9]{0,6})$/.test(url.search))refuse('BAD_REQUEST',400);
        const cursor=Number(url.searchParams.get('cursor'));if(cursor>(r.logChunks??0))refuse('BAD_REQUEST',400);
        return json(await ledger.logPage(r,cursor));
      }
      if(match[1]==='stop')await close(r,vast,'STOPPED');else await update(r,vast,github);
      const admittedPr=await ledger.get('run-pr:'+key);
      return json(expose(r,key,admittedPr===undefined?r.pr:admittedPr));
    }
    const target=await github.authorize(c);
    if(target.notNeeded)return json({state:'NOT_NEEDED',verdict:'PASS',run:key,sha:target.sha,pr:target.pr,log_tail_sanitised:'',bytes_dropped:0});
    const prior=await ledger.granted(key);
    if(prior){await recover(prior,vast);return json(expose(prior,key,target.pr));}
    const active=await ledger.active();
    if(active&&active.ended===null){
      if((active.pr!==null&&active.pr===target.pr||active.pr===null&&target.pr===null&&active.branch===target.branch)&&active.sha!==target.sha){await close(active,vast,'SUPERSEDED');refuse('BUSY',409);}
      refuse('UNAVAILABLE_BUDGET',409);
    }
    const cached=await ledger.get('sha:'+target.sha),force=c.event_name==='workflow_dispatch'&&Number(c.run_attempt)>1;
    if(cached&&!force){const r=await ledger.get('record:'+cached.key);if(r?.verdict&&r.ended!==null){await ledger.grant(key,r,target.pr);return json(expose(r,key,target.pr));}refuse();}
    if(cached&&now()-cached.started<cfg.gap)refuse('UNAVAILABLE_BUDGET',409); // MUTANT gap
    if(!validSecret(env.RUN_TOKEN_SECRET)||!validSecret(env.OWNER_STOP_TOKEN))refuse();
    const offer=await vast.offer(),cost=offer.dph_total*(cfg.ttl+cfg.cronPeriod)/3600000,credit=await vast.credit();
    if(typeof credit!=='number'||!Number.isFinite(credit)||credit<cfg.reserve+cost)refuse('UNAVAILABLE_BUDGET',409); // MUTANT credit
    await ledger.budgets(cfg,now(),cost);
    const n=random?random():Array.from(crypto.getRandomValues(new Uint8Array(16)),x=>x.toString(16).padStart(2,'0')).join('');
    const started=now(),r={...target,key,label:'kf-ci-'+key,nonce:n,started,ended:null,ttl:cfg.ttl,billingTtl:cfg.ttl+cfg.cronPeriod,provision:cfg.provision,grace:cfg.grace,gpu:offer.gpu_name,reportExpiry:Math.floor((started+cfg.ttl+cfg.cronPeriod)/1000),
      price:offer.dph_total,id:null,closed:null,verdict:null,log:'',bytesDropped:0,reportCount:0,reportBytes:0,lastSeq:0,lastDigest:null,rateWindow:0,rateCount:0,liveCheckedAt:null};
    const runToken=await tokens.issue(r),script=onstart(target.sha,n,cfg.ttl/60000,cfg.repo,{url:cfg.audience,run:key,token:runToken,path:env.BROKER_PATH_PREFIX??'',graceMinutes:cfg.grace/60000});
    r.scriptHash=await sha256Hex(script);
    await ledger.reserve(r);
    r.id=await vast.create(offer,r,script);await ledger.save(r);
    console.log(JSON.stringify({event:'rented'}));return json(expose(r));
  }
  const releaseUpload=async request=>{if(request.body&&!request.body.locked)await request.body.cancel().catch(()=>{});};
  return {
    async fetch(request){
      if(busy){await releaseUpload(request);return json({state:'BUSY',verdict:'UNAVAILABLE',log_tail_sanitised:'',bytes_dropped:0},409);}
      busy=true;try{return await perform(request);}catch(e){return json({state:e instanceof BrokerError?e.state:'UNAVAILABLE',verdict:'UNAVAILABLE',log_tail_sanitised:'',bytes_dropped:0},e instanceof BrokerError?e.status:503);}finally{await releaseUpload(request);busy=false;}
    },
    async sweep(){
      if(busy)return;busy=true;
      try{const cfg=configuration(env),records=await ledger.openRecords();for(const r of records){try{await update(r,new Vast(fetcher,cfg,env.VAST_API_KEY),new Github(fetcher,cfg,null,{cache:githubCache,now}));}catch{console.log(JSON.stringify({event:'cleanup_unavailable'}));}}}
      catch{console.log(JSON.stringify({event:'cleanup_unavailable'}));}finally{busy=false;}
    }
  };
}
export class BrokerLedger {
  constructor(ctx,env){this.service=createService({env,storage:ctx.storage});}
  async fetch(request){if(new URL(request.url).origin==='https://internal.invalid'&&new URL(request.url).pathname==='/cron'){await this.service.sweep();return new Response('ok');}return this.service.fetch(request);}
}
// Buffer only the admitted prefix before entering the serialized ledger. An
// unfinished network upload must never hold its busy flag or block cron.
async function detachedUpload(request,path){
  if(!request.body)return request;
  const limit=(/^\/report\//.test(path)?65536:/^\/kill\//.test(path)?256:0)+1;
  const reader=request.body.getReader(),chunks=[];let length=0;
  try{while(length<limit){const {value,done}=await reader.read();if(done)break;const kept=value.slice(0,limit-length);chunks.push(kept);length+=kept.byteLength;}}
  finally{await reader.cancel().catch(()=>{});}
  const bytes=new Uint8Array(length);let at=0;for(const chunk of chunks){bytes.set(chunk,at);at+=chunk.byteLength;}
  const headers=new Headers(request.headers);headers.delete('Transfer-Encoding');headers.set('Content-Length',String(length));
  return new Request(request.url,{method:request.method,headers,body:bytes});
}
export default {
  async fetch(request,env){const path=await routePath(request,env);if(path===null||!/^\/(?:rent|stop-all|(?:report|kill|status|stop|logs)\/[1-9][0-9]{0,19}-[1-9][0-9]{0,19})$/.test(path))return notFound();let buffered;try{buffered=await detachedUpload(request,path);}catch{return Response.json({state:'UNAVAILABLE',verdict:'UNAVAILABLE',log_tail_sanitised:'',bytes_dropped:0},{status:503,headers:{'Cache-Control':'no-store','X-Content-Type-Options':'nosniff'}});}
    return env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account')).fetch(buffered);},
  scheduled(_event,env,ctx){ctx.waitUntil(env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account')).fetch(new Request('https://internal.invalid/cron',{method:'POST'})));}
};
