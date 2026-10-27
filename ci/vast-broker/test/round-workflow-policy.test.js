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
for (const item of ['push', 'no-sha-input', 'concurrency'])
  test('round reviewed YAML policy ' + item, async () => {
    const text = await workflow();
    if (item === 'push') assert.match(text, /  push:\n    branches: \[master\]/);
    if (item === 'no-sha-input') assert.doesNotMatch(text, /workflow_dispatch:|inputs\.sha/);
    if (item === 'concurrency')
      assert.match(
        text.split('concurrency:')[1].split('\nenv:')[0],
        /group:[\s\S]*?github\.event\.pull_request\.head\.sha \|\| github\.sha/,
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
test('round YAML excludes unintended automatic push branches', async () => {
  const text = await workflow();
  const block = text.split('  push:\n')[1].split('  pull_request_target:')[0];
  const branches = /branches: \[([^\]]+)\]/
    .exec(block)[1]
    .split(',')
    .map((x) => x.trim());
  assert.ok(branches.every((x) => x === 'master'));
});
