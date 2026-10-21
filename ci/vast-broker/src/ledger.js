import {refuse} from './policy.js';
export class Ledger {
  constructor(storage){this.storage=storage;}
  get(key){return this.storage.get(key);}
  put(key,value){return this.storage.put(key,value);}
  async save(record){await this.put('record:'+record.key,record);if(record.id)await this.put('id:'+record.id,record.key);}
  async active(){const key=await this.get('active');return key?this.get('record:'+key):null;}
  async granted(run){const key=await this.get('run:'+run);return key?this.get('record:'+key):null;}
  async grant(run,record,pr=record.pr){await this.put('run:'+run,record.key);await this.put('run-pr:'+run,pr);}
  async owned(id,run){
    const record=await this.granted(run);
    if(!record||record.id!==id)refuse('NOT_FOUND',404);
    return record;
  }
  async reserve(record){
    await this.save(record);await this.put('active',record.key);await this.grant(record.key,record);
    await this.put('sha:'+record.sha,{key:record.key,started:record.started});
    await this.put('cost:'+record.key,{started:record.started,ended:null,price:record.price,ttl:record.ttl});
  }
  async settle(record,now){
    record.ended=now;await this.save(record);await this.put('cost:'+record.key,{started:record.started,ended:now,price:record.price,ttl:record.ttl});
    if(await this.get('active')===record.key)await this.put('active',null);
  }
  async budgets(cfg,now,cost){
    const entries=await this.storage.list({prefix:'cost:',limit:1000});if(entries.size>=1000)refuse('UNAVAILABLE_BUDGET',409);
    const day=Math.floor(now/86400000)*86400000,rolling=now-30*86400000;let daily=0,monthly=0,count=0;
    for(const [key,v] of entries){
      if(!Number.isFinite(v.started)||!Number.isFinite(v.price)||v.price<0||v.started>now||v.ended!==null&&!Number.isFinite(v.ended))refuse();
      const end=v.ended??now;
      if(end<v.started)refuse();
      const remaining=v.ended===null?Math.max(0,v.started+v.ttl-now)*v.price/3600000:0;
      daily+=Math.max(0,end-Math.max(v.started,day))*v.price/3600000+remaining;
      monthly+=Math.max(0,end-Math.max(v.started,rolling))*v.price/3600000+remaining;
      if(v.started>=day)count++;
      if(v.ended!==null&&end<rolling)await this.storage.delete(key);
    }
    if(count>=cfg.dailyCount)refuse('UNAVAILABLE_BUDGET',409); // MUTANT daily-count
    if(daily+cost>cfg.daily)refuse('UNAVAILABLE_BUDGET',409); // MUTANT daily
    if(monthly+cost>cfg.rolling)refuse('UNAVAILABLE_BUDGET',409); // MUTANT rolling
  }
}
