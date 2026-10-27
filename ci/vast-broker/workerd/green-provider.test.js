import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env } from '../test/helpers.js';

// FALSIFIER: provider replacement is already atomic and cannot reset its run deadline/allowance.
const bundle = await build({
  entryPoints: [new URL('./fixture.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
async function fixture(t) {
  const mf = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: bundle.outputFiles[0].text,
      compatibilityDate: '2026-10-21',
      bindings: env,
      durableObjects: { TEST_LEDGER: { className: 'AdmissionFixture', useSQLite: true } },
      outboundService: () => {
        throw Error('No outbound allowed');
      },
    }),
  );
  t.after(() => mf.dispose());
  const ns = await mf.getDurableObjectNamespace('TEST_LEDGER');
  const stub = ns.get(ns.idFromName(t.name));
  const call = async (path, body) => {
    const r = await stub.fetch('https://test.invalid' + path, {
      method: body === undefined ? 'GET' : 'POST',
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    return { status: r.status, data: await r.json() };
  };
  const now = Date.now();
  const previous = {
    key: '123-1',
    run: '123',
    attempt: '1',
    sha: 'a'.repeat(40),
    branch: 'test',
    profile: 'check',
    started: now - 2000,
    ended: now - 1000,
    price: 0.2,
    ttl: 7200000,
    machineId: 100001,
    id: '900',
    nonce: 'b'.repeat(32),
    runnerNonce: 'f'.repeat(64),
    closed: 'PROVIDER_FAULT',
    verdict: 'PROVIDER_FAULT',
    hardDeadline: now + 7200000,
    softDeadline: now + 6600000,
    runBudgetUsd: 1,
    providerRetries: 0,
  };
  await call('/seed', [
    ['record:123-1', previous],
    ['run:123-1', '123-1'],
    ['ended:123-1', previous.ended],
    ['sha:' + previous.sha, { key: previous.key, started: previous.started }],
    [
      'cost:123-1',
      {
        started: previous.started,
        ended: previous.ended,
        price: 0.2,
        ttl: 7200000,
        profile: 'check',
        created: true,
      },
    ],
  ]);
  const record = {
    ...previous,
    started: now,
    ended: null,
    id: null,
    nonce: 'c'.repeat(32),
    closed: null,
    verdict: null,
    machineId: 100002,
    providerRetries: 1,
    retryCostUsd: 0.2 / 3600,
    originalStarted: previous.started,
  };
  return { call, record, previous };
}
for (const count of [2, 4])
  test('workerd one atomic provider replacement among ' + count, async (t) => {
    const { call, record, previous } = await fixture(t);
    const replies = await Promise.all(
      Array.from({ length: count }, () =>
        call('/reserve', {
          record,
          retryNonce: previous.nonce,
        }),
      ),
    );
    assert.equal(replies.filter((r) => r.status === 200).length, 1, JSON.stringify(replies));
    const snapshot = (await call('/snapshot')).data;
    assert.equal(snapshot.filter(([k]) => k.startsWith('cost:')).length, 2);
    assert.equal(snapshot.filter(([k]) => k.startsWith('open:')).length, 1);
    assert.equal(snapshot.find(([k]) => k === 'record:123-1')[1].providerRetries, 1);
    assert.equal(
      snapshot.some(([k]) => k === 'ended:123-1'),
      false,
    );
  });
for (const kind of [
  'same-machine',
  'deadline',
  'allowance',
  'unclean',
  'nonce',
  'repeat',
  'cost',
  'cap',
])
  for (const variant of [1, 2])
    test('workerd provider replacement rejects ' + kind + variant, async (t) => {
      const { call, record, previous } = await fixture(t);
      if (kind === 'same-machine') record.machineId = previous.machineId;
      if (kind === 'deadline') record.hardDeadline += variant;
      if (kind === 'allowance') record.runBudgetUsd += variant;
      if (kind === 'unclean') {
        previous.ended = null;
        await call('/seed', [['record:123-1', previous]]);
      }
      if (kind === 'repeat') {
        previous.providerRetries = 1;
        await call('/seed', [['record:123-1', previous]]);
      }
      const before = await call('/snapshot');
      const response = await call('/reserve', {
        record,
        retryNonce: kind === 'nonce' ? 'd'.repeat(32) : previous.nonce,
        ...(kind === 'cost' ? { cost: 1 + variant / 10 } : {}),
        ...(kind === 'cap' ? { config: variant === 1 ? { daily: 0.01 } : { rolling: 0.01 } } : {}),
      });
      assert.notEqual(response.status, 200);
      assert.deepEqual(await call('/snapshot'), before);
    });

for (const variant of [1, 2])
  test('workerd provider retry does not consume another daily slot ' + variant, async (t) => {
    const { call, record, previous } = await fixture(t);
    const response = await call('/reserve', {
      record,
      retryNonce: previous.nonce,
      config: { dailyCount: 1 },
    });
    assert.equal(response.status, 200, JSON.stringify(response.data));
    const admin = await call('/admin/list');
    assert.equal(admin.data.budget.profiles.check.count_today, 1);
  });
