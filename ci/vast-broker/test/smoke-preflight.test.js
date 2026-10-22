import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
// FALSIFIER: eligible read-only preflight still claims the owner-resolved
// no-SSH launch contract is blocked, or performs any provider mutation.
for(const verification of [{verified:true},{verification:'verified'}])test('local preflight reports eligible offer after SSH ruling '+JSON.stringify(verification),()=>{
 const script=`import sys,json
sys.path.insert(0,sys.argv[1])
import smoke
offer={'id':5,'vms_enabled':True,'rentable':True,'rented':False,'num_gpus':1,'gpu_name':'RTX 4070','reliability':.999,'disk_space':200,'dph_total':.2,**json.loads(sys.argv[2])}
calls=[]
def provider(path,method='GET',body=None):
 calls.append((path,method))
 if path=='/api/v0/bundles/':return {'offers':[offer]}
 if path=='/api/v0/users/current/':return {'credit':39.5,'balance':0}
 raise AssertionError('unexpected provider mutation')
smoke.provider=provider
print(json.dumps(smoke.preflight()))
`;
 const p=spawnSync('python3',['-c',script,new URL('../',import.meta.url).pathname,JSON.stringify(verification)],{encoding:'utf8'});
 assert.equal(p.status,0,p.stderr);const result=JSON.parse(p.stdout);
 assert.equal(result.state,'ELIGIBLE_OFFER_FOUND');assert.equal(result.rentals_created,0);
});
