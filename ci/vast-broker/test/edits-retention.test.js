import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, nonce, env } from './helpers.js';
import { configuration } from '../src/policy.js';
import { DEFAULTS } from '../src/defaults.js';
// Owner ruling 2026-10-24 items 2 and 4: the log relay is not persistent. FALSIFIER: a log page
// is unreadable during the run or its grace, any per-run log/record key survives
// LOG_RETENTION_HOURS after ended_at, the spend/sha-gap rows are lost with it, or the ledger
// grows with every rental.
const HOUR = 3600000;
const runKeys = (h, key) =>
  [...h.storage.data.keys()].filter(
    (k) =>
      k === 'record:' + key ||
      k === 'run:' + key ||
      k === 'ended:' + key ||
      k.startsWith('log:' + key + ':') ||
      k.startsWith('log-hash:' + key + ':'),
  );
async function finished(overrides = {}) {
  const h = await setup(overrides);
  assert.equal((await h.call()).status, 200);
  assert.equal((await h.report({ seq: 1, kind: 'log', text: 'hello box\n' })).status, 200);
  const page = async () => h.call('/logs/123-1?cursor=0', { method: 'GET' });
  assert.match((await (await page()).json()).log_increment_sanitised, /hello box/);
  const result = { seq: 2, kind: 'result', result: { nonce, verdict: 'PASS' } };
  assert.equal((await h.report(result)).status, 200);
  assert.equal((await (await h.call('/status/123-1')).json()).state, 'GRACE');
  assert.match((await (await page()).json()).log_increment_sanitised, /hello box/);
  h.time.value += 6 * 60000;
  await h.service.sweep();
  const ended = (await h.storage.get('record:123-1')).ended;
  assert.equal(typeof ended, 'number');
  return { h, page, ended };
}
test('item 2: logs readable in run, grace and retention, gone after 6 h', async () => {
  const { h, page, ended } = await finished();
  assert.equal(DEFAULTS.logRetentionHours, 6);
  h.time.value = ended + 6 * HOUR - 60000;
  await h.service.sweep();
  assert.equal((await page()).status, 200);
  assert.equal((await h.call('/status/123-1')).status, 200);
  h.time.value = ended + 6 * HOUR;
  await h.service.sweep();
  assert.equal((await page()).status, 404);
  assert.equal((await h.call('/status/123-1')).status, 404);
  assert.deepEqual(runKeys(h, '123-1'), []);
  assert.equal(await h.storage.get('id:900'), undefined);
});
test('item 2: retention keeps the spend row and the sha gap', async () => {
  const { h, ended } = await finished();
  h.time.value = ended + 6 * HOUR;
  await h.service.sweep();
  assert.ok(await h.storage.get('cost:123-1'));
  assert.ok(await h.storage.get('sha:' + 'a'.repeat(40)));
  h.cloud.run.run_attempt = 2;
  const again = await h.call('/rent', { claims: { run_attempt: '2' } });
  assert.equal(again.status, 409);
  assert.equal(h.cloud.creates, 1);
});
for (const hours of ['1', '48'])
  test('item 2: LOG_RETENTION_HOURS=' + hours + ' bounds the retention', async () => {
    const { h, page, ended } = await finished({ LOG_RETENTION_HOURS: hours });
    h.time.value = ended + Number(hours) * HOUR - 60000;
    await h.service.sweep();
    assert.equal((await page()).status, 200);
    h.time.value = ended + Number(hours) * HOUR;
    await h.service.sweep();
    assert.equal((await page()).status, 404);
  });
for (const value of ['0', '49', '1.5', '-1', 'six', ''])
  test('item 2: invalid LOG_RETENTION_HOURS ' + JSON.stringify(value) + ' is named', async () => {
    assert.throws(
      () => configuration({ ...env, LOG_RETENTION_HOURS: value }),
      (e) => e.state === 'CONFIG_VAR:LOG_RETENTION_HOURS',
    );
    const h = await setup({ LOG_RETENTION_HOURS: value });
    const response = await h.call();
    assert.equal(response.status, 503);
    assert.equal((await response.json()).state, 'CONFIG_VAR:LOG_RETENTION_HOURS');
    assert.equal(h.cloud.creates, 0);
  });
test('item 2: the 4 MiB per-run log cap stays', () => {
  assert.equal(DEFAULTS.reportBytes, 4194304);
  assert.throws(() => configuration({ ...env, MAX_REPORT_BYTES: '4194305' }));
});
test('item 2: the ledger does not grow with every rental (40 days, one run a day)', async () => {
  const h = await setup();
  const counts = [];
  for (let day = 1; day <= 40; day++) {
    const id = 1000 + day,
      key = id + '-1',
      head = day.toString(16).padStart(40, '0');
    Object.assign(h.cloud.run, { id, head_sha: head });
    h.cloud.run.pull_requests = [{ number: 7, head: { sha: head } }];
    const rented = await h.call('/rent', { claims: { run_id: String(id) } });
    assert.equal(rented.status, 200, 'day ' + day);
    const text = 'x'.repeat(1000) + '\n';
    assert.equal((await h.report({ seq: 1, kind: 'log', text }, { run: key })).status, 200);
    const result = { seq: 2, kind: 'result', result: { nonce, verdict: 'PASS' } };
    assert.equal((await h.report(result, { run: key })).status, 200);
    h.time.value += 6 * 60000;
    await h.service.sweep();
    h.time.value += 24 * HOUR - 6 * 60000;
    await h.service.sweep();
    assert.deepEqual(runKeys(h, key), [], 'day ' + day);
    counts.push(h.storage.data.size);
  }
  // Only the 30-day spend rows and sha-gap rows remain, each bounded by the rolling window.
  const prefixes = [...h.storage.data.keys()].map((k) => k.split(':')[0]);
  assert.ok(prefixes.filter((p) => p === 'cost').length <= 31, String(counts));
  assert.ok(prefixes.filter((p) => p === 'sha').length <= 31, String(counts));
  assert.ok(counts.at(-1) <= counts[31], String(counts));
  assert.ok(counts.at(-1) < 2 * 31 + 16, String(counts));
});
