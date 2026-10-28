import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// FALSIFIER: PR metadata can change during pagination without authorizing an old-head skip.
for (const mode of ['policy', 'start', 'completion'])
  for (const change of ['head', 'base', 'count', 'stable'])
    test('Site files binding ' + mode + ' ' + change, () => {
      const run = spawnSync(
        'python3',
        ['-I', new URL('./site_head_race_fixture.py', import.meta.url).pathname, mode, change],
        { encoding: 'utf8', timeout: 10000 },
      );
      assert.equal(run.status, 0, run.stdout + run.stderr);
    });
