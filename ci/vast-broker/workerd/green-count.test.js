import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env } from '../test/helpers.js';

// FALSIFIER: real DO admission already accepts the proposed21..40 configuration range.
const bundle = await build({
  entryPoints: [new URL('./fixture.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
for (const count of [21, 40])
  test('workerd count setting admits at ' + count, async (t) => {
    const mf = new Miniflare(
      convertV4MiniflareOptions({
        modules: true,
        script: bundle.outputFiles[0].text,
        compatibilityDate: '2026-10-21',
        bindings: { ...env, DAILY_RENTALS: String(count) },
        durableObjects: { TEST_LEDGER: { className: 'AdmissionFixture', useSQLite: true } },
        outboundService: () => {
          throw Error('No outbound allowed');
        },
      }),
    );
    t.after(() => mf.dispose());
    const ns = await mf.getDurableObjectNamespace('TEST_LEDGER');
    const stub = ns.get(ns.idFromName(t.name));
    const now = Date.now();
    const r = await stub.fetch('https://test.invalid/reserve', {
      method: 'POST',
      body: JSON.stringify({
        record: {
          key: '123-1',
          run: '123',
          attempt: '1',
          sha: 'a'.repeat(40),
          branch: 'test',
          profile: 'check',
          started: now,
          ended: null,
          price: 0.2,
          ttl: 1200000,
          machineId: 100001,
        },
      }),
    });
    const data = await r.json();
    assert.equal(r.status, 200, JSON.stringify(data));
    assert.equal(data.state, 'RESERVED');
  });
