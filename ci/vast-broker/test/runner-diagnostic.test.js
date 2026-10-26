import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
// FALSIFIER: a failed first Worker call leaves the owner unable to tell the cause, or the
// one-line diagnostic carries a setting value, URL, path, header or body, or it changes the
// exit code / state of a failed run. The diagnostic is trusted runner text, so it is built
// only from an integer status, a sanitised media type, a class name, a setting name and a
// fixed reason.
const SENTINEL = 'SeNtInEl9f3a7c1d';
const REJECT = 'runner: setting rejected before any request: ';
const REPLY = 'runner: Worker reply not accepted: ';
const run = (args) => {
  const p = spawnSync(
    'python3',
    [
      '-I',
      new URL('runner_diagnostic_fixture.py', import.meta.url).pathname,
      JSON.stringify({ sentinel: SENTINEL, ...args }),
    ],
    { encoding: 'utf8', timeout: 10000 },
  );
  assert.equal(p.status, 0, p.stderr);
  return JSON.parse(p.stdout);
};
// GitHub's masking protocol prints a masked value on purpose; every other place must be clean.
const outside = (r) =>
  [r.stdout, r.stderr, r.summary, r.outputs]
    .join('\n')
    .split('\n')
    .filter((l) => !l.startsWith('::add-mask::'))
    .join('\n');
const diagnostics = (r) => r.stdout.split('\n').filter((l) => l.startsWith('runner: '));
const sentinelSecret = 'a'.repeat(32) + SENTINEL;
const cases = [
  {
    name: 'Access-style 403 text/html',
    args: { reply: 'html403' },
    line: REPLY + 'HTTP 403, content-type text/html;?charset=utf-8',
  },
  {
    name: 'Worker 404 text/plain',
    args: { reply: 'text404' },
    line: REPLY + 'HTTP 404, content-type text/plain',
  },
  {
    name: 'redirect 302',
    args: { reply: 'redirect302' },
    line: REPLY + 'HTTP 302, content-type text/html',
  },
  {
    name: 'odd content-type is sanitised and truncated to 64 characters',
    args: { reply: 'weird' },
    line: REPLY + 'HTTP 502, content-type x?y?z?' + 'q'.repeat(58),
  },
  {
    name: 'Worker 401 JSON with a bare state names the state',
    args: { reply: 'json401' },
    line: REPLY + 'HTTP 401, content-type application/json, state UNAUTHORIZED',
  },
  {
    name: 'Worker 401 JSON with an unknown state shows no body text',
    args: { reply: 'json401hostile' },
    line: REPLY + 'HTTP 401, content-type application/json',
  },
  {
    name: 'timeout',
    args: { reply: 'timeout' },
    line: 'runner: network failure: TimeoutError',
  },
  {
    name: 'connection reset',
    args: { reply: 'reset' },
    line: 'runner: network failure: ConnectionResetError',
  },
  {
    name: 'empty WORKER_PATH is accepted, so the Worker answers',
    args: { reply: 'text404', env: { WORKER_PATH: '' } },
    line: REPLY + 'HTTP 404, content-type text/plain',
  },
  {
    name: 'WORKER_URL trailing slash is accepted, so the Worker answers',
    args: { reply: 'text404', env: { WORKER_URL: 'https://quiet-tenant.workers.example/' } },
    line: REPLY + 'HTTP 404, content-type text/plain',
  },
  {
    name: 'secret with the sentinel as a valid value',
    args: { reply: 'html403', env: { CF_ACCESS_CLIENT_SECRET: sentinelSecret } },
    line: REPLY + 'HTTP 403, content-type text/html;?charset=utf-8',
  },
  {
    name: 'invalid secret characters',
    args: { env: { CF_ACCESS_CLIENT_SECRET: sentinelSecret + ' !\n' } },
    line: REJECT + 'CF_ACCESS_CLIENT_SECRET bad characters',
  },
  {
    name: 'short secret',
    args: { env: { CF_ACCESS_CLIENT_SECRET: SENTINEL } },
    line: REJECT + 'CF_ACCESS_CLIENT_SECRET too short',
  },
  {
    name: 'over-long secret',
    args: { env: { CF_ACCESS_CLIENT_SECRET: SENTINEL + 'a'.repeat(300) } },
    line: REJECT + 'CF_ACCESS_CLIENT_SECRET bad characters',
  },
  {
    name: 'secret set without an id',
    args: { env: { CF_ACCESS_CLIENT_ID: '', CF_ACCESS_CLIENT_SECRET: sentinelSecret } },
    line: REJECT + 'CF_ACCESS_CLIENT_ID empty',
  },
  {
    name: 'id set without a secret',
    args: { env: { CF_ACCESS_CLIENT_SECRET: '', CF_ACCESS_CLIENT_ID: SENTINEL } },
    line: REJECT + 'CF_ACCESS_CLIENT_SECRET empty',
  },
  {
    name: 'invalid id characters',
    args: { env: { CF_ACCESS_CLIENT_ID: SENTINEL + ' x' } },
    line: REJECT + 'CF_ACCESS_CLIENT_ID bad characters',
  },
  {
    name: 'URL with a path',
    args: { env: { WORKER_URL: 'https://quiet-tenant.workers.example/' + SENTINEL } },
    line: REJECT + 'WORKER_URL trailing slash or path',
  },
  {
    name: 'URL with a trailing slash after a path',
    args: { env: { WORKER_URL: 'https://quiet-tenant.workers.example/p/' } },
    line: REJECT + 'WORKER_URL trailing slash or path',
  },
  {
    name: 'URL that is not https',
    args: { env: { WORKER_URL: 'http://' + SENTINEL + '.example' } },
    line: REJECT + 'WORKER_URL bad characters',
  },
  {
    name: 'empty URL',
    args: { env: { WORKER_URL: '' } },
    line: REJECT + 'WORKER_URL empty',
  },
  {
    name: 'missing URL',
    args: { env: { WORKER_URL: null } },
    line: REJECT + 'WORKER_URL empty',
  },
  {
    name: 'short path',
    args: { env: { WORKER_PATH: SENTINEL } },
    line: REJECT + 'WORKER_PATH too short',
  },
  {
    name: 'path with a slash',
    args: { env: { WORKER_PATH: SENTINEL + '/' + 'p'.repeat(40) } },
    line: REJECT + 'WORKER_PATH trailing slash or path',
  },
  {
    name: 'path with bad characters',
    args: { env: { WORKER_PATH: SENTINEL + '!' + 'p'.repeat(40) } },
    line: REJECT + 'WORKER_PATH bad characters',
  },
  {
    name: 'missing OIDC request URL',
    args: { reply: 'text404', env: { ACTIONS_ID_TOKEN_REQUEST_URL: null } },
    line: REJECT + 'GITHUB_OIDC empty',
  },
  {
    name: 'missing OIDC request token',
    args: { reply: 'text404', env: { ACTIONS_ID_TOKEN_REQUEST_TOKEN: '' } },
    line: REJECT + 'GITHUB_OIDC empty',
  },
];
for (const c of cases)
  test('runner diagnostic: ' + c.name, () => {
    const r = run(c.args);
    const lines = diagnostics(r);
    assert.equal(lines.length, 1);
    assert.equal(lines[0].split(' phase=')[0], c.line);
    assert.match(lines[0], / phase=[a-z]+ state=[A-Z_]+ http=(?:none|[1-5]xx)/);
    assert.ok(!outside(r).includes(SENTINEL), 'a sentinel value reached the output');
    assert.ok(!outside(r).includes('quiet-tenant'), 'the Worker host reached the output');
    assert.ok(!outside(r).includes('unit-path-only'), 'the Worker path reached the output');
    assert.equal(r.status, 1);
    assert.match(r.outputs, /^state=UNAVAILABLE\nverdict=UNAVAILABLE\nverified=0\n$/);
  });
test('runner diagnostic sits outside the untrusted block', () => {
  const r = run({ reply: 'html403' });
  const at = r.stdout.indexOf(REPLY);
  assert.ok(at >= 0);
  const group = r.stdout.indexOf('::group::Final bounded log');
  const end = r.stdout.indexOf('::endgroup::');
  assert.ok(group >= 0 && end > group);
  assert.ok(at < group || at > end, 'the diagnostic is inside the untrusted group');
  assert.ok(r.stdout.slice(0, at).split('\n').pop() === '', 'the diagnostic starts a line');
});
test('runner diagnostic is silent on a successful run', () => {
  const p = spawnSync(
    'python3',
    ['-I', new URL('round3_main_fixture.py', import.meta.url).pathname, '{}'],
    { encoding: 'utf8', timeout: 10000 },
  );
  assert.equal(p.status, 0, p.stderr);
  const r = JSON.parse(p.stdout);
  assert.equal(r.status, 0);
  assert.ok(!/^runner: /m.test(r.stdout));
});
test('runner diagnostic is silent when a transient failure recovers', () => {
  const p = spawnSync(
    'python3',
    ['-I', new URL('round3_main_fixture.py', import.meta.url).pathname, '{"failure":"rent"}'],
    { encoding: 'utf8', timeout: 10000 },
  );
  assert.equal(p.status, 0, p.stderr);
  assert.ok(!/^runner: /m.test(JSON.parse(p.stdout).stdout));
});
