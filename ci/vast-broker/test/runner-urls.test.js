import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// Public log/status URLs, compact stage lines and the unbudgeted overview + verdict (2026-11-12).
// Each case drives the real Runner against a fake Worker (runner_urls_fixture.py).
// FALSIFIER: the runner does not print the URLs before the box runs, builds one from an id the
// runner did not validate, or loses the verdict when the display budget is spent.
// KF_RUNNER_PATH lets scripts/ci/runner_urls_mutants.py swap the runner.
const CASES = [
  'start',
  'start-order',
  'urls-exact',
  'bad-run',
  'bad-leg',
  'bases-pinned',
  'verdict-under-budget',
  'verdict-budget-spent-first',
  'stage-lines',
  'summary-has-urls',
  'urls-count',
  'bad-run-execute',
  'bad-leg-announce',
  'verdict-flood-lines',
  'verdict-small-log',
  'stage-name-unterminated',
  'stage-name-long',
  'stage-marked',
  'stage-cap',
  'stage-cap-overview',
  'summary-log-url',
  'summary-status-url',
  'mask-survives-ci-origin',
  'mask-survives-workers-dev-origin',
  'mask-no-labels',
  'mask-secrets-kept',
];
for (const name of CASES)
  test('runner urls: ' + name, () => {
    const p = spawnSync(
      'python3',
      ['-I', new URL('runner_urls_fixture.py', import.meta.url).pathname, name],
      {
        encoding: 'utf8',
        timeout: 60000,
        maxBuffer: 1 << 22,
        env: { PATH: process.env.PATH, KF_RUNNER_PATH: process.env.KF_RUNNER_PATH || '' },
      },
    );
    assert.equal(p.status, 0, p.stderr + p.stdout);
  });
