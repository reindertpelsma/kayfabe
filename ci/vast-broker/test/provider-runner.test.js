import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
for (const scenario of ['pass', 'fail', 'fault', 'reported', 'outage', 'waiting', 'failed-hash'])
  for (const variant of [1, 2])
    test('runner provider ' + scenario + ' ' + variant, () => {
      const result = spawnSync(
        'python3',
        [
          '-I',
          fileURLToPath(new URL('./provider_runner_fixture.py', import.meta.url)),
          scenario,
          String(variant),
        ],
        { encoding: 'utf8', timeout: 30000 },
      );
      assert.equal(result.status, 0, result.stderr || result.stdout);
    });

// The runner's random stop-commands token may contain the port digits; that is not a leak.
// FALSIFIER: the fixture fails when the random token happens to contain the port.
for (const variant of [1, 2])
  test('runner provider fail ' + variant + ' with a random token that contains the port', () => {
    const result = spawnSync(
      'python3',
      [
        '-I',
        fileURLToPath(new URL('./provider_runner_fixture.py', import.meta.url)),
        'fail',
        String(variant),
        'token-has-port',
      ],
      { encoding: 'utf8', timeout: 30000 },
    );
    assert.equal(result.status, 0, result.stderr || result.stdout);
  });
