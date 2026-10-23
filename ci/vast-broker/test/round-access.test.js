import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { setup } from './helpers.js';
// FALSIFIER H6: a partial Access pair is admitted, or A/B token scopes cross transports.
for (const which of ['BOX_ACCESS_ID', 'BOX_ACCESS_SECRET']) {
  test('round partial box Access config fails before spending ' + which, async () => {
    const h = await setup({ [which]: which === 'BOX_ACCESS_ID' ? 'box.access' : 's'.repeat(64) });
    assert.notEqual((await h.call()).status, 200);
    assert.equal(h.cloud.creates, 0);
  });
}
for (const name of ['KF_ACCESS_ID', 'KF_ACCESS_SECRET']) {
  test('round complete Access B is injected only as box environment ' + name, async () => {
    const h = await setup({ BOX_ACCESS_ID: 'box-id.access', BOX_ACCESS_SECRET: 's'.repeat(64) });
    assert.equal((await h.call()).status, 200);
    const request = h.cloud.calls.find((x) => x.url.includes('/asks/')).body;
    assert.equal(request.env[name], name.endsWith('_ID') ? 'box-id.access' : 's'.repeat(64));
    assert.ok(!JSON.stringify([...h.storage.data]).includes('s'.repeat(64)));
  });
}
for (const side of ['runner', 'reporter', 'daemon', 'kill', 'hold', 'blob']) {
  test('round optional Access headers use the intended transport ' + side, () => {
    const result = spawnSync('python3', ['ci/vast-broker/test/round_access_fixture.py', side], {
      encoding: 'utf8',
      timeout: 5000,
    });
    assert.equal(result.status, 0, result.stdout + result.stderr);
  });
}
for (const path of ['.github/workflows/gpu-vast.yml', '.github/workflows/gpu-vast-sweep.yml']) {
  test('round BASE runner has independent Access A env ' + path, async () => {
    const text = await readFile(new URL('../../../' + path, import.meta.url), 'utf8');
    assert.match(text, /CF_ACCESS_CLIENT_ID:/);
    assert.match(text, /CF_ACCESS_CLIENT_SECRET:/);
    assert.doesNotMatch(text, /BOX_ACCESS_ID|BOX_ACCESS_SECRET|KF_ACCESS_SECRET/);
  });
}
for (const job of ['pending', 'report'])
  test('round Access A secret cannot cross the broker job ' + job, async () => {
    const { checkWorkflows } = await import('../workflow-guard.js');
    const text = await readFile('.github/workflows/gpu-vast.yml', 'utf8');
    const changed = text.replace(
      '  ' + job + ':\n',
      '  ' +
        job +
        ':\n    env:\n' +
        '      WRONG_ACCESS: ${{ secrets.VAST_BROKER_ACCESS_SECRET }}\n',
    );
    assert.ok(checkWorkflows({ 'gpu-vast.yml': changed }).length > 0);
  });

for (const manual of [false, true])
  test('round actual controller Access A headers and masking ' + manual, () => {
    const p = spawnSync(
      'python3',
      [
        '-I',
        'ci/vast-broker/test/round3_main_fixture.py',
        JSON.stringify({ access: true, manual }),
      ],
      { encoding: 'utf8', timeout: 5000 },
    );
    assert.equal(p.status, 0, p.stderr);
    const r = JSON.parse(p.stdout);
    assert.equal(r.status, 0);
    for (const call of r.calls) {
      if (call.url.startsWith('https://oidc.example/')) {
        assert.ok(!call.headers['CF-Access-Client-Id']);
      } else {
        assert.equal(call.headers['CF-Access-Client-Id'], 'runner-a.access');
        assert.equal(call.headers['CF-Access-Client-Secret'], 'a'.repeat(64));
        assert.ok(call.masks.includes('a'.repeat(64)));
      }
    }
    assert.ok(!r.summary.includes('a'.repeat(64)));
  });
