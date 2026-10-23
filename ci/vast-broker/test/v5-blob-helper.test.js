import test from 'node:test';
import assert from 'node:assert/strict';
import { helperProbe, repoSource } from './repo-onstart.js';
for (const prefix of ['', 'x'.repeat(32)])
  test('blob helper scoped URL and HTTPS ' + prefix.length, async () => {
    const base = 'https://broker.example' + (prefix ? '/' + prefix : '');
    const r = await helperProbe('kf-box-blob-url', ['dummy-blobs-20261022', 'blob-a.txt'], {
      KF_BROKER_URL: base,
    });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(r.args.includes(base + '/blob-url/123-1/dummy-blobs-20261022/blob-a.txt'));
    assert.ok(r.args.includes('=https'));
    assert.ok(r.stdin.includes('Authorization: Bearer v1.123.1.'));
    assert.ok(!r.args.join(' ').includes('v1.'));
    assert.ok(!r.args.includes('--location'));
    assert.match(repoSource('vast_onstart.sh'), /install -m 700 "\$here\/kf-box-blob-url"/);
    for (const name of ['../bad', '%2f', '..'])
      assert.notEqual(
        (await helperProbe('kf-box-blob-url', ['tag', name], { KF_BROKER_URL: base })).status,
        0,
      );
  });
