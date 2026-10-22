// Local-only native workerd adapter. Never deploy this entrypoint.
import worker,{createService} from './src/index.js';
import {Vast} from './src/vast.js';
import {configuration} from './src/policy.js';
export class BrokerLedger {
 constructor(ctx,env){
  this.storage=ctx.storage;this.env=env;this.offset=0;this.mock=env.NATIVE_MOCK==='true';
  const fetcher=async(url,init={})=>{
   const u=new URL(url);if(u.hostname!=='console.vast.ai')return fetch(url,init);
   if(!this.mock){if(init.method&&init.method!=='GET'&&!u.pathname.includes('/bundles/'))throw Error('READ_ONLY');return fetch(url,init);}
   const path=u.pathname.replace(/\/$/,'');let value;
   if(path==='/api/v0/bundles')value={offers:[{id:5,verification:'verified',vms_enabled:true,rentable:true,rented:false,num_gpus:1,gpu_name:'RTX 4070',reliability:.999,disk_space:200,dph_total:.1}]};
   else if(path==='/api/v0/users/current')value={credit:39.5,balance:0};
   else if(path==='/api/v0/asks/5'){
    const b=JSON.parse(init.body);await this.storage.put('mock-instance',{id:900,label:b.label,actual_status:'created',onstart:b.onstart,image_uuid:b.image});value={success:true,new_contract:900};
   }else if(path==='/api/v0/instances/900'){
    if(init.method==='DELETE'){await this.storage.delete('mock-instance');value={success:true};}
    else value={instances:await this.storage.get('mock-instance')??null};
   }else throw Error('Unexpected native mock endpoint');
   return Response.json(value);
  };
  this.fetcher=fetcher;
  this.service=createService({env,storage:ctx.storage,fetch:fetcher,now:()=>Date.now()+this.offset});
 }
 async fetch(request){
  const u=new URL(request.url);
  if(u.origin==='https://internal.invalid'&&u.pathname==='/cron'){await this.service.sweep();return new Response('ok');}
  if(u.hostname==='127.0.0.1'&&u.pathname==='/__v4/control'){
   if(request.method!=='POST'||request.headers.get('Authorization')!=='Bearer '+this.env.OWNER_STOP_TOKEN)return new Response('no',{status:401});
   const p=await request.json();let result={ok:true};
   if(!this.mock){
    if(p.action!=='preflight')return new Response('no',{status:403});
    let count=0;const tracked=async(url,init)=>{const r=await this.fetcher(url,init);if(new URL(url).pathname.includes('/bundles/'))count=(await r.clone().json()).offers?.length;return r;};
    const vast=new Vast(tracked,configuration(this.env),this.env.VAST_API_KEY),credit=await vast.credit();
    try{const o=await vast.offer();result={state:'BLOCKED_LAUNCH_CONTRACT',eligible_offer_found:true,hourly_price:o.dph_total};}catch{result={state:'UNAVAILABLE',eligible_offer_found:false};}
    return Response.json({...result,offers_returned:count,prepaid_reserve_met:typeof credit==='number'&&Number.isFinite(credit)&&credit>=5.175,rentals_created:0});
   }
   if(p.action==='reset'){this.offset=0;}
   else if(p.action==='advance'){if(!Number.isSafeInteger(p.offset)||p.offset<0||p.offset>3600000)throw Error('offset');this.offset=p.offset;}
   else if(p.action==='running'){const i=await this.storage.get('mock-instance');i.actual_status='running';await this.storage.put('mock-instance',i);}
   else if(p.action==='cron')await this.service.sweep();
   else if(p.action==='capability'){
    const r=await this.storage.get('record:'+p.run),i=await this.storage.get('mock-instance');
    result={token:i?.onstart.match(/^token='([^']+)'/m)?.[1],nonce:r?.nonce,present:!!i,settled:r?.ended!==null,report_count:r?.reportCount,rate_count:r?.rateCount};
   }else return new Response('no',{status:404});
   return Response.json(result);
  }
  return this.service.fetch(request);
 }
}
export default {...worker,fetch(request,env,ctx){if(new URL(request.url).pathname==='/__v4/control')return env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account')).fetch(request);return worker.fetch(request,env,ctx);}};
