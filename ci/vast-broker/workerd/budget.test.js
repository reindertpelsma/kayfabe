import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env } from '../test/helpers.js';
// FALSIFIER: no-instance count correction, diagnostics or read-only admin fail in workerd.
const bundle = await build({
  entryPoints: [new URL('./fixture.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
for (const legacy of [false, true])
  test('workerd zero-instance accounting and admin ' + legacy, async (t) => {
    const mf = new Miniflare(
      convertV4MiniflareOptions({
        modules: true,
        script: bundle.outputFiles[0].text,
        compatibilityDate: '2026-10-21',
        bindings: { ...env, DAILY_RENTALS: '1' },
        durableObjects: { TEST_LEDGER: { className: 'AdmissionFixture', useSQLite: true } },
        outboundService: () => {
          throw Error('No network allowed');
        },
      }),
    );
    t.after(() => mf.dispose());
    const ns = await mf.getDurableObjectNamespace('TEST_LEDGER');
    const stub = ns.get(ns.idFromName(t.name));
    const call = async (path, data) => {
      const r = await stub.fetch('https://internal.invalid' + path, {
        method: data ? 'POST' : 'GET',
        ...(data ? { body: JSON.stringify(data) } : {}),
      });
      return { status: r.status, data: await r.json() };
    };
    const now = Date.now(),
      failed = {
        key: '100-1',
        run: '100',
        sha: 'a'.repeat(40),
        started: now - 1000,
        ended: now,
        price: 0,
        bandwidthCost: 0,
        ttl: 3600000,
        closed: 'GONE',
        id: null,
      };
    if (legacy)
      await call('/seed', [
        ['record:100-1', failed],
        [
          'cost:100-1',
          {
            started: failed.started,
            ended: now,
            price: 0,
            bandwidthCost: 0,
            ttl: failed.ttl,
          },
        ],
      ]);
    else await call('/settle', { record: failed, now });
    const fresh = {
      ...failed,
      key: '101-1',
      run: '101',
      sha: 'b'.repeat(40),
      branch: 'other',
      started: now,
      ended: null,
      closed: null,
      price: 0.5,
      machineId: 1234,
    };
    const reserve = await call('/reserve', { record: fresh, config: {}, now, credit: 20 });
    assert.equal(reserve.status, 200, JSON.stringify(reserve.data));
    const before = await call('/snapshot');
    const admin = await call('/admin/list');
    assert.equal(admin.status, 200);
    assert.equal(admin.data.budget.profiles.check.count_today, 1);
    assert.equal(admin.data.budget.outstanding_rows.length, 1);
    assert.equal(admin.data.budget.spend_rows.find((x) => x.key === '100-1').billed_usd, 0);
    assert.deepEqual(await call('/snapshot'), before, 'admin must not prune or mutate');
    const retry = await call('/reserve', { record: fresh, config: {}, credit: 20 });
    assert.equal(retry.status, 409);
    assert.equal(retry.data.reason, 'BUDGET:cost-key');
    const receipt = await call('/snapshot');
    assert.ok(receipt.data.some(([key]) => key === 'cost:100-1'));
  });

for (const failAt of ['cost:', 'ended:'])
  test('workerd settlement rolls back at ' + failAt, async (t) => {
    const mf = new Miniflare(
      convertV4MiniflareOptions({
        modules: true,
        script: bundle.outputFiles[0].text,
        compatibilityDate: '2026-10-21',
        bindings: env,
        durableObjects: { TEST_LEDGER: { className: 'AdmissionFixture', useSQLite: true } },
        outboundService: () => {
          throw Error('No network allowed');
        },
      }),
    );
    t.after(() => mf.dispose());
    const ns = await mf.getDurableObjectNamespace('TEST_LEDGER');
    const stub = ns.get(ns.idFromName(t.name));
    const call = async (path, data) => {
      const r = await stub.fetch('https://internal.invalid' + path, {
        method: data ? 'POST' : 'GET',
        ...(data ? { body: JSON.stringify(data) } : {}),
      });
      return { status: r.status, data: await r.json() };
    };
    const now = Date.now(),
      record = {
        key: '100-1',
        run: '100',
        sha: 'a'.repeat(40),
        started: now - 1000,
        ended: null,
        price: 0.5,
        bandwidthCost: 0,
        ttl: 3600000,
        id: '900',
      };
    await call('/seed', [
      ['record:100-1', record],
      ['cost:100-1', record],
      ['open:100-1', true],
    ]);
    const before = await call('/snapshot');
    assert.equal((await call('/settle', { record, now, failAt })).status, 503);
    assert.deepEqual(await call('/snapshot'), before);
  });
