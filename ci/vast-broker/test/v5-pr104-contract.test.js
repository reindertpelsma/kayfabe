import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// FALSIFIER: approved repo-side protocol 5 cannot build a bound PASS from the
// documented minimal result, or missing hardware can turn that file into PASS.
for (const hardware of [true, false])
  test('PR104 minimal payload result hardware ' + hardware, () => {
    const code = `import importlib.util,json,sys,tempfile,pathlib
s=importlib.util.spec_from_file_location('report',sys.argv[1]);m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
env={'KF_NONCE':'b'*32,'KF_SHA':'a'*40,'KF_RUNNER_NONCE':'f'*64}
with tempfile.TemporaryDirectory() as d:
 p=pathlib.Path(d)/'result';doc={'verdict':'PASS','suites':[{'name':'self_test','pass':1,'fail':0,'skip':0}]}
 if sys.argv[2]=='true':doc['hardware']={'gpu_name':'RTX 3090','compute_cap':860,'driver_version':'580.95.05'}
 p.write_text(json.dumps(doc));r=m.final_result(env,0,False,None,str(p));assert r['sha']==env['KF_SHA'] and r['nonce']==env['KF_NONCE'] and r['runner_nonce']==env['KF_RUNNER_NONCE'];assert r['verdict']==('PASS' if sys.argv[2]=='true' else 'FAIL')
`;
    const r = spawnSync(
      'python3',
      [
        '-I',
        '-c',
        code,
        new URL('../../../scripts/ci/vast_report.py', import.meta.url).pathname,
        String(hardware),
      ],
      { encoding: 'utf8' },
    );
    assert.equal(r.status, 0, r.stderr);
  });
