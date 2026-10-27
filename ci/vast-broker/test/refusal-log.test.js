import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, env } from './helpers.js';
// A 401 is the same bare body for every cause, so the Worker logs WHICH check refused (a fixed
// code, never a token, claim value, header or URL) for the owner's Workers observability.
// FALSIFIER: a refusal logs nothing, or a log line carries any token/claim value.
const capture = async (fn) => {
  const logged = [],
    original = console.log;
  console.log = (...args) => logged.push(args.join(' '));
  try {
    await fn();
  } finally {
    console.log = original;
  }
  return logged.map((l) => JSON.parse(l)).filter((l) => l.event === 'refused');
};
const sentinel = 'zq-sentinel-value';
for (const [name, patch, expected] of [
  ['wrong audience', { aud: 'https://other.example' }, 'oidc:ERR_JWT_CLAIM_VALIDATION_FAILED:aud'],
  ['wrong issuer', { iss: 'https://evil.example' }, 'oidc:ERR_JWT_CLAIM_VALIDATION_FAILED:iss'],
  [
    'wrong environment subject',
    { sub: 'repo:x/y:ref:refs/heads/master' + sentinel },
    'identity:sub',
  ],
  ['wrong repository id', { repository_id: '1' }, 'identity:repository_id'],
  ['wrong ref', { ref: 'refs/heads/other' }, 'identity:ref'],
  [
    'wrong workflow',
    { workflow_ref: 'a/b/.github/workflows/x.yml@refs/heads/master' },
    'identity:workflow',
  ],
])
  test('refusal log names the failing check: ' + name, async () => {
    const h = await setup();
    const lines = await capture(async () => {
      const response = await h.call('/availability?profile=check', {
        method: 'GET',
        claims: patch,
      });
      assert.equal(response.status, 401);
      assert.ok(!(await response.text()).includes(expected), 'the code reached the response');
    });
    assert.equal(lines.length, 1);
    assert.equal(lines[0].status, 401);
    assert.equal(lines[0].why, expected);
    assert.ok(!JSON.stringify(lines).includes(sentinel));
    assert.ok(!JSON.stringify(lines).includes(env.WORKER_URL));
  });

for (const field of ['run_attempt', 'status'])
  test('run refusal logs only the fixed metadata field: ' + field, async () => {
    const h = await setup();
    h.cloud.run[field] = sentinel;
    const lines = await capture(async () => {
      const response = await h.call('/availability?profile=check', { method: 'GET' });
      assert.equal(response.status, 401);
      assert.deepEqual(await response.json(), {
        state: 'UNAUTHORIZED',
        verdict: 'UNAVAILABLE',
        log_tail_sanitised: '',
        bytes_dropped: 0,
      });
    });
    assert.deepEqual(lines, [
      {
        event: 'refused',
        route: 'availability',
        status: 401,
        why: 'github:run:' + field,
      },
    ]);
    assert.ok(!JSON.stringify(lines).includes(sentinel));
    assert.ok(!JSON.stringify(lines).includes(env.WORKER_URL));
  });
