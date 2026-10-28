import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// FALSIFIER: current code already transfers only complete, same-tree trusted evidence.
const cases = [
  'valid',
  'tree',
  'status-failure',
  'status-pending',
  'creator-id',
  'creator-login',
  'ci-pending',
  'ci-failed',
  'ci-forged',
  'ci-missing',
  'ci-truncated',
  'run-failed',
  'run-sha',
  'run-path',
  'fork',
  'tag',
  'pr',
  'logs-missing',
  'logs-fail',
  'logs-revision',
  'logs-duplicate-stage',
  'logs-missing-guest-stage',
  'not-needed',
  'api-error',
  'release-valid',
  'release-forged',
  'release-forged-id',
  'release-forged-login',
  'release-lost-parent',
  'release-tree',
];
for (const variant of ['merge', 'associated-pr', 'fast-forward'])
  for (const name of cases)
    test('reuse ' + variant + ' ' + name, () => {
      const p = spawnSync(
        'python3',
        ['-I', new URL('./gate_reuse_fixture.py', import.meta.url).pathname, name, variant],
        { encoding: 'utf8', timeout: 10000 },
      );
      assert.equal(p.status, 0, p.stdout + p.stderr);
    });
