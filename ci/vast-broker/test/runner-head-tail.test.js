import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// Gate visibility, 2026-10-26: the job log must show the HEAD and the TAIL of the box output inside
// the same display budget, with an explicit middle-drop marker. Each case drives the real
// Runner.execute() against a fake Worker (runner_head_tail_fixture.py).
// FALSIFIER: a long box log whose verdict-deciding lines are at its end shows only its beginning.
const CASES = [
  'markers',
  'count',
  'head-size',
  'tail-size',
  'wide-budget',
  'wide-sizes',
  'ragged',
  'ragged-again',
  'ragged-ramp',
  'flood-lines',
  'scrub',
  'summary-tail',
  'small-log',
  'exact-head',
  'budget-line-kept',
  'constants',
  'direct-show-budget-unchanged',
];
for (const name of CASES)
  test('runner head+tail display: ' + name, () => {
    const p = spawnSync(
      'python3',
      ['-I', new URL('runner_head_tail_fixture.py', import.meta.url).pathname, name],
      { encoding: 'utf8', timeout: 30000, maxBuffer: 1 << 20, env: { PATH: process.env.PATH } },
    );
    assert.equal(p.status, 0, p.stderr + p.stdout);
  });
