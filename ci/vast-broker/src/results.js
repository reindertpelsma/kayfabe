import {refuse} from './policy.js';
export const SUITE_COUNTS={provision:1,kf_tests:null,gates:9,kf3_build:1,raw_suite:30,thin_suite:30,birth_census:1,cuda_host:4,cuda_fat:4};
const errorSuites=['preflight','timeout','infrastructure','merge_bar','app_exit','xid'];
export function checkedResult(result){
  const object=x=>x!==null&&typeof x==='object'&&!Array.isArray(x);
  if(!object(result)||Object.keys(result).sort().join(',')!=='hardware,nonce,runner_nonce,sha,suites,verdict'||!['PASS','FAIL','UNAVAILABLE'].includes(result.verdict)||!/^([0-9a-f]{32})$/.test(result.nonce)||!/^([0-9a-f]{64})$/.test(result.runner_nonce)||!/^([0-9a-f]{40})$/.test(result.sha))refuse('BAD_REPORT',400);
  const rows=result.suites;if(!Array.isArray(rows)||rows.length<1||rows.length>20||new Set(rows.map(x=>x?.name)).size!==rows.length)refuse('BAD_REPORT',400);
  for(const r of rows)if(!object(r)||Object.keys(r).sort().join(',')!=='fail,name,pass,skip'||!Object.hasOwn(SUITE_COUNTS,r.name)&&!errorSuites.includes(r.name)||!['pass','fail','skip'].every(k=>Number.isSafeInteger(r[k])&&r[k]>=0&&r[k]<=10000000))refuse('BAD_REPORT',400);
  const h=result.hardware;if(!object(h)||Object.keys(h).sort().join(',')!=='compute_cap,driver_version,gpu_name'||typeof h.gpu_name!=='string'||!/^[A-Za-z0-9_. -]{1,100}$/.test(h.gpu_name)||!Number.isSafeInteger(h.compute_cap)||h.compute_cap<0||h.compute_cap>2000||typeof h.driver_version!=='string'||!/^(?:unknown|\d{3}\.\d{1,3}(?:\.\d{1,2})?)$/.test(h.driver_version))refuse('BAD_REPORT',400);
  if(result.verdict==='PASS'){
    if(rows.length!==Object.keys(SUITE_COUNTS).length||rows.some(r=>!Object.hasOwn(SUITE_COUNTS,r.name)||r.fail!==0||r.skip!==0||r.pass<=0||SUITE_COUNTS[r.name]!==null&&r.pass!==SUITE_COUNTS[r.name])||h.gpu_name==='unknown'||h.driver_version==='unknown'||h.compute_cap<750)refuse('BAD_REPORT',400);
  }else if(!rows.some(r=>r.fail>0))refuse('BAD_REPORT',400);
  return result;
}
