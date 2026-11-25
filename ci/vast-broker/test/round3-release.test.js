import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
for (const name of [
  'valid',
  'wrong-sha',
  'wrong-status',
  'wrong-creator',
  'wrong-url',
  'wrong-workflow',
  'wrong-event',
  'wrong-run-sha',
  'wrong-conclusion',
  'wrong-repo',
  // Review W4 (2026-10-23): the gate follows the current gpu-vast triggers.
  'valid-none',
  'dispatch-run',
  'pr-target-run',
  'wrong-branch',
  'not-needed',
  'no-description',
])
  test('R3 release same-SHA provenance gate: ' + name, () => {
    const p = spawnSync(
      'python3',
      ['-I', new URL('round3_release_fixture.py', import.meta.url).pathname, name],
      { encoding: 'utf8', timeout: 5000 },
    );
    assert.equal(p.status, 0, p.stderr + p.stdout);
  });
