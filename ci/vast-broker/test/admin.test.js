import test from 'node:test';
import assert from 'node:assert/strict';
import { BrokerLedger, createGateway, createService } from '../src/index.js';
import { env, MemoryStorage, setup } from './helpers.js';

// FALSIFIER: any public admin request reaches storage, any non-summary field escapes,
// or an internal stop bypasses the existing owner's cleanup/absence-verification path.
const internal = (path, body, origin = 'https://internal.invalid') =>
  new Request(origin + '/admin/' + path, {
    method: body === undefined ? 'GET' : 'POST',
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
const record = (key, started, patch = {}) => ({
  key,
  sha: 'a'.repeat(40),
  branch: 'master',
  profile: 'check',
  die: 'AD104',
  gpu: 'RTX_4070',
  machineId: 12,
  hostId: 13,
  id: 14,
  phase: 'TESTING',
  verdict: null,
  started,
  ended: null,
  firstReportAt: started + 42000,
  computeStoragePrice: 0.5,
  bandwidthCost: 0.01,
  nonce: 'PRIVATE-SENTINEL',
  log: 'PRIVATE-SENTINEL',
  runnerNonce: 'PRIVATE-SENTINEL',
  tokens: { secret: 'PRIVATE-SENTINEL' },
  ...patch,
});
const fields = [
  'run',
  'sha',
  'branch',
  'profile',
  'die',
  'gpu_model',
  'machine_id',
  'host_id',
  'instance_id',
  'state',
  'phase',
  'verdict',
  'cost_estimate_usd',
  'started_at',
  'ended_at',
  'boot_seconds',
].sort();

for (const path of ['list', 'stop']) {
  test('admin public gateway hides ' + path, async () => {
    let calls = 0;
    const gateway = createGateway({ authorize: async () => calls++ });
    const response = await gateway.fetch(internal(path), {
      ...env,
      LEDGER: {
        idFromName: () => 1,
        get: () => {
          calls++;
          throw Error();
        },
      },
    });
    assert.equal(response.status, 404);
    assert.equal(calls, 0);
  });
  test('admin public gateway hides prefixed ' + path, async () => {
    let calls = 0;
    const prefix = 'p'.repeat(32);
    const gateway = createGateway({ authorize: async () => calls++ });
    const response = await gateway.fetch(
      new Request('https://broker.example/' + prefix + '/admin/' + path),
      { ...env, BROKER_PATH_PREFIX: prefix },
    );
    assert.equal(response.status, 404);
    assert.equal(calls, 0);
  });
  test('admin DO rejects a public origin for ' + path, async () => {
    const object = new BrokerLedger({ storage: new MemoryStorage() }, env);
    object.service = {
      fetch: () => {
        throw Error('must not forward');
      },
    };
    const response = await object.fetch(internal(path, undefined, 'https://broker.example'));
    assert.equal(response.status, 404);
  });
  test('admin DO rejects HTTP internal hostname for ' + path, async () => {
    const object = new BrokerLedger({ storage: new MemoryStorage() }, env);
    object.service = {
      fetch: () => {
        throw Error('must not forward');
      },
    };
    const response = await object.fetch(internal(path, undefined, 'http://internal.invalid'));
    assert.equal(response.status, 404);
  });
}

test('admin list includes active and ended runs, newest first, exact whitelist', async () => {
  const storage = new MemoryStorage();
  const older = record('999-1', 1000, { ended: 61000, verdict: 'PASS' });
  await storage.put('record:999-1', older);
  await storage.put('record:1000-1', record('1000-1', 2000));
  await storage.put('log:1000-1:0', 'PRIVATE-SENTINEL');
  const object = new BrokerLedger({ storage }, env);
  const response = await object.fetch(internal('list'));
  assert.equal(response.status, 200);
  const data = await response.json();
  assert.deepEqual(Object.keys(data), ['runs']);
  assert.deepEqual(
    data.runs.map((r) => r.run),
    ['1000-1', '999-1'],
  );
  for (const row of data.runs) assert.deepEqual(Object.keys(row).sort(), fields);
  assert.equal(data.runs[0].boot_seconds, 42);
  assert.equal(data.runs[1].state, 'NONE');
  assert.equal(data.runs[1].phase, 'NONE');
  assert.equal(data.runs[1].cost_estimate_usd, 0.5 / 60 + 0.01);
  assert.equal(JSON.stringify(data).includes('PRIVATE-SENTINEL'), false);
});

test('admin list caps at 100 newest records and excludes nested/non-scalar values', async () => {
  const storage = new MemoryStorage();
  for (let i = 1; i <= 120; i++)
    await storage.put('record:' + i + '-1', record(i + '-1', i * 1000));
  await storage.put(
    'record:120-1',
    record('120-1', 120000, {
      gpu: { token: 'PRIVATE-SENTINEL' },
      branch: { token: 'PRIVATE-SENTINEL' },
      verdict: { token: 'PRIVATE-SENTINEL' },
      hardware: { gpu_name: { token: 'PRIVATE-SENTINEL' } },
    }),
  );
  const response = await new BrokerLedger({ storage }, env).fetch(internal('list'));
  assert.equal(response.status, 200);
  const data = await response.json();
  assert.equal(data.runs.length, 100);
  assert.equal(data.runs[0].run, '120-1');
  assert.equal(data.runs.at(-1).run, '21-1');
  assert.equal(JSON.stringify(data).includes('PRIVATE-SENTINEL'), false);
  for (const row of data.runs) assert.deepEqual(Object.keys(row).sort(), fields);
});

test('admin list refuses a truncated ledger instead of claiming newest records', async () => {
  const storage = new MemoryStorage();
  for (let i = 1; i <= 1000; i++) await storage.put('record:' + i, record(String(i), i));
  assert.equal((await new BrokerLedger({ storage }, env).fetch(internal('list'))).status, 503);
});

for (const body of [{ run: '123-1' }, { all: true }]) {
  const label = body.all ? 'all' : 'one';
  test('admin stop forwards exactly once to owner route: ' + label, async () => {
    const bindings = { ...env, BROKER_PATH_PREFIX: 'p'.repeat(32) };
    const object = new BrokerLedger({ storage: new MemoryStorage() }, bindings);
    const calls = [];
    object.service = {
      fetch: async (request) => {
        calls.push(request);
        return Response.json({ state: 'NONE', log: 'PRIVATE-SENTINEL' });
      },
    };
    const response = await object.fetch(internal('stop', body));
    assert.equal(response.status, 200);
    assert.deepEqual(await response.json(), { state: 'NONE' });
    assert.equal(calls.length, 1);
    assert.equal(
      new URL(calls[0].url).pathname,
      '/' + bindings.BROKER_PATH_PREFIX + (body.all ? '/stop-all' : '/stop/123-1'),
    );
    assert.equal(calls[0].method, 'POST');
    assert.equal(calls[0].headers.get('Authorization'), 'Bearer ' + env.OWNER_STOP_TOKEN);
    assert.equal(await calls[0].text(), '');
  });

  test('admin stop preserves actual owner cleanup and settlement: ' + label, async () => {
    const h = await setup();
    assert.equal((await h.call()).status, 200);
    const object = new BrokerLedger({ storage: h.storage }, h.bindings);
    object.service = createService({
      env: h.bindings,
      storage: h.storage,
      fetch: h.cloud.fetch,
      now: () => h.time.value,
    });
    h.cloud.calls.length = 0;
    assert.equal((await object.fetch(internal('stop', body))).status, 200);
    const saved = await h.storage.get('record:123-1');
    assert.equal(saved.closed, 'STOPPED');
    assert.equal(saved.ended, h.time.value);
    assert.equal(saved.liveCheckedAt, null);
    assert.equal((await h.storage.get('cost:123-1')).ended, h.time.value);
    assert.equal(await h.storage.get('open:123-1'), undefined);
    assert.deepEqual(h.cloud.deletes, ['900']);
    const calls = h.cloud.calls;
    const deletion = calls.findIndex((c) => c.method === 'DELETE');
    assert.ok(deletion >= 0);
    assert.ok(
      calls
        .slice(deletion + 1)
        .some((c) => c.method === 'GET' && new URL(c.url).pathname === '/api/v1/instances/'),
    );
  });

  test('admin stop cannot settle when provider deletion fails: ' + label, async () => {
    const h = await setup();
    await h.call();
    h.cloud.vastError = 503;
    const object = new BrokerLedger({ storage: h.storage }, h.bindings);
    object.service = createService({ env: h.bindings, storage: h.storage, fetch: h.cloud.fetch });
    assert.equal((await object.fetch(internal('stop', body))).status, 503);
    assert.equal((await h.storage.get('record:123-1')).ended, null);
    assert.equal((await h.storage.get('record:123-1')).closed, 'STOPPED');
    assert.equal(await h.storage.get('open:123-1'), true);
  });
}

for (const body of [
  {},
  { all: false },
  { run: '../cron' },
  { all: true, run: '123-1' },
  { run: '123-1', extra: true },
  { run: '0-1' },
  null,
  [],
])
  test('admin stop rejects malformed selection ' + JSON.stringify(body), async () => {
    const object = new BrokerLedger({ storage: new MemoryStorage() }, env);
    object.service = {
      fetch: () => {
        throw Error('must not forward');
      },
    };
    assert.equal((await object.fetch(internal('stop', body))).status, 400);
  });

for (const path of ['list', 'stop'])
  test('admin rejects wrong method/query ' + path, async () => {
    const object = new BrokerLedger({ storage: new MemoryStorage() }, env);
    object.service = {
      fetch: () => {
        throw Error('must not forward');
      },
    };
    const wrong = internal(path, path === 'list' ? {} : undefined);
    assert.equal((await object.fetch(wrong)).status, 404);
    assert.equal((await object.fetch(internal(path + '?leak=1', {}))).status, 404);
  });
