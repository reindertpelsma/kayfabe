import test from 'node:test';
import assert from 'node:assert/strict';
import { createHmac, timingSafeEqual } from 'node:crypto';
import { jwtVerify } from 'jose';
import { setup, env } from './helpers.js';
// Owner addendum 19:45 item 3: run tokens are jose HS256 JWTs (claims run, attempt, sha, nonce,
// exp <= the hard limit; RUN_TOKEN_SECRET; algorithms pinned), and owner/prefix comparisons use
// the platform's timingSafeEqual behind a seam.
// FALSIFIER: the issued token is not an HS256 JWT over those claims, outlives the hard limit,
// or the Worker accepts a token with another algorithm, key, run, claim or an expired exp.
const b64 = (value) => Buffer.from(JSON.stringify(value)).toString('base64url');
const sign = (header, claims, secret = env.RUN_TOKEN_SECRET, alg = 'sha256') => {
  const input = b64(header) + '.' + b64(claims);
  return input + '.' + createHmac(alg, secret).update(input).digest('base64url');
};
async function rented() {
  const h = await setup();
  assert.equal((await h.call()).status, 200);
  const record = await h.storage.get('record:123-1');
  const token = h.cloud.calls.find((c) => c.url.includes('/asks/')).body.env.KF_RUN_TOKEN;
  return { h, record, token };
}
const progress = (h, token) => h.report({ seq: 1, kind: 'progress', text: 'x' }, { token });
test('run token: issued as an HS256 JWT over run, attempt, sha, nonce and exp', async () => {
  const { record, token } = await rented();
  const { payload, protectedHeader } = await jwtVerify(
    token,
    new TextEncoder().encode(env.RUN_TOKEN_SECRET),
    { algorithms: ['HS256'], currentDate: new Date(record.started) },
  );
  assert.deepEqual(protectedHeader, { alg: 'HS256' });
  assert.deepEqual(Object.keys(payload).sort(), ['attempt', 'exp', 'nonce', 'run', 'sha']);
  assert.deepEqual(
    [payload.run, payload.attempt, payload.sha, payload.nonce],
    ['123', '1', record.sha, record.nonce],
  );
  assert.equal(payload.exp, Math.floor(record.hardDeadline / 1000));
  assert.equal(record.reportExpiry, payload.exp);
});
test('run token: the issued token authenticates its own box', async () => {
  const { h, token } = await rented();
  assert.equal((await progress(h, token)).status, 200);
});
for (const [name, forge] of [
  ['alg none', (c) => b64({ alg: 'none' }) + '.' + b64(c) + '.'],
  ['HS512 under the same secret', (c) => sign({ alg: 'HS512' }, c, undefined, 'sha512')],
  ['another secret', (c) => sign({ alg: 'HS256' }, c, 'x'.repeat(43))],
  ['another attempt', (c) => sign({ alg: 'HS256' }, { ...c, attempt: '2' })],
  ['another sha', (c) => sign({ alg: 'HS256' }, { ...c, sha: 'e'.repeat(40) })],
  ['another nonce', (c) => sign({ alg: 'HS256' }, { ...c, nonce: 'e'.repeat(32) })],
  ['a later exp', (c) => sign({ alg: 'HS256' }, { ...c, exp: c.exp + 600 })],
  ['no exp', (c) => sign({ alg: 'HS256' }, { ...c, exp: undefined })],
  ['a tampered signature', (c) => sign({ alg: 'HS256' }, c).slice(0, -2) + 'AA'],
])
  test('run token: the Worker refuses ' + name, async () => {
    const { h, record } = await rented();
    const claims = {
      run: '123',
      attempt: '1',
      sha: record.sha,
      nonce: record.nonce,
      exp: record.reportExpiry,
    };
    const response = await progress(h, forge(claims));
    assert.equal(response.status, 401);
  });
test('run token: a correctly signed copy of the issued claims is accepted', async () => {
  const { h, record } = await rented();
  const claims = { run: '123', attempt: '1', sha: record.sha, nonce: record.nonce };
  const token = sign({ alg: 'HS256' }, { ...claims, exp: record.reportExpiry });
  assert.equal((await progress(h, token)).status, 200);
});
test('run token: refused once exp has passed', async () => {
  const { h, record, token } = await rented();
  h.time.value = record.reportExpiry * 1000;
  assert.equal((await progress(h, token)).status, 401);
});
test('timing-safe compare comes from the platform seam and fails closed without it', async () => {
  const { constantTimeEqual, useTimingSafeEqual } = await import('../src/tokens.js');
  const calls = [];
  useTimingSafeEqual((a, b) => (calls.push([a.byteLength, b.byteLength]), timingSafeEqual(a, b)));
  try {
    assert.equal(await constantTimeEqual('a'.repeat(40), 'a'.repeat(40)), true);
    assert.equal(await constantTimeEqual('a'.repeat(40), 'b'), false);
    assert.deepEqual(calls, [
      [32, 32],
      [32, 32],
    ]);
    useTimingSafeEqual(undefined);
    await assert.rejects(constantTimeEqual('a'.repeat(40), 'a'.repeat(40)));
  } finally {
    useTimingSafeEqual(timingSafeEqual);
  }
});
