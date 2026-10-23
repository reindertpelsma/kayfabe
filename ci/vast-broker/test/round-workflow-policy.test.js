import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const repoRoot = fileURLToPath(new URL('../../../', import.meta.url));
async function workflow() {
  return readFile(repoRoot + '.github/workflows/gpu-vast.yml', 'utf8');
}
// FALSIFIER: trigger expansion spends on unintended branches or changes the BASE controller SHA.
for (const item of ['push', 'sha-input', 'concurrency'])
  test('round reviewed YAML policy ' + item, async () => {
    const text = await workflow();
    if (item === 'push') assert.match(text, /  push:\n    branches: \[master\]/);
    if (item === 'sha-input') assert.match(text, /workflow_dispatch:\n    inputs:\n      sha:/);
    if (item === 'concurrency')
      assert.match(
        text.split('concurrency:')[1].split('\nenv:')[0],
        /group:[\s\S]*?github\.event\.inputs\.sha/,
      );
    assert.doesNotMatch(text, /  workflow_run:/);
  });
for (const access of [false, true])
  test('round BASE controller supports push event ' + access, () => {
    const p = spawnSync(
      'python3',
      ['-I', 'ci/vast-broker/test/round3_main_fixture.py', JSON.stringify({ push: true, access })],
      { encoding: 'utf8', timeout: 5000, cwd: repoRoot },
    );
    assert.equal(p.status, 0, p.stderr);
    const r = JSON.parse(p.stdout);
    assert.equal(r.status, 0);
    assert.equal(r.calls.find((x) => x.url.endsWith('/rent')).request.tested_sha, 'a'.repeat(40));
  });
test('round YAML target SHA includes the manual selection', async () => {
  const text = await workflow();
  const environment = text.split('\nenv:\n')[1].split('\njobs:\n')[0];
  assert.match(environment, /TARGET_SHA:[\s\S]*github\.event\.inputs\.sha/);
});
test('round YAML excludes unintended automatic push branches', async () => {
  const text = await workflow();
  const block = text.split('  push:\n')[1].split('  pull_request_target:')[0];
  const branches = /branches: \[([^\]]+)\]/
    .exec(block)[1]
    .split(',')
    .map((x) => x.trim());
  assert.ok(branches.every((x) => x === 'master'));
});
