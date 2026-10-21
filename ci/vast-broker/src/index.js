import {BrokerError,refuse,configuration,numericId} from './policy.js';
import {OidcVerifier} from './oidc.js';
import {Github} from './github.js';
import {Vast} from './vast.js';
import {Ledger} from './ledger.js';
import {onstart} from './onstart.js';
import {sanitizeLog,parseResult} from './sanitize.js';
export function createService({env,storage,fetch:fetcher=fetch,now=Date.now,random}={}){
  const ledger=new Ledger(storage),verifier=new OidcVerifier(fetcher,now);let busy=false;const githubCache=new Map();
  const runKey=c=>c.run_id+'-'+c.run_attempt;
  const json=(data,status=200)=>Response.json(data,{status,headers:{'Cache-Control':'no-store','X-Content-Type-Options':'nosniff'}});
  const expose=r=>({id:r.id,sha:r.sha,pr:r.pr,state:r.verdict?'COMPLETE':'RUNNING',verdict:r.verdict||'UNAVAILABLE',log:r.log||''});
  const secrets=[env.VAST_API_KEY].filter(Boolean);
  async function recover(record,vast){
    if(!record.id){record.id=await vast.reconcile(record);await ledger.save(record);}return record;
  }
  async function cleanup(record,vast){
    await recover(record,vast);await vast.destroy(record);await ledger.settle(record,now());
    console.log(JSON.stringify({event:'destroyed',id:record.id}));
  }
  async function update(record,vast,github,requestSecrets=[]){
    if(record.ended!==null)return;
    await recover(record,vast);
    if(now()-record.started>=record.ttl){record.verdict||='FAIL';record.log||='[box] rental TTL reached\n';await ledger.save(record);await cleanup(record,vast);return;}
    if(record.verdict){await cleanup(record,vast);return;}
    const current=record.pr===null?null:await github.pr(record.pr);
    if(current&&(current.head?.sha!==record.sha||current.state!=='open')){
      record.verdict='FAIL';record.log='[box] PR head superseded or PR closed\n';await ledger.save(record);await cleanup(record,vast);return;
    }
    const instance=await vast.inspect(record);
    if(instance===null){record.verdict='UNAVAILABLE';await ledger.settle(record,now());return;}
    try{
      const bytes=await vast.logs(record);
      if(bytes!==null){record.log=sanitizeLog(bytes,8192,[...secrets,...requestSecrets]);record.verdict=parseResult(bytes,record.nonce);}
    }finally{await ledger.save(record);}
    if(record.verdict){await cleanup(record,vast);return;}
    if(['exited','stopped','offline','destroyed','error'].includes(instance.actual_status)){
      record.verdict='FAIL';await ledger.save(record);await cleanup(record,vast);return;
    }
    const run=await github.run(record.run,true);
    if(run===null||run.status==='completed'){record.verdict='FAIL';record.log||='[box] workflow run is gone\n';await ledger.save(record);await cleanup(record,vast);}
  }
  async function perform(request){
    const url=new URL(request.url);
    if(url.pathname==='/rent'&&env.RENTING_ENABLED!=='true')return json({state:'RENTING_DISABLED',verdict:'UNAVAILABLE',log:''},503);
    const cfg=configuration(env);
    if(url.search)refuse('BAD_REQUEST',400);
    const match=/^\/(status|destroy)\/([1-9][0-9]*)$/.exec(url.pathname);
    if(!(url.pathname==='/rent'&&request.method==='POST')&&!(match&&request.method===(match[1]==='status'?'GET':'POST')))refuse('NOT_FOUND',404);
    if(request.body!==null)refuse('BAD_REQUEST',400);
    const auth=request.headers.get('Authorization');if(!auth?.startsWith('Bearer '))refuse('UNAUTHORIZED',401);
    const c=await verifier.verify(auth.slice(7),cfg.audience);
    if(c.repository!==cfg.repo||!numericId(c.actor_id)||!numericId(c.run_id)||!numericId(c.run_attempt)||!['pull_request','workflow_dispatch'].includes(c.event_name))refuse('UNAUTHORIZED',401);
    const token=request.headers.get('X-GH-Token');
    if(token&&(token.length>1024||!/^[\x21-\x7e]+$/.test(token)))refuse();
    const key=runKey(c),vast=new Vast(fetcher,cfg,env.VAST_API_KEY),github=new Github(fetcher,cfg,token,{cache:githubCache,now});
    if(match){
      const record=await ledger.owned(match[2],key);
      if(match[1]==='destroy'){record.verdict||='FAIL';await ledger.save(record);if(record.ended===null)await cleanup(record,vast);}
      else await update(record,vast,github,token?[token]:[]);
      const admittedPr=await ledger.get('run-pr:'+key);
      return json({...expose(record),pr:admittedPr===undefined?record.pr:admittedPr});
    }
    const target=await github.authorize(c);
    if(target.notNeeded)return json({state:'NOT_NEEDED',verdict:'PASS',sha:target.sha,pr:target.pr,log:''});
    const prior=await ledger.granted(key);
    if(prior){await recover(prior,vast);return json({...expose(prior),pr:target.pr});}
    const active=await ledger.active();
    if(active&&active.ended===null){
      if((active.pr!==null&&active.pr===target.pr||active.pr===null&&target.pr===null&&active.branch===target.branch)&&active.sha!==target.sha){active.verdict='FAIL';active.log='[box] superseded by current PR head\n';await ledger.save(active);await cleanup(active,vast);refuse('BUSY',409);}
      refuse('UNAVAILABLE_BUDGET',409);
    }
    const cached=await ledger.get('sha:'+target.sha);
    const force=c.event_name==='workflow_dispatch'&&Number(c.run_attempt)>1;
    if(cached&&!force){const record=await ledger.get('record:'+cached.key);if(record?.verdict&&record.ended!==null){await ledger.grant(key,record,target.pr);return json({...expose(record),pr:target.pr});}refuse();}
    if(cached&&now()-cached.started<cfg.gap)refuse('UNAVAILABLE_BUDGET',409); // MUTANT gap
    const offer=await vast.offer();const cost=offer.dph_total*cfg.ttl/3600000;
    const balance=await vast.balance();
    if(typeof balance!=='number'||!Number.isFinite(balance)||balance<cfg.reserve+cost)refuse('UNAVAILABLE_BUDGET',409); // MUTANT credit
    await ledger.budgets(cfg,now(),cost);
    const n=random?random():Array.from(crypto.getRandomValues(new Uint8Array(16)),x=>x.toString(16).padStart(2,'0')).join('');
    const record={...target,key,label:'kf-ci-'+key,nonce:n,script:onstart(target.sha,n,cfg.ttl/60000,cfg.repo),
      started:now(),ended:null,ttl:cfg.ttl,price:offer.dph_total,id:null,verdict:null,log:'',logUrl:null};
    await ledger.reserve(record);
    record.id=await vast.create(offer,record);await ledger.save(record);
    console.log(JSON.stringify({event:'rented',id:record.id}));return json(expose(record));
  }
  return {
    async fetch(request){
      if(busy)return json({state:'BUSY',verdict:'UNAVAILABLE',log:''},409);
      busy=true;try{return await perform(request);}catch(e){return json({state:e instanceof BrokerError?e.state:'UNAVAILABLE',verdict:'UNAVAILABLE',log:''},e instanceof BrokerError?e.status:503);}finally{busy=false;}
    },
    async sweep(){
      if(busy)return;busy=true;
      try{const cfg=configuration(env),active=await ledger.active();if(active&&active.ended===null)await update(active,new Vast(fetcher,cfg,env.VAST_API_KEY),new Github(fetcher,cfg,null,{cache:githubCache,now}));}
      catch{console.log(JSON.stringify({event:'cleanup_unavailable'}));}finally{busy=false;}
    }
  };
}
export class BrokerLedger {
  constructor(ctx,env){this.service=createService({env,storage:ctx.storage});}
  async fetch(request){
    if(new URL(request.url).origin==='https://internal.invalid'&&new URL(request.url).pathname==='/cron'){await this.service.sweep();return new Response('ok');}
    return this.service.fetch(request);
  }
}
export default {
  fetch(request,env){if(!/^\/(rent|status\/[1-9][0-9]*|destroy\/[1-9][0-9]*)$/.test(new URL(request.url).pathname))return new Response('Not found',{status:404});return env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account')).fetch(request);},
  scheduled(_event,env,ctx){ctx.waitUntil(env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account')).fetch(new Request('https://internal.invalid/cron',{method:'POST'})));}
};
