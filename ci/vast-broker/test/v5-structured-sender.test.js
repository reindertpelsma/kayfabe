import test from 'node:test';
import assert from 'node:assert/strict';
import { probe } from './repo-onstart.js';
// Binding is supplied by the validated env; an echoed conflicting field refuses.
for (const mode of [
  'missing',
  'wrong-nonce',
  'wrong-sha',
  'wrong-runner',
  'valid',
  'payload-only',
  'missing-hardware',
  'self_test',
  'gpu_probe',
])
  test('V5 sender requires bound structured payload ' + mode, () => {
    const r = probe(
      `
mode=sys.argv[3]
with tempfile.TemporaryDirectory() as d:
 p=pathlib.Path(d)/'kf-result.json';name=mode if mode in ('self_test','gpu_probe') else 'self_test'
 doc={'verdict':'PASS','suites':[{'name':name,'pass':1,'fail':0,'skip':0}],'hardware':{'gpu_name':'RTX_4070','compute_cap':890,'driver_version':'580.159.04'}}
 if mode=='wrong-nonce':doc['nonce']='0'*32
 if mode=='wrong-sha':doc['sha']='0'*40
 if mode=='wrong-runner':doc['runner_nonce']='0'*64
 if mode=='missing-hardware':del doc['hardware']
 if mode!='missing':p.write_text(json.dumps(doc))
 result=m.final_result(env,0,False,None,str(p))
 expected='PASS' if mode in ('valid','payload-only','self_test','gpu_probe') else 'FAIL'
 assert result['verdict']==expected,result
 assert result['sha']==env['KF_SHA'] and result['nonce']==env['KF_NONCE'] and result['runner_nonce']==env['KF_RUNNER_NONCE']
 if expected=='PASS':assert result['suites']==doc['suites']
`,
      {},
      [mode],
    );
    assert.equal(r.status, 0, r.stderr);
  });
