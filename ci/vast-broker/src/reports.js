import {refuse} from './policy.js';
export async function readReportBody(request,record,cfg){
  const remaining=cfg.reportBytes-record.reportBytes,limit=Math.min(65536,remaining);
  const reader=request.body?.getReader();if(!reader)refuse('BAD_REPORT',400);
  const chunks=[];let length=0,oversized=false;
  try{for(;;){const {value,done}=await reader.read();if(done)break;length+=value.byteLength;if(length>limit){oversized=true;break;}chunks.push(value);}}
  finally{await reader.cancel().catch(()=>{});}
  record.reportBytes=Math.min(cfg.reportBytes,record.reportBytes+length);
  if(oversized)refuse('REPORT_TOO_LARGE',413); // MUTANT body-cap
  const bytes=new Uint8Array(length);let offset=0;for(const chunk of chunks){bytes.set(chunk,offset);offset+=chunk.length;}
  return new TextDecoder('utf-8').decode(bytes);
}
export function parseReport(text){
  let depth=0,quoted=false,escaped=false;
  for(const char of text){if(quoted){if(escaped)escaped=false;else if(char==='\\')escaped=true;else if(char==='"')quoted=false;}else if(char==='"')quoted=true;else if(char==='{'||char==='['){if(++depth>4)refuse('BAD_REPORT',400);}else if(char==='}'||char===']'){if(--depth<0)refuse('BAD_REPORT',400);}}
  let p;try{p=JSON.parse(text);}catch{refuse('BAD_REPORT',400);}
  const object=x=>x!==null&&typeof x==='object'&&!Array.isArray(x);
  const keys=(x,allowed)=>Object.keys(x).every(k=>allowed.includes(k));
  if(!object(p)||!keys(p,['seq','kind','text','result'])||!Number.isSafeInteger(p.seq)||p.seq<1||p.seq>2147483647||!['progress','log','result'].includes(p.kind))refuse('BAD_REPORT',400);
  if(p.text!==undefined&&(typeof p.text!=='string'||new TextEncoder().encode(p.text).length>60000))refuse('BAD_REPORT',400);
  if(p.kind==='result'){
    if(!object(p.result)||!keys(p.result,['nonce','verdict'])||typeof p.result.nonce!=='string'||!/^[0-9a-f]{32}$/.test(p.result.nonce)||!['PASS','FAIL','UNAVAILABLE'].includes(p.result.verdict))refuse('BAD_REPORT',400);
  }else if(typeof p.text!=='string'||p.result!==undefined)refuse('BAD_REPORT',400);
  return {seq:p.seq,kind:p.kind,...(p.text===undefined?{}:{text:p.text}),...(p.result===undefined?{}:{result:{nonce:p.result.nonce,verdict:p.result.verdict}})};
}
