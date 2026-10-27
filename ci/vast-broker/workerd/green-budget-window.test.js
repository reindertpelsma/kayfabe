import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env, jwt, MockCloud } from '../test/helpers.js';

// FALSIFIER: first admission already retains the configured dollar limit, not the offer quote.
const prefix = 'workerd-window-' + 'p'.repeat(32);
const bundle = await build({
  entryPoints: [new URL('./fixture.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
for (const daily of [3, 6])
  test('workerd retains the configured run dollar allowance ' + daily, async (t) => {
    const cloud = new MockCloud();
    const mf = new Miniflare(
      convertV4MiniflareOptions({
        modules: true,
        script: bundle.outputFiles[0].text,
        compatibilityDate: '2026-10-21',
        bindings: { ...env, BROKER_PATH_PREFIX: prefix, DAILY_SPEND_USD: String(daily) },
        durableObjects: { TEST_LEDGER: { className: 'AdmissionFixture', useSQLite: true } },
        outboundService: async (request) =>
          cloud.fetch(request.url, {
            method: request.method,
            headers: request.headers,
            ...(['GET', 'HEAD'].includes(request.method) ? {} : { body: await request.text() }),
          }),
      }),
    );
    t.after(() => mf.dispose());
    const ns = await mf.getDurableObjectNamespace('TEST_LEDGER');
    const stub = ns.get(ns.idFromName(t.name));
    const now = Math.floor(Date.now() / 1000);
    const token = await jwt({ iat: now, nbf: now - 1, exp: now + 300 });
    const response = await stub.fetch(env.WORKER_URL + '/' + prefix + '/rent', {
      method: 'POST',
      headers: { Authorization: 'Bearer ' + token, 'X-KF-Protocol': '6' },
      body: JSON.stringify({
        runner_nonce: 'f'.repeat(64),
        tested_sha: cloud.run.head_sha,
        profile: 'check',
        remaining_seconds: 12000,
      }),
    });
    await response.json();
    assert.equal(response.status, 200);
    const snapshot = await (await stub.fetch('https://test.invalid/snapshot')).json();
    assert.equal(snapshot.find(([key]) => key === 'record:123-1')[1].runBudgetUsd, daily);
  });
