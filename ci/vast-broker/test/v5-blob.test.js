import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { setup, callbackToken } from './helpers.js';
import { sanitizeLog } from '../src/sanitize.js';
const route = '/blob-url/123-1/dummy-blobs-20261022/blob-a.txt',
  signed =
    'https://release-assets.githubusercontent.com/github-production-release-asset/123/blob?sig=private-short&se=2026-10-23';
// FALSIFIER: a closed/wrong-run bearer mints, GitHub bytes/redirect chains are
// consumed, an unsafe locator escapes, or a signed URL reaches retained logs.
async function harness(mode = 'ok', extra = {}) {
  const h = await setup({
    BLOB_REPO: 'owner/private-store',
    BLOB_ALLOWED_TAG_PREFIXES: '["dummy-blobs-"]',
    BLOB_GITHUB_TOKEN: 'mock-blob-read-secret',
    ...extra,
  });
  await h.call();
  const base = h.cloud.fetch;
  const calls = [];
  let reads = 0,
    cancels = 0;
  h.cloud.fetch = async (url, init = {}) => {
    if (!String(url).includes('/repos/owner/private-store/')) return base(url, init);
    calls.push({ url, init });
    if (String(url).includes('/releases/tags/')) {
      if (mode === 'tag404') return new Response('', { status: 404 });
      if (mode === 'huge') return new Response('x'.repeat(262145));
      return Response.json({
        assets:
          mode === 'missing'
            ? []
            : [
                { id: 17, name: 'blob-a.txt', size: 49 },
                { id: 18, name: 'blob-b.txt', size: 270177 },
              ],
      });
    }
    const headers = {
      Location:
        mode === 'evil'
          ? 'https://evil.example/p?secret=x'
          : mode === 'evil-suffix'
            ? 'https://release-assets.githubusercontent.com.evil.example/p'
            : mode === 'http'
              ? signed.replace('https:', 'http:')
              : mode === 'userinfo'
                ? signed.replace('https://', 'https://user@')
                : mode === 'port'
                  ? signed.replace('.com/', '.com:444/')
                  : mode === 'no-location'
                    ? ''
                    : signed,
    };
    const response = {
      status: mode === '200' ? 200 : mode === '307' ? 307 : 302,
      redirected: mode === 'chain',
      headers: new Headers(headers),
      body: {
        cancel: async () => {
          cancels++;
        },
      },
      getReader() {
        reads++;
        throw Error('BINARY_READ');
      },
      arrayBuffer() {
        reads++;
        throw Error('BINARY_READ');
      },
      text() {
        reads++;
        throw Error('BINARY_READ');
      },
    };
    return response;
  };
  h.restart();
  const token = callbackToken(await h.storage.get('record:123-1'));
  return {
    h,
    calls,
    token,
    get reads() {
      return reads;
    },
    get cancels() {
      return cancels;
    },
    async get(path = route, t = token) {
      return h.service.fetch(
        new Request(h.bindings.WORKER_URL + path, { headers: { Authorization: 'Bearer ' + t } }),
      );
    },
  };
}
for (const name of ['blob-a.txt', 'blob-b.txt'])
  test('blob exact asset resolution without binary read ' + name, async () => {
    const x = await harness();
    const r = await x.get(route.replace('blob-a.txt', name));
    assert.equal(r.status, 200);
    const d = await r.json();
    assert.deepEqual(Object.keys(d).sort(), ['name', 'size', 'url']);
    assert.equal(d.name, name);
    assert.equal(d.size, name === 'blob-a.txt' ? 49 : 270177);
    assert.equal(d.url, signed);
    assert.equal(x.reads, 0);
    assert.equal(x.cancels, 1);
    assert.equal(x.calls.length, 2);
    assert.ok(x.calls[1].url.endsWith('/' + (name === 'blob-a.txt' ? 17 : 18)));
    assert.equal(x.calls[1].init.redirect, 'manual');
    assert.equal(x.calls[1].init.headers.Accept, 'application/octet-stream');
    assert.equal(x.calls[1].init.headers.Authorization, 'Bearer mock-blob-read-secret');
    assert.ok(!JSON.stringify([...x.h.storage.data]).includes(signed));
  });
for (const mode of [
  '200',
  '307',
  'chain',
  'evil',
  'evil-suffix',
  'http',
  'userinfo',
  'port',
  'no-location',
  'tag404',
  'missing',
  'huge',
])
  test('blob refuses hostile provider ' + mode, async () => {
    const x = await harness(mode);
    assert.notEqual((await x.get()).status, 200);
    assert.equal(x.reads, 0);
    if (!['tag404', 'missing', 'huge'].includes(mode)) assert.equal(x.cancels, 1);
  });
for (const mode of [
  'wrong-run',
  'bad-token',
  'closed',
  'expired',
  'missing-token',
  'missing-repo',
  'prefix',
])
  test('blob capability/config fail closed ' + mode, async () => {
    const extra =
      mode === 'missing-token'
        ? { BLOB_GITHUB_TOKEN: '' }
        : mode === 'missing-repo'
          ? { BLOB_REPO: '' }
          : mode === 'prefix'
            ? { BLOB_ALLOWED_TAG_PREFIXES: '["img-"]' }
            : {};
    const x = await harness('ok', extra);
    let t = x.token;
    if (mode === 'wrong-run')
      t = callbackToken(await x.h.storage.get('record:123-1'), undefined, { run: '124' });
    if (mode === 'bad-token') t = 'bad';
    if (['closed', 'expired'].includes(mode)) {
      const r = await x.h.storage.get('record:123-1');
      if (mode === 'closed') r.closed = 'STOPPED';
      else x.h.time.value = r.hardDeadline;
      await x.h.storage.put('record:123-1', r);
    }
    assert.notEqual((await x.get(route, t)).status, 200);
    assert.equal(x.calls.length, 0);
  });
for (const path of ['../blob', '%2f', 'b%2Fa', 'caf%C3%A9', 'a?x=1', '.', '..'])
  test('blob name path refused ' + path, async () => {
    const x = await harness();
    assert.notEqual((await x.get(route.replace('blob-a.txt', path))).status, 200);
    assert.equal(x.calls.length, 0);
  });
for (const limit of ['run', 'day'])
  test('blob bounded mint attempts ' + limit, async () => {
    const x = await harness(
      'ok',
      limit === 'day' ? { BLOB_DAILY_MINTS: '2' } : { BLOB_RUN_MINTS: '2' },
    );
    for (let i = 0; i < 2; i++) assert.equal((await x.get()).status, 200);
    assert.equal((await x.get()).status, 429);
    assert.equal(x.calls.length, 4);
  });
test('blob remains available in unexpired result grace', async () => {
  const x = await harness();
  const r = await x.h.storage.get('record:123-1');
  r.resultAt = x.h.time.value;
  r.resultDeadline = x.h.time.value + 60000;
  r.phase = 'GRACE';
  await x.h.storage.put('record:123-1', r);
  assert.equal((await x.get()).status, 200);
});
for (const host of ['release-assets.githubusercontent.com', 'objects.githubusercontent.com'])
  test('blob signed locator independently scrubbed ' + host, () => {
    const url = signed.replace('release-assets.githubusercontent.com', host);
    assert.ok(!sanitizeLog(new TextEncoder().encode(url)).includes('private-short'));
    const r = spawnSync(
      'python3',
      [
        '-I',
        '-c',
        "import importlib.util,sys;s=importlib.util.spec_from_file_location('runner',sys.argv[1]);m=importlib.util.module_from_spec(s);sys.modules['runner']=m;s.loader.exec_module(m);assert 'private-short' not in m.scrub(sys.argv[2])[0]",
        new URL('../runner.py', import.meta.url).pathname,
        url,
      ],
      { encoding: 'utf8' },
    );
    assert.equal(r.status, 0, r.stderr);
  });

for (const prefix of ['x'.repeat(32), 'y'.repeat(48)])
  test('blob public entrypoint forwards exact route ' + prefix.length, async () => {
    const { default: worker } = await import('../src/index.js');
    const x = await harness('ok', {});
    x.h.bindings.BROKER_PATH_PREFIX = prefix;
    let forwarded = false;
    const bindings = {
      ...x.h.bindings,
      LEDGER: {
        idFromName: () => 1,
        get: () => ({
          fetch: async (req) => {
            forwarded = true;
            const u = new URL(req.url);
            u.pathname = u.pathname.slice(prefix.length + 1);
            return x.h.service.fetch(new Request(u, req));
          },
        }),
      },
    };
    const r = await worker.fetch(
      new Request(x.h.bindings.WORKER_URL + (prefix ? '/' + prefix : '') + route, {
        headers: { Authorization: 'Bearer ' + x.token },
      }),
      bindings,
    );
    assert.equal(r.status, 200);
    assert.ok(forwarded);
  });
