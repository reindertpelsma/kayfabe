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
  'ci-other-workflow',
  'ci-run-pending',
  'ci-run-head',
  'ci-suite',
  'ci-ambiguous-failed',
  'ci-ambiguous-pending',
  'ci-multiple-success',
  'run-failed',
  'run-sha',
  'run-path',
  'fork',
  'tag',
  'pr',
  'logs-missing',
  'logs-fail',
  'logs-revision',
  'logs-no-guest-start',
  'logs-no-host-start',
  'logs-conflicting-final',
  'logs-duplicate-final',
  'logs-schema-overflow',
  'logs-duplicate-stage',
  'logs-missing-guest-stage',
  'not-needed',
  'api-error',
  'prepare-pending',
  'release-valid',
  'release-child-missing',
  'release-child-pending',
  'release-child-failed',
  'release-child-foreign',
  'release-child-workflow',
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
