import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// FALSIFIER: availability display becomes a rental decision, or provider-like
// IDs, unknown fields, coercions or command strings cross the BASE runner.
for (const name of [
  'good',
  'id-field',
  'command-name',
  'negative-price',
  'string-price',
  'bool-count',
  'bad-compute',
  'extra-threshold',
  'verified-off',
  'unknown-reason',
  'oversize',
  'cache-age',
])
  test('V5 hostile availability ' + name, () => {
    const p = spawnSync(
      'python3',
      ['-I', new URL('v5_availability_runner.py', import.meta.url).pathname, name],
      { encoding: 'utf8', timeout: 5000 },
    );
    assert.equal(p.status, 0, p.stderr + p.stdout);
  });
