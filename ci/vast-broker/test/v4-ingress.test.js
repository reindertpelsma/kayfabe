import test from 'node:test';
import assert from 'node:assert/strict';
import worker from '../src/index.js';
// FALSIFIER: an incomplete hostile upload enters the serialized DO handler,
// or the forwarded request still owns a network-backed stream at response time.
for (const route of ['report', 'kill'])
  test('V4 ' + route + ' upload waits outside ledger so cron stays available', async () => {
    let finish,
      forwarded = 0,
      cron = 0;
    const body = new ReadableStream({
      start(c) {
        finish = () => {
          c.enqueue(new TextEncoder().encode('{}'));
          c.close();
        };
      },
    });
    const request = new Request('https://broker.example/' + route + '/123-1', {
      method: 'POST',
      body,
      duplex: 'half',
    });
    const env = {
      LEDGER: {
        idFromName: (x) => x,
        get: () => ({
          async fetch(req) {
            if (new URL(req.url).pathname === '/cron') {
              cron++;
              return new Response('ok');
            }
            forwarded++;
            await req.arrayBuffer();
            return new Response('done');
          },
        }),
      },
    };
    const pending = worker.fetch(request, env);
    await new Promise((resolve) => setImmediate(resolve));
    let scheduled;
    worker.scheduled({}, env, { waitUntil: (p) => (scheduled = p) });
    await scheduled;
    try {
      assert.equal(cron, 1);
      assert.equal(forwarded, 0);
    } finally {
      finish();
      await pending;
    }
  });
for (const route of ['report', 'kill'])
  test('V4 ' + route + ' forwarded body is detached and bounded', async () => {
    let observed;
    const body = new ReadableStream({
      start(c) {
        c.enqueue(new Uint8Array(100000));
        c.close();
      },
    });
    const request = new Request('https://broker.example/' + route + '/123-1', {
      method: 'POST',
      body,
      duplex: 'half',
      headers: { 'Content-Length': '100000' },
    });
    const env = {
      LEDGER: {
        idFromName: (x) => x,
        get: () => ({
          async fetch(req) {
            observed = {
              used: request.bodyUsed,
              bytes: (await req.arrayBuffer()).byteLength,
              length: req.headers.get('Content-Length'),
            };
            return new Response('done');
          },
        }),
      },
    };
    await worker.fetch(request, env);
    assert.ok(observed);
    assert.equal(observed.bytes, (route === 'report' ? 65536 : 256) + 1);
    assert.equal(Number(observed.length), observed.bytes);
    assert.equal(observed.used, true);
  });
