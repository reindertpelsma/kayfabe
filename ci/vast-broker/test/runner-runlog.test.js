import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// Display work of the gpu-vast runner (2026-11-05). Each case drives the real Runner.execute()
// against a fake Worker (runner_runlog_fixture.py).
// FALSIFIERS: (a) a box line imitating a key line, a workflow command or a 10 KB census prefix reaches
// the job log unsanitised or unprefixed; (b) a value split by control characters is not redacted, or one
// long line stalls the runner; (c) the key section or the census pushes
// the job log past its display budget or drops the tail; (d) the runner sends the deployed Worker a
// different request than before; (e) the runner writes a file or reads a directory setting.
const CASES = [
  'key-lines-cover-the-middle',
  'key-lines-budget',
  'key-lines-red-first',
  'key-lines-first-and-last',
  'key-lines-imitation-stays-untrusted',
  'key-lines-hostile-commands',
  'key-lines-use-the-sanitised-text',
  'key-lines-work-is-bounded',
  'redaction-survives-a-split-value',
  'key-lines-absent-when-log-is-small',
  'key-lines-are-data',
  'census-groups-the-middle',
  'census-counts-are-exact',
  'census-only-twelve-groups',
  'census-hostile-prefix',
  'census-many-keys-are-only-counted',
  'census-absent-when-nothing-dropped',
  'census-budget-holds',
  'markers-are-honest',
  'display-marker-split',
  'diagnostics-for-a-vanished-box',
  'diagnostics-claims-only-what-is-true',
  'runner-writes-no-log-file',
  'diagnostics-absent-for-a-clean-result',
  'diagnostics-hostile-stage-name',
  'diagnostics-no-protocol-change',
  'protocol-requests-unchanged',
];
for (const name of CASES)
  test('runner run-log: ' + name, () => {
    const p = spawnSync(
      'python3',
      ['-I', new URL('runner_runlog_fixture.py', import.meta.url).pathname, name],
      { encoding: 'utf8', timeout: 60000, maxBuffer: 1 << 24, env: { PATH: process.env.PATH } },
    );
    assert.equal(p.status, 0, p.stderr + p.stdout);
  });
