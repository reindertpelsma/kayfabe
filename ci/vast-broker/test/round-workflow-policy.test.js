import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
async function workflow() {
  return readFile('.github/workflows/gpu-vast.yml', 'utf8');
}
// FALSIFIER: trigger expansion spends on unintended branches or changes the BASE controller SHA.
for (const item of ['push', 'sha-input', 'concurrency'])
  test('round reviewed YAML policy ' + item, async () => {
    const text = await workflow();
    if (item === 'push') assert.match(text, /  push:\n    branches: \[master\]/);
    if (item === 'sha-input') assert.match(text, /workflow_dispatch:\n    inputs:\n      sha:/);
    if (item === 'concurrency') assert.match(text, /group:[\s\S]*?github\.event\.inputs\.sha/);
    assert.doesNotMatch(text, /  workflow_run:/);
  });
for (const access of [false, true])
  test('round BASE controller supports push event ' + access, () => {
    const p = spawnSync(
      'python3',
      ['-I', 'ci/vast-broker/test/round3_main_fixture.py', JSON.stringify({ push: true, access })],
      { encoding: 'utf8', timeout: 5000 },
    );
    assert.equal(p.status, 0, p.stderr);
    const r = JSON.parse(p.stdout);
    assert.equal(r.status, 0);
    assert.equal(r.calls.find((x) => x.url.endsWith('/rent')).request.tested_sha, 'a'.repeat(40));
  });
for (const selected of ['e'.repeat(40), 'f'.repeat(40)])
  test('round dispatch selects SHA while retaining BASE ' + selected[0], () => {
    const p = spawnSync(
      'python3',
      [
        '-I',
        'ci/vast-broker/test/round3_main_fixture.py',
        JSON.stringify({ manual: true, selected_sha: selected }),
      ],
      { encoding: 'utf8', timeout: 5000 },
    );
    assert.equal(p.status, 0, p.stderr);
    const r = JSON.parse(p.stdout);
    assert.equal(r.status, 0);
    assert.equal(r.workflow_sha, 'a'.repeat(40));
    assert.equal(r.calls.find((x) => x.url.endsWith('/rent')).request.tested_sha, selected);
  });
for (const selected of [[], {}])
  test('round dispatch rejects non-string selected SHA ' + JSON.stringify(selected), () => {
    const p = spawnSync(
      'python3',
      [
        '-I',
        'ci/vast-broker/test/round3_main_fixture.py',
        JSON.stringify({ manual: true, selected_sha: selected }),
      ],
      { encoding: 'utf8', timeout: 5000 },
    );
    assert.equal(p.status, 0, p.stderr);
    const r = JSON.parse(p.stdout);
    assert.equal(r.status, 1);
    assert.equal(r.calls.length, 0);
  });
