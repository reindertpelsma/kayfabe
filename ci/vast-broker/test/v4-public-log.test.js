import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { setup, nonce, sha } from './helpers.js';
// FALSIFIERS: repeated cursor output, omitted bounded final log, command/secret
// survives, untrusted verdict controls status, or a box fence escapes summary.
async function rent(extra = {}) {
  const h = await setup(extra);
  assert.equal((await h.call()).status, 200);
  return h;
}
const logs = (h, cursor = 0, options = {}) =>
  h.call('/logs/123-1?cursor=' + cursor, { method: 'GET', ...options });
for (const [name, text] of [
  ['IPv4', ['203', '0', '113', '18'].join('.')],
  ['second IPv4', ['198', '51', '100', '2'].join('.')],
  ['IPv6', '2001' + ':db8::19'],
  ['IPv6 compressed', ':' + ':1'],
  ['instance ID', '900'],
  ['instance label', 'kf-ci-123-1'],
  ['bearer', 'Bearer ' + Array(40).fill('Q').join('')],
  ['JWT', ['eyJhbGciOiJSUzI1NiJ9', 'eyJzdWIiOiJwbGFudGVkIn0', 'S'.repeat(80)].join('.')],
  ['opaque', 'secret'.repeat(8)],
])
  test('V4 public log redacts planted ' + name, async () => {
    const h = await rent();
    assert.equal((await h.report({ seq: 1, kind: 'log', text: 'value ' + text })).status, 200);
    const status = await (await h.call('/status/123-1')).json();
    assert.ok(!status.log_tail_sanitised.includes(text), status.log_tail_sanitised);
    if (name.includes('IPv')) assert.match(status.log_tail_sanitised, /\[address\]/);
    const response = await logs(h);
    assert.equal(response.status, 200);
    const page = await response.json();
    assert.ok(!page.log_increment_sanitised.includes(text));
  });
test('V4 full log retains earlier lines beyond tail and cursor emits once', async () => {
  const h = await rent();
  let expected = '';
  for (let seq = 1; seq <= 4; seq++) {
    const text = Array.from({ length: 300 }, (_, i) => `message ${seq} line ${i}\n`).join('');
    assert.equal((await h.report({ seq, kind: 'log', text })).status, 200);
    expected += text;
  }
  h.restart();
  let cursor = 0,
    all = '';
  for (let i = 0; i < 20; i++) {
    const p = await (await logs(h, cursor)).json();
    assert.equal(p.cursor, cursor);
    assert.ok(Number.isSafeInteger(p.next_cursor));
    all += p.log_increment_sanitised;
    if (p.next_cursor === cursor) break;
    cursor = p.next_cursor;
  }
  assert.match(all, /message 1 line 0/);
  assert.match(all, /message 4 line 299/);
  assert.equal(all.match(/message 1 line 0\n/g)?.length, 1);
  const status = await (await h.call('/status/123-1')).json();
  assert.doesNotMatch(status.log_tail_sanitised, /message 1 line 0/);
  assert.ok(status.log_cursor >= cursor);
  if (status.log_cursor > cursor) {
    const own = await (await logs(h, cursor)).json();
    assert.match(own.log_increment_sanitised, /\[Worker\]/);
    cursor = own.next_cursor;
    assert.equal(status.log_cursor, cursor);
  }
  assert.equal((await (await logs(h, cursor)).json()).log_increment_sanitised, '');
  assert.ok(Buffer.byteLength(all) <= 4194304);
});
for (const cursor of ['-1', '1.5', '1&extra=x', '99999999999'])
  test('V4 log cursor rejects ' + cursor, async () => {
    const h = await rent();
    assert.equal((await logs(h, cursor)).status, 400);
  });
test('V4 full log denied to another run and to box capability', async () => {
  const h = await rent();
  assert.equal((await logs(h, 0, { claims: { run_id: '124' } })).status, 401);
  const { callbackToken } = await import('./helpers.js');
  assert.equal(
    (await logs(h, 0, { token: callbackToken(await h.storage.get('record:123-1')) })).status,
    401,
  );
});
test('V4 bounded log cap counts dropped rendered bytes', async () => {
  const h = await rent({ MAX_REPORT_BYTES: '1200' });
  await h.report({ seq: 1, kind: 'log', text: 'short\n'.repeat(140) });
  const p = await (await logs(h)).json();
  assert.ok(Buffer.byteLength(p.log_increment_sanitised) <= 1200);
  assert.ok(p.bytes_dropped > 0);
});
async function render(mode, data, log = '') {
  const { publicLogRenderer } = await import('../public-log.js');
  const dir = await mkdtemp(tmpdir() + '/v4-render-');
  try {
    await writeFile(dir + '/data.json', JSON.stringify(data));
    await writeFile(dir + '/log.txt', log);
    await writeFile(dir + '/summary', '');
    const result = spawnSync(
      'python3',
      ['-c', publicLogRenderer, mode, dir + '/data.json', dir + '/log.txt'],
      { encoding: 'utf8', env: { ...process.env, GITHUB_STEP_SUMMARY: dir + '/summary' } },
    );
    assert.equal(result.status, 0, result.stderr);
    return { stdout: result.stdout, summary: await readFile(dir + '/summary', 'utf8') };
  } finally {
    await rm(dir, { recursive: true });
  }
}
test('V4 summary uses structured verdict, metadata and fence longer than box text', async () => {
  const text =
    '[box] ' + String.fromCharCode(96).repeat(9) + '\n[box] FAIL in a log is not the verdict\n';
  const r = await render('summary', {
    state: 'COMPLETE',
    verdict: 'PASS',
    sha,
    gpu_model: 'RTX 4070',
    started_at: 1000,
    result_at: 3000,
    cost_estimate_usd: 0.01,
    log_tail_sanitised: text,
    bytes_dropped: 7,
  });
  assert.match(r.summary, /UNTRUSTED output of the rented box/);
  assert.match(r.summary, /\| Verdict \| PASS \|/);
  assert.match(r.summary, /RTX 4070/);
  assert.match(r.summary, /0.010000/);
  assert.match(r.summary, /aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/);
  assert.ok(
    r.summary
      .split('\n')
      .filter((x) => /^`+$/.test(x))
      .every((x) => x.length > 9),
  );
  assert.doesNotMatch(r.summary, /nonce/);
});
for (const state of ['NOT_ELIGIBLE', 'NEEDS_APPROVAL', 'RENTING_DISABLED', 'BUSY'])
  test('V4 public refusal section explains ' + state, async () => {
    const r = await render('summary', { state, verdict: 'UNAVAILABLE' });
    assert.match(r.summary, new RegExp(state));
    assert.match(r.summary, /No box result/);
  });
test('V4 rendered group cannot be closed by hostile box commands', async () => {
  const r = await render(
    'final',
    { bytes_dropped: 100 },
    '[box] ::endgroup::\n[box] ##[group]evil\n\u001b[31m[box] red\n',
  );
  assert.equal((r.stdout.match(/::endgroup::/g) || []).length, 1);
  assert.equal((r.stdout.match(/::group::/g) || []).length, 1);
  assert.doesNotMatch(r.stdout, /##\[|\x1b/);
  assert.match(r.stdout, /100/);
});
test('V4 workflow now fetches the exact BASE controller bootstrap', async () => {
  const { runnerBootstrap } = await import('../workflow-bootstrap.js'),
    workflow = await readFile(
      new URL('../../../.github/workflows/gpu-vast.yml', import.meta.url),
      'utf8',
    );
  const embedded = workflow
    .split('  gpu:\n')[1]
    .split('  report:\n')[0]
    .split('        run: |\n')[1]
    .split('\n')
    .map((x) => (x.startsWith('          ') ? x.slice(10) : x))
    .join('\n');
  assert.equal(embedded.trim(), runnerBootstrap.trim());
  const controller = await readFile(new URL('../runner.py', import.meta.url), 'utf8');
  assert.match(controller, /Final bounded log/);
  assert.match(controller, /self.pages/);
  assert.match(controller, /if not terminal and not self.infrastructure_failure:/);
});
for (const restart of [false, true])
  test('V4 incremental log cursor survives new output ' + restart, async () => {
    const h = await rent();
    await h.report({ seq: 1, kind: 'log', text: 'first message' });
    const first = await (await logs(h)).json();
    assert.match(first.log_increment_sanitised, /first message/);
    if (restart) h.restart();
    await h.report({ seq: 2, kind: 'log', text: 'second message' });
    const next = await (await logs(h, first.next_cursor)).json();
    assert.match(next.log_increment_sanitised, /second message/);
    assert.doesNotMatch(next.log_increment_sanitised, /first message/);
  });
test('V4 summary rejects log-implied PASS and fence escape', async () => {
  const r = await render('summary', {
    state: 'COMPLETE',
    verdict: 'FAIL',
    log_tail_sanitised:
      '[box] PASS\n' + String.fromCharCode(96).repeat(13) + '\n::error::x\n##[group]x',
  });
  assert.match(r.summary, /\| Verdict \| FAIL \|/);
  assert.doesNotMatch(r.summary, /::|##\[/);
  assert.ok(r.summary.match(/(`+)text\n/)[1].length > 13);
  assert.ok(r.summary.trimEnd().split('\n').at(-1).length > 13);
});
test('V4 smaller full log cap bounds prefix expansion', async () => {
  const h = await rent({ MAX_REPORT_BYTES: '600' });
  await h.report({ seq: 1, kind: 'log', text: 'short\n'.repeat(65) });
  const page = await (await logs(h)).json();
  assert.ok(Buffer.byteLength(page.log_increment_sanitised) <= 600);
  assert.ok(page.bytes_dropped > 0);
});
