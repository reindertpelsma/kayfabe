import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { checkWorkflows } from '../workflow-guard.js';
const main = new URL('../../../.github/workflows/gpu-vast.yml', import.meta.url),
  sweep = new URL('../../../.github/workflows/gpu-vast-sweep.yml', import.meta.url);
// FALSIFIER: default work is killed by a short workflow (check: the 300-minute master gate with
// the Windows stage, lane 2 2026-11-04; sweep: three hours), or the optional sweep posts a merge
// status / consumes code or wider secrets.
for (const [file, job, step] of [
  [main, '345', '340'],
  [sweep, '210', '205'],
])
  for (const part of ['job', 'step'])
    test(
      'V5 ' + (file === main ? 'check' : 'sweep') + ' workflow accommodates its lifecycle ' + part,
      async () => {
        const s = await readFile(file, 'utf8');
        assert.match(s, new RegExp('timeout-minutes: ' + (part === 'job' ? job : step) + '\\n'));
      },
    );
for (const kind of ['separate', 'guard'])
  test('V5 sweep workflow is owner-dispatch finding ' + kind, async () => {
    const a = await readFile(main, 'utf8'),
      b = await readFile(sweep, 'utf8');
    if (kind === 'guard')
      assert.deepEqual(checkWorkflows({ 'gpu-vast.yml': a, 'gpu-vast-sweep.yml': b }), []);
    else {
      assert.match(b, /  workflow_dispatch:/);
      assert.match(b, /--profile sweep/);
      assert.doesNotMatch(
        b,
        /pull_request:|pull_request_target:|statuses:|context:"gpu-vast"|actions\/checkout|contents:|github.token/,
      );
      assert.match(b, /id-token: write/);
    }
  });

for (const trigger of ['pull_request', 'push'])
  test('V5 guard rejects automatic sweep ' + trigger, async () => {
    const a = await readFile(main, 'utf8'),
      b = (await readFile(sweep, 'utf8')).replace('  workflow_dispatch:', '  ' + trigger + ':');
    assert.ok(checkWorkflows({ 'gpu-vast.yml': a, 'gpu-vast-sweep.yml': b }).length);
  });
for (const permission of ['statuses: write', 'contents: read', 'pull-requests: read'])
  test('V5 guard rejects sweep extra token ' + permission, async () => {
    const a = await readFile(main, 'utf8'),
      b = (await readFile(sweep, 'utf8')).replace(
        '      id-token: write',
        '      id-token: write\n      ' + permission,
      );
    assert.ok(checkWorkflows({ 'gpu-vast.yml': a, 'gpu-vast-sweep.yml': b }).length);
  });
