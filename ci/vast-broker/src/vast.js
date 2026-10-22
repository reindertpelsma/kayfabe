import {fetchJson,refuse,IMAGE,MODELS,validOffer} from './policy.js';
import {sha256Hex} from './tokens.js';
export class Vast {
  constructor(fetcher,cfg,key){this.fetcher=fetcher;this.cfg=cfg;this.headers={Authorization:'Bearer '+key,'Content-Type':'application/json'};if(!key)refuse();}
  async api(path,method='GET',body,missing=false){return fetchJson(this.fetcher,'https://console.vast.ai'+path,{method,headers:this.headers,...(body===undefined?{}:{body:JSON.stringify(body)})},missing);}
  async offer(){
    const q={limit:20,type:'ondemand',verified:{eq:true},vms_enabled:{eq:true},rentable:{eq:true},rented:{eq:false},
      num_gpus:{eq:1},gpu_name:{in:MODELS.map(x=>x.replaceAll('_',' '))},reliability:{gte:.99},disk_space:{gte:120},dph_total:{lte:this.cfg.price},allocated_storage:120,order:[['dph_total','asc']]};
    const result=await this.api('/api/v0/bundles','POST',q);
    if(!Array.isArray(result.offers)||result.offers.length>20)refuse();
    const offered=result.offers.filter(x=>validOffer(x,this.cfg)).sort((a,b)=>a.dph_total-b.dph_total)[0];if(!offered)refuse();return offered;
  }
  async balance(){const r=await this.api('/api/v0/users/current');return r.balance;}
  async create(offer,record,script){
    const r=await this.api('/api/v0/asks/'+offer.id,'PUT',{image:IMAGE,vm:true,disk:120,runtype:'args',args:['sleep','infinity'],
      onstart:script,label:record.label,target_state:'running',cancel_unavail:true});
    if(r.success!==true||!Number.isSafeInteger(r.new_contract)||r.new_contract<=0)refuse();return String(r.new_contract);
  }
  async reconcile(record){
    const query=new URLSearchParams({limit:'2',select_filters:JSON.stringify({label:{eq:record.label}})});
    const r=await this.api('/api/v1/instances?'+query);
    if(r.success!==true||!Array.isArray(r.instances)||r.next_token)refuse();
    const matches=r.instances.filter(x=>x.label===record.label);
    if(matches.length!==1)refuse();const candidate=matches[0];
    if(typeof candidate.onstart!=='string'||await sha256Hex(candidate.onstart)!==record.scriptHash||candidate.image_uuid!==IMAGE||!Number.isSafeInteger(candidate.id)||candidate.id<=0)refuse();
    return String(candidate.id);
  }
  async inspect(record){
    const response=await this.api('/api/v0/instances/'+record.id,'GET',undefined,true);
    if(response===null)return null;
    const i=response.instances;if(!i||String(i.id)!==record.id||!record.label.startsWith('kf-ci-')||i.label!==record.label)refuse('OWNERSHIP_MISMATCH',409);
    return i;
  }
  async destroy(record){
    const instance=await this.inspect(record);if(instance===null)return;
    const r=await this.api('/api/v0/instances/'+record.id,'DELETE');if(r.success!==true)refuse();
  }
}
