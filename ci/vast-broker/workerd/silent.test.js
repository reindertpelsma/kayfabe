import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env } from '../test/helpers.js';
// FALSIFIER: legacy rows need the new receipt marker to release daily slots in real storage.
const bundle = await build({
  entryPoints: [new URL('./fixture.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
for (const id of [null, undefined])
  test('workerd settled legacy no-instance proof ' + id, async (t) => {
    const mf = new Miniflare(
      convertV4MiniflareOptions({
        modules: true,
        script: bundle.outputFiles[0].text,
        compatibilityDate: '2026-10-21',
        bindings: { ...env, DAILY_RENTALS: '1' },
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
      const r = await stub.fetch('https://internal.invalid' + path, {
        method: body ? 'POST' : 'GET',
        ...(body ? { body: JSON.stringify(body) } : {}),
      });
      return { status: r.status, data: await r.json() };
    };
    const now = Date.now(),
      started = now - 1000;
    await call('/seed', [
      ['cost:100-1', { started, ended: now, price: 0.5, bandwidthCost: 0, ttl: 3600000 }],
      ['record:100-1', { key: '100-1', started, ended: now, closed: 'GONE', id, price: 0 }],
    ]);
    const before = await call('/snapshot');
    const admin = await call('/admin/list');
    assert.equal(admin.data.budget.profiles.check.count_today, 0);
    assert.equal(admin.data.budget.rolling_usd, 0);
    assert.deepEqual(await call('/snapshot'), before);
    const r = await call('/reserve', {
      record: {
        key: '101-1',
        run: '101',
        attempt: '1',
        sha: 'a'.repeat(40),
        branch: 'fresh',
        profile: 'check',
        started: now,
        ended: null,
        price: 0.5,
        ttl: 3600000,
        machineId: 123,
      },
      config: {},
      credit: 20,
    });
    assert.equal(r.status, 200, JSON.stringify(r.data));
  });
