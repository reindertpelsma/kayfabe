import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import {
  env,
  clock,
  nonce,
  sha,
  jwt,
  MemoryStorage,
  MockCloud,
  callbackToken,
  sampleResult,
} from './helpers.js';
test('local HTTP integration: owner rent to authenticated reports to automatic destroy', async () => {
  const { createService } = await import('../src/index.js');
  const cloud = new MockCloud();
  const server = createServer(async (req, res) => {
    try {
      const chunks = [];
      for await (const c of req) chunks.push(c);
      const r = await cloud.fetch(req.headers['x-mock-target'], {
        method: req.method,
        headers: req.headers,
        body: chunks.length ? Buffer.concat(chunks).toString() : undefined,
      });
      res.writeHead(r.status, Object.fromEntries(r.headers));
      res.end(Buffer.from(await r.arrayBuffer()));
    } catch {
      res.writeHead(500);
      res.end('mock error');
    }
  });
  await new Promise((resolve) => server.listen(0, 'localhost', resolve));
  try {
    const localFetch = (url, init = {}) =>
      fetch('http://localhost:' + server.address().port, {
        ...init,
        headers: { ...init.headers, 'x-mock-target': String(url) },
      });
    const storage = new MemoryStorage();
    const service = createService({
      env: { ...env, BROKER_PATH_PREFIX: 'i'.repeat(32) },
      storage,
      fetch: localFetch,
      now: () => clock,
      random: () => nonce,
    });
    const authorization = 'Bearer ' + (await jwt());
    const r = await service.fetch(
      new Request(env.WORKER_URL + '/' + 'i'.repeat(32) + '/rent', {
        method: 'POST',
        headers: { authorization },
        body: JSON.stringify({ runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: 7 }),
      }),
    );
    assert.equal(r.status, 200);
    const token = callbackToken(await storage.get('record:123-1'));
    const report = (payload) =>
      service.fetch(
        new Request(env.WORKER_URL + '/' + 'i'.repeat(32) + '/report/123-1', {
          method: 'POST',
          headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        }),
      );
    assert.equal((await report({ seq: 1, kind: 'log', text: '::error::fake' })).status, 200);
    let response = await service.fetch(
      new Request(env.WORKER_URL + '/' + 'i'.repeat(32) + '/status/123-1', {
        headers: { authorization },
      }),
    );
    assert.equal((await response.json()).verdict, 'UNAVAILABLE');
    assert.equal(
      (
        await report({
          seq: 2,
          kind: 'result',
          result: { ...sampleResult(), nonce, verdict: 'PASS', runner_nonce: 'f'.repeat(64), sha },
        })
      ).status,
      200,
    );
    response = await service.fetch(
      new Request(env.WORKER_URL + '/' + 'i'.repeat(32) + '/status/123-1', {
        headers: { authorization },
      }),
    );
    const result = await response.json();
    assert.equal(result.verdict, 'PASS');
    assert.match(result.log_tail_sanitised, /\[box\] : :error/);
    assert.deepEqual(cloud.deletes, []);
    await service.fetch(
      new Request(env.WORKER_URL + '/' + 'i'.repeat(32) + '/stop/123-1', {
        method: 'POST',
        headers: { authorization },
      }),
    );
    assert.deepEqual(cloud.deletes, ['900']);
    assert.equal(
      cloud.calls.some((c) => c.url.includes('request_logs')),
      false,
    );
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
});
test('scheduled binding reaches Durable Object cleanup', async () => {
  const source = await import('../src/index.js');
  let swept = 0;
  const object = new source.BrokerLedger({ storage: new MemoryStorage() }, env);
  object.service = {
    sweep: async () => {
      swept++;
    },
    fetch: async () => new Response('not cleanup', { status: 404 }),
  };
  let promise;
  source.default.scheduled(
    {},
    { LEDGER: { idFromName: (n) => n, get: () => object } },
    { waitUntil: (p) => (promise = p) },
  );
  await promise;
  assert.equal(swept, 1);
});
test('public request cannot invoke the internal cron route', async () => {
  const source = await import('../src/index.js');
  let forwarded = 0;
  const response = await source
    .createGateway({ authorize: async () => {} })
    .fetch(new Request('https://broker.example/cron', { method: 'POST' }), {
      LEDGER: {
        idFromName: (n) => n,
        get: () => ({
          fetch: () => {
            forwarded++;
            return new Response('bad');
          },
        }),
      },
    });
  assert.equal(response.status, 404);
  assert.equal(forwarded, 0);
});
test('bundled scheduled binding reaches Durable Object cleanup', async () => {
  const { spawnSync } = await import('node:child_process');
  assert.equal(
    spawnSync(process.execPath, ['bundle.js'], { cwd: new URL('../', import.meta.url) }).status,
    0,
  );
  const source = await import('../dist/worker.js');
  let swept = 0;
  const object = new source.BrokerLedger({ storage: new MemoryStorage() }, env);
  object.service = {
    sweep: async () => {
      swept++;
    },
    fetch: async () => new Response('not cleanup', { status: 404 }),
  };
  let promise;
  source.default.scheduled(
    {},
    { LEDGER: { idFromName: (n) => n, get: () => object } },
    { waitUntil: (p) => (promise = p) },
  );
  await promise;
  assert.equal(swept, 1);
});
test('bundled public request cannot invoke internal cron route', async () => {
  const source = await import('../dist/worker.js');
  let forwarded = 0;
  const response = await source
    .createGateway({ authorize: async () => {} })
    .fetch(new Request('https://broker.example/cron', { method: 'POST' }), {
      LEDGER: {
        idFromName: (n) => n,
        get: () => ({
          fetch: () => {
            forwarded++;
            return new Response('bad');
          },
        }),
      },
    });
  assert.equal(response.status, 404);
  assert.equal(forwarded, 0);
});
