import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// FALSIFIER: current Site policy already always reports and makes these conservative decisions.
for (const variant of ['pr', 'push'])
  for (const name of [
    'untouched',
    'site',
    'docs',
    'workflow',
    'removed',
    'renamed-out',
    'renamed-in',
    'odd',
    'unknown-status',
    'count',
    'missing',
    'sha',
    'truncated',
    'too-many-files',
    'prefix-only',
    'tag',
    'schedule',
    'release',
    'workflow_dispatch',
    'status-no-build',
    'status-untouched',
    'status-failure',
    'status-foreign',
    'status-wrong-path',
    'status-fork',
    'status-stale',
    'status-pr-start',
    'status-late-start',
    'status-late-run',
    'status-bad-annotation',
    'reuse-valid',
    'reuse-tree',
    'reuse-creator',
    'reuse-pending',
    'reuse-ci',
    'reuse-ci-pending',
    'reuse-ci-app',
    'reuse-run-sha',
    'reuse-run',
    'reuse-pr',
    'reuse-tag',
  ])
    test('site policy ' + variant + ' ' + name, () => {
      const run = spawnSync(
        'python3',
        ['-I', new URL('./site_policy_fixture.py', import.meta.url).pathname, name, variant],
        { encoding: 'utf8', timeout: 10000 },
      );
      assert.equal(run.status, 0, run.stdout + run.stderr);
    });
