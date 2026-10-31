import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, callbackToken } from './helpers.js';
import { BENCH_OBJECTS } from '../src/benchdata.js';
// FALSIFIER: a request without a live run token, for an unlisted object, from a country outside
// the allow-list, after the run ended or past the per-object limit receives a redirect; or an
// allowed request gets a lifetime above 20 minutes, a lifetime past the run's deadline, a Location
// outside the R2 endpoint, or any secret in a header or body.
const account = '0123456789abcdef0123456789abcdef',
  accessKey = 'f'.repeat(32),
  secretKey = 'e'.repeat(64),
  name = 'specviewperf-15.0.0.tar.zst',
  route = (n = name, run = '123-1') => '/bench-data/' + run + '/' + n;
async function harness(extra = {}) {
  const h = await setup({
    R2_ACCOUNT_ID: account,
    R2_BUCKET: 'kayfabe-bench-enam',
    R2_READ_ACCESS_KEY_ID: accessKey,
    R2_READ_SECRET_ACCESS_KEY: secretKey,
    ...extra,
  });
  await h.call();
  // Test clocks: the next deadline (boot limit in the fixture) must not cap the URL lifetime.
  const live = await h.storage.get('record:123-1');
  live.bootDeadline = h.time.value + 3600000;
  live.hardDeadline = h.time.value + 7200000;
  await h.storage.put('record:123-1', live);
  const token = callbackToken(live);
  return {
    h,
    token,
    async get(path = route(), { t = token, country = 'US', headers = {}, method = 'GET' } = {}) {
      return h.service.fetch(
        new Request(h.bindings.WORKER_URL + path, {
          method,
          headers: {
            ...(t === null ? {} : { Authorization: 'Bearer ' + t }),
            ...(country === null ? {} : { 'CF-IPCountry': country }),
            ...headers,
          },
        }),
      );
    },
  };
}
const everything = async (r) =>
  JSON.stringify([...r.headers]) + (await r.clone().text()) + (r.headers.get('Location') ?? '');
test('bench-data allowed request redirects to a short presigned R2 URL', async () => {
  const x = await harness();
  const r = await x.get();
  assert.equal(r.status, 302);
  assert.equal(r.headers.get('Cache-Control'), 'no-store');
  assert.equal(await r.text(), '');
  const url = new URL(r.headers.get('Location'));
  assert.equal(url.protocol, 'https:');
  assert.equal(url.host, account + '.r2.cloudflarestorage.com');
  assert.equal(url.pathname, '/kayfabe-bench-enam/specviewperf-15.0.0/specviewperf-15.0.0.tar.zst');
  const expires = Number(url.searchParams.get('X-Amz-Expires'));
  assert.ok(expires > 0 && expires <= 1200, String(expires));
  assert.equal(url.searchParams.get('X-Amz-SignedHeaders'), 'host');
  assert.match(url.searchParams.get('X-Amz-Credential'), new RegExp('^' + accessKey + '/'));
  assert.match(url.searchParams.get('X-Amz-Signature'), /^[0-9a-f]{64}$/);
  const seen = await everything(r);
  assert.ok(!seen.includes(secretKey));
  assert.ok(!seen.includes(x.token));
});
for (const [object, key] of BENCH_OBJECTS)
  test('bench-data object ' + object, async () => {
    const x = await harness();
    const r = await x.get(route(object));
    assert.equal(r.status, 302);
    assert.equal(new URL(r.headers.get('Location')).pathname, '/kayfabe-bench-enam/' + key);
  });
test('bench-data object table is exactly the four licensed-copy keys', () => {
  assert.deepEqual([...BENCH_OBJECTS.values()].sort(), [
    'specviewperf-15.0.0/CHECKSUMS.txt',
    'specviewperf-15.0.0/SPEC_General_License.pdf',
    'specviewperf-15.0.0/original/SPECviewperf-Setup-15.0.0.exe',
    'specviewperf-15.0.0/specviewperf-15.0.0.tar.zst',
  ]);
});
test('bench-data lifetime is capped at 1200 s and at the run deadline', async () => {
  const x = await harness();
  const lifetime = async () =>
    Number(
      new URL(
        (await x.get(route('specviewperf-15.0.0.CHECKSUMS.txt'))).headers.get('Location'),
      ).searchParams.get('X-Amz-Expires'),
    );
  assert.equal(await lifetime(), 1200);
  const r = await x.h.storage.get('record:123-1');
  // 90 s left to the next deadline: the URL may not outlive the run.
  r.hardDeadline = x.h.time.value + 90000;
  await x.h.storage.put('record:123-1', r);
  const left = await lifetime();
  assert.ok(left <= 90 && left >= 30, String(left));
});
test('bench-data refuses a URL that would live under 30 s', async () => {
  const x = await harness();
  const r = await x.h.storage.get('record:123-1');
  r.hardDeadline = x.h.time.value + 20000;
  await x.h.storage.put('record:123-1', r);
  const res = await x.get();
  assert.equal(res.status, 401);
  assert.equal(res.headers.get('Location'), null);
});
for (const mode of ['missing', 'bad', 'wrong-run', 'other-run-path', 'closed', 'ended', 'expired'])
  test('bench-data run token required and run live: ' + mode, async () => {
    const x = await harness();
    let t = x.token,
      path = route();
    if (mode === 'missing') t = null;
    if (mode === 'bad') t = 'bad';
    if (mode === 'wrong-run')
      t = callbackToken(await x.h.storage.get('record:123-1'), undefined, { run: '124' });
    if (mode === 'other-run-path') path = route(name, '124-1');
    if (['closed', 'ended', 'expired'].includes(mode)) {
      const r = await x.h.storage.get('record:123-1');
      if (mode === 'closed') r.closed = 'STOPPED';
      else if (mode === 'ended') r.ended = x.h.time.value;
      else x.h.time.value = r.hardDeadline;
      await x.h.storage.put('record:123-1', r);
    }
    const res = await x.get(path, { t });
    assert.ok([401, 403].includes(res.status), String(res.status));
    assert.equal(res.headers.get('Location'), null);
  });
test('bench-data requires the provider to report the instance running', async () => {
  const x = await harness();
  x.h.cloud.instances.get('900').actual_status = 'exited';
  const r = await x.get();
  assert.equal(r.status, 401);
  assert.equal(r.headers.get('Location'), null);
});
for (const path of [
  '/bench-data/123-1/unknown.bin',
  '/bench-data/123-1/specviewperf-15.0.0.tar.zst.sig',
  '/bench-data/123-1/..',
  '/bench-data/123-1/.',
  '/bench-data/123-1/__proto__',
  '/bench-data/123-1/constructor',
  '/bench-data/123-1/',
  '/bench-data/123-1',
  '/bench-data/123-1/%2e%2e%2fsecret',
  '/bench-data/123-1/specviewperf-15.0.0%2ftar.zst',
  '/bench-data/123-1/specviewperf-15.0.0/CHECKSUMS.txt',
  '/bench-data/123-1/specviewperf-15.0.0.tar.zst?list=1',
  '/bench-data/123-1/specviewperf-15.0.0.tar.zst?x-id=ListObjectsV2',
]) {
  test('bench-data unknown or malformed object is refused: ' + path, async () => {
    const x = await harness();
    const r = await x.get(path);
    assert.ok([400, 404].includes(r.status), String(r.status));
    assert.equal(r.headers.get('Location'), null);
  });
}
test('bench-data unknown name with a valid token is 404, not a listing', async () => {
  const x = await harness();
  assert.equal((await x.get('/bench-data/123-1/unknown.bin')).status, 404);
  assert.equal((await x.get('/bench-data/123-1/specviewperf-15.0.0')).status, 404);
});
test('bench-data accepts GET only', async () => {
  const x = await harness();
  for (const method of ['POST', 'PUT', 'DELETE'])
    assert.notEqual((await x.get(route(), { method })).status, 302);
});
for (const country of ['DE', 'CN', 'RU', 'XX', 'T1', 'us', 'USA', '', null])
  test('bench-data country not allowed: ' + JSON.stringify(country), async () => {
    const x = await harness();
    const r = await x.get(route(), { country });
    assert.equal(r.status, 403);
    assert.equal(r.headers.get('Location'), null);
    const body = await r.json();
    assert.equal(body.reason, 'BENCH_DATA_COUNTRY_NOT_ALLOWED');
  });
for (const country of ['US', 'CA'])
  test('bench-data default allow-list admits ' + country, async () => {
    const x = await harness();
    assert.equal((await x.get(route(), { country })).status, 302);
  });
test('bench-data allow-list is the owner var', async () => {
  const x = await harness({ BENCH_DATA_ALLOWED_COUNTRIES: 'US' });
  assert.equal((await x.get(route(), { country: 'CA' })).status, 403);
  assert.equal((await x.get(route(), { country: 'US' })).status, 302);
  const y = await harness({ BENCH_DATA_ALLOWED_COUNTRIES: 'US,DE' });
  assert.equal((await y.get(route(), { country: 'DE' })).status, 302);
});
for (const bad of ['us', 'US;CA', 'US,', ',US', '["US"]', 'USA', ''])
  test('bench-data malformed allow-list fails closed: ' + JSON.stringify(bad), async () => {
    const x = await harness({ BENCH_DATA_ALLOWED_COUNTRIES: bad });
    const r = await x.get(route(), { country: 'US' });
    assert.equal(r.status, 503);
    assert.equal(r.headers.get('Location'), null);
  });
test('bench-data denied countries still win over the allow-list', async () => {
  const x = await harness({
    BENCH_DATA_ALLOWED_COUNTRIES: 'US,CN',
    DENIED_COUNTRIES: '["CN"]',
  });
  assert.equal((await x.get(route(), { country: 'CN' })).status, 403);
  assert.equal((await x.get(route(), { country: 'US' })).status, 302);
});
test('bench-data offer country on the record must be allowed too', async () => {
  const x = await harness();
  const r = await x.h.storage.get('record:123-1');
  r.country = 'DE';
  await x.h.storage.put('record:123-1', r);
  assert.equal((await x.get(route(), { country: 'US' })).status, 403);
  r.country = 'UNKNOWN';
  await x.h.storage.put('record:123-1', r);
  assert.equal((await x.get(route(), { country: 'US' })).status, 302);
});
test('bench-data at most four URLs per object per run, counted per object', async () => {
  const x = await harness();
  for (let i = 0; i < 4; i++) assert.equal((await x.get()).status, 302, 'mint ' + i);
  const fifth = await x.get();
  assert.equal(fifth.status, 429);
  assert.equal(fifth.headers.get('Location'), null);
  // Another object has its own count.
  assert.equal((await x.get(route('specviewperf-15.0.0.CHECKSUMS.txt'))).status, 302);
  x.h.restart();
  assert.equal((await x.get()).status, 429, 'the count is kept in the ledger record');
});
test('bench-data refused requests do not consume the per-object limit', async () => {
  const x = await harness();
  for (let i = 0; i < 6; i++) await x.get(route(), { country: 'DE' });
  for (let i = 0; i < 4; i++) {
    x.h.time.value += 11000; // stay under the generic 20 requests / 10 s rate limit
    assert.equal((await x.get()).status, 302);
  }
});
test('bench-data total per run is bounded by objects times four', async () => {
  const x = await harness();
  let minted = 0;
  for (let round = 0; round < 6; round++)
    for (const object of BENCH_OBJECTS.keys()) {
      x.h.time.value += 11000; // stay clear of the generic 20 requests / 10 s window
      if ((await x.get(route(object))).status === 302) minted++;
    }
  assert.equal(minted, BENCH_OBJECTS.size * 4);
});
test('bench-data without R2 settings fails closed with no redirect', async () => {
  for (const missing of [
    'R2_ACCOUNT_ID',
    'R2_BUCKET',
    'R2_READ_ACCESS_KEY_ID',
    'R2_READ_SECRET_ACCESS_KEY',
  ]) {
    const x = await harness({ [missing]: '' });
    const r = await x.get();
    assert.equal(r.status, 503, missing);
    assert.equal(r.headers.get('Location'), null);
    const seen = await everything(r);
    assert.ok(!seen.includes(secretKey) && !seen.includes(accessKey));
  }
});
test('bench-data logs run, object, country and expiry without any secret', async () => {
  const x = await harness();
  const lines = [],
    real = console.log;
  console.log = (...a) => lines.push(a.join(' '));
  try {
    assert.equal((await x.get()).status, 302);
  } finally {
    console.log = real;
  }
  const event = lines.map((l) => JSON.parse(l)).find((e) => e.event === 'bench_data_url');
  assert.deepEqual(Object.keys(event).sort(), ['country', 'event', 'expires', 'object', 'run']);
  assert.equal(event.run, '123-1');
  assert.equal(event.object, name);
  assert.equal(event.country, 'US');
  assert.equal(event.expires, Math.floor(x.h.time.value / 1000) + 1200);
  const all = lines.join('\n');
  for (const s of [secretKey, accessKey, x.token, 'X-Amz-Signature'])
    assert.ok(!all.includes(s), s);
});
test('bench-data URL is not stored in the ledger', async () => {
  const x = await harness();
  const r = await x.get();
  const url = r.headers.get('Location');
  const stored = JSON.stringify([...x.h.storage.data]);
  assert.ok(!stored.includes(new URL(url).searchParams.get('X-Amz-Signature')));
  assert.ok(!stored.includes(secretKey));
});
test('bench-data public entrypoint forwards the country and the exact route', async () => {
  const { default: worker } = await import('../src/index.js');
  const x = await harness();
  const prefix = 'x'.repeat(40);
  x.h.bindings.BROKER_PATH_PREFIX = prefix;
  let forwarded = null;
  const bindings = {
    ...x.h.bindings,
    LEDGER: {
      idFromName: () => 1,
      get: () => ({
        fetch: async (req) => {
          forwarded = req.headers.get('CF-IPCountry');
          const u = new URL(req.url);
          u.pathname = u.pathname.slice(prefix.length + 1);
          return x.h.service.fetch(new Request(u, req));
        },
      }),
    },
  };
  const ask = (path, t = x.token, country = 'US') =>
    worker.fetch(
      new Request(x.h.bindings.WORKER_URL + '/' + prefix + path, {
        headers: { Authorization: 'Bearer ' + t, 'CF-IPCountry': country },
      }),
      bindings,
    );
  const ok = await ask(route());
  assert.equal(ok.status, 302);
  assert.equal(forwarded, 'US');
  assert.equal((await ask(route(), x.token, 'DE')).status, 403);
  forwarded = null;
  // A token for another run is stopped at the front door and never reaches the ledger.
  const other = callbackToken(await x.h.storage.get('record:123-1'), undefined, { run: '124' });
  assert.equal((await ask(route(), other)).status, 401);
  assert.equal(forwarded, null);
  assert.equal((await ask(route(), 'bad')).status, 401);
  assert.equal((await ask('/bench-data/123-1/unknown.bin')).status, 404);
});
