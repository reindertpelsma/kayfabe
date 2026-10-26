import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { Ledger } from '../src/ledger.js';
import { configuration } from '../src/policy.js';
import { env, clock, MemoryStorage, setup } from './helpers.js';
// FALSIFIER WK1: valid JSON 5xx exits silently, or an ended legacy no-instance record is
// counted merely because the cost receipt lacks created=false or retains an old price.
for (const [name, phase, http] of [
  ['availability-503', 'availability', '5xx'],
  ['availability-502', 'availability', '5xx'],
  ['availability-shape', 'availability', '2xx'],
  ['availability-json', 'availability', '2xx'],
  ['availability-runtime', 'availability', 'none'],
  ['rent-refusal', 'rent', '4xx'],
  ['rent-503', 'rent', '5xx'],
  ['rent-502', 'rent', '5xx'],
  ['no-offers', 'rent', '4xx'],
  ['no-preferences', 'rent', '4xx'],
  ['bundle', 'rent', '2xx'],
  ['failed-result', 'result', '2xx'],
  ['manifest', 'manifest', 'none'],
  ['preferences', 'preferences', 'none'],
  ['oidc-json', 'oidc', 'none'],
  ['oidc-shape', 'oidc', 'none'],
  ['oidc-timeout', 'oidc', 'none'],
  ['output-pass', 'output', 'none'],
  ['summary', 'summary', 'none'],
  ['missing-summary', 'setup', 'none'],
  ['output', 'rent', '4xx'],
])
  test('silent nonzero controller emits one trusted diagnosis ' + name, () => {
    const p = spawnSync(
      'python3',
      ['-I', new URL('./silent_runner_fixture.py', import.meta.url).pathname, name],
      { encoding: 'utf8', timeout: 10000 },
    );
    assert.equal(p.status, 0, p.stderr);
    const r = JSON.parse(p.stdout);
    assert.equal(r.status, 1);
    const lines = r.stdout.split('\n').filter((x) => x.startsWith('runner: '));
    assert.equal(lines.length, 1, r.stdout);
    assert.ok(lines[0].length <= 1024);
    assert.match(lines[0], new RegExp('phase=' + phase + '\\b'));
    assert.match(lines[0], new RegExp('http=' + http + '\\b'));
    assert.match(lines[0], /state=[A-Z_]+/);
    assert.doesNotMatch(lines[0], /PRIVATE-SENTINEL|broker\.example|pppppppp|mock-token/);
    if (name === 'no-offers') assert.match(lines[0], /NO_ALLOWED_OFFERS/);
    if (name === 'no-preferences') assert.match(lines[0], /NO_ALLOWED_PREFERENCE/);
    if (name.startsWith('availability-5')) {
      assert.equal(r.calls.filter((x) => x === 'availability').length, 6);
      assert.equal(r.calls.includes('rent'), false);
    }
  });
for (const count of ['21', '40'])
  test('silent configuration diagnosis keeps current daily-count limit ' + count, async () => {
    assert.throws(() => configuration({ ...env, DAILY_RENTALS: count }));
    const h = await setup({ DAILY_RENTALS: count });
    assert.equal((await h.call('/availability?profile=check', { method: 'GET' })).status, 503);
    assert.equal(h.cloud.calls.length, 0);
  });
for (const id of [null, undefined])
  for (const price of [0, 0.5])
    test(`silent legacy count uses settled record proof ${id} ${price}`, async () => {
      const storage = new MemoryStorage(),
        ledger = new Ledger(storage);
      await storage.put('cost:100-1', {
        started: clock - 1000,
        ended: clock,
        price,
        ttl: 3600000,
        bandwidthCost: 0,
      });
      await storage.put('record:100-1', {
        key: '100-1',
        started: clock - 1000,
        ended: clock,
        closed: 'GONE',
        id,
        price: 0,
      });
      const before = [...(await storage.list())];
      const s = await ledger.budgetSummary(configuration(env), clock);
      assert.equal(s.profiles.check.count_today, 0);
      assert.equal(s.rolling_usd, 0);
      assert.deepEqual([...(await storage.list())], before);
      await ledger.budgets({ ...configuration(env), dailyCount: 1 }, clock, 0.1);
      assert.ok(await storage.get('cost:100-1'), 'the attempt receipt is retained');
    });
for (const patch of [
  { id: '900' },
  { id: '901' },
  { ended: null },
  { closed: 'UNAVAILABLE' },
  { closed: 'BOOT_TIMEOUT' },
  { started: 1 },
  { started: clock - 500 },
])
  test('silent legacy uncertain or real rental still counts ' + JSON.stringify(patch), async () => {
    const storage = new MemoryStorage(),
      ledger = new Ledger(storage);
    await storage.put('cost:100-1', {
      started: clock - 1000,
      ended: clock,
      price: 0.5,
      ttl: 3600000,
      bandwidthCost: 0,
    });
    await storage.put('record:100-1', {
      started: clock - 1000,
      ended: clock,
      closed: 'GONE',
      id: null,
      price: 0,
      ...patch,
    });
    const s = await ledger.budgetSummary(configuration(env), clock);
    assert.equal(s.profiles.check.count_today, 1);
    assert.ok(s.rolling_usd > 0);
  });

for (const bytes of [2000, 3000])
  test('silent diagnostic stays one bounded line ' + bytes, () => {
    const p = spawnSync(
      'python3',
      ['-I', new URL('./silent_runner_fixture.py', import.meta.url).pathname, 'bounded-' + bytes],
      { encoding: 'utf8' },
    );
    assert.equal(p.status, 0, p.stderr);
    const line = JSON.parse(p.stdout).line;
    assert.ok(line.length <= 1024);
    assert.equal(line.split('\n').length, 2);
    assert.match(line, /phase=availability state=UNAVAILABLE http=5xx status=503/);
  });
