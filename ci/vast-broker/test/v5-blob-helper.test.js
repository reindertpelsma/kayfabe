import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { onstart } from './repo-onstart.js';
import { sha, nonce, env } from './helpers.js';
for (const prefix of ['', 'x'.repeat(32)])
  test('blob helper scoped URL and HTTPS ' + prefix.length, () => {
    const s = onstart(sha, nonce, 180, env.REPOSITORY, {
      url: env.WORKER_URL,
      run: '123-1',
      token: 'v1.123.1.1800030000.' + 'A'.repeat(43),
      path: prefix,
    });
    const helper = s.split("<<'KF_BLOB'\n")[1]?.split('\nKF_BLOB')[0];
    assert.ok(helper);
    const probe = helper.replace('exec curl', 'printf "%s\\n"');
    const r = spawnSync('sh', ['-s', 'dummy-blobs-20261022', 'blob-a.txt'], {
      input: probe,
      encoding: 'utf8',
    });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(
      r.stdout.includes(
        env.WORKER_URL +
          (prefix ? '/' + prefix : '') +
          '/blob-url/123-1/dummy-blobs-20261022/blob-a.txt',
      ),
    );
    assert.ok(r.stdout.includes('=https'));
    assert.ok(!r.stdout.includes('--location'));
    assert.ok(!s.includes(env.VAST_API_KEY));
    for (const name of ['../bad', '%2f', '..'])
      assert.notEqual(
        spawnSync('sh', ['-s', 'tag', name], { input: probe, encoding: 'utf8' }).status,
        0,
      );
  });
