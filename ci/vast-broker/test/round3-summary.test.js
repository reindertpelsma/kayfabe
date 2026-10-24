import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
for (const field of ['GPU', 'timing/cost'])
  test('R3 public runner summary retains bounded ' + field, () => {
    const p = spawnSync(
      'python3',
      ['-I', new URL('round3_main_fixture.py', import.meta.url).pathname, '{}'],
      { encoding: 'utf8', timeout: 5000 },
    );
    assert.equal(p.status, 0, p.stderr);
    const r = JSON.parse(p.stdout);
    assert.equal(r.status, 0);
    if (field === 'GPU') assert.match(r.summary, /RTX 4070/);
    else {
      assert.match(r.summary, /Elapsed seconds \| 1.000/);
      assert.match(r.summary, /Estimated USD \| 0.010000/);
    }
  });
