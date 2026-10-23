import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { setup } from './helpers.js';
// Status protocol 5 (2026-10-23): the status shape lost MANUAL and manual_deadline_at, so the
// version is bumped explicitly and each side refuses the other version.
// FALSIFIER: a protocol-4 runner is served, or a protocol-5 runner accepts a protocol-4 Worker.
const repoRoot = fileURLToPath(new URL('../../../', import.meta.url));
const vastCalls = (h) => h.cloud.calls.filter((x) => x.url.startsWith('https://console.vast.ai'));
test('status protocol: Worker answers with protocol_version 5', async () => {
  const h = await setup();
  const rent = await h.call();
  assert.equal(rent.status, 200);
  assert.equal((await rent.json()).protocol_version, 5);
  assert.equal((await (await h.call('/status/123-1')).json()).protocol_version, 5);
});
test('status protocol: the committed manifest names protocol 5', async () => {
  const manifest = JSON.parse(await readFile(new URL('../deployment.json', import.meta.url)));
  assert.equal(manifest.protocol_version, 5);
});
for (const protocol of [null, '4', '6', '05'])
  for (const path of ['/rent', '/status/123-1', '/availability?profile=check'])
    test('status protocol: Worker refuses runner protocol ' + protocol + ' ' + path, async () => {
      const h = await setup();
      const method = path === '/rent' ? 'POST' : 'GET';
      const response = await h.call(path, { protocol, method });
      assert.equal(response.status, 409);
      assert.equal((await response.json()).state, 'PROTOCOL_MISMATCH');
      assert.deepEqual(vastCalls(h), []);
    });
const runMain = (args) => {
  const p = spawnSync(
    'python3',
    ['-I', 'ci/vast-broker/test/round3_main_fixture.py', JSON.stringify(args)],
    { encoding: 'utf8', timeout: 5000, cwd: repoRoot },
  );
  assert.equal(p.status, 0, p.stderr);
  return JSON.parse(p.stdout);
};
test('status protocol: runner sends X-KF-Protocol 5 to the broker only', () => {
  const r = runMain({ manifest_protocol: 5 });
  assert.equal(r.status, 0);
  for (const call of r.calls)
    assert.equal(
      call.headers['X-KF-Protocol'],
      call.url.startsWith('https://oidc.example/') ? undefined : '5',
    );
});
for (const [name, args] of [
  ['manifest 4', { manifest_protocol: 4 }],
  ['status 4', { manifest_protocol: 5, status_protocol: 4 }],
])
  test('status protocol: runner refuses ' + name, () => {
    const r = runMain(args);
    assert.equal(r.status, 1);
    assert.doesNotMatch(r.outputs, /verified=1/);
  });
