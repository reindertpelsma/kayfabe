import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env, jwt, MockCloud } from '../test/helpers.js';

// FALSIFIER: caps, run-attempt/SHA identity or machine exclusion depend on a
// service closure's queue rather than a single DO storage transaction.
// All outbound traffic is intercepted; no network request can reach a provider.
const bundled = await build({
  entryPoints: [new URL('./fixture.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
const script = bundled.outputFiles[0].text;
const bindings = { ...env, BROKER_PATH_PREFIX: 'workerd-test-' + 'p'.repeat(32) };
const instant = Date.now();
const record = (i, patch = {}) => ({
  key: `${1000 + i}-1`,
  run: String(1000 + i),
  attempt: '1',
  sha: i.toString(16).padStart(40, '0'),
  branch: 'branch-' + i,
  profile: 'check',
  machineId: 200000 + i,
  started: instant,
  ended: null,
  price: 0.2,
  ttl: 1200000,
  ...patch,
});

async function runtime(t, extra = {}) {
  const cloud = new MockCloud();
  cloud.contracts = Array.from({ length: 20 }, (_, i) => 900 + i);
  const mf = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script,
      compatibilityDate: '2026-10-21',
      bindings: { ...bindings, ...extra },
      durableObjects: {
        LEDGER: { className: 'BrokerLedger', useSQLite: true },
        TEST_LEDGER: { className: 'AdmissionFixture', useSQLite: true },
      },
      outboundService: async (request) => {
        const run = request.headers.get('X-Test-Run');
        const copy = new MockCloud();
        const fetcher = copy.fetch;
        Object.assign(copy, cloud);
        if (run) {
          const n = Number(run);
          copy.offer = { ...cloud.offer, id: n, machine_id: 200000 + n };
          copy.run = cloud.runs.get(run);
        }
        const response = await fetcher(request.url, {
          method: request.method,
          headers: request.headers,
          ...(['GET', 'HEAD'].includes(request.method) ? {} : { body: await request.text() }),
        });
        return response;
      },
    }),
  );
  t.after(() => mf.dispose());
  const ns = await mf.getDurableObjectNamespace('TEST_LEDGER');
  const stub = ns.get(ns.idFromName(t.name));
  const call = (path, body) =>
    stub.fetch('https://test.invalid' + path, {
      method: body === undefined ? 'GET' : 'POST',
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
  return { mf, stub, cloud, call };
}

test('workerd admission is transactional without shared service state', async (t) => {
  for (const [name, config, candidates, expected, cost, credit] of [
    ['account slots', { maxConcurrent: 1 }, [record(1), record(2)], 'BUSY'],
    ['account slots four', { maxConcurrent: 2 }, [1, 2, 3, 4].map((i) => record(i)), 'BUSY'],
    [
      'profile slots',
      { maxConcurrent: 4, maxConcurrentProfile: { check: 1 } },
      [record(1), record(2)],
      'BUSY',
    ],
    [
      'profile slots three',
      { maxConcurrent: 4, maxConcurrentProfile: { check: 2 } },
      [record(1), record(2), record(3)],
      'BUSY',
    ],
    [
      'daily rentals',
      { maxConcurrent: 4, dailyCount: 1 },
      [record(1), record(2)],
      'UNAVAILABLE_BUDGET',
    ],
    [
      'daily rentals three',
      { maxConcurrent: 4, dailyCount: 2 },
      [record(1), record(2), record(3)],
      'UNAVAILABLE_BUDGET',
    ],
    [
      'daily spend',
      { maxConcurrent: 4, daily: 0.15 },
      [record(1), record(2)],
      'UNAVAILABLE_BUDGET',
      0.1,
    ],
    [
      'daily spend tight',
      { maxConcurrent: 4, daily: 0.16 },
      [record(1), record(2)],
      'UNAVAILABLE_BUDGET',
      0.1,
    ],
    [
      'rolling spend',
      { maxConcurrent: 4, rolling: 0.15 },
      [record(1), record(2)],
      'UNAVAILABLE_BUDGET',
      0.1,
    ],
    [
      'rolling spend tight',
      { maxConcurrent: 4, rolling: 0.16 },
      [record(1), record(2)],
      'UNAVAILABLE_BUDGET',
      0.1,
    ],
    [
      'credit reserve',
      { maxConcurrent: 4, reserve: 8 },
      [record(1), record(2)],
      'UNAVAILABLE_BUDGET',
      0.1,
      8.15,
    ],
    [
      'credit reserve tight',
      { maxConcurrent: 4, reserve: 8 },
      [record(1), record(2)],
      'UNAVAILABLE_BUDGET',
      0.1,
      8.16,
    ],
    [
      'same attempt',
      { maxConcurrent: 4 },
      [record(1), record(1, { sha: 'f'.repeat(40) })],
      'UNAVAILABLE_BUDGET',
    ],
    [
      'same attempt nonce',
      { maxConcurrent: 4 },
      [record(1), record(1, { runnerNonce: 'other', sha: 'e'.repeat(40) })],
      'UNAVAILABLE_BUDGET',
    ],
    [
      'same SHA',
      { maxConcurrent: 4 },
      [record(1), record(2, { sha: record(1).sha })],
      'UNAVAILABLE_BUDGET',
    ],
    [
      'same SHA rerun',
      { maxConcurrent: 4 },
      [record(1, { attempt: '2' }), record(2, { attempt: '2', sha: record(1).sha })],
      'UNAVAILABLE_BUDGET',
    ],
    [
      'older branch discovered',
      { maxConcurrent: 4 },
      [record(1), record(2, { branch: record(1).branch })],
      'BUSY',
    ],
    [
      'older branch cross profile',
      { maxConcurrent: 4 },
      [record(1), record(2, { branch: record(1).branch, profile: 'sweep' })],
      'BUSY',
    ],
    [
      'same machine',
      { maxConcurrent: 4 },
      [record(1), record(2, { machineId: record(1).machineId })],
      'UNAVAILABLE',
    ],
    [
      'same machine different profiles',
      { maxConcurrent: 4 },
      [record(1), record(2, { profile: 'sweep', machineId: record(1).machineId })],
      'UNAVAILABLE',
    ],
  ]) {
    await t.test(name, async (t) => {
      const { call } = await runtime(t, { MAX_CONCURRENT_RENTALS: '4' });
      const responses = await Promise.all(
        candidates.map((r) =>
          call('/reserve', {
            record: r,
            config,
            ...(cost === undefined ? {} : { cost }),
            ...(credit === undefined ? {} : { credit }),
          }),
        ),
      );
      const states = await Promise.all(responses.map((r) => r.json()));
      assert.ok(
        states.some((s) => s.state === expected),
        JSON.stringify(states),
      );
      const rows = await (await call('/snapshot')).json();
      const cap =
        name === 'account slots four' ||
        name === 'profile slots three' ||
        name === 'daily rentals three'
          ? 2
          : 1;
      assert.equal(rows.filter(([k]) => k.startsWith('cost:')).length, cap);
      assert.equal(rows.filter(([k]) => k.startsWith('open:')).length, cap);
    });
  }
  for (const i of [1, 2]) {
    await t.test('newer branch discovered ' + i, async (t) => {
      const { call } = await runtime(t, { MAX_CONCURRENT_RENTALS: '4' });
      const newer = record(10 + i, { branch: record(i).branch });
      await call('/seed', [
        ['open:' + newer.key, true],
        ['record:' + newer.key, newer],
      ]);
      const response = await call('/reserve', { record: record(i) });
      assert.equal(response.status, 409);
      assert.deepEqual(await response.json(), { state: 'NOT_ELIGIBLE' });
    });
    await t.test('configured bad machine ' + i, async (t) => {
      const { call } = await runtime(t);
      const response = await call('/reserve', {
        record: record(i),
        config: { badMachines: [record(i).machineId] },
      });
      assert.equal(response.status, 503);
      assert.deepEqual(await response.json(), { state: 'UNAVAILABLE' });
    });
  }
  for (const count of [1, 2, 257, 258]) {
    await t.test('bad-machine entries ' + count, async (t) => {
      const { call } = await runtime(t);
      await call(
        '/seed',
        Array.from({ length: count }, (_, i) => [
          'bad-machine:' + ((count > 256 ? 300001 : 200001) + i),
          { at: instant },
        ]),
      );
      const response = await call('/reserve', { record: record(1) });
      assert.equal(response.status, 503);
      assert.deepEqual(await response.json(), { state: 'UNAVAILABLE' });
      assert.equal(
        (await (await call('/snapshot')).json()).filter(([k]) => k.startsWith('cost:')).length,
        0,
      );
    });
  }
  for (const failAt of ['sha:', 'cost:']) {
    await t.test('intent rollback at ' + failAt, async (t) => {
      const { call } = await runtime(t);
      const response = await call('/reserve', { record: record(1), failAt });
      assert.equal(response.status, 500);
      assert.deepEqual(await (await call('/snapshot')).json(), []);
    });
  }
  for (const cap of [1, 2]) {
    await t.test('simultaneous /rent independent services cap ' + cap, async (t) => {
      const { stub, cloud, call } = await runtime(t, {
        MAX_CONCURRENT_RENTALS: String(cap),
      });
      const tokens = await Promise.all(
        [1, 2, 3, 4].map(async (i) => {
          const id = String(1000 + i),
            head = i.toString(16).padStart(40, '0');
          cloud.runs.set(id, {
            ...cloud.run,
            id: Number(id),
            head_sha: head,
            head_branch: 'branch-' + i,
          });
          return jwt({
            run_id: id,
            sha: head,
            iat: Math.floor(Date.now() / 1000),
            nbf: Math.floor(Date.now() / 1000) - 1,
            exp: Math.floor(Date.now() / 1000) + 300,
          });
        }),
      );
      const responses = await Promise.all(
        tokens.map((token, i) =>
          stub.fetch(env.WORKER_URL + '/' + bindings.BROKER_PATH_PREFIX + '/rent', {
            method: 'POST',
            headers: {
              Authorization: 'Bearer ' + token,
              'X-KF-Protocol': '6',
              'X-Test-Run': String(1001 + i),
            },
            body: JSON.stringify({
              runner_nonce: 'b'.repeat(64),
              tested_sha: (i + 1).toString(16).padStart(40, '0'),
              profile: 'check',
              remaining_seconds: 3600,
            }),
          }),
        ),
      );
      const data = await Promise.all(responses.map((r) => r.json()));
      assert.equal(
        data.filter((r) => r.state === 'BOOTING').length,
        cap,
        JSON.stringify(data.map((r) => r.state)),
      );
      assert.equal(data.filter((r) => r.state === 'BUSY').length, 4 - cap);
      const rows = await (await call('/snapshot')).json();
      assert.equal(rows.filter(([k]) => k.startsWith('cost:')).length, cap);
      assert.equal(cloud.created.length, cap);
    });
  }
});
