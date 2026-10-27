import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env, jwt, MockCloud, rentBody } from '../test/helpers.js';

// FALSIFIER: the actual Worker already admits attempt 3/4 against exact attempt metadata.
const prefix = 'attempt-' + 'p'.repeat(32);
const bundle = await build({
  entryPoints: [new URL('./fixture.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
for (const attempt of [3, 4])
  test(
    'workerd attempt ' + attempt + ' availability and rental ignore stale latest run',
    async (t) => {
      const cloud = new MockCloud();
      cloud.run.event = 'push';
      cloud.run.run_attempt = attempt;
      const reads = [];
      const mf = new Miniflare(
        convertV4MiniflareOptions({
          modules: true,
          script: bundle.outputFiles[0].text,
          compatibilityDate: '2026-10-21',
          bindings: { ...env, BROKER_PATH_PREFIX: prefix },
          durableObjects: { TEST_LEDGER: { className: 'AdmissionFixture', useSQLite: true } },
          outboundService: async (request) => {
            const path = new URL(request.url).pathname;
            if (/\/actions\/runs\/123(?:\/attempts\/[34])?$/.test(path)) {
              reads.push(path);
              return Response.json(
                path.endsWith('/attempts/' + attempt)
                  ? cloud.run
                  : {
                      ...cloud.run,
                      run_attempt: attempt - 1,
                      status: 'completed',
                    },
              );
            }
            return cloud.fetch(request.url, {
              method: request.method,
              headers: request.headers,
              ...(['GET', 'HEAD'].includes(request.method) ? {} : { body: await request.text() }),
            });
          },
        }),
      );
      t.after(() => mf.dispose());
      const ns = await mf.getDurableObjectNamespace('TEST_LEDGER');
      const stub = ns.get(ns.idFromName(t.name));
      const now = Math.floor(Date.now() / 1000);
      const token = await jwt({
        event_name: 'push',
        run_attempt: String(attempt),
        iat: now,
        nbf: now - 1,
        exp: now + 300,
      });
      const call = async (path, body) => {
        const response = await stub.fetch(env.WORKER_URL + '/' + prefix + path, {
          method: body === undefined ? 'GET' : 'POST',
          headers: {
            Authorization: 'Bearer ' + token,
            'X-KF-Protocol': '6',
            'Content-Type': 'application/json',
          },
          ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        });
        return { status: response.status, body: await response.json() };
      };
      const available = await call('/availability?profile=check');
      assert.equal(available.status, 200);
      assert.equal(cloud.creates, 0);
      const rented = await call('/rent', rentBody(cloud.run.head_sha));
      assert.equal(rented.status, 200);
      assert.equal(cloud.creates, 1);
      const rows = new Map(await (await stub.fetch('https://test.invalid/snapshot')).json());
      const record = rows.get('record:123-' + attempt);
      assert.equal(record.attempt, String(attempt));
      assert.equal(record.sha, cloud.run.head_sha);
      assert.equal([...rows.keys()].filter((key) => key.startsWith('cost:')).length, 1);
      assert.deepEqual(
        reads,
        Array(2).fill('/repos/' + env.REPOSITORY + '/actions/runs/123/attempts/' + attempt),
      );
    },
  );
