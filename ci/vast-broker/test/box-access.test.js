import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { setup } from './helpers.js';
// Owner ruling 2026-10-23 21:45: the workers.dev hostname sits behind a hostname-wide
// Cloudflare Access policy, so boxes carry the shared box service-token pair. The Worker only
// passes it through (it never reads or verifies Access headers or JWTs).
// FALSIFIER: a pair is injected when only half is set, is missing when both are set, or leaks.
const id = '0123456789abcdef0123456789abcdef.access';
const secret = 'f0e1d2c3b4a5968778695a4b3c2d1e0f'.repeat(2);
const createEnv = (h) => h.cloud.calls.find((c) => c.url.includes('/asks/'))?.body.env;
for (const [name, pair, expected] of [
  ['both', { BOX_ACCESS_ID: id, BOX_ACCESS_SECRET: secret }, true],
  ['none', {}, false],
  ['id only', { BOX_ACCESS_ID: id }, false],
  ['secret only', { BOX_ACCESS_SECRET: secret }, false],
])
  test('box Access pair is injected only when both secrets are set: ' + name, async () => {
    const h = await setup(pair);
    assert.equal((await h.call()).status, 200);
    const env = createEnv(h);
    if (expected) {
      assert.equal(env.KF_ACCESS_ID, id);
      assert.equal(env.KF_ACCESS_SECRET, secret);
    } else {
      assert.ok(!('KF_ACCESS_ID' in env) && !('KF_ACCESS_SECRET' in env));
    }
  });
test('box Access pair never appears in responses, logs or error bodies', async () => {
  const logged = [];
  const original = console.log;
  console.log = (...args) => logged.push(args.join(' '));
  try {
    const h = await setup({ BOX_ACCESS_ID: id, BOX_ACCESS_SECRET: secret });
    const texts = [await (await h.call()).text()];
    texts.push(
      await (await h.report({ seq: 1, kind: 'progress', text: secret + ' ' + id })).text(),
    );
    texts.push(await (await h.call('/status/123-1')).text());
    texts.push(await (await h.call('/logs/123-1?cursor=0', { method: 'GET' })).text());
    texts.push(await (await h.report({ seq: 1, kind: 'bogus' })).text());
    texts.push(await (await h.call('/rent', { token: 'x.y.z' })).text());
    for (const text of [...texts, ...logged]) {
      assert.ok(!text.includes(secret), 'secret leaked');
      assert.ok(!text.includes(id), 'client id leaked');
    }
  } finally {
    console.log = original;
  }
});
test('injected box Access pair passes the tested reporter contract', async () => {
  const h = await setup({ BOX_ACCESS_ID: id, BOX_ACCESS_SECRET: secret });
  assert.equal((await h.call()).status, 200);
  const code = `import importlib.util,json,sys
s=importlib.util.spec_from_file_location('report',sys.argv[1])
m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
e=json.loads(sys.stdin.read());assert m.validate_env(e)==[],m.validate_env(e)
names=[n for n,_,_ in m.SPEC]
assert 'KF_ACCESS_ID' in names and 'KF_ACCESS_SECRET' in names
`;
  const r = spawnSync(
    'python3',
    ['-I', '-c', code, new URL('../../../scripts/ci/vast_report.py', import.meta.url).pathname],
    { input: JSON.stringify(createEnv(h)), encoding: 'utf8' },
  );
  assert.equal(r.status, 0, r.stderr);
});
