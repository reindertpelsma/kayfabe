import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { setup, sha, nonce, env, jwt } from './helpers.js';
import { createGateway } from '../src/index.js';
// Component tests inject auth; v5-front-auth tests the real gateway.
const worker = createGateway({ authorize: async () => {} });
const request = { runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: 7 };
for (const size of [180, 700])
  test('R3 public ingress preserves bounded nonce rent body ' + size, async () => {
    let received;
    const bindings = {
      ...env,
      BROKER_PATH_PREFIX: 'g'.repeat(32),
      LEDGER: {
        idFromName: () => 1,
        get: () => ({
          fetch: async (r) => {
            received = await r.text();
            return Response.json({});
          },
        }),
      },
    };
    const body = JSON.stringify(request) + ' '.repeat(size);
    await worker.fetch(
      new Request(env.WORKER_URL + '/' + 'g'.repeat(32) + '/rent', { method: 'POST', body }),
      bindings,
    );
    assert.equal(received, body.slice(0, 2049));
  });
// FALSIFIER: runner nonce is optional/replaced on replay, or final/page hashes
// do not bind the exact UTF-8 chunks that the runner retrieves in cursor order.
for (const restart of [false, true])
  test('R3 rent binds caller nonce through status ' + restart, async () => {
    const h = await setup();
    const response = await h.call('/rent', { body: request });
    assert.equal(response.status, 200);
    if (restart) h.restart();
    const status = await (await h.call('/status/123-1')).json();
    assert.equal(status.runner_nonce, request.runner_nonce);
    assert.equal(status.sha, sha);
    assert.equal(status.protocol_version, 4);
    const page = await (await h.call('/logs/123-1?cursor=0', { method: 'GET' })).json();
    let expected = '0'.repeat(64);
    for (const text of page.chunks) {
      const bytes = Buffer.from(text),
        len = Buffer.alloc(4);
      len.writeUInt32BE(bytes.length);
      expected = createHash('sha256')
        .update(Buffer.concat([Buffer.from(expected, 'hex'), len, bytes]))
        .digest('hex');
    }
    assert.equal(status.log_sha256, expected);
  });
test('R3 missing rent nonce refuses before create', async () => {
  const h = await setup();
  const response = await h.service.fetch(
    new Request(env.WORKER_URL + '/rent', {
      method: 'POST',
      headers: { Authorization: 'Bearer ' + (await jwt()), 'X-GH-Token': 'mock-job-token' },
    }),
  );
  assert.equal(response.status, 400);
  assert.equal(h.cloud.creates, 0);
});
for (const key of ['tested_sha', 'pr_number'])
  test('R3 rent crosschecks caller ' + key, async () => {
    const h = await setup();
    const response = await h.call('/rent', {
      body: { ...request, [key]: key === 'tested_sha' ? 'd'.repeat(40) : 8 },
    });
    assert.equal(response.status, 409);
    assert.equal(h.cloud.creates, 0);
  });
test('R3 same run cannot replace runner nonce', async () => {
  const h = await setup();
  assert.equal((await h.call('/rent', { body: request })).status, 200);
  assert.equal(
    (await h.call('/rent', { body: { ...request, runner_nonce: 'e'.repeat(64) } })).status,
    409,
  );
  assert.equal(h.cloud.creates, 1);
});
for (const restart of [false, true])
  test('R3 a new run nonce cannot inherit cached PASS ' + restart, async () => {
    const h = await setup();
    assert.equal((await h.call('/rent', { body: request })).status, 200);
    assert.equal(
      (await h.report({ seq: 1, kind: 'result', result: { nonce, verdict: 'PASS' } })).status,
      200,
    );
    h.time.value += 300000;
    await h.service.sweep();
    if (restart) h.restart();
    h.cloud.run.id = 124;
    assert.equal(
      (
        await h.call('/rent', {
          claims: { run_id: '124' },
          body: { ...request, runner_nonce: 'e'.repeat(64) },
        })
      ).status,
      409,
    );
    assert.equal(h.cloud.creates, 1);
  });
for (const text of ['ordinary log\n', 'é raw UTF8\n'])
  test(
    'R3 final structured binding hashes exact fetched chunks ' + JSON.stringify(text),
    async () => {
      const h = await setup();
      assert.equal((await h.call('/rent', { body: request })).status, 200);
      assert.equal((await h.report({ seq: 1, kind: 'log', text })).status, 200);
      assert.equal(
        (
          await h.report({
            seq: 2,
            kind: 'result',
            result: { nonce, verdict: 'PASS', runner_nonce: request.runner_nonce, sha },
          })
        ).status,
        200,
      );
      const status = await (await h.call('/status/123-1')).json();
      const page = await (await h.call('/logs/123-1?cursor=0', { method: 'GET' })).json();
      let hash = '0'.repeat(64);
      for (const chunk of page.chunks) {
        const b = Buffer.from(chunk);
        const length = Buffer.alloc(4);
        length.writeUInt32BE(b.length);
        hash = createHash('sha256')
          .update(Buffer.concat([Buffer.from(hash, 'hex'), length, b]))
          .digest('hex');
      }
      assert.equal(status.runner_nonce, request.runner_nonce);
      assert.equal(status.sha, sha);
      assert.equal(status.log_sha256, hash);
      assert.equal(page.log_sha256, hash);
      assert.equal(page.next_cursor, page.chunks.length);
    },
  );
for (const key of ['runner_nonce', 'sha'])
  test('R3 box result must echo ' + key, async () => {
    const h = await setup();
    assert.equal((await h.call('/rent', { body: request })).status, 200);
    const response = await h.report({
      seq: 1,
      kind: 'result',
      result: {
        nonce,
        verdict: 'PASS',
        runner_nonce: request.runner_nonce,
        sha,
        [key]: '0'.repeat(key === 'sha' ? 40 : 64),
      },
    });
    assert.equal(response.status, 400);
    assert.equal((await h.storage.get('record:123-1')).verdict, null);
  });
