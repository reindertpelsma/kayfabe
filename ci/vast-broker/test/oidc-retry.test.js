import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// FALSIFIER (2026-11-11): a transient failure of the GitHub token endpoint (network, 408, 429, 5xx,
// a garbled 200) ends a healthy run; or a refusal (401/403/404, any other 4xx, a redirect) is
// retried; or the retry is unbounded (more than 20 requests, or a request started after 180 s); or
// a failed fetch is not named with its status and attempt; or any line shows a token, the request
// token, the URL or a body. KF_RUNNER_PATH lets scripts/ci/oidc_retry_mutants.py swap the runner.
const run = (args) => {
  const p = spawnSync(
    'python3',
    ['-I', new URL('oidc_retry_fixture.py', import.meta.url).pathname, JSON.stringify(args)],
    {
      encoding: 'utf8',
      timeout: 20000,
      env: { PATH: process.env.PATH, KF_RUNNER_PATH: process.env.KF_RUNNER_PATH || '' },
    },
  );
  assert.equal(p.status, 0, p.stderr);
  const r = JSON.parse(p.stdout);
  // GitHub's masking protocol prints a masked value on purpose; every other place must be clean.
  r.visible =
    r.stdout
      .split('\n')
      .filter((l) => !l.startsWith('::add-mask::'))
      .join('\n') +
    r.summary +
    r.outputs;
  r.notes = r.stdout.split('\n').filter((l) => l.startsWith('oidc: '));
  r.fatal = r.stdout.split('\n').filter((l) => l.startsWith('runner: '));
  r.waits = r.sleeps.map((s) => s[1]);
  return r;
};
const clean = (r) => {
  for (const secret of [r.sentinel, r.token, r.request_token, 'oidc.example', 'audience', 'test=1'])
    assert.ok(!r.visible.includes(secret), secret);
  for (const line of [...r.notes, ...r.fatal]) assert.match(line, /^[\x20-\x7e]{1,200}$/);
};
const PASS = ['availability', 'rent', 'logs', 'stop'];
const fresh = (r) => {
  // Every fetch is a new request for the same audience with the request token; the Worker only
  // ever receives the returned token.
  for (const call of r.oidc) {
    assert.equal(
      new URL(call.url).searchParams.get('audience'),
      'https://quiet-tenant.workers.example',
    );
    assert.equal(call.authorization, 'Bearer ' + r.request_token);
  }
  assert.deepEqual(r.bearers, r.routes.length ? ['Bearer ' + r.token] : []);
};

for (const [name, entry, note, wait] of [
  ['503', 503, 'HTTP 503', 2],
  ['500', 500, 'HTTP 500', 2],
  ['599', 599, 'HTTP 599', 2],
  ['408', 408, 'HTTP 408', 2],
  ['429 without Retry-After', 429, 'HTTP 429', 2],
  ['429 with Retry-After 25', { status: 429, retry_after: '25' }, 'HTTP 429', 25],
  ['429 with Retry-After 1 keeps the step', { status: 429, retry_after: '1' }, 'HTTP 429', 2],
  ['429 with Retry-After 9999 is capped', { status: 429, retry_after: '9999' }, 'HTTP 429', 60],
  ['429 with a date Retry-After', { status: 429, retry_after: 'Wed, 11 Nov 2026' }, 'HTTP 429', 2],
  ['503 ignores Retry-After', { status: 503, retry_after: '50' }, 'HTTP 503', 2],
  ['timeout', 'timeout', 'network TimeoutError', 2],
  ['connection reset', 'reset', 'network ConnectionResetError', 2],
  ['garbled body', 'garbled', 'HTTP 200 body not accepted', 2],
  ['token of the wrong shape', 'shape', 'HTTP 200 body not accepted', 2],
  ['token too long', 'long', 'HTTP 200 body not accepted', 2],
])
  test('oidc retry: ' + name + ' then 200 passes', () => {
    const r = run({ script: [entry] });
    assert.equal(r.status, 0, r.stdout);
    assert.deepEqual(r.routes, PASS);
    assert.equal(r.oidc.length, PASS.length + 1);
    assert.deepEqual(r.waits, [wait]);
    assert.deepEqual(r.notes, [
      'oidc: token fetch failed: ' + note + ', attempt 1 of 20, retry in ' + wait + ' s',
    ]);
    assert.deepEqual(r.fatal, []);
    assert.match(r.outputs, /verdict=PASS/);
    fresh(r);
    clean(r);
  });

test('oidc retry: a mixed outage in the middle of a run (TESTING) is survived', () => {
  const r = run({
    pending: true,
    skip: 2,
    script: ['timeout', 502, { status: 429, retry_after: '30' }, 'garbled', 503],
  });
  assert.equal(r.status, 0, r.stdout);
  assert.deepEqual(r.routes, ['availability', 'rent', 'status', 'logs', 'stop']);
  assert.equal(r.oidc.length, 5 + 5);
  // The first 20 s is the runner's own pause before the status poll.
  assert.deepEqual(r.waits, [20, 2, 10, 30, 10, 10]);
  assert.deepEqual(
    r.notes.map((l) => l.match(/attempt (\d+) of 20/)[1]),
    ['1', '2', '3', '4', '5'],
  );
  assert.match(r.notes[1], /^oidc: token fetch failed: HTTP 502, attempt 2 of 20, retry in 10 s$/);
  fresh(r);
  clean(r);
});

for (const status of [400, 401, 403, 404, 409, 410, 422, 204, 302])
  test('oidc retry: permanent ' + status + ' fails fast with no retry', () => {
    const r = run({ script: [status], repeat: true });
    assert.equal(r.status, 1);
    assert.deepEqual(r.routes, []);
    assert.deepEqual(r.waits, []);
    assert.deepEqual(r.notes, []);
    // One fetch for the first Worker call and one for the best-effort stop: neither is retried.
    assert.equal(r.oidc.length, 2);
    assert.deepEqual(r.fatal, [
      'runner: OIDC reply rejected: HTTP ' +
        status +
        ', attempt 1 of 20, not retried phase=oidc state=UNAVAILABLE http=none',
    ]);
    assert.match(r.outputs, /verdict=UNAVAILABLE/);
    fresh(r);
    clean(r);
  });

test('oidc retry: a redirect is not followed and not retried', () => {
  const r = run({ script: ['redirect'], repeat: true });
  assert.equal(r.status, 1);
  assert.deepEqual([r.routes, r.waits, r.oidc.length], [[], [], 2]);
  assert.deepEqual(r.fatal, [
    'runner: OIDC reply rejected: redirect, attempt 1 of 20, not retried phase=oidc state=UNAVAILABLE http=none',
  ]);
  clean(r);
});

test('oidc retry: one redirect ends the run even when the next reply would be good', () => {
  const r = run({ script: ['redirect'] });
  assert.equal(r.status, 1);
  assert.deepEqual(r.waits, []);
  assert.deepEqual(r.notes, []);
  assert.equal(r.routes.includes('rent'), false);
  assert.match(r.fatal[0], /^runner: OIDC reply rejected: redirect, attempt 1 of 20, not retried /);
  assert.match(r.outputs, /verdict=UNAVAILABLE/);
  clean(r);
});

test('oidc retry: a 401 after transient failures still fails at once', () => {
  const r = run({ script: [503, 503, 401], repeat: true });
  assert.equal(r.status, 1);
  assert.deepEqual(r.waits, [2, 10]);
  assert.equal(r.oidc.length, 3 + 1);
  assert.match(r.fatal[0], /^runner: OIDC reply rejected: HTTP 401, attempt 3 of 20, not retried /);
  clean(r);
});

// One fetch = one Worker call. A failed run makes one more Worker call (the best-effort stop), so
// a permanent outage costs two budgets; the checks below are per fetch.
const first = (r) => ({
  requests: r.oidc.slice(0, r.oidc.length / 2),
  waits: r.sleeps.filter((s) => s[0] <= r.oidc.length / 2).map((s) => s[1]),
});
for (const [name, entry, line] of [
  ['503', 503, 'runner: OIDC reply rejected: HTTP 503, attempt 20 of 20'],
  ['timeout', 'timeout', 'runner: network failure: OIDC network TimeoutError, attempt 20 of 20'],
  [
    'garbled body',
    'garbled',
    'runner: OIDC reply rejected: HTTP 200 body not accepted, attempt 20 of 20',
  ],
  [
    'token of the wrong shape',
    'shape',
    'runner: OIDC reply rejected: HTTP 200 body not accepted, attempt 20 of 20',
  ],
  [
    'token too long',
    'long',
    'runner: OIDC reply rejected: HTTP 200 body not accepted, attempt 20 of 20',
  ],
])
  test('oidc retry: permanent ' + name + ' is fatal after exactly the budget', () => {
    const r = run({ script: [entry], repeat: true });
    assert.equal(r.status, 1);
    assert.deepEqual(r.routes, []);
    assert.equal(r.oidc.length, 40);
    const { requests, waits } = first(r);
    assert.deepEqual(waits, [2, ...Array(17).fill(10), 8]);
    assert.equal(
      waits.reduce((a, b) => a + b),
      180,
    );
    assert.equal(requests.at(-1).at, 180);
    assert.deepEqual(r.fatal, [line + ' phase=oidc state=UNAVAILABLE http=none']);
    assert.equal(r.notes.length, 38);
    assert.match(r.notes[18], /attempt 19 of 20, retry in 8 s$/);
    assert.match(r.outputs, /verdict=UNAVAILABLE/);
    fresh(r);
    clean(r);
  });

test('oidc retry: Retry-After cannot stretch the budget', () => {
  const r = run({ script: [{ status: 429, retry_after: '9999' }], repeat: true });
  assert.equal(r.status, 1);
  assert.equal(r.oidc.length, 8);
  assert.deepEqual(first(r).waits, [60, 60, 60]);
  assert.equal(first(r).requests.at(-1).at, 180);
  assert.deepEqual(r.fatal, [
    'runner: OIDC reply rejected: HTTP 429, attempt 4 of 20 phase=oidc state=UNAVAILABLE http=none',
  ]);
  clean(r);
});

for (const entry of [503, 'timeout'])
  test('oidc retry: the attempt cap ends the loop when the clock stands still ' + entry, () => {
    const r = run({ script: [entry], repeat: true, frozen: true });
    assert.equal(r.status, 1);
    assert.equal(r.oidc.length, 40);
    assert.match(r.fatal[0], /attempt 20 of 20 phase=oidc/);
    clean(r);
  });

for (const [seconds, requests] of [
  [20, 7],
  [45, 4],
  [200, 1],
])
  test(`oidc retry: no request starts after 180 s when one takes ${seconds} s`, () => {
    const r = run({ script: ['timeout'], repeat: true, seconds });
    assert.equal(r.status, 1);
    assert.equal(r.oidc.length, 2 * requests);
    const start = r.oidc[requests].at;
    for (const call of r.oidc.slice(0, requests)) assert.ok(call.at < 180, String(call.at));
    for (const call of r.oidc.slice(requests)) assert.ok(call.at - start < 180);
    // Hard cap: the last request may run its own 20 s timeout past the budget, nothing more.
    assert.ok(start <= 180 + seconds);
    assert.match(
      r.fatal[0],
      new RegExp(
        '^runner: network failure: OIDC network TimeoutError, attempt ' + requests + ' of 20 ',
      ),
    );
    clean(r);
  });
