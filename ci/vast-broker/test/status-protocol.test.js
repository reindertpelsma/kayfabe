import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { setup } from './helpers.js';
// Status protocol 6 (owner ruling 2026-10-24 item 9): the status object gained machine_id and
// host_id and lost pr, so the version is bumped and each side refuses the other version.
// (Protocol 5, 2026-10-23, had dropped MANUAL and manual_deadline_at.)
// FALSIFIER: a protocol-5 runner is served, or a protocol-6 runner accepts a protocol-5 Worker.
const repoRoot = fileURLToPath(new URL('../../../', import.meta.url));
const vastCalls = (h) => h.cloud.calls.filter((x) => x.url.startsWith('https://console.vast.ai'));
test('status protocol: Worker answers with protocol_version 6', async () => {
  const h = await setup();
  const rent = await h.call('/rent', { protocol: '6' });
  assert.equal(rent.status, 200);
  assert.equal((await rent.json()).protocol_version, 6);
  const status = await h.call('/status/123-1', { protocol: '6' });
  assert.equal((await status.json()).protocol_version, 6);
});
test('status protocol: the committed manifest and defaults name protocol 6', async () => {
  const manifest = JSON.parse(await readFile(new URL('../deployment.json', import.meta.url)));
  const defaults = JSON.parse(await readFile(new URL('../defaults.json', import.meta.url)));
  assert.equal(manifest.protocol_version, 6);
  assert.equal(defaults.statusProtocol, 6);
});
for (const protocol of [null, '4', '5', '7', '06'])
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
test('status protocol: runner sends X-KF-Protocol 6 to the broker only', () => {
  const r = runMain({ manifest_protocol: 6 });
  assert.equal(r.status, 0);
  for (const call of r.calls)
    assert.equal(
      call.headers['X-KF-Protocol'],
      call.url.startsWith('https://oidc.example/') ? undefined : '6',
    );
});
for (const [name, args] of [
  ['manifest 4', { manifest_protocol: 4 }],
  ['manifest 5', { manifest_protocol: 5 }],
  ['manifest 7', { manifest_protocol: 7 }],
  ['status 5 from a protocol-5 Worker', { manifest_protocol: 6, status_protocol: 5 }],
  ['status 7', { manifest_protocol: 6, status_protocol: 7 }],
])
  test('status protocol: runner refuses ' + name, () => {
    const r = runMain(args);
    assert.equal(r.status, 1);
    assert.doesNotMatch(r.outputs, /verified=1/);
  });
