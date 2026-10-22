import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, env, jwt, callbackToken, sha, nonce, sampleResult } from './helpers.js';
import worker, { BrokerLedger } from '../src/index.js';
import { onstart } from '../src/onstart.js';
const prefix = 'unit-path-only-' + 'p'.repeat(32);
const request = async (
  h,
  path,
  {
    method = path.includes('/status/') ? 'GET' : 'POST',
    token,
    body = path.endsWith('/rent')
      ? { runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: 7 }
      : undefined,
  } = {},
) =>
  h.service.fetch(
    new Request(env.WORKER_URL + path, {
      method,
      headers: { Authorization: 'Bearer ' + (token ?? (await jwt())) },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    }),
  );
const rent = async () => {
  const h = await setup({ BROKER_PATH_PREFIX: prefix });
  assert.equal((await request(h, '/' + prefix + '/rent')).status, 200);
  return h;
};
const report = async (h, text, seq = 1) =>
  request(h, '/' + prefix + '/report/123-1', {
    token: callbackToken(await h.storage.get('record:123-1')),
    body: { seq, kind: 'log', text },
  });
for (const path of [
  '/rent',
  '/status/123-1',
  '/report/123-1',
  '/stop/123-1',
  '/stop-all',
  '/unknown',
])
  test('unprefixed endpoint has generic 404 ' + path, async () => {
    const h = await setup({ BROKER_PATH_PREFIX: prefix });
    const r = await request(h, path, { token: env.OWNER_STOP_TOKEN });
    assert.equal(r.status, 404);
    assert.equal(await r.text(), 'Not found');
    assert.equal(h.cloud.calls.length, 0);
  });
for (const wrong of [
  'wrong-' + 'x'.repeat(32),
  'short',
  prefix + 'x',
  encodeURIComponent(prefix).replace('unit', '%75nit'),
])
  test('wrong or encoded prefix fails before auth ' + wrong.slice(0, 8), async () => {
    const h = await setup({ BROKER_PATH_PREFIX: prefix });
    const r = await request(h, '/' + wrong + '/rent');
    assert.equal(r.status, 404);
    assert.equal(await r.text(), 'Not found');
    assert.equal(h.cloud.calls.length, 0);
  });
for (const bad of ['short', 'x'.repeat(129), 'bad/path', 'x'.repeat(32) + '\n', 42])
  test('invalid prefix config fails closed ' + String(bad).length, async () => {
    const h = await setup({ BROKER_PATH_PREFIX: bad });
    const r = await request(h, '/rent');
    assert.equal(r.status, 404);
    assert.equal(h.cloud.creates, 0);
    const exact = await request(h, '/' + String(bad) + '/rent');
    assert.equal(exact.status, 404);
    assert.equal(h.cloud.creates, 0);
  });
for (const action of ['status', 'stop', 'stop-all'])
  test('correct prefix reaches protected ' + action, async () => {
    const h = await rent();
    const r = await request(
      h,
      '/' + prefix + '/' + action + (action === 'stop-all' ? '' : '/123-1'),
      action === 'stop-all' ? { token: env.OWNER_STOP_TOKEN } : {},
    );
    assert.equal(r.status, 200);
    if (action !== 'status') assert.deepEqual(h.cloud.deletes, ['900']);
  });
for (const path of ['/rent', '/status/123-1', '/report/123-1', '/stop/123-1'])
  test('prefix knowledge is not auth ' + path, async () => {
    const h = await setup({ BROKER_PATH_PREFIX: prefix });
    assert.equal((await request(h, '/' + prefix + path, { token: 'bad' })).status, 401);
    assert.equal(h.cloud.creates, 0);
  });
for (const suffix of ['/unknown', '/cron', '//rent'])
  test('correct prefix unknown path uses same generic 404 ' + suffix, async () => {
    const h = await setup({ BROKER_PATH_PREFIX: prefix });
    const r = await request(h, '/' + prefix + suffix);
    assert.equal(r.status, 404);
    assert.equal(await r.text(), 'Not found');
  });
for (const candidate of ['', 'different-' + 'z'.repeat(32)])
  test('prefix compares through native crypto ' + candidate.length, async () => {
    const h = await setup({ BROKER_PATH_PREFIX: prefix });
    const original = crypto.subtle.verify;
    let calls = 0;
    crypto.subtle.verify = function (...args) {
      calls++;
      return original.apply(this, args);
    };
    try {
      await request(h, '/' + candidate + '/rent');
      assert.ok(calls >= 1);
    } finally {
      crypto.subtle.verify = original;
    }
  });
for (const path of ['/rent', '/stop-all', '/' + prefix + '/unknown', '/' + prefix + '/cron'])
  test('public routing rejects without forwarding ' + path, async () => {
    let calls = 0;
    const r = await worker.fetch(new Request(env.WORKER_URL + path), {
      ...env,
      BROKER_PATH_PREFIX: prefix,
      LEDGER: {
        idFromName: () => 0,
        get: () => ({
          fetch: () => {
            calls++;
            return new Response('forwarded');
          },
        }),
      },
    });
    assert.equal(r.status, 404);
    assert.equal(calls, 0);
  });
test('public routing forwards a correctly prefixed request unchanged', async () => {
  let received;
  const r = await worker.fetch(new Request(env.WORKER_URL + '/' + prefix + '/rent'), {
    ...env,
    BROKER_PATH_PREFIX: prefix,
    LEDGER: {
      idFromName: () => 0,
      get: () => ({
        fetch: (req) => {
          received = req.url;
          return new Response('forwarded');
        },
      }),
    },
  });
  assert.equal(r.status, 200);
  assert.equal(received, env.WORKER_URL + '/' + prefix + '/rent');
});
test('scheduled cleanup works with a prefix and no public cron route', async () => {
  const h = await rent();
  h.time.value += 151 * 60000;
  const ledger = new BrokerLedger({ storage: h.storage }, h.bindings);
  ledger.service = h.service;
  assert.equal(
    (await ledger.fetch(new Request('https://internal.invalid/cron', { method: 'POST' }))).status,
    200,
  );
  assert.deepEqual(h.cloud.deletes, ['900']);
});
test('disabled renting still requires prefix', async () => {
  const h = await setup({ BROKER_PATH_PREFIX: prefix, RENTING_ENABLED: 'false' });
  assert.equal((await request(h, '/rent')).status, 404);
  assert.equal((await request(h, '/' + prefix + '/rent')).status, 503);
});
test('prefixed authenticated result retains its own instance until grace', async () => {
  const h = await rent();
  const token = callbackToken(await h.storage.get('record:123-1'));
  const r = await request(h, '/' + prefix + '/report/123-1', {
    token,
    body: {
      seq: 1,
      kind: 'result',
      result: { ...sampleResult(), nonce, verdict: 'PASS', runner_nonce: 'f'.repeat(64), sha },
    },
  });
  assert.equal(r.status, 200);
  assert.deepEqual(h.cloud.deletes, []);
  h.time.value += 300000;
  await h.service.sweep();
  assert.deepEqual(h.cloud.deletes, ['900']);
});
for (const bad of ["';$(id)", 'bad/path', 'p'.repeat(129), 'short'])
  test('onstart rejects prefix interpolation ' + bad.length, () =>
    assert.throws(() =>
      onstart(sha, nonce, 20, env.REPOSITORY, {
        url: env.WORKER_URL,
        path: bad,
        run: '123-1',
        token: 'v1.123.1.1800009000.' + 'A'.repeat(43),
      }),
    ),
  );
test('fixed callback URL includes the exact optional prefix', async () => {
  const h = await rent();
  const script = h.cloud.calls.find((x) => x.url.endsWith('/asks/5/')).body.onstart;
  assert.ok(script.includes(env.WORKER_URL + '/' + prefix + '/report/123-1'));
  assert.doesNotMatch(script, /BROKER_PATH_PREFIX|OWNER_STOP_TOKEN|RUN_TOKEN_SECRET/);
});
for (const text of [
  env.WORKER_URL,
  'broker.example',
  'broker',
  'example',
  prefix,
  encodeURIComponent(env.WORKER_URL),
])
  test('box log cannot reflect known endpoint component ' + text.slice(0, 8), async () => {
    const h = await rent();
    assert.equal((await report(h, 'value=' + text)).status, 200);
    const data = await (await request(h, '/' + prefix + '/status/123-1')).json();
    assert.ok(!data.log_tail_sanitised.includes(text), data.log_tail_sanitised);
  });
for (const p of [undefined, ''])
  test('unset optional prefix preserves original routes ' + String(p), async () => {
    const h = await setup({ BROKER_PATH_PREFIX: p });
    assert.equal((await h.call()).status, 200);
  });
