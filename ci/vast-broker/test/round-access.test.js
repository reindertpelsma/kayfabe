import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
const repoRoot = fileURLToPath(new URL('../../../', import.meta.url));
// FALSIFIER H6: runner Access A leaves the broker step or reaches the OIDC issuer.
test('round optional Access headers use the intended transport runner', () => {
  const result = spawnSync('python3', ['ci/vast-broker/test/round_access_fixture.py', 'runner'], {
    encoding: 'utf8',
    timeout: 5000,
    cwd: repoRoot,
  });
  assert.equal(result.status, 0, result.stdout + result.stderr);
});
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
    const text = await readFile(repoRoot + '.github/workflows/gpu-vast.yml', 'utf8');
    const changed = text.replace(
      '  ' + job + ':\n',
      '  ' +
        job +
        ':\n    env:\n' +
        '      WRONG_ACCESS: ${{ secrets.VAST_BROKER_ACCESS_SECRET }}\n',
    );
    assert.ok(checkWorkflows({ 'gpu-vast.yml': changed }).length > 0);
  });

for (const push of [false, true])
  test('round actual controller Access A headers and masking ' + push, () => {
    const p = spawnSync(
      'python3',
      ['-I', 'ci/vast-broker/test/round3_main_fixture.py', JSON.stringify({ access: true, push })],
      { encoding: 'utf8', timeout: 5000, cwd: repoRoot },
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
