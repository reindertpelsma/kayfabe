import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env, jwt, MockCloud, callbackToken, rentBody } from '../test/helpers.js';
import { readFileSync } from 'node:fs';

// FALSIFIER: the actual incident can already replace once across independent request closures.
const prefix = 'incident-' + 'p'.repeat(32);
const bundle = await build({
  entryPoints: [new URL('./fixture.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
const faultSummary = readFileSync(
  new URL('../test/fixtures/provider-57945969.txt', import.meta.url),
  'utf8',
);
for (const failureKind of ['declared', 'summary'])
  for (const seedCount of [0, 255])
    test(
      'workerd actual incident ' + failureKind + ', existing bad machines=' + seedCount,
      async (t) => {
        const cloud = new MockCloud();
        const machine = failureKind === 'summary' ? 57994 : 144805;
        Object.assign(cloud.offer, {
          machine_id: machine,
          host_id: 224600,
          gpu_name: 'RTX_3090',
          compute_cap: 860,
        });
        cloud.contracts = [57890737, 57890999];
        // Replay the historical incident before its static host quarantine. Production
        // uses the seeded default; badhost.test.js checks that it refuses this host.
        const bindings = {
          ...env,
          BROKER_PATH_PREFIX: prefix,
          DAILY_RENTALS: '1',
          BAD_HOST_IDS: '[]',
        };
        const mf = new Miniflare(
          convertV4MiniflareOptions({
            modules: true,
            script: bundle.outputFiles[0].text,
            compatibilityDate: '2026-10-21',
            bindings,
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
        const snapshot = async () =>
          new Map(await (await stub.fetch('https://test.invalid/snapshot')).json());
        await (
          await stub.fetch('https://test.invalid/seed', {
            method: 'POST',
            body: JSON.stringify(
              Array.from({ length: seedCount }, (_, i) => ['bad-machine:' + (200000 + i), true]),
            ),
          })
        ).json();
        const now = Math.floor(Date.now() / 1000);
        const token = await jwt({ iat: now, nbf: now - 1, exp: now + 300 });
        const call = async (path, body, bearer = token) => {
          const response = await stub.fetch(env.WORKER_URL + '/' + prefix + path, {
            method: body === undefined ? 'GET' : 'POST',
            headers: {
              Authorization: 'Bearer ' + bearer,
              'X-KF-Protocol': '6',
              'Content-Type': 'application/json',
            },
            ...(body === undefined ? {} : { body: JSON.stringify(body) }),
          });
          // Miniflare requires its proxy response body to be consumed immediately.
          return new Response(await response.arrayBuffer(), {
            status: response.status,
            headers: response.headers,
          });
        };
        const rent = () => call('/rent', rentBody(cloud.run.head_sha));
        assert.equal((await rent()).status, 200);
        const original = (await snapshot()).get('record:123-1');
        const fault = (record) =>
          call(
            '/report/123-1',
            {
              seq: 1,
              kind: 'result',
              ...(failureKind === 'summary' ? { text: faultSummary } : {}),
              result: {
                nonce: record.nonce,
                runner_nonce: record.runnerNonce,
                sha: record.sha,
                verdict: failureKind === 'summary' ? 'FAIL' : 'PROVIDER_FAULT',
                suites: [
                  {
                    name: failureKind === 'summary' ? 'host_xid' : 'provider_gpu_unavailable',
                    pass: 0,
                    fail: 1,
                    skip: 0,
                  },
                ],
                hardware: { gpu_name: 'RTX_3090', compute_cap: 860, driver_version: 'unknown' },
              },
            },
            callbackToken(record),
          );
        assert.equal((await fault(original)).status, 200);
        assert.equal(cloud.instances.has('57890737'), false);
        assert.ok((await snapshot()).has('bad-machine:' + machine));
        cloud.extraOffers = [
          { ...cloud.offer, id: 6, machine_id: 144806, host_id: 224601, price: 0.21 },
        ];
        const replacement = await rent();
        assert.equal(replacement.status, 200);
        assert.equal((await replacement.json()).id, '57890999');
        const fresh = (await snapshot()).get('record:123-1');
        assert.equal(fresh.machineId, 144806);
        assert.equal(fresh.hostId, 224601);
        assert.equal(fresh.hardDeadline, original.hardDeadline);
        assert.notEqual(fresh.nonce, original.nonce);
        assert.equal((await fault(original)).status, 401);
        assert.equal((await fault(fresh)).status, 200);
        assert.equal((await (await rent()).json()).verdict, 'PROVIDER_FAULT');
        assert.equal(cloud.created.length, 2);
        assert.deepEqual(cloud.deletes.sort(), ['57890737', '57890999']);
        assert.ok(cloud.instances.has('777') && cloud.instances.has('888'));
        const rows = await snapshot();
        assert.equal([...rows.keys()].filter((key) => key.startsWith('cost:')).length, 2);
        const summary = await (await stub.fetch('https://internal.invalid/admin/list')).json();
        assert.equal(summary.budget.profiles.check.count_today, 1);
        if (seedCount === 255)
          assert.equal((await call('/availability?profile=check')).status, 503);
      },
    );
