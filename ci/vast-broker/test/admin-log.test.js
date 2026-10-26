import test from 'node:test';
import assert from 'node:assert/strict';
import { BrokerLedger, createGateway } from '../src/index.js';
import { Ledger } from '../src/ledger.js';
import { env, MemoryStorage } from './helpers.js';

// FALSIFIER: external callers reach logs, a page leaks capability fields or exceeds 16 KiB,
// cursor traversal loses/duplicates text, or pruned logs are reported as available.
const request = (query = 'run=123-1', origin = 'https://internal.invalid', method = 'GET') =>
  new Request(origin + '/admin/log?' + query, { method });
const fixture = async (patch = {}) => {
  const storage = new MemoryStorage();
  const ledger = new Ledger(storage);
  const summary =
    '[box] KF_FINAL_SUMMARY_BEGIN verdict=FAIL\n' +
    '[box] stage=tests rc=1\n[box] KF_FINAL_SUMMARY_END\n';
  const record = {
    key: '123-1',
    ended: 123,
    started: 0,
    logChunks: 0,
    nonce: 'PRIVATE-CAPABILITY',
    runnerNonce: 'PRIVATE-CAPABILITY',
    result: { secret: 'PRIVATE-CAPABILITY' },
    log: '[box] prelude\n' + summary + '[Worker] teardown\n',
    ...patch,
  };
  const chunks = Array.from({ length: 6 }, (_, i) => '[box] ' + i + ':' + 'x'.repeat(16000) + '\n');
  for (const chunk of chunks) await ledger.appendLog(record, chunk);
  await ledger.save(record);
  return { storage, ledger, record, summary, chunks, object: new BrokerLedger({ storage }, env) };
};

for (const origin of ['https://broker.example', 'http://internal.invalid'])
  test('admin log rejects non-internal origin ' + origin, async () => {
    const h = await fixture();
    assert.equal((await h.object.fetch(request('run=123-1', origin))).status, 404);
  });

for (const path of ['/admin/log?run=123-1', '/' + 'p'.repeat(32) + '/admin/log?run=123-1'])
  test('admin log is absent from public gateway ' + path, async () => {
    let calls = 0;
    const worker = createGateway({ authorize: async () => calls++ });
    const response = await worker.fetch(new Request('https://broker.example' + path), {
      ...env,
      RENTING_ENABLED: 'false',
      BROKER_PATH_PREFIX: path.startsWith('/admin') ? '' : 'p'.repeat(32),
      LEDGER: {
        idFromName: () => {
          calls++;
          throw Error();
        },
      },
    });
    assert.equal(response.status, 404);
    assert.equal(calls, 0);
  });

for (const ended of [null, 123])
  test('admin log traverses exact bounded scalar pages, ended=' + ended, async () => {
    const h = await fixture({ ended });
    let cursor = 0;
    let joined = '';
    let pages = 0;
    do {
      const response = await h.object.fetch(request('run=123-1&cursor=' + cursor));
      assert.equal(response.status, 200);
      assert.equal(response.headers.get('cache-control'), 'no-store');
      const page = await response.json();
      assert.deepEqual(Object.keys(page).sort(), ['cursor', 'next_cursor', 'summary', 'text']);
      assert.equal(page.cursor, cursor);
      assert.equal(page.summary, h.summary);
      assert.equal(typeof page.text, 'string');
      assert.ok(new TextEncoder().encode(page.text).length <= 16384);
      assert.equal(JSON.stringify(page).includes('PRIVATE-CAPABILITY'), false);
      joined += page.text;
      pages++;
      if (page.next_cursor === null) break;
      assert.equal(page.next_cursor, cursor + 1);
      cursor = page.next_cursor;
      assert.ok(pages < 10);
    } while (true);
    assert.equal(pages, 6);
    assert.equal(joined, h.chunks.join(''));
  });

for (const suffix of ['', '&cursor=0'])
  test('admin log absent/pruned record is expired ' + suffix, async () => {
    const h = await fixture();
    await h.ledger.settle(h.record, 1000);
    await h.ledger.expire(1000 + 6 * 3600000, { logRetention: 6 * 3600000, gap: 0 });
    const response = await h.object.fetch(request('run=123-1' + suffix));
    assert.equal(response.status, 404);
    assert.deepEqual(await response.json(), {
      text: 'log expired',
      summary: '',
      cursor: 0,
      next_cursor: null,
    });
  });

for (const cursor of [1, 2])
  test('admin log rejects out-of-range cursor on empty relay ' + cursor, async () => {
    const h = await fixture();
    await h.ledger.save({ ...h.record, logChunks: 0 });
    assert.equal((await h.object.fetch(request('run=123-1&cursor=' + cursor))).status, 400);
  });

for (const size of [16385, 65536])
  test('admin log refuses oversized stored page ' + size, async () => {
    const h = await fixture();
    await h.storage.put('log:123-1:0', 'x'.repeat(size));
    assert.equal((await h.object.fetch(request())).status, 503);
  });

for (const query of [
  '',
  'run=../secret',
  'run=123-1&run=456-1',
  'run=123-1&cursor=-1',
  'run=123-1&cursor=1.5',
  'run=123-1&cursor=99999999',
  'run=123-1&extra=1',
])
  test('admin log refuses malformed query ' + query, async () => {
    const h = await fixture();
    assert.equal((await h.object.fetch(request(query))).status, 400);
  });

test('admin log rejects POST and does not invent a summary', async () => {
  const h = await fixture({ log: '[box] ordinary retained tail\n' });
  assert.equal((await h.object.fetch(request('run=123-1', undefined, 'POST'))).status, 404);
  const response = await h.object.fetch(request());
  assert.equal(response.status, 200);
  assert.equal((await response.json()).summary, '');
});

test('admin log keeps UTF-8 bytes intact and reports an empty relay', async () => {
  const h = await fixture({ log: '' });
  await h.storage.put('log:123-1:0', 'λ'.repeat(8000));
  let response = await h.object.fetch(request());
  assert.equal(response.status, 200);
  assert.equal((await response.json()).text, 'λ'.repeat(8000));
  await h.ledger.save({ ...h.record, logChunks: 0, log: '' });
  response = await h.object.fetch(request());
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { text: '', summary: '', cursor: 0, next_cursor: null });
});
