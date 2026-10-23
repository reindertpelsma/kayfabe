import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { setup, clock, nonce, callbackToken } from './helpers.js';
// FALSIFIER H2: infrastructure status faults destroy paid work or polling bypasses fact caching.
for (const failure of [
  'BUSY',
  'RATE_LIMITED',
  'UNAVAILABLE',
  'exhausted',
  'exhausted-rate',
  'malformed-503',
  'malformed-403',
]) {
  test('round runner retries status without destructive cleanup ' + failure, () => {
    const r = spawnSync('python3', ['ci/vast-broker/test/round_runner_fixture.py', failure], {
      encoding: 'utf8',
      timeout: 5000,
    });
    assert.equal(r.status, 0, r.stdout + r.stderr);
  });
}
for (const profile of ['pr', 'branch']) {
  test('round lifecycle reads share bounded GitHub cache ' + profile, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '180' });
    const claims = {};
    if (profile === 'branch') {
      h.cloud.run.event = 'workflow_dispatch';
      h.cloud.run.head_branch = 'master';
      h.cloud.run.pull_requests = [];
      h.cloud.prs = [];
      claims.event_name = 'workflow_dispatch';
      claims.ref = 'refs/heads/master';
    }
    assert.equal((await h.call('/rent', { claims })).status, 200);
    await h.report({ seq: 1, kind: 'progress', text: 'running' });
    const before = h.cloud.calls.filter((x) => x.url.includes('api.github.com')).length;
    for (let i = 0; i < 180; i++) {
      h.time.value = clock + i * 20000;
      const r = await h.storage.get('record:123-1');
      r.lastPostAt = h.time.value;
      await h.storage.put('record:123-1', r);
      assert.equal((await h.call('/status/123-1', { claims })).status, 200);
    }
    const after = h.cloud.calls.filter((x) => x.url.includes('api.github.com')).length;
    assert.ok(after - before <= (profile === 'pr' ? 120 : 60), String(after - before));
  });
}
// FALSIFIER: quota exhaustion still buys upstream inspections; full logs discard control state.
for (const quota of ['run', 'day']) {
  test('round exhausted blob scope performs no provider inspection ' + quota, async () => {
    const h = await setup({
      BLOB_REPO: 'example/private',
      BLOB_GITHUB_TOKEN: 'mock-blob',
      BLOB_ALLOWED_TAG_PREFIXES: '["release-"]',
    });
    await h.call();
    const r = await h.storage.get('record:123-1');
    if (quota === 'run') r.blobMints = 8;
    else await h.storage.put('blob-day:' + Math.floor(clock / 86400000), 64);
    await h.storage.put('record:123-1', r);
    const token = callbackToken(r);
    const before = h.cloud.inspected.length;
    for (let i = 0; i < 5; i++) {
      const result = await h.call('/blob-url/123-1/release-v1/asset', {
        method: 'GET',
        token,
      });
      assert.equal(result.status, 429);
    }
    assert.equal(h.cloud.inspected.length, before);
  });
}
for (const action of ['hold', 'stop-timer']) {
  test('round full log cannot discard manual control state ' + action, async () => {
    const h = await setup();
    await h.call();
    await h.report({ seq: 1, kind: 'result', result: { nonce, verdict: 'PASS' } });
    const r = await h.storage.get('record:123-1');
    r.fullBytes = 4194304;
    if (action === 'stop-timer') {
      r.phase = 'MANUAL';
      r.manualAt = clock;
    }
    await h.storage.put('record:123-1', r);
    const result = await h.call('/' + action + '/123-1', {
      token: action === 'hold' ? callbackToken(r) : h.bindings.OWNER_STOP_TOKEN,
    });
    assert.equal(result.status, 200);
    const saved = await h.storage.get('record:123-1');
    if (action === 'hold') assert.equal(saved.phase, 'MANUAL');
    else assert.equal(saved.hardDeadline, null);
  });
}
// FALSIFIER: lower observed contract prices erase already reserved/billed liability.
for (const kind of ['compute', 'network'])
  test('round contract decrease never releases historical liability ' + kind, async () => {
    const h = await setup();
    h.cloud.offer.inet_up_cost = 0.001;
    h.cloud.offer.inet_down_cost = 0.001;
    await h.call();
    const before = await h.storage.get('cost:123-1');
    h.time.value += 5 * 60000;
    const instance = h.cloud.instances.get('900');
    if (kind === 'compute') Object.assign(instance, { dph_base: 0.1, dph_total: 0.12 });
    else Object.assign(instance, { inet_up_cost: 0.00001, inet_down_cost: 0.00001 });
    assert.equal((await h.call('/status/123-1')).status, 200);
    const after = await h.storage.get('cost:123-1');
    assert.ok(after.price >= before.price);
    assert.ok(after.bandwidthCost >= before.bandwidthCost);
  });
