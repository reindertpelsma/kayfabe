import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env } from '../test/helpers.js';

// FALSIFIER: the real DO differs from Node on origin checks, bounded paging or expiry.
const bundle = await build({
  stdin: {
    contents: `
    import worker, {BrokerLedger} from './src/index.js';
    import {Ledger} from './src/ledger.js';
    export default worker;
    export class AdminLogFixture extends BrokerLedger {
      async fetch(request) {
        if (new URL(request.url).pathname === '/seed') {
          const r = {key:'123-1', ended:0, started:0, log:'[box] tail', nonce:'PRIVATE-CAP'};
          const ledger = new Ledger(this.ctx.storage);
          for (let i=0;i<6;i++) await ledger.appendLog(r, '[box] '+i+' '+'x'.repeat(16000)+'\\n');
          await ledger.save(r);
          await ledger.put('ended:123-1', 0);
          return Response.json({ok:true});
        }
        if (new URL(request.url).pathname === '/expire') {
          await new Ledger(this.ctx.storage).expire(3600000, {logRetention:1, gap:0});
          return Response.json({ok:true});
        }
        return super.fetch(request);
      }
    }`,
    resolveDir: new URL('../', import.meta.url).pathname,
  },
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
const runtime = async (t) => {
  const mf = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: bundle.outputFiles[0].text,
      compatibilityDate: '2026-10-23',
      bindings: { ...env, RENTING_ENABLED: 'false' },
      durableObjects: { LEDGER: { className: 'AdminLogFixture', useSQLite: true } },
      outboundService: () => {
        throw Error('No outbound traffic allowed');
      },
    }),
  );
  t.after(() => mf.dispose());
  const ns = await mf.getDurableObjectNamespace('LEDGER');
  const stub = ns.get(ns.idFromName(t.name));
  const read = async (path) => {
    const r = await stub.fetch(path);
    const text = await r.text();
    return {
      status: r.status,
      data: r.headers.get('content-type')?.includes('json') ? JSON.parse(text) : { text },
    };
  };
  await read('https://test.invalid/seed');
  return { mf, read };
};

for (const cursor of [0, 1])
  test('workerd admin log bounded whitelist cursor ' + cursor, async (t) => {
    const { read } = await runtime(t);
    const response = await read('https://internal.invalid/admin/log?run=123-1&cursor=' + cursor);
    assert.equal(response.status, 200);
    assert.deepEqual(Object.keys(response.data).sort(), [
      'cursor',
      'next_cursor',
      'summary',
      'text',
    ]);
    assert.ok(new TextEncoder().encode(response.data.text).length <= 16384);
    assert.ok(response.data.text.startsWith('[box] ' + cursor));
    assert.equal(response.data.next_cursor, cursor + 1);
    assert.equal(JSON.stringify(response.data).includes('PRIVATE-CAP'), false);
  });

for (const origin of ['https://public.example', 'http://internal.invalid'])
  test('workerd admin log rejects origin ' + origin, async (t) => {
    const { mf, read } = await runtime(t);
    const response = await mf.dispatchFetch('https://broker.example/admin/log?run=123-1');
    assert.equal(response.status, 404);
    await response.text();
    const direct = await read(origin + '/admin/log?run=123-1');
    assert.equal(direct.status, 404);
  });

for (const cursor of [0, 1])
  test('workerd admin log expires after retained record deletion ' + cursor, async (t) => {
    const { read } = await runtime(t);
    await read('https://test.invalid/expire');
    const response = await read('https://internal.invalid/admin/log?run=123-1&cursor=' + cursor);
    assert.equal(response.status, 404);
    assert.equal(response.data.text, 'log expired');
  });
