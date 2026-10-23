import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { setup } from './helpers.js';
// FALSIFIER: REST env is CLI flag text rather than the object sent by Vast's
// create_instance_impl after parse_env, or the real reporter rejects its values.
test('Vast REST environment is a JSON map with protocol-seven values', async () => {
  const h = await setup();
  assert.equal((await h.call()).status, 200);
  const { env } = h.cloud.calls.find((c) => c.url.endsWith('/asks/5/')).body;
  assert.equal(typeof env, 'object');
  assert.equal(Array.isArray(env), false);
  assert.equal(env.KF_PROTOCOL, '7');
  assert.match(env.KF_RUN_TOKEN, /^v2\./);
  assert.equal(env.KF_RUNNER_NONCE.length, 64);
  assert.ok(Object.values(env).every((v) => typeof v === 'string'));
  assert.ok(Object.keys(env).every((k) => /^KF_[A-Z_]+$/.test(k)));
});
test('Vast REST environment round-trips into the canonical PR104 reporter', async () => {
  const h = await setup();
  assert.equal((await h.call()).status, 200);
  const { env } = h.cloud.calls.find((c) => c.url.endsWith('/asks/5/')).body;
  const code = `import importlib.util,json,sys
s=importlib.util.spec_from_file_location('report',sys.argv[1]);m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
e=json.loads(sys.stdin.read());assert type(e) is dict;assert m.validate_env(e)==[]
assert e['KF_PROTOCOL']=='7' and e['KF_RUNNER_NONCE'] and e['KF_SOFT_DEADLINE'] and e['KF_HEARTBEAT_SECONDS'] and e['KF_GRACE_FAIL']
`;
  const r = spawnSync(
    'python3',
    ['-I', '-c', code, new URL('../../../scripts/ci/vast_report.py', import.meta.url).pathname],
    { input: JSON.stringify(env), encoding: 'utf8' },
  );
  assert.equal(r.status, 0, r.stderr);
});
