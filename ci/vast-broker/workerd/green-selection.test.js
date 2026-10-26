import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env, jwt, MockCloud } from '../test/helpers.js';

// FALSIFIER: default /rent already selects the cheapest eligible die in real workerd.
// Every outbound call is intercepted: these tests cannot rent or reach the provider.
const prefix = 'workerd-green-' + 'p'.repeat(32);
const bundle = await build({
  entryPoints: [new URL('./fixture.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
for (const gpu of ['RTX_4090', 'RTX_4070']) {
  test('workerd default check chooses cheaper die ' + gpu, async (t) => {
    const cloud = new MockCloud();
    cloud.offer.gpu_name = 'RTX_3060';
    cloud.offer.compute_cap = 860;
    cloud.price = 0.4;
    cloud.extraOffers = [
      { ...cloud.offer, id: 6, machine_id: 100002, gpu_name: gpu, compute_cap: 890, price: 0.1 },
    ];
    const mf = new Miniflare(
      convertV4MiniflareOptions({
        modules: true,
        script: bundle.outputFiles[0].text,
        compatibilityDate: '2026-10-21',
        bindings: { ...env, BROKER_PATH_PREFIX: prefix },
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
    const data = await response.json();
    assert.equal(response.status, 200);
    assert.equal(data.machine_id, 100002);
    assert.equal(cloud.created[0].ask, 6);
  });
}
