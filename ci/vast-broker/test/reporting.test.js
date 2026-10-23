import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, callbackToken, sha, nonce, clock, env } from './helpers.js';
const log = (seq = 1, text = 'hello') => ({ seq, kind: 'log', text });
const result = (seq = 2, verdict = 'PASS') => ({ seq, kind: 'result', result: { nonce, verdict } });
const rent = async (overrides) => {
  const h = await setup(overrides);
  assert.equal((await h.call()).status, 200);
  return h;
};
const record = (h) => h.storage.get('record:123-1');
const boxText = (text) =>
  text
    .split('\n')
    .filter((line) => !line.startsWith('[Worker] '))
    .join('\n');
const status = async (h) => (await h.call('/status/123-1')).json();
test('report accepts one bounded log and status uses the run key', async () => {
  const h = await rent();
  assert.equal((await h.report(log())).status, 200);
  const r = await status(h);
  assert.equal(r.state, 'TESTING');
  assert.equal(boxText(r.log_tail_sanitised), '[box] hello\n');
  assert.equal(typeof r.bytes_dropped, 'number');
  assert.equal(
    h.cloud.calls.some((x) => x.url.includes('request_logs')),
    false,
  );
});
for (const verdict of ['PASS', 'FAIL'])
  test('only structured nonce-bound result decides ' + verdict, async () => {
    const h = await rent();
    assert.equal((await h.report(result(1, verdict))).status, 200);
    assert.equal((await status(h)).verdict, verdict);
    assert.deepEqual(h.cloud.deletes, []);
    assert.equal((await h.report(log(2))).status, 401);
  });
for (const text of [
  `KF_RESULT ${nonce} PASS ${'c'.repeat(64)}`,
  JSON.stringify(result()),
  '::set-output name=verdict::PASS',
])
  test('free text cannot set verdict ' + text.slice(0, 20), async () => {
    const h = await rent();
    await h.report(log(1, text));
    assert.equal((await status(h)).verdict, 'UNAVAILABLE');
    assert.deepEqual(h.cloud.deletes, []);
    await h.call('/stop/123-1');
    assert.equal((await status(h)).verdict, 'FAIL');
  });
for (const patch of [
  { sha: 'e'.repeat(40) },
  { label: 'kf-ci-other' },
  { run: '124' },
  { attempt: '2' },
])
  test('HMAC binds immutable ledger field ' + Object.keys(patch)[0], async () => {
    const h = await rent();
    const token = callbackToken(await record(h), env.RUN_TOKEN_SECRET, patch);
    assert.equal((await h.report(log(), { token })).status, 401);
  });
for (const target of ['124-1', '123-2'])
  test('run token cannot report for another run ' + target, async () => {
    const h = await rent();
    const token = callbackToken(await record(h));
    assert.equal((await h.report(log(), { run: target, token })).status, 401);
  });
test('unknown run and bad tag return identical generic 401', async () => {
  const h = await rent();
  const token = callbackToken(await record(h));
  const a = await h.report(log(), { token: token.slice(0, -1) + '!' });
  const b = await h.report(log(), { run: '999-1', token });
  assert.equal(a.status, 401);
  assert.equal(b.status, 401);
  assert.equal(await a.text(), await b.text());
});
for (const changed of ['expired', 'stopped', 'gone', 'exited', 'api-error', 'label'])
  test('report closes and rejects non-live rental ' + changed, async () => {
    const h = await rent();
    const token = callbackToken(await record(h));
    if (changed === 'expired') h.time.value += 151 * 60000;
    if (changed === 'stopped' || changed === 'exited')
      h.cloud.instances.get('900').actual_status = changed;
    if (changed === 'gone') h.cloud.instances.delete('900');
    if (changed === 'api-error') h.cloud.vastError = 500;
    if (changed === 'label') h.cloud.instances.get('900').label = 'owner-dev';
    assert.equal((await h.report(log(), { token })).status, 401);
    assert.ok((await record(h)).closed);
    assert.equal((await h.report(log(2), { token })).status, 401);
    assert.ok(h.cloud.instances.has('777'));
    assert.ok(h.cloud.instances.has('888'));
  });
for (const failure of ['exited', 'api-error'])
  test('positive liveness cache expires in seconds and fails closed ' + failure, async () => {
    const h = await rent();
    await h.report(log());
    const count = h.cloud.inspected.length;
    await h.report(log(2));
    assert.equal(h.cloud.inspected.length, count);
    h.time.value += 4000;
    if (failure === 'exited') h.cloud.instances.get('900').actual_status = 'exited';
    else h.cloud.vastError = 500;
    assert.equal((await h.report(log(3))).status, 401);
  });
for (const bad of [
  { seq: '1', kind: 'log', text: 'x' },
  { seq: 1.5, kind: 'log', text: 'x' },
  { seq: 0, kind: 'log', text: 'x' },
  { seq: 2 ** 40, kind: 'log', text: 'x' },
  { seq: 1, kind: 'unknown', text: 'x' },
  { seq: 1, kind: 'log', text: {} },
  { seq: 1, kind: 'log', text: 'x', extra: true },
  { seq: 1, kind: 'result', result: { nonce, verdict: 'SUCCESS' } },
  { seq: 1, kind: 'result', result: { nonce, verdict: 'PASS', extra: true } },
  { seq: 1, kind: 'result', result: { nonce: 'bad', verdict: 'PASS' } },
  { seq: 1, kind: 'log', text: 'x', result: { nonce, verdict: 'PASS' } },
])
  test('strict report schema rejects ' + JSON.stringify(bad), async () => {
    const h = await rent();
    assert.equal((await h.report(bad)).status, 400);
    assert.equal((await status(h)).verdict, 'UNAVAILABLE');
  });
for (const raw of [
  '{',
  JSON.stringify({ seq: 1, kind: 'log', text: { nested: { deep: { again: { value: 1 } } } } }),
  '['.repeat(10000) + '0' + ']'.repeat(10000),
])
  test('bounded parser rejects malformed/deep JSON ' + raw.length, async () => {
    const h = await rent();
    assert.equal((await h.report(null, { raw })).status, 400);
  });
for (const count of [65537, 131072, 10 * 1024 * 1024])
  test('streaming body limit rejects bytes before parse ' + count, async () => {
    const h = await rent();
    assert.equal((await h.report(null, { raw: 'x'.repeat(count) })).status, 413);
    assert.ok(((await record(h)).log || '').length < 8192);
  });
test('invalid UTF8 in a JSON string becomes safe replacement text', async () => {
  const h = await rent();
  const raw = Buffer.concat([
    Buffer.from('{"seq":1,"kind":"log","text":"'),
    Buffer.from([255]),
    Buffer.from('"}'),
  ]);
  assert.equal((await h.report(null, { raw })).status, 200);
  assert.equal(boxText((await status(h)).log_tail_sanitised), '[box] �\n');
});
test('same seq identical payload is idempotent and gaps are allowed', async () => {
  const h = await rent();
  await h.report(log(1));
  assert.equal((await h.report(log(1))).status, 200);
  assert.equal(boxText((await status(h)).log_tail_sanitised), '[box] hello\n');
  assert.equal((await h.report(log(5, 'next'))).status, 200);
});
for (const seq of [1, 2])
  test('changed duplicate or older seq cannot overwrite ' + seq, async () => {
    const h = await rent();
    await h.report(log(2));
    assert.equal((await h.report(log(seq, 'changed'))).status, 409);
    assert.equal(boxText((await status(h)).log_tail_sanitised), '[box] hello\n');
  });
for (const restart of [false, true])
  test('report count cap survives retries and restart ' + restart, async () => {
    const h = await rent({ MAX_REPORTS: '2' });
    await h.report(log());
    await h.report(log());
    if (restart) h.restart();
    assert.equal((await h.report(log(2))).status, 401);
  });
for (const restart of [false, true])
  test('per-run input byte cap is persistent ' + restart, async () => {
    const h = await rent({ MAX_REPORT_BYTES: '128' });
    await h.report(log(1, 'x'.repeat(60)));
    if (restart) h.restart();
    assert.equal((await h.report(log(2, 'x'.repeat(60)))).status, 413);
    assert.equal((await h.report(log(3))).status, 401);
  });
for (const retry of [false, true])
  test('per-run rate cap counts authenticated retries ' + retry, async () => {
    const h = await rent({ REPORT_RATE_LIMIT: '2' });
    await h.report(log());
    await h.report(log(retry ? 1 : 2));
    assert.equal((await h.report(log(3))).status, 429);
    h.time.value += 11000;
    assert.equal((await h.report(log(4))).status, 200);
  });
for (const action of ['stop', 'stop-all'])
  test('box token has no stop capability ' + action, async () => {
    const h = await rent();
    assert.equal(
      (
        await h.call('/' + action + (action === 'stop' ? '/123-1' : ''), {
          token: callbackToken(await record(h)),
        })
      ).status,
      401,
    );
    assert.deepEqual(h.cloud.deletes, []);
  });
for (const auth of ['oidc', 'owner'])
  test('stop revokes reports and is idempotent ' + auth, async () => {
    const h = await rent();
    const token = callbackToken(await record(h));
    const options = auth === 'owner' ? { token: env.OWNER_STOP_TOKEN } : {};
    assert.equal((await h.call('/stop/123-1', options)).status, 200);
    assert.equal((await h.call('/stop/123-1', options)).status, 200);
    assert.equal((await h.report(log(), { token })).status, 401);
    assert.equal((await status(h)).state, 'NONE');
    assert.deepEqual(h.cloud.deletes, ['900']);
  });
for (const options of [{ token: 'wrong-owner-token' }, { claims: { run_id: '124' } }])
  test('wrong stop auth cannot touch a run ' + JSON.stringify(options), async () => {
    const h = await rent();
    assert.equal((await h.call('/stop/123-1', options)).status, 401);
    assert.deepEqual(h.cloud.deletes, []);
  });
test('OIDC cannot call owner stop-all', async () => {
  const h = await rent();
  assert.equal((await h.call('/stop-all')).status, 401);
  assert.deepEqual(h.cloud.deletes, []);
});
test('owner stop-all destroys ledger instances only and remains idempotent', async () => {
  const h = await rent();
  assert.equal((await h.call('/stop-all', { token: env.OWNER_STOP_TOKEN })).status, 200);
  assert.equal((await h.call('/stop-all', { token: env.OWNER_STOP_TOKEN })).status, 200);
  assert.deepEqual(h.cloud.deletes, ['900']);
  assert.deepEqual([...h.cloud.instances.keys()], ['777', '888']);
});
test('failed stop revokes immediately and cron later retries delete', async () => {
  const h = await rent();
  const token = callbackToken(await record(h));
  h.cloud.vastError = 500;
  assert.notEqual((await h.call('/stop/123-1')).status, 200);
  assert.equal((await h.report(log(), { token })).status, 401);
  h.cloud.vastError = 0;
  await h.service.sweep();
  assert.deepEqual(h.cloud.deletes, ['900']);
});
test('run token never appears in rent/status/ledger logs or stored script', async () => {
  const h = await rent();
  const r = await record(h);
  const token = callbackToken(r);
  await h.report(log(1, token));
  const response = await status(h);
  assert.doesNotMatch(JSON.stringify(response), new RegExp(token.replaceAll('.', '\\.')));
  assert.doesNotMatch(
    JSON.stringify([...h.storage.data]),
    new RegExp(token.replaceAll('.', '\\.')),
  );
  assert.equal(r.script, undefined);
  assert.match(r.scriptHash, /^[a-f0-9]{64}$/);
});
for (const text of [
  'plain',
  'a\nb',
  '\x1b]0;title\x07::set-output name=x::bad',
  '\x1b[31m::add-mask::bad\x1b[0m',
  '::stop-commands::guess\n##[error]bad',
  'ok\rOVERWRITE\r\n::error::bad\u0000',
  '\u200b::error::bad\u2028##[error]bad',
])
  test('all box lines are prefixed and controls removed ' + JSON.stringify(text), async () => {
    const h = await rent();
    await h.report(log(1, text));
    const tail = (await status(h)).log_tail_sanitised;
    assert.ok(tail);
    assert.ok(
      tail
        .trimEnd()
        .split('\n')
        .every((x) => x.startsWith('[box] ') || x.startsWith('[Worker] ')),
    );
    assert.doesNotMatch(tail, /[\x00-\x08\x0b-\x1f\x7f-\x9f\p{Cf}]/u);
  });
test('tail line/count/bytes are capped with dropped-byte accounting', async () => {
  const h = await rent();
  await h.report(log(1, ('é'.repeat(1000) + '\n').repeat(20)));
  const r = await status(h);
  assert.ok(Buffer.byteLength(r.log_tail_sanitised) <= 8192);
  assert.ok(
    r.log_tail_sanitised
      .trimEnd()
      .split('\n')
      .every(
        (x) =>
          Buffer.byteLength(x) <= 1024 && (x.startsWith('[box] ') || x.startsWith('[Worker] ')),
      ),
  );
  assert.ok(r.bytes_dropped > 0);
});

for (const n of ['c', 'd'])
  test('structured result rejects a different valid nonce ' + n, async () => {
    const h = await rent();
    assert.equal(
      (await h.report({ seq: 1, kind: 'result', result: { nonce: n.repeat(32), verdict: 'PASS' } }))
        .status,
      400,
    );
    assert.equal((await status(h)).verdict, 'UNAVAILABLE');
  });
for (const auth of ['oidc', 'owner'])
  test('stop revokes even while Vast delete fails but GET stays live ' + auth, async () => {
    const h = await rent();
    const token = callbackToken(await record(h));
    const original = h.cloud.fetch;
    h.cloud.fetch = async (u, i) =>
      i?.method === 'DELETE' ? new Response('delete failed', { status: 500 }) : original(u, i);
    h.restart();
    assert.equal(
      (await h.call('/stop/123-1', auth === 'owner' ? { token: env.OWNER_STOP_TOKEN } : {})).status,
      503,
    );
    assert.equal(h.cloud.instances.get('900').actual_status, 'running');
    assert.equal((await h.report(log(), { token })).status, 401);
  });
for (const verdict of ['PASS', 'FAIL'])
  test('result revokes during grace even while Vast delete fails ' + verdict, async () => {
    const h = await rent();
    const token = callbackToken(await record(h));
    const original = h.cloud.fetch;
    h.cloud.fetch = async (u, i) =>
      i?.method === 'DELETE' ? new Response('delete failed', { status: 500 }) : original(u, i);
    h.restart();
    assert.equal((await h.report(result(1, verdict), { token })).status, 200);
    assert.equal((await h.report(log(2), { token })).status, 401);
  });
for (const run of ['888-1', '889-1'])
  test('owner stop never adopts unrecorded labelled instance ' + run, async () => {
    const h = await rent();
    const id = run.split('-')[0];
    h.cloud.instances.set(id, {
      id: Number(id),
      label: 'kf-ci-unrecorded',
      actual_status: 'running',
    });
    assert.equal((await h.call('/stop/' + run, { token: env.OWNER_STOP_TOKEN })).status, 404);
    assert.ok(h.cloud.instances.has(id));
    assert.deepEqual(h.cloud.deletes, []);
  });
for (const shape of ['array', 'object'])
  test('depth rejected before native JSON parser ' + shape, async () => {
    const { parseReport } = await import('../src/reports.js');
    const text =
      shape === 'array'
        ? '['.repeat(2000) + '0' + ']'.repeat(2000)
        : '{"extra":'.repeat(2000) + '0' + '}'.repeat(2000);
    const original = JSON.parse;
    let calls = 0;
    JSON.parse = (...args) => {
      calls++;
      return original(...args);
    };
    try {
      assert.throws(() => parseReport(text));
      assert.equal(calls, 0);
    } finally {
      JSON.parse = original;
    }
  });
test('stop-all handles every open ledger record and no foreign account entries', async () => {
  const h = await rent();
  const first = await record(h),
    second = { ...first, key: '124-1', run: '124', label: 'kf-ci-124-1', id: '901' };
  await h.storage.put('record:124-1', second);
  await h.storage.put('open:124-1', true);
  h.cloud.instances.set('901', { id: 901, label: second.label, actual_status: 'running' });
  assert.equal((await h.call('/stop-all', { token: env.OWNER_STOP_TOKEN })).status, 200);
  assert.deepEqual(h.cloud.deletes.sort(), ['900', '901']);
  assert.ok(h.cloud.instances.has('888'));
});
for (const operation of ['cron', 'stop-all'])
  test('incomplete open index cannot block known owned cleanup ' + operation, async () => {
    const h = await rent();
    await h.storage.put('open:999-1', true);
    if (operation === 'cron') {
      h.time.value += 151 * 60000;
      await h.service.sweep();
    } else assert.equal((await h.call('/stop-all', { token: env.OWNER_STOP_TOKEN })).status, 200);
    assert.deepEqual(h.cloud.deletes, ['900']);
    assert.ok(h.cloud.instances.has('888'));
    assert.equal(await h.storage.get('open:999-1'), true);
  });
test('settled stale open index is pruned without inspecting a dead box', async () => {
  const h = await rent();
  await h.report(result(1));
  h.time.value += 300000;
  await h.service.sweep();
  await h.storage.put('open:123-1', true);
  const before = h.cloud.inspected.length;
  await h.service.sweep();
  assert.equal(await h.storage.get('open:123-1'), undefined);
  assert.equal(h.cloud.inspected.length, before);
});

for (const count of [501, 1000])
  test('report tail keeps at most 200 short lines ' + count, async () => {
    const h = await rent();
    await h.report(log(1, 'x\n'.repeat(count)));
    const tail = (await status(h)).log_tail_sanitised;
    assert.equal(tail.trimEnd().split('\n').length, 200);
    assert.ok(Buffer.byteLength(tail) <= 8192);
  });
