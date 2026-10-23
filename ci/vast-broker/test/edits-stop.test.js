import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { setup, nonce } from './helpers.js';
// Owner ruling 2026-10-24 item 4: destroy the box as soon as the runner has the final result.
// FALSIFIER: the runner sends no stop after a verified final fetch, sends one after a final
// fetch that failed validation, lets a failed stop change the verdict, or the Worker keeps a
// GRACE box (or drops its verdict or log) when the run's own stop arrives.
for (const name of [
  'stop-after-verified-final',
  'stop-failure-is-ignored',
  'stop-not-after-invalid-final',
])
  test('item 4: runner ' + name, () => {
    const p = spawnSync(
      'python3',
      ['-I', new URL('edits_runner_fixture.py', import.meta.url).pathname, name],
      { encoding: 'utf8' },
    );
    assert.equal(p.status, 0, p.stderr + p.stdout);
  });
test('item 4: runner stop in GRACE destroys at once, keeps verdict and log', async () => {
  const h = await setup();
  assert.equal((await h.call()).status, 200);
  assert.equal((await h.report({ seq: 1, kind: 'log', text: 'final box line\n' })).status, 200);
  const result = { seq: 2, kind: 'result', result: { nonce, verdict: 'PASS' } };
  assert.equal((await h.report(result)).status, 200);
  const grace = await (await h.call('/status/123-1')).json();
  assert.equal(grace.state, 'GRACE');
  assert.equal(grace.cleanup_pending, true);
  h.time.value += 1000; // well inside the 5-minute pass grace
  const stopped = await h.call('/stop/123-1');
  assert.equal(stopped.status, 200);
  const data = await stopped.json();
  assert.equal(data.state, 'NONE');
  assert.equal(data.verdict, 'PASS');
  assert.equal(data.cleanup_pending, false);
  assert.equal(data.ended_at, h.time.value);
  assert.deepEqual(h.cloud.deletes, ['900']);
  const page = await h.call('/logs/123-1?cursor=0', { method: 'GET' });
  assert.equal(page.status, 200);
  assert.match((await page.json()).log_increment_sanitised, /final box line/);
  // Retention counts from ended_at, the stop time, not from the grace deadline.
  h.time.value = data.ended_at + 6 * 3600000;
  await h.service.sweep();
  assert.equal((await h.call('/logs/123-1?cursor=0', { method: 'GET' })).status, 404);
});
