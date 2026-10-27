// Local SQLite footprint model, not a provider call or production measurement.
// FALSIFIER: the modeled per-run footprint leaves no room under the conservative Free limit.
import { createRequire } from 'node:module';
import assert from 'node:assert/strict';
const require = createRequire(new URL('../../ci/vast-broker/package.json', import.meta.url));
const { Miniflare, convertV4MiniflareOptions } = require('miniflare');
const script = `
export default {fetch() {return new Response('fixture');}};
export class StorageProbe {
  constructor(ctx) {this.ctx=ctx;}
  async fetch(request) {
    const count=Number(new URL(request.url).searchParams.get('chunks'));
    const total=4194304, key='12345678901234567890-12345678901234567890';
    let payload=0, batch={};
    for (let i=0;i<count;i++) {
      const size=Math.floor(total/count)+(i<total%count?1:0);
      batch['log:'+key+':'+i]='x'.repeat(size);
      batch['log-hash:'+key+':'+i]='0'.repeat(64);
      payload+=size;
      if (Object.keys(batch).length===128) {
        await this.ctx.storage.put(batch); batch={};
      }
    }
    if (Object.keys(batch).length) await this.ctx.storage.put(batch);
    await this.ctx.storage.put('record:'+key,{key,log:'x'.repeat(8192),logChunks:count});
    return Response.json({chunks:count,payload,sqlite_bytes:this.ctx.storage.sql.databaseSize});
  }
}`;
const mf = new Miniflare(
  convertV4MiniflareOptions({
    modules: true,
    script,
    compatibilityDate: '2026-10-23',
    durableObjects: { PROBE: { className: 'StorageProbe', useSQLite: true } },
    outboundService: () => {
      throw Error('No network permitted');
    },
  }),
);
try {
  const ns = await mf.getDurableObjectNamespace('PROBE');
  for (const chunks of [256, 2000, 131072]) {
    const stub = ns.get(ns.idFromName('chunks-' + chunks));
    const response = await stub.fetch('https://fixture.invalid/?chunks=' + chunks);
    const result = await response.json();
    assert.equal(result.payload, 4194304);
    assert.equal(result.chunks, chunks);
    console.log(JSON.stringify(result));
  }
} finally {
  await mf.dispose();
}
