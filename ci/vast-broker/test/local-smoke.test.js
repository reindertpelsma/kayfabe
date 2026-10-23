import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, jwt, env, jwk, sha } from './helpers.js';

// H1 FALSIFIER: workerd's empty streaming POST cannot rent/stop, or a nonempty body rents.
for (const path of ['/rent', '/stop-all'])
  test('native bounded streaming POST accepts ' + path, async () => {
    const h = await setup();
    const token = path === '/rent' ? await jwt() : env.OWNER_STOP_TOKEN;
    const body = new ReadableStream({
      start(c) {
        if (path === '/rent')
          c.enqueue(
            new TextEncoder().encode(
              JSON.stringify({ runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: 7 }),
            ),
          );
        c.close();
      },
    });
    const r = await h.service.fetch(
      new Request(env.WORKER_URL + path, {
        method: 'POST',
        headers: { Authorization: 'Bearer ' + token },
        body,
        duplex: 'half',
      }),
    );
    assert.equal(r.status, 200);
  });
for (const path of ['/stop-all', '/stop/123-1'])
  test('native empty POST bad bearer is unauthorized ' + path, async () => {
    const h = await setup();
    const body = new ReadableStream({
      start(c) {
        c.close();
      },
    });
    const r = await h.service.fetch(
      new Request(env.WORKER_URL + path, {
        method: 'POST',
        headers: { Authorization: 'Bearer bad' },
        body,
        duplex: 'half',
      }),
    );
    assert.equal(r.status, 401);
  });
for (const body of ['x', '{}'])
  test('streaming nonempty POST still refuses ' + body, async () => {
    const h = await setup();
    const r = await h.service.fetch(
      new Request(env.WORKER_URL + '/rent', {
        method: 'POST',
        headers: { Authorization: 'Bearer ' + (await jwt()) },
        body,
        duplex: 'half',
      }),
    );
    assert.equal(r.status, 400);
    assert.equal(h.cloud.creates, 0);
  });

// H2 FALSIFIER: test trust activates without flag + loopback caller + loopback issuer.
const issuer = 'http://127.0.0.1:18788';
async function localCall({
  flag = 'true',
  origin = 'http://127.0.0.1:18787',
  testIssuer = issuer,
  claimsIssuer = issuer,
  skip = true,
} = {}) {
  const h = await setup({ ALLOW_TEST_ISSUER: flag, TEST_ISSUER: testIssuer });
  h.cloud.files = skip ? [{ filename: 'docs/test.md', status: 'modified' }] : h.cloud.files;
  const base = h.cloud.fetch;
  let localReads = 0;
  h.cloud.fetch = async (input, init) => {
    const u = new URL(input);
    if (u.origin === issuer) {
      localReads++;
      if (u.pathname === '/.well-known/jwks') return Response.json({ keys: [jwk] });
      return base('https://api.github.com' + u.pathname + u.search, init);
    }
    return base(input, init);
  };
  const { createService } = await import('../src/index.js');
  const service = createService({
    env: h.bindings,
    storage: h.storage,
    fetch: h.cloud.fetch,
    now: () => 1800000000000,
  });
  const r = await service.fetch(
    new Request(origin + '/' + h.bindings.BROKER_PATH_PREFIX + '/rent', {
      method: 'POST',
      headers: { Authorization: 'Bearer ' + (await jwt({ iss: claimsIssuer })) },
      body: JSON.stringify({ runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: 7 }),
    }),
  );
  return { r, h, localReads };
}
for (const skip of [true, false])
  test('loopback signed identity routes JWKS and GitHub ' + skip, async () => {
    const { r, h, localReads } = await localCall({ skip });
    assert.equal(r.status, 200);
    assert.ok(localReads >= 2); // JWKS and the one GitHub run read.
    assert.equal(h.cloud.creates, 1);
  });
for (const flag of [undefined, 'false'])
  test('local identity requires explicit flag ' + flag, async () => {
    const { r, localReads } = await localCall({ flag: flag ?? '' });
    assert.notEqual(r.status, 200);
    assert.equal(localReads, 0);
  });
for (const origin of ['https://broker.example', 'https://worker.workers.dev'])
  test('public request cannot enable local identity ' + origin, async () => {
    const { r, localReads } = await localCall({ origin });
    assert.notEqual(r.status, 200);
    assert.equal(localReads, 0);
  });
for (const testIssuer of [
  'https://evil.example',
  'http://127.0.0.1.evil.example:18788',
  'http://user:password@127.0.0.1:18788',
  'http://127.0.0.1:18788/path',
])
  test('test issuer must be strict loopback origin ' + testIssuer, async () => {
    const { r, h, localReads } = await localCall({ testIssuer });
    assert.notEqual(r.status, 200);
    assert.equal(localReads, 0);
    assert.equal(h.cloud.calls.length, 0);
  });
for (const claimsIssuer of ['https://evil.example', 'https://token.actions.githubusercontent.com'])
  test('local trust requires exact local issuer ' + claimsIssuer, async () => {
    const { r } = await localCall({ claimsIssuer });
    assert.notEqual(r.status, 200);
  });
