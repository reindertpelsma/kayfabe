import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { onstart } from './repo-onstart.js';
import { sha, nonce, env, clock } from './helpers.js';
// FALSIFIER: creation-time expiry executes work or fails with a shell syntax
// error instead of the required timeout exit; startup delay must consume time.
for (const elapsed of [173, 181])
  test('V5 expired creation soft deadline exits timeout ' + elapsed, () => {
    const source = onstart(sha, nonce, 180, env.REPOSITORY, {
      url: env.WORKER_URL,
      run: '123-1',
      token: 'v1.123.1.1800030000.' + 'A'.repeat(43),
      runnerNonce: 'f'.repeat(64),
      softDeadline: clock + 170 * 60000,
    });
    const python = source
      .split("python3 - <<'KF_REPORT'\n")[1]
      .split('post("progress", "Suite starting")')[0];
    const r = spawnSync(
      'python3',
      [
        '-I',
        '-c',
        "import sys,time;time.time=lambda:float(sys.argv[2]);scope={};exec(sys.argv[1],scope);print(scope['work'],end='')",
        python,
        String(clock / 1000 + elapsed * 60),
      ],
      { encoding: 'utf8' },
    );
    assert.equal(r.status, 0, r.stderr);
    const shell = spawnSync('bash', ['-s'], { input: r.stdout, encoding: 'utf8' });
    assert.equal(shell.status, 124, shell.stderr);
    assert.equal(shell.stdout, '');
  });
