import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { configuration } from '../src/policy.js';
import { repoSource, helperProbe } from './repo-onstart.js';
import { setup, env, nonce, sha, callbackToken } from './helpers.js';
// FALSIFIERS: early result deletion; cross-run kill; ACK-only settlement;
// reserve excludes cron; pending boots destroyed before provisioning bound.
const settings = { HARD_RUNTIME_MINUTES: '20', BOOT_LIMIT_MINUTES: '10' };
async function rent(extra = {}) {
  const h = await setup({ ...settings, ...extra });
  assert.equal((await h.call()).status, 200);
  return h;
}
const record = (h) => h.storage.get('record:123-1');
const result = (verdict) => ({ seq: 1, kind: 'result', result: { nonce, verdict } });
async function kill(h, { run = '123-1', token } = {}) {
  const r = await record(h);
  return h.call('/kill/' + run, { token: token ?? callbackToken(r), body: { reason: 'done' } });
}
for (const verdict of ['PASS', 'FAIL', 'UNAVAILABLE'])
  test('V4 ' + verdict + ' result retains owned box until grace cron', async () => {
    const h = await rent();
    assert.equal((await h.report(result(verdict))).status, 200);
    assert.deepEqual(h.cloud.deletes, []);
    let r = await record(h);
    assert.equal(r.resultAt, h.time.value);
    assert.equal(r.ended, null);
    h.cloud.run.status = 'completed';
    h.time.value += (verdict === 'PASS' ? 300000 : 600000) - 1;
    await h.service.sweep();
    assert.deepEqual(h.cloud.deletes, []);
    h.time.value++;
    await h.service.sweep();
    assert.deepEqual(h.cloud.deletes, ['900']);
    r = await record(h);
    assert.equal(r.verdict, verdict);
    assert.notEqual(r.ended, null);
  });
for (const verdict of ['PASS', 'FAIL'])
  test('V4 own box kill bypasses ' + verdict + ' grace and verifies absence', async () => {
    const h = await rent();
    await h.report(result(verdict));
    const r = await record(h),
      token = callbackToken(r);
    const before = h.cloud.inspected.length;
    assert.equal((await kill(h, { token })).status, 200);
    assert.ok(h.cloud.inspected.length >= before + 2);
    assert.equal(h.cloud.instances.has('900'), false);
    assert.notEqual((await record(h)).ended, null);
    assert.equal((await record(h)).verdict, verdict);
    assert.equal((await h.report({ seq: 2, kind: 'log', text: 'dead' }, { token })).status, 401);
    assert.equal((await kill(h, { token })).status, 200);
    assert.deepEqual(h.cloud.deletes, ['900']);
  });
for (const mode of ['before-result', 'restart'])
  test('V4 box kill ' + mode + ' never invents PASS', async () => {
    const h = await rent();
    if (mode === 'restart') h.restart();
    assert.equal((await kill(h)).status, 200);
    const r = await record(h);
    assert.equal(r.verdict, 'FAIL');
    assert.equal(r.closed, 'STOPPED');
    assert.notEqual(r.ended, null);
  });
for (const run of ['124-1', '999-1'])
  test('V4 capability cannot kill foreign run ' + run, async () => {
    const h = await rent(),
      r = await record(h);
    await h.storage.put('record:' + run, {
      ...r,
      key: run,
      run: run.split('-')[0],
      label: 'kf-ci-' + run,
      id: '901',
    });
    h.cloud.instances.set('901', { id: 901, label: 'kf-ci-' + run, actual_status: 'running' });
    assert.equal((await kill(h, { run })).status, 401);
    assert.deepEqual(h.cloud.deletes, []);
  });
for (const endpoint of ['stop', 'kill'])
  test('V4 ' + endpoint + ' retains liability until DELETE absence confirmed', async () => {
    const h = await rent(),
      original = h.cloud.fetch;
    h.cloud.fetch = async (u, i) =>
      i?.method === 'DELETE' ? Response.json({ success: true }) : original(u, i);
    h.restart();
    const response =
      endpoint === 'stop'
        ? await h.call('/stop/123-1', { token: env.OWNER_STOP_TOKEN })
        : await kill(h);
    assert.equal(response.status, 503);
    assert.equal((await record(h)).ended, null);
    assert.ok(await h.storage.get('open:123-1'));
    h.cloud.fetch = original;
    h.restart();
    await h.service.sweep();
    assert.notEqual((await record(h)).ended, null);
  });
for (const elapsed of [599999, 600000])
  test('V4 pending provision bound ' + elapsed, async () => {
    const h = await rent();
    h.cloud.instances.get('900').actual_status = 'created';
    h.time.value += elapsed;
    await h.service.sweep();
    assert.deepEqual(h.cloud.deletes, elapsed < 600000 ? [] : ['900']);
  });
test('V4 accepted callback ends provisioning deadline', async () => {
  const h = await rent();
  await h.report({ seq: 1, kind: 'progress', text: 'started' });
  h.time.value += 600000;
  await h.service.sweep();
  assert.deepEqual(h.cloud.deletes, []);
  h.time.value += 600000;
  await h.service.sweep();
  assert.deepEqual(h.cloud.deletes, ['900']);
});
test('V4 kill shares report rate limiter and cannot extend grace', async () => {
  const h = await rent({ REPORT_RATE_LIMIT: '1' });
  await h.report({ seq: 1, kind: 'progress', text: 'started' });
  assert.equal((await kill(h)).status, 429);
  assert.deepEqual(h.cloud.deletes, []);
  h.time.value += 10000;
  assert.equal((await kill(h)).status, 200);
});
test('V4 default bounds and cron liability', () => {
  const cfg = configuration({
    ...env,
    HARD_RUNTIME_MINUTES: undefined,
    BOOT_LIMIT_MINUTES: undefined,
  });
  assert.equal(cfg.ttl, 180 * 60000);
  assert.equal(cfg.provision, 40 * 60000);
  assert.equal(cfg.grace, 5 * 60000);
  assert.equal(cfg.cronPeriod, 60000);
  for (const patch of [
    { HARD_RUNTIME_MINUTES: '10' },
    { PASS_GRACE_MINUTES: '1441' },
    { BOOT_LIMIT_MINUTES: '1441' },
  ])
    assert.throws(() => configuration({ ...env, HARD_RUNTIME_MINUTES: '20', ...patch }));
});
test('V4 prepaid reserve includes hard TTL plus cron period', async () => {
  const h = await setup(settings);
  h.cloud.price = 0.5;
  h.cloud.credit = 8 + (0.5 * 20.5) / 60;
  assert.equal((await h.call()).status, 409);
  assert.equal(h.cloud.creates, 0);
  h.cloud.credit = 8 + (0.5 * 21) / 60 + 0.00001;
  assert.equal((await h.call()).status, 200);
  assert.equal((await h.storage.get('cost:123-1')).ttl, 21 * 60000);
});
test('V4 onstart ships plain curl own-run kill CLI and test-only timeout', () => {
  const entry = repoSource('vast_onstart.sh'),
    kill = repoSource('kf-box-kill');
  assert.match(kill, /\/kill\/\$\{KF_RUN_ID/);
  assert.match(kill, /--proto '=https'/);
  assert.match(kill, /--max-redirs 0/);
  assert.match(entry, /timeout --signal=TERM/);
  assert.match(entry, /bash "\$payload" "\$KF_SHA" "\$KF_NONCE"/);
  assert.match(repoSource('vast_report.py'), /ack.get\("grace_remaining_seconds"\)/);
});
test('V4 free-plan cron is every minute and has no DO alarm', async () => {
  assert.match(
    await readFile(new URL('../wrangler.toml', import.meta.url), 'utf8'),
    /crons = \["\* \* \* \* \*"\]/,
  );
  const source = await readFile(new URL('../src/index.js', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /setAlarm|deleteAlarm|async alarm/);
});
for (const verdict of ['PASS', 'FAIL'])
  test('V4 grace cannot extend hard TTL ' + verdict, async () => {
    const h = await rent();
    await h.report({ seq: 1, kind: 'progress', text: 'started' });
    h.time.value += 19 * 60000;
    await h.report({ seq: 2, kind: 'result', result: { nonce, verdict } });
    h.time.value += 60000;
    await h.service.sweep();
    assert.deepEqual(h.cloud.deletes, ['900']);
  });
for (const bad of ['bad-capability', env.OWNER_STOP_TOKEN])
  test('V4 kill rejects non-box authority ' + bad.slice(0, 3), async () => {
    const h = await rent();
    assert.equal((await kill(h, { token: bad })).status, 401);
    assert.deepEqual(h.cloud.deletes, []);
  });
for (const action of ['report', 'cron'])
  test('V4 pending callback cannot establish liveness ' + action, async () => {
    const h = await rent();
    h.cloud.instances.get('900').actual_status = 'created';
    if (action === 'report')
      assert.equal((await h.report({ seq: 1, kind: 'progress', text: 'not proof' })).status, 409);
    else await h.service.sweep();
    assert.equal((await record(h)).firstReportAt, undefined);
    assert.deepEqual(h.cloud.deletes, []);
  });
test('V4 no callback while provider running also expires provisioning', async () => {
  const h = await rent();
  h.time.value += 600000;
  await h.service.sweep();
  assert.deepEqual(h.cloud.deletes, ['900']);
});
test('V4 low-priced reservation still includes cron period', async () => {
  const h = await setup(settings);
  h.cloud.price = 0.2;
  h.cloud.credit = 8 + (0.2 * 20.5) / 60;
  assert.equal((await h.call()).status, 409);
  assert.equal(h.cloud.creates, 0);
});
for (const patch of [{ HARD_RUNTIME_MINUTES: '10' }, { PASS_GRACE_MINUTES: '1441' }])
  test('V4 invalid lifetime knob prevents rent ' + Object.keys(patch)[0], async () => {
    const h = await setup({ ...settings, ...patch });
    assert.equal((await h.call()).status, 503);
    assert.equal(h.cloud.creates, 0);
  });
for (const reason of ['done', 'a reason with spaces'])
  test('V4 generated kill CLI sends limited capability ' + reason, async () => {
    const r = await helperProbe('kf-box-kill', [reason]);
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.args.at(-1), 'https://broker.example/kill/123-1');
    assert.ok(r.args.includes(reason));
    assert.ok(!r.args.includes('-L'));
    assert.ok(r.stdin.includes('Authorization: Bearer v1.123.1.'));
    assert.ok(!r.args.join(' ').includes('v1.123.1.'));
  });
test('V4 hard TTL survives restart with accepted progress and no result', async () => {
  const h = await rent();
  await h.report({ seq: 1, kind: 'progress', text: 'started' });
  h.restart();
  h.time.value += 1200000;
  await h.service.sweep();
  assert.deepEqual(h.cloud.deletes, ['900']);
});
